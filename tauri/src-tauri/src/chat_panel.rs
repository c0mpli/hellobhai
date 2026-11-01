//! The typed side chat (`ui/chat.*`), port of the Mac's `ChatPanelManager`: a
//! 380 px card that slides in from the right or left edge of the screen under
//! the mouse (full work-area height, 12 px inset), minimizes to a 30x112 tab
//! flush with the edge (unread dot once reply text arrives), hands keyboard
//! focus back to the user's app before [TYPE]/[KEY] and on minimize, and lets
//! clicks pass through while bhai acts on screen. Also owns the chat rows and
//! the bhai mode (Voice / Chat), so the brain and the settings page share one
//! place for them.
//!
//! Tauri can't animate a window frame, so a task steps the position and size at
//! ~60 Hz with the Mac's timing curves; the page fades its own content in and
//! out (the window is transparent), standing in for the Mac's alpha animation.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tauri::async_runtime::JoinHandle;
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, State, WebviewWindow, WindowEvent};

use crate::app_state::AppState;
use crate::brain_engine::BrainEngine;
use crate::chat_conversation::{
    chat_model_label, find_spoken_sentence_byte_range, utf16_offset_for_byte_offset, ChatConversation, ChatMessage, ChatTabSummary, ChatTabs,
};
use crate::page_bridge::{locked_down_window_builder, monitor_containing_point};
use crate::platform::{self, PlatformError};
use crate::screen_mapping::{monitors_from_tauri, MonitorDescription, PhysicalRect};
use crate::settings::SharedSettings;
use crate::ui_commands::save_and_announce_settings;

pub const CHAT_WINDOW_LABEL: &str = "chat";

const PANEL_LOGICAL_WIDTH: f64 = 380.0;
const GAP_FROM_SCREEN_EDGES_LOGICAL: f64 = 12.0;
const MINIMIZED_TAB_LOGICAL_WIDTH: f64 = 30.0;
const MINIMIZED_TAB_LOGICAL_HEIGHT: f64 = 112.0;
/// How far past its spot the card starts (or ends) its slide, as a share of its width.
const SLIDE_DISTANCE_FRACTION_OF_WIDTH: f64 = 0.35;

const SHOW_ANIMATION_DURATION: Duration = Duration::from_millis(280);
const HIDE_ANIMATION_DURATION: Duration = Duration::from_millis(220);
const MOVE_ANIMATION_DURATION: Duration = Duration::from_millis(260);
const ANIMATION_FRAME_INTERVAL: Duration = Duration::from_millis(16);

/// Which screen edge the chat slides in from (Mac `ChatPanelSide`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum ChatPanelSide {
    Left,
    #[default]
    Right,
}

impl ChatPanelSide {
    pub fn opposite(self) -> ChatPanelSide {
        match self {
            ChatPanelSide::Left => ChatPanelSide::Right,
            ChatPanelSide::Right => ChatPanelSide::Left,
        }
    }
}

/// The Mac's two timing curves.
#[derive(Clone, Copy, Debug)]
enum AnimationCurve {
    /// `CAMediaTimingFunction(controlPoints: 0.2, 0.9, 0.3, 1)`: fast start, soft landing.
    SoftLanding,
    /// `.easeIn` (0.42, 0, 1, 1), for sliding away.
    EaseIn,
}

impl AnimationCurve {
    fn progress(self, time_fraction: f64) -> f64 {
        match self {
            AnimationCurve::SoftLanding => cubic_bezier_progress(0.2, 0.9, 0.3, 1.0, time_fraction),
            AnimationCurve::EaseIn => cubic_bezier_progress(0.42, 0.0, 1.0, 1.0, time_fraction),
        }
    }
}

/// A CSS/Core Animation cubic-bezier timing curve: finds the curve parameter whose
/// x is `time_fraction` (by bisection, x is monotonic) and returns its y.
pub fn cubic_bezier_progress(control_x1: f64, control_y1: f64, control_x2: f64, control_y2: f64, time_fraction: f64) -> f64 {
    let time_fraction = time_fraction.clamp(0.0, 1.0);
    let bezier_component = |first_control: f64, second_control: f64, curve_parameter: f64| {
        let inverse = 1.0 - curve_parameter;
        3.0 * inverse * inverse * curve_parameter * first_control
            + 3.0 * inverse * curve_parameter * curve_parameter * second_control
            + curve_parameter * curve_parameter * curve_parameter
    };
    let mut lower_parameter = 0.0;
    let mut upper_parameter = 1.0;
    for _ in 0..40 {
        let middle_parameter = (lower_parameter + upper_parameter) / 2.0;
        if bezier_component(control_x1, control_x2, middle_parameter) < time_fraction {
            lower_parameter = middle_parameter;
        } else {
            upper_parameter = middle_parameter;
        }
    }
    bezier_component(control_y1, control_y2, (lower_parameter + upper_parameter) / 2.0)
}

fn logical_to_physical(logical_length: f64, scale_factor: f64) -> i32 {
    (logical_length * scale_factor).round() as i32
}

/// Full height of the work area (above the taskbar), inset from the chosen edge
/// so it reads as a floating card. Physical pixels.
pub fn on_screen_frame(work_area: PhysicalRect, scale_factor: f64, panel_side: ChatPanelSide) -> PhysicalRect {
    let gap = logical_to_physical(GAP_FROM_SCREEN_EDGES_LOGICAL, scale_factor);
    let panel_width = logical_to_physical(PANEL_LOGICAL_WIDTH, scale_factor);
    let panel_height = (work_area.height as i32 - gap * 2).max(0);
    let origin_x = match panel_side {
        ChatPanelSide::Right => work_area.x + work_area.width as i32 - panel_width - gap,
        ChatPanelSide::Left => work_area.x + gap,
    };
    PhysicalRect { x: origin_x, y: work_area.y + gap, width: panel_width as u32, height: panel_height as u32 }
}

/// Flush with the screen edge so it reads as the chat tucked away there, centred
/// on where the chat was and kept inside the work area.
pub fn minimized_tab_frame(work_area: PhysicalRect, scale_factor: f64, panel_side: ChatPanelSide, center_y: i32) -> PhysicalRect {
    let tab_width = logical_to_physical(MINIMIZED_TAB_LOGICAL_WIDTH, scale_factor);
    let tab_height = logical_to_physical(MINIMIZED_TAB_LOGICAL_HEIGHT, scale_factor);
    let origin_x = match panel_side {
        ChatPanelSide::Right => work_area.x + work_area.width as i32 - tab_width,
        ChatPanelSide::Left => work_area.x,
    };
    let lowest_origin_y = work_area.y + work_area.height as i32 - tab_height;
    let origin_y = (center_y - tab_height / 2).min(lowest_origin_y).max(work_area.y);
    PhysicalRect { x: origin_x, y: origin_y, width: tab_width as u32, height: tab_height as u32 }
}

/// Where the card slides in from and out to: shifted outward by a third of its width.
pub fn off_screen_frame(on_screen_frame: PhysicalRect, scale_factor: f64, panel_side: ChatPanelSide) -> PhysicalRect {
    let slide_distance = logical_to_physical(PANEL_LOGICAL_WIDTH * SLIDE_DISTANCE_FRACTION_OF_WIDTH, scale_factor);
    let horizontal_offset = match panel_side {
        ChatPanelSide::Right => slide_distance,
        ChatPanelSide::Left => -slide_distance,
    };
    PhysicalRect { x: on_screen_frame.x + horizontal_offset, ..on_screen_frame }
}

pub fn interpolated_frame(start_frame: PhysicalRect, end_frame: PhysicalRect, progress: f64) -> PhysicalRect {
    let blend = |start_value: f64, end_value: f64| start_value + (end_value - start_value) * progress;
    PhysicalRect {
        x: blend(start_frame.x as f64, end_frame.x as f64).round() as i32,
        y: blend(start_frame.y as f64, end_frame.y as f64).round() as i32,
        width: blend(start_frame.width as f64, end_frame.width as f64).round().max(1.0) as u32,
        height: blend(start_frame.height as f64, end_frame.height as f64).round().max(1.0) as u32,
    }
}

/// Pushed to the chat page as `chat-state` after every change.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatStateSnapshot {
    pub messages: Vec<ChatMessage>,
    pub is_minimized: bool,
    pub has_unread_reply: bool,
    /// False while sliding away, so the page fades out.
    pub is_shown: bool,
    pub chat_panel_side: ChatPanelSide,
    pub model_label: String,
    /// The chat's model and effort menus show the shared settings.
    pub brain_engine: BrainEngine,
    pub claude_model: String,
    pub reasoning_effort: Option<String>,
    pub speaks_chat_replies: bool,
    /// The strip under the header, in order.
    pub tabs: Vec<ChatTabSummary>,
    pub active_chat_tab_id: u64,
    /// Font size in px for the messages and the composer.
    pub chat_text_point_size: u32,
    pub speech_progress: Option<ChatSpeechProgress>,
}

/// The sentence of a chat reply being spoken right now, in UTF-16 offsets into
/// its text (how the page's strings count), so the page can light it up.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatSpeechProgress {
    pub message_id: u64,
    pub sentence_start: usize,
    pub sentence_end: usize,
}

struct ChatPanelState {
    /// The active tab's rows; the other tabs' rows are parked in `chat_tabs`.
    conversation: ChatConversation,
    chat_tabs: ChatTabs,
    /// The chat reply being read out, and how far into its text (bytes) the
    /// sentence search has got, so a repeated sentence matches the right occurrence.
    spoken_chat_message_id: Option<u64>,
    spoken_message_search_start_byte_offset: usize,
    chat_speech_progress: Option<ChatSpeechProgress>,
    is_minimized: bool,
    has_unread_reply: bool,
    /// Set while the slide-away runs, so a show during it cancels the pending hide.
    is_closing: bool,
    /// The monitor the chat is on (the one under the mouse when it opened).
    current_monitor: Option<MonitorDescription>,
    /// The window that had the keyboard before the chat took it.
    user_app_window_id: Option<isize>,
    frame_animation_task: Option<JoinHandle<()>>,
}

pub struct ChatPanel {
    app: AppHandle,
    settings: SharedSettings,
    state: Mutex<ChatPanelState>,
}

impl ChatPanel {
    pub fn new(app: AppHandle, settings: SharedSettings) -> Arc<Self> {
        Arc::new(ChatPanel {
            app,
            settings,
            state: Mutex::new(ChatPanelState {
                conversation: ChatConversation::default(),
                chat_tabs: ChatTabs::default(),
                spoken_chat_message_id: None,
                spoken_message_search_start_byte_offset: 0,
                chat_speech_progress: None,
                is_minimized: false,
                has_unread_reply: false,
                is_closing: false,
                current_monitor: None,
                user_app_window_id: None,
                frame_animation_task: None,
            }),
        })
    }

    fn chat_window(&self) -> Option<WebviewWindow> {
        self.app.get_webview_window(CHAT_WINDOW_LABEL)
    }

    fn panel_side(&self) -> ChatPanelSide {
        self.settings.read().unwrap().chat_panel_side
    }

    pub fn is_visible(&self) -> bool {
        self.chat_window().is_some_and(|chat_window| chat_window.is_visible().unwrap_or(false))
    }

    pub fn is_open_and_focused(&self) -> bool {
        let Some(chat_window) = self.chat_window() else { return false };
        chat_window.is_visible().unwrap_or(false)
            && chat_window.is_focused().unwrap_or(false)
            && !self.state.lock().unwrap().is_minimized
    }

    pub fn chat_state_snapshot(&self) -> ChatStateSnapshot {
        let settings = self.settings.read().unwrap().clone();
        let state = self.state.lock().unwrap();
        ChatStateSnapshot {
            messages: state.conversation.messages().to_vec(),
            is_minimized: state.is_minimized,
            has_unread_reply: state.has_unread_reply,
            is_shown: !state.is_closing,
            chat_panel_side: settings.chat_panel_side,
            model_label: chat_model_label(settings.brain_engine, &settings.claude_model),
            brain_engine: settings.brain_engine,
            claude_model: settings.claude_model,
            reasoning_effort: settings.reasoning_effort,
            speaks_chat_replies: settings.speaks_chat_replies,
            tabs: state.chat_tabs.summaries(&state.conversation),
            active_chat_tab_id: state.chat_tabs.active_tab_id(),
            chat_text_point_size: settings.chat_text_size.point_size(),
            speech_progress: state.chat_speech_progress,
        }
    }

    pub fn emit_chat_state(&self) {
        let _ = self.app.emit_to(CHAT_WINDOW_LABEL, "chat-state", self.chat_state_snapshot());
    }

    // MARK: Messages (called by the brain)

    /// Adds a row and returns its id.
    pub fn append_message(&self, is_from_user: bool, text: String, is_streaming: bool) -> u64 {
        let message_id = self.state.lock().unwrap().conversation.append(is_from_user, text, is_streaming);
        self.mark_unread_if_reply_arrived_while_minimized();
        self.emit_chat_state();
        message_id
    }

    pub fn update_message(&self, message_id: u64, text: String, is_streaming: bool) {
        self.state.lock().unwrap().conversation.update(message_id, text, is_streaming);
        self.mark_unread_if_reply_arrived_while_minimized();
        self.emit_chat_state();
    }

    pub fn remove_message(&self, message_id: u64) {
        self.state.lock().unwrap().conversation.remove(message_id);
        self.emit_chat_state();
    }

    pub fn finish_streaming_messages(&self, message_ids: &[u64]) {
        {
            let mut state = self.state.lock().unwrap();
            state.conversation.finish_streaming(message_ids);
            state.chat_tabs.finish_streaming_in_parked_tabs(message_ids);
        }
        self.emit_chat_state();
    }

    // MARK: Tabs (the brain decides; it owns each tab's history)

    /// Runs a change on the tabs and the active tab's rows, then redraws the page.
    pub fn change_chat_tabs<ChangeResult>(&self, change: impl FnOnce(&mut ChatTabs, &mut ChatConversation) -> ChangeResult) -> ChangeResult {
        let change_result = {
            let mut state = self.state.lock().unwrap();
            let state = &mut *state;
            change(&mut state.chat_tabs, &mut state.conversation)
        };
        self.emit_chat_state();
        change_result
    }

    // MARK: Following speech (Mac `speakChatReply` / `advanceChatSpeechProgress`)

    /// Called right after a chat reply starts being spoken.
    pub fn begin_speaking_chat_reply(&self, message_id: u64) {
        {
            let mut state = self.state.lock().unwrap();
            state.spoken_chat_message_id = Some(message_id);
            state.spoken_message_search_start_byte_offset = 0;
            state.chat_speech_progress = None;
        }
        self.emit_chat_state();
    }

    /// A sentence's audio just started: light it up in the reply being spoken.
    pub fn advance_chat_speech_progress(&self, spoken_sentence: &str) {
        {
            let mut state = self.state.lock().unwrap();
            let Some(spoken_chat_message_id) = state.spoken_chat_message_id else { return };
            let Some(message_text) = state.conversation.message_text(spoken_chat_message_id) else { return };
            let Some(sentence_byte_range) =
                find_spoken_sentence_byte_range(message_text, spoken_sentence, state.spoken_message_search_start_byte_offset)
            else {
                return;
            };
            let chat_speech_progress = ChatSpeechProgress {
                message_id: spoken_chat_message_id,
                sentence_start: utf16_offset_for_byte_offset(message_text, sentence_byte_range.start),
                sentence_end: utf16_offset_for_byte_offset(message_text, sentence_byte_range.end),
            };
            state.spoken_message_search_start_byte_offset = sentence_byte_range.end;
            state.chat_speech_progress = Some(chat_speech_progress);
        }
        self.emit_chat_state();
    }

    /// Speech stopped or finished: the reply reads normally again.
    pub fn clear_chat_speech_progress(&self) {
        {
            let mut state = self.state.lock().unwrap();
            if state.spoken_chat_message_id.is_none() && state.chat_speech_progress.is_none() {
                return;
            }
            state.spoken_chat_message_id = None;
            state.chat_speech_progress = None;
        }
        self.emit_chat_state();
    }

    /// The empty placeholder row added while a reply is on its way doesn't count.
    fn mark_unread_if_reply_arrived_while_minimized(&self) {
        let mut state = self.state.lock().unwrap();
        let last_message_is_reply_text = state
            .conversation
            .messages()
            .last()
            .is_some_and(|last_message| !last_message.is_from_user && !last_message.text.is_empty());
        if state.is_minimized && last_message_is_reply_text {
            state.has_unread_reply = true;
        }
    }

    // MARK: bhai mode and chat settings

    /// Voice shows the orb and puts the chat away; Chat shows the chat sidebar
    /// instead of the orb.
    pub fn set_chat_input_mode(&self, is_chat_input_mode: bool) {
        self.save_setting(|settings| settings.is_chat_input_mode = is_chat_input_mode);
        self.apply_chat_input_mode(is_chat_input_mode);
    }

    /// Shows the chat or the orb for a bhai mode that is already saved (the X,
    /// or the settings page's General tab through `change_setting`).
    pub fn apply_chat_input_mode(&self, is_chat_input_mode: bool) {
        self.hide_orb_for_chat_mode(is_chat_input_mode);
        if is_chat_input_mode {
            self.show();
        } else {
            self.hide();
        }
    }

    fn hide_orb_for_chat_mode(&self, is_chat_input_mode: bool) {
        if let Some(app_state) = self.app.try_state::<AppState>() {
            app_state.orb_window.set_hidden_for_chat_mode(is_chat_input_mode);
        }
    }

    /// The chat side changed in the settings page: follow it.
    pub fn apply_chat_panel_side(&self) {
        self.emit_chat_state();
        self.move_to_current_side();
    }

    /// Opens the chat. Usually that also switches to chat mode; a spoken question
    /// with a text-only reply opens it without leaving voice mode.
    pub fn show_chat_panel(&self, switches_to_chat_mode: bool) {
        let is_chat_input_mode = self.settings.read().unwrap().is_chat_input_mode;
        if switches_to_chat_mode && !is_chat_input_mode {
            self.save_setting(|settings| settings.is_chat_input_mode = true);
            self.hide_orb_for_chat_mode(true);
        }
        self.show();
    }

    pub fn set_speaks_chat_replies(&self, speaks_chat_replies: bool) {
        self.save_setting(|settings| settings.speaks_chat_replies = speaks_chat_replies);
        self.emit_chat_state();
    }

    pub fn set_chat_panel_side(&self, chat_panel_side: ChatPanelSide) {
        self.save_setting(|settings| settings.chat_panel_side = chat_panel_side);
        self.apply_chat_panel_side();
    }

    fn save_setting(&self, change_settings: impl FnOnce(&mut crate::settings::Settings)) {
        let updated_settings = {
            let mut settings = self.settings.write().unwrap();
            change_settings(&mut settings);
            settings.clone()
        };
        save_and_announce_settings(&self.app, updated_settings);
    }

    // MARK: Showing, hiding, minimizing

    /// Slides the panel in on the monitor the mouse is on and focuses the text box.
    pub fn show(&self) {
        let Some(chat_window) = self.chat_window().or_else(|| self.create_chat_window()) else { return };
        let monitors = monitors_from_tauri(&self.app);
        let monitor_under_mouse = self
            .app
            .cursor_position()
            .ok()
            .and_then(|cursor_position| monitor_containing_point(&monitors, cursor_position.x, cursor_position.y).cloned());
        let Some(target_monitor) = monitor_under_mouse.or_else(|| self.state.lock().unwrap().current_monitor.clone()) else {
            return;
        };
        let target_frame = on_screen_frame(target_monitor.work_area, target_monitor.scale_factor, self.panel_side());

        self.remember_user_app_window();
        {
            let mut state = self.state.lock().unwrap();
            state.is_minimized = false;
            state.has_unread_reply = false;
            state.is_closing = false;
            state.current_monitor = Some(target_monitor.clone());
        }
        if !chat_window.is_visible().unwrap_or(false) {
            set_window_frame(&chat_window, off_screen_frame(target_frame, target_monitor.scale_factor, self.panel_side()));
        }
        let _ = chat_window.show();
        let _ = chat_window.set_focus();
        // A spoken question with a text-only reply opens the chat in voice mode,
        // where the orb is still shown: keep it under the chat's send button.
        if let Some(app_state) = self.app.try_state::<AppState>() {
            app_state.orb_window.place_below_window(Some(&chat_window));
        }
        self.emit_chat_state();
        self.animate_window_frame(target_frame, SHOW_ANIMATION_DURATION, AnimationCurve::SoftLanding, false);
    }

    /// Slides the panel back out past the screen edge, then hides it.
    pub fn hide(&self) {
        let Some(chat_window) = self.chat_window() else { return };
        if !chat_window.is_visible().unwrap_or(false) {
            // Hidden only for a screenshot: keep it hidden when the capture ends.
            crate::screen_capture::hide_window(&chat_window);
            return;
        }
        let Some(current_frame) = window_frame(&chat_window) else { return };
        let scale_factor = chat_window.scale_factor().unwrap_or(1.0);
        self.state.lock().unwrap().is_closing = true;
        self.emit_chat_state();
        self.animate_window_frame(off_screen_frame(current_frame, scale_factor, self.panel_side()), HIDE_ANIMATION_DURATION, AnimationCurve::EaseIn, true);
    }

    /// Shrinks the chat to a tab on the screen edge, centred on where the chat was.
    pub fn minimize(&self) {
        let Some(chat_window) = self.chat_window() else { return };
        if !chat_window.is_visible().unwrap_or(false) || self.state.lock().unwrap().is_minimized {
            return;
        }
        let Some(current_frame) = window_frame(&chat_window) else { return };
        // The tab has no text box, so the keyboard goes back to the user's app.
        self.give_keyboard_focus_back_to_user_app();
        self.state.lock().unwrap().is_minimized = true;
        let Some(current_monitor) = self.current_monitor_still_connected(current_frame) else { return };
        let center_y = current_frame.y + current_frame.height as i32 / 2;
        let tab_frame = minimized_tab_frame(current_monitor.work_area, current_monitor.scale_factor, self.panel_side(), center_y);
        self.emit_chat_state();
        self.animate_window_frame(tab_frame, MOVE_ANIMATION_DURATION, AnimationCurve::SoftLanding, false);
    }

    pub fn expand_from_minimized(&self) {
        let Some(chat_window) = self.chat_window() else { return };
        if !self.state.lock().unwrap().is_minimized {
            return;
        }
        let Some(current_frame) = window_frame(&chat_window) else { return };
        let Some(current_monitor) = self.current_monitor_still_connected(current_frame) else { return };
        self.remember_user_app_window();
        {
            let mut state = self.state.lock().unwrap();
            state.is_minimized = false;
            state.has_unread_reply = false;
        }
        let _ = chat_window.show();
        let _ = chat_window.set_focus();
        self.emit_chat_state();
        let target_frame = on_screen_frame(current_monitor.work_area, current_monitor.scale_factor, self.panel_side());
        self.animate_window_frame(target_frame, MOVE_ANIMATION_DURATION, AnimationCurve::SoftLanding, false);
    }

    fn move_to_current_side(&self) {
        let Some(chat_window) = self.chat_window() else { return };
        if !chat_window.is_visible().unwrap_or(false) {
            return;
        }
        let Some(current_frame) = window_frame(&chat_window) else { return };
        let Some(current_monitor) = self.current_monitor_still_connected(current_frame) else { return };
        let target_frame = if self.state.lock().unwrap().is_minimized {
            let center_y = current_frame.y + current_frame.height as i32 / 2;
            minimized_tab_frame(current_monitor.work_area, current_monitor.scale_factor, self.panel_side(), center_y)
        } else {
            on_screen_frame(current_monitor.work_area, current_monitor.scale_factor, self.panel_side())
        };
        self.animate_window_frame(target_frame, MOVE_ANIMATION_DURATION, AnimationCurve::SoftLanding, false);
    }

    /// After a display is unplugged the remembered monitor is gone; use the one
    /// the panel is on now (or the first). Refreshes the work area too, in case
    /// the taskbar moved.
    fn current_monitor_still_connected(&self, current_frame: PhysicalRect) -> Option<MonitorDescription> {
        let monitors = monitors_from_tauri(&self.app);
        let remembered_monitor = self.state.lock().unwrap().current_monitor.clone();
        let still_connected_monitor = remembered_monitor
            .and_then(|remembered_monitor| monitors.iter().find(|monitor| monitor.rect == remembered_monitor.rect).cloned());
        let current_monitor = still_connected_monitor.or_else(|| {
            let frame_center_x = current_frame.x as f64 + current_frame.width as f64 / 2.0;
            let frame_center_y = current_frame.y as f64 + current_frame.height as f64 / 2.0;
            monitor_containing_point(&monitors, frame_center_x, frame_center_y).cloned()
        })?;
        self.state.lock().unwrap().current_monitor = Some(current_monitor.clone());
        Some(current_monitor)
    }

    // MARK: Focus and clicks

    /// Remembers which window had the keyboard before the chat takes it, unless
    /// one of our own windows has it (then the earlier one stays remembered).
    fn remember_user_app_window(&self) {
        let one_of_our_windows_is_focused = self
            .app
            .webview_windows()
            .values()
            .any(|app_window| app_window.is_focused().unwrap_or(false));
        if one_of_our_windows_is_focused {
            return;
        }
        if let Some(foreground_window_id) = platform::foreground_window_id() {
            self.state.lock().unwrap().user_app_window_id = Some(foreground_window_id);
        }
    }

    /// Hands keyboard focus back to the app underneath, so [TYPE] and [KEY]
    /// actions land there instead of in the chat's own text box.
    pub fn give_keyboard_focus_back_to_user_app(&self) {
        let Some(chat_window) = self.chat_window() else { return };
        if !chat_window.is_focused().unwrap_or(false) {
            return;
        }
        let user_app_window_id = self.state.lock().unwrap().user_app_window_id;
        let focus_result = match user_app_window_id {
            Some(window_id) => platform::bring_window_to_foreground(window_id),
            None => Err(PlatformError::Failed("no window remembered".to_string())),
        };
        // Nothing remembered (e.g. chat mode was switched on from the settings
        // page, or Wayland), the window is gone, or the OS refused: the Mac's
        // trick of ordering the chat out and back in. Hiding it hands the
        // keyboard to the window underneath; showing it again can take the
        // keyboard back (Windows' SW_SHOW, window managers focusing on map), so
        // that window is read and activated after. Otherwise [TYPE] would type
        // into the chat.
        if let Err(focus_error) = focus_result {
            eprintln!("chat: handing the keyboard back by hiding the chat for a moment ({focus_error})");
            let _ = chat_window.hide();
            // ponytail: a fixed wait for the hide and the OS's focus change to land
            // (both are asynchronous); raise it if the keyboard stays with the chat.
            std::thread::sleep(Duration::from_millis(50));
            let window_that_got_the_keyboard = platform::foreground_window_id();
            let _ = chat_window.show();
            if let Some(window_id) = window_that_got_the_keyboard {
                self.state.lock().unwrap().user_app_window_id = Some(window_id);
                let _ = platform::bring_window_to_foreground(window_id);
            }
        }
    }

    /// On while bhai clicks for the user: screenshots leave the chat out, so a
    /// click aimed at something behind it must reach it.
    pub fn set_lets_clicks_pass_through(&self, lets_clicks_pass_through: bool) {
        if let Some(chat_window) = self.chat_window() {
            let _ = chat_window.set_ignore_cursor_events(lets_clicks_pass_through);
        }
    }

    // MARK: Window

    fn create_chat_window(&self) -> Option<WebviewWindow> {
        let build_result = locked_down_window_builder(&self.app, CHAT_WINDOW_LABEL, "chat.html")
            .title("bhai")
            .inner_size(PANEL_LOGICAL_WIDTH, 600.0)
            // The page draws the dark rounded card; the corners stay see-through.
            .transparent(true)
            .decorations(false)
            .shadow(false)
            .resizable(false)
            .always_on_top(true)
            .focused(false)
            .build();
        let chat_window = match build_result {
            Ok(chat_window) => chat_window,
            Err(build_error) => {
                eprintln!("couldn't create the chat window: {build_error}");
                return None;
            }
        };
        let app_for_window_events = self.app.clone();
        chat_window.on_window_event(move |window_event| {
            // Alt+F4 minimizes like Esc instead of destroying the window.
            if let WindowEvent::CloseRequested { api, .. } = window_event {
                api.prevent_close();
                if let Some(app_state) = app_for_window_events.try_state::<AppState>() {
                    app_state.chat_panel.minimize();
                }
            }
        });
        Some(chat_window)
    }

    /// Steps the window from its current frame to `target_frame`. A newer
    /// animation replaces a running one, which also cancels a pending hide.
    fn animate_window_frame(&self, target_frame: PhysicalRect, duration: Duration, animation_curve: AnimationCurve, hides_when_finished: bool) {
        let Some(chat_window) = self.chat_window() else { return };
        let start_frame = window_frame(&chat_window).unwrap_or(target_frame);
        let app = self.app.clone();
        let animation_task = tauri::async_runtime::spawn(async move {
            let animation_start = Instant::now();
            loop {
                let time_fraction = (animation_start.elapsed().as_secs_f64() / duration.as_secs_f64()).min(1.0);
                set_window_frame(&chat_window, interpolated_frame(start_frame, target_frame, animation_curve.progress(time_fraction)));
                if time_fraction >= 1.0 {
                    break;
                }
                tokio::time::sleep(ANIMATION_FRAME_INTERVAL).await;
            }
            if hides_when_finished {
                let Some(app_state) = app.try_state::<AppState>() else { return };
                let chat_panel = &app_state.chat_panel;
                let was_closing = std::mem::replace(&mut chat_panel.state.lock().unwrap().is_closing, false);
                // Reopened while it was sliding away: keep it.
                if was_closing {
                    crate::screen_capture::hide_window(&chat_window);
                    app_state.orb_window.place_below_window(None);
                }
            }
        });
        let mut state = self.state.lock().unwrap();
        if let Some(previous_animation_task) = state.frame_animation_task.replace(animation_task) {
            previous_animation_task.abort();
        }
    }
}

fn window_frame(window: &WebviewWindow) -> Option<PhysicalRect> {
    let position = window.outer_position().ok()?;
    let size = window.outer_size().ok()?;
    Some(PhysicalRect { x: position.x, y: position.y, width: size.width, height: size.height })
}

fn set_window_frame(window: &WebviewWindow, frame: PhysicalRect) {
    let _ = window.set_position(PhysicalPosition::new(frame.x, frame.y));
    // Not `set_size`: GTK won't shrink a non-resizable window below its first size.
    platform::set_fixed_size_window_size(window, PhysicalSize::new(frame.width, frame.height));
}

// MARK: - Commands (chat page and settings page)

#[tauri::command]
pub fn get_chat_state(state: State<'_, AppState>) -> ChatStateSnapshot {
    state.chat_panel.chat_state_snapshot()
}

#[tauri::command]
pub fn send_chat_message(text: String, state: State<'_, AppState>) {
    state.companion_brain.send_typed_chat_message(text);
}

/// The "+" button or Ctrl+T.
#[tauri::command]
pub fn open_new_chat_tab(state: State<'_, AppState>) {
    state.companion_brain.open_new_chat_tab();
}

#[tauri::command]
pub fn switch_to_chat_tab(tab_id: u64, state: State<'_, AppState>) {
    state.companion_brain.switch_to_chat_tab(tab_id);
}

/// A tab's x, or Ctrl+W for the active tab.
#[tauri::command]
pub fn close_chat_tab(tab_id: u64, state: State<'_, AppState>) {
    state.companion_brain.close_chat_tab(tab_id);
}

#[tauri::command]
pub fn minimize_chat(state: State<'_, AppState>) {
    state.chat_panel.minimize();
}

#[tauri::command]
pub fn expand_chat(state: State<'_, AppState>) {
    state.chat_panel.expand_from_minimized();
}

/// The X: closing the chat (not minimizing it) goes back to voice mode, which
/// also slides the chat away.
#[tauri::command]
pub fn close_chat(state: State<'_, AppState>) {
    state.chat_panel.set_chat_input_mode(false);
}

#[tauri::command]
pub fn swap_chat_side(state: State<'_, AppState>) {
    let opposite_side = state.settings.read().unwrap().chat_panel_side.opposite();
    state.chat_panel.set_chat_panel_side(opposite_side);
}

/// The "Replies: Text / Text + voice" toggle under the composer (true = text + voice).
/// The settings page changes the same setting through `change_setting`.
#[tauri::command]
pub fn set_speaks_chat_replies(speaks_chat_replies: bool, state: State<'_, AppState>) {
    state.chat_panel.set_speaks_chat_replies(speaks_chat_replies);
}

#[cfg(test)]
mod tests {
    use super::*;

    const WORK_AREA: PhysicalRect = PhysicalRect { x: 0, y: 0, width: 1920, height: 1032 };

    #[test]
    fn the_card_fills_the_work_area_height_inset_from_its_edge() {
        assert_eq!(on_screen_frame(WORK_AREA, 1.0, ChatPanelSide::Right), PhysicalRect { x: 1528, y: 12, width: 380, height: 1008 });
        assert_eq!(on_screen_frame(WORK_AREA, 1.0, ChatPanelSide::Left), PhysicalRect { x: 12, y: 12, width: 380, height: 1008 });
        assert_eq!(on_screen_frame(WORK_AREA, 1.5, ChatPanelSide::Right), PhysicalRect { x: 1902 - 570, y: 18, width: 570, height: 996 });
        // A monitor left of the primary with a top taskbar.
        let left_monitor_work_area = PhysicalRect { x: -1920, y: 48, width: 1920, height: 1032 };
        assert_eq!(on_screen_frame(left_monitor_work_area, 1.0, ChatPanelSide::Left), PhysicalRect { x: -1908, y: 60, width: 380, height: 1008 });
    }

    #[test]
    fn the_tab_is_flush_with_the_edge_and_stays_inside_the_work_area() {
        assert_eq!(minimized_tab_frame(WORK_AREA, 1.0, ChatPanelSide::Right, 516), PhysicalRect { x: 1890, y: 460, width: 30, height: 112 });
        assert_eq!(minimized_tab_frame(WORK_AREA, 1.0, ChatPanelSide::Left, 516), PhysicalRect { x: 0, y: 460, width: 30, height: 112 });
        assert_eq!(minimized_tab_frame(WORK_AREA, 1.0, ChatPanelSide::Right, 5).y, 0);
        assert_eq!(minimized_tab_frame(WORK_AREA, 1.0, ChatPanelSide::Right, 5000).y, 1032 - 112);
        assert_eq!(minimized_tab_frame(WORK_AREA, 2.0, ChatPanelSide::Right, 516), PhysicalRect { x: 1860, y: 404, width: 60, height: 224 });
    }

    #[test]
    fn the_slide_starts_a_third_of_a_width_outward() {
        let right_frame = on_screen_frame(WORK_AREA, 1.0, ChatPanelSide::Right);
        assert_eq!(off_screen_frame(right_frame, 1.0, ChatPanelSide::Right).x, 1528 + 133);
        let left_frame = on_screen_frame(WORK_AREA, 1.0, ChatPanelSide::Left);
        assert_eq!(off_screen_frame(left_frame, 1.0, ChatPanelSide::Left).x, 12 - 133);
        assert_eq!(off_screen_frame(left_frame, 1.0, ChatPanelSide::Left).width, 380);
    }

    #[test]
    fn timing_curves_start_at_zero_end_at_one_and_have_the_mac_shape() {
        for (x1, y1, x2, y2) in [(0.2, 0.9, 0.3, 1.0), (0.42, 0.0, 1.0, 1.0)] {
            assert!(cubic_bezier_progress(x1, y1, x2, y2, 0.0).abs() < 1e-6);
            assert!((cubic_bezier_progress(x1, y1, x2, y2, 1.0) - 1.0).abs() < 1e-6);
        }
        // The soft landing is most of the way there by the middle; ease-in has barely started.
        assert!(AnimationCurve::SoftLanding.progress(0.5) > 0.85);
        assert!(AnimationCurve::EaseIn.progress(0.5) < 0.35);
        assert!((cubic_bezier_progress(0.0, 0.0, 1.0, 1.0, 0.3) - 0.3).abs() < 0.05);
    }

    #[test]
    fn frames_blend_between_start_and_end() {
        let start_frame = PhysicalRect { x: 0, y: 0, width: 380, height: 1000 };
        let end_frame = PhysicalRect { x: 350, y: 444, width: 30, height: 112 };
        assert_eq!(interpolated_frame(start_frame, end_frame, 0.0), start_frame);
        assert_eq!(interpolated_frame(start_frame, end_frame, 1.0), end_frame);
        assert_eq!(interpolated_frame(start_frame, end_frame, 0.5), PhysicalRect { x: 175, y: 222, width: 205, height: 556 });
    }

    #[test]
    fn side_swaps_and_serializes_like_the_settings_file() {
        assert_eq!(ChatPanelSide::Right.opposite(), ChatPanelSide::Left);
        assert_eq!(serde_json::to_value(ChatPanelSide::Left).unwrap(), serde_json::json!("left"));
        assert_eq!(ChatPanelSide::default(), ChatPanelSide::Right);
    }
}
