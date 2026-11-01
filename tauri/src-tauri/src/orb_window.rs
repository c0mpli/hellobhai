//! The orb: bhai's home on screen in voice mode, port of the Mac's
//! `OrbWindow.swift`. A small transparent, always-on-top, non-focusable window
//! showing `generated/orb.html` (the Mac page: `orb.riv` drawn by Rive's web
//! runtime). `ui/orb-host-bridge.js` hands every mouse event to the commands
//! here: drag moves it (anywhere on a monitor except a taskbar along the top,
//! the bottom taskbar row included), a click is tap-to-talk, hover wakes it.
//!
//! Its animation mirrors the brain (`buddy-voice-state`, `speaking-changed`):
//! listening, thinking (processing and responding), speaking (only while audio
//! plays), asleep (before onboarding or after 5 idle minutes; shown as the idle
//! orb at 45 % opacity, because orb.riv's own asleep animation shrinks it to
//! nothing), idle otherwise. It fades out and turns click-through while bhai
//! rests, the real pointer is hidden and the foreground window fills its
//! monitor (fullscreen video). Click-through during on-screen actions
//! (`OrbClickThroughWhileActing`). Hidden in the cursor voice style and in chat
//! mode. The overlays launch the pointer from its centre (`buddy-home-changed`).

use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tauri::webview::PageLoadEvent;
use tauri::{AppHandle, Listener, Manager, PhysicalPosition, PhysicalSize, State, WebviewWindow};
use tokio::sync::mpsc::UnboundedSender;

use crate::app_state::AppState;
use crate::page_bridge::{emit_to_overlays, locked_down_window_builder};
use crate::platform;
use crate::push_to_talk::HotkeyEvent;
use crate::screen_mapping::{monitors_from_tauri, MonitorDescription, PhysicalRect};
use crate::settings::{save_settings, SharedSettings};

pub const ORB_WINDOW_LABEL: &str = "orb";
/// Injected with `initialization_script` into the orb page only.
const ORB_HOST_BRIDGE_SCRIPT: &str = include_str!("../../ui/orb-host-bridge.js");

/// Orb side length in logical pixels (Mac points), set with the Buddy tab's slider.
pub const DEFAULT_ORB_SIDE_LENGTH: f64 = 88.0;
pub const MINIMUM_ORB_SIDE_LENGTH: f64 = 48.0;
pub const MAXIMUM_ORB_SIDE_LENGTH: f64 = 160.0;
/// The Mac slider moves in 2 pt steps.
const ORB_SIDE_LENGTH_STEP: f64 = 2.0;
/// Gap between the default spot and the screen's right edge, logical pixels.
const DEFAULT_MARGIN_FROM_SCREEN_EDGE: f64 = 20.0;
/// A bottom taskbar shorter than this counts as auto-hidden (Mac: Dock hidden).
const MINIMUM_BOTTOM_TASKBAR_HEIGHT: f64 = 20.0;
/// Closest the default spot gets to the bottom edge when the orb is taller than the taskbar.
const MINIMUM_GAP_TO_BOTTOM_EDGE: f64 = 4.0;
/// A press that moves less than this (logical pixels) is a click, not a drag.
const DRAG_START_DISTANCE: f64 = 3.0;
const ASLEEP_OPACITY: f64 = 0.45;
const FALLS_ASLEEP_AFTER_IDLE: Duration = Duration::from_secs(5 * 60);
/// How often the fullscreen check and the asleep timer run (Mac: 0.25 s timer).
const WATCH_INTERVAL: Duration = Duration::from_millis(250);
/// Every this many watch ticks (2 s): keep the orb above the taskbar and on a
/// connected monitor.
const WATCH_TICKS_BETWEEN_SLOW_CHECKS: u64 = 8;

/// orb.riv's `color` input: 0-9 (higher values look like 9). Default 8, Aurora.
pub const DEFAULT_ORB_COLOR: u8 = 8;
pub const MAXIMUM_ORB_COLOR: u8 = 9;

/// One of orb.riv's built-in color variants, for the Buddy tab's dropdown.
#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OrbColorVariant {
    pub color_value: u8,
    pub display_name: &'static str,
    /// Two colors from the variant's swirl, for the swatch (Mac `swatchGradientHexColors`).
    pub swatch_gradient_hex_colors: [&'static str; 2],
}

pub const ORB_COLOR_VARIANTS: [OrbColorVariant; 10] = [
    OrbColorVariant { color_value: 0, display_name: "Rainbow", swatch_gradient_hex_colors: ["#FFE03C", "#FF3B5C"] },
    OrbColorVariant { color_value: 1, display_name: "Candy", swatch_gradient_hex_colors: ["#FF4FD8", "#3FD0F5"] },
    OrbColorVariant { color_value: 2, display_name: "Red", swatch_gradient_hex_colors: ["#FF8A80", "#E8322E"] },
    OrbColorVariant { color_value: 3, display_name: "Coral", swatch_gradient_hex_colors: ["#FF7A8A", "#FFC93C"] },
    OrbColorVariant { color_value: 4, display_name: "Sunset", swatch_gradient_hex_colors: ["#FFE14D", "#F08A24"] },
    OrbColorVariant { color_value: 5, display_name: "Green", swatch_gradient_hex_colors: ["#7DF59A", "#1DB954"] },
    OrbColorVariant { color_value: 6, display_name: "Teal", swatch_gradient_hex_colors: ["#7FFFF2", "#12BFAE"] },
    OrbColorVariant { color_value: 7, display_name: "Blue", swatch_gradient_hex_colors: ["#4FC3F7", "#1D4ED8"] },
    OrbColorVariant { color_value: 8, display_name: "Aurora", swatch_gradient_hex_colors: ["#3FD5F5", "#7A4DFF"] },
    OrbColorVariant { color_value: 9, display_name: "Violet", swatch_gradient_hex_colors: ["#E07BFF", "#6A4DFF"] },
];

/// How bhai shows up in voice mode (Buddy tab -> "Voice style").
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VoiceBuddyStyle {
    /// The draggable orb; the pointer launches out of it for each job.
    #[default]
    Orb,
    /// The original look: the blue triangle follows the mouse (waveform while
    /// listening, spinner while thinking) and the orb is hidden.
    Cursor,
}

/// What the orb's animation shows. Hover is layered on top by `orb_page_state_name`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OrbVisualState {
    Asleep,
    Idle,
    Listening,
    Thinking,
    Speaking,
}

impl OrbVisualState {
    fn is_resting(self) -> bool {
        matches!(self, OrbVisualState::Idle | OrbVisualState::Asleep)
    }
}

/// Mac `refreshOrbVisualState`. `voice_state` is the brain's `buddy-voice-state`
/// name. Responding means waiting for the voice, a text-only reply or on-screen
/// actions, so it shows thinking; speaking only shows while audio plays.
pub fn orb_visual_state(voice_state: &str, is_speaking: bool, has_completed_onboarding: bool, idle_duration: Duration) -> OrbVisualState {
    if is_speaking {
        return OrbVisualState::Speaking;
    }
    match voice_state {
        "listening" => OrbVisualState::Listening,
        "processing" | "responding" => OrbVisualState::Thinking,
        _ if !has_completed_onboarding || idle_duration > FALLS_ASLEEP_AFTER_IDLE => OrbVisualState::Asleep,
        _ => OrbVisualState::Idle,
    }
}

/// The name passed to the page's `hb.setOrbState`. Hover only shows over a
/// resting orb; asleep is drawn as idle (dimmed by `orb_opacity`).
pub fn orb_page_state_name(visual_state: OrbVisualState, is_hovering: bool) -> &'static str {
    if is_hovering && visual_state.is_resting() {
        return "hover";
    }
    match visual_state {
        OrbVisualState::Asleep | OrbVisualState::Idle => "idle",
        OrbVisualState::Listening => "listening",
        OrbVisualState::Thinking => "thinking",
        OrbVisualState::Speaking => "speaking",
    }
}

pub fn orb_opacity(visual_state: OrbVisualState, is_hovering: bool, is_hidden_for_fullscreen: bool) -> f64 {
    if is_hidden_for_fullscreen {
        0.0
    } else if visual_state == OrbVisualState::Asleep && !is_hovering {
        ASLEEP_OPACITY
    } else {
        1.0
    }
}

/// Voice mode is tap to talk: the first click listens, the second sends.
pub fn tap_to_talk_hotkey_event(is_listening: bool) -> HotkeyEvent {
    if is_listening {
        HotkeyEvent::PushToTalkReleased
    } else {
        HotkeyEvent::PushToTalkPressed
    }
}

/// Clamped to 48-160 and rounded to the slider's 2-point steps; garbage means the default.
pub fn clamped_orb_side_length(requested_side_length: f64) -> f64 {
    if !requested_side_length.is_finite() || requested_side_length <= 0.0 {
        return DEFAULT_ORB_SIDE_LENGTH;
    }
    let stepped_side_length = (requested_side_length / ORB_SIDE_LENGTH_STEP).round() * ORB_SIDE_LENGTH_STEP;
    stepped_side_length.clamp(MINIMUM_ORB_SIDE_LENGTH, MAXIMUM_ORB_SIDE_LENGTH)
}

fn physical_side_length(logical_side_length: f64, scale_factor: f64) -> u32 {
    (logical_side_length * scale_factor).round().max(1.0) as u32
}

fn rect_bottom(rect: PhysicalRect) -> i32 {
    rect.y + rect.height as i32
}

fn rect_right(rect: PhysicalRect) -> i32 {
    rect.x + rect.width as i32
}

fn rects_intersect(first: PhysicalRect, second: PhysicalRect) -> bool {
    first.x < rect_right(second) && second.x < rect_right(first) && first.y < rect_bottom(second) && second.y < rect_bottom(first)
}

/// Everything except a taskbar along the top edge, so the orb can sit in the
/// bottom taskbar's row (Mac: everything below the menu bar, Dock row included).
pub fn area_orb_may_use(monitor: &MonitorDescription) -> PhysicalRect {
    let top_edge = monitor.work_area.y.max(monitor.rect.y);
    PhysicalRect {
        x: monitor.rect.x,
        y: top_edge,
        width: monitor.rect.width,
        height: (rect_bottom(monitor.rect) - top_edge).max(0) as u32,
    }
}

fn distance_squared_to_rect(point_x: f64, point_y: f64, rect: PhysicalRect) -> f64 {
    let nearest_x = point_x.clamp(rect.x as f64, rect_right(rect) as f64);
    let nearest_y = point_y.clamp(rect.y as f64, rect_bottom(rect) as f64);
    (point_x - nearest_x).powi(2) + (point_y - nearest_y).powi(2)
}

/// The monitor containing the point, else the nearest one.
pub fn monitor_nearest_point(monitors: &[MonitorDescription], point_x: f64, point_y: f64) -> Option<&MonitorDescription> {
    monitors.iter().find(|monitor| monitor.rect.contains_point(point_x, point_y)).or_else(|| {
        monitors.iter().min_by(|first, second| {
            distance_squared_to_rect(point_x, point_y, first.rect).total_cmp(&distance_squared_to_rect(point_x, point_y, second.rect))
        })
    })
}

/// In the bottom taskbar's row at the right edge of the primary monitor, centred
/// on the taskbar's height (Mac: the Dock's row). Taskbar hidden or on another
/// edge: the bottom-right corner, 20 px in.
pub fn default_orb_origin(primary_monitor: &MonitorDescription, orb_side: u32) -> (i32, i32) {
    let scale_factor = primary_monitor.scale_factor;
    let margin = (DEFAULT_MARGIN_FROM_SCREEN_EDGE * scale_factor).round() as i32;
    let orb_side = orb_side as i32;
    let monitor_bottom = rect_bottom(primary_monitor.rect);
    let bottom_taskbar_height = monitor_bottom - rect_bottom(primary_monitor.work_area);
    let origin_y = if bottom_taskbar_height as f64 > MINIMUM_BOTTOM_TASKBAR_HEIGHT * scale_factor {
        let minimum_gap = (MINIMUM_GAP_TO_BOTTOM_EDGE * scale_factor).round() as i32;
        monitor_bottom - orb_side - ((bottom_taskbar_height - orb_side) / 2).max(minimum_gap)
    } else {
        monitor_bottom - orb_side - margin
    };
    (rect_right(primary_monitor.rect) - orb_side - margin, origin_y)
}

/// Keeps the whole orb inside the usable area of the monitor it is mostly on.
pub fn origin_kept_on_screen(proposed_origin: (i32, i32), orb_side: u32, monitors: &[MonitorDescription]) -> (i32, i32) {
    let orb_side = orb_side as i32;
    let center_x = (proposed_origin.0 + orb_side / 2) as f64;
    let center_y = (proposed_origin.1 + orb_side / 2) as f64;
    let Some(monitor_under_orb) = monitor_nearest_point(monitors, center_x, center_y) else {
        return proposed_origin;
    };
    let usable_area = area_orb_may_use(monitor_under_orb);
    // max then min like the Mac, so an orb wider than the area pins to its left/top edge.
    (
        proposed_origin.0.max(usable_area.x).min(rect_right(usable_area) - orb_side),
        proposed_origin.1.max(usable_area.y).min(rect_bottom(usable_area) - orb_side),
    )
}

/// The saved spot when it still touches a connected monitor's usable area, else the default.
pub fn saved_or_default_origin(
    saved_origin: Option<(i32, i32)>,
    orb_side: u32,
    monitors: &[MonitorDescription],
    primary_monitor: &MonitorDescription,
) -> (i32, i32) {
    if let Some(saved_origin) = saved_origin {
        let saved_frame = PhysicalRect { x: saved_origin.0, y: saved_origin.1, width: orb_side, height: orb_side };
        if monitors.iter().any(|monitor| rects_intersect(area_orb_may_use(monitor), saved_frame)) {
            return origin_kept_on_screen(saved_origin, orb_side, monitors);
        }
    }
    default_orb_origin(primary_monitor, orb_side)
}

/// What the overlays need to place the pointer's home and the onboarding bubbles.
#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BuddyHomeState {
    pub voice_buddy_style: VoiceBuddyStyle,
    pub is_chat_mode: bool,
    /// Global physical pixels; known even while the orb is hidden.
    pub orb_frame: PhysicalRect,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BuddySettingsSnapshot {
    pub voice_buddy_style: VoiceBuddyStyle,
    pub orb_side_length: f64,
    pub minimum_orb_side_length: f64,
    pub maximum_orb_side_length: f64,
    pub orb_color: u8,
    pub orb_color_variants: Vec<OrbColorVariant>,
}

struct OrbDrag {
    cursor_at_press: (f64, f64),
    origin_at_press: (i32, i32),
    is_dragging: bool,
}

struct OrbState {
    /// The brain's last `buddy-voice-state` name.
    voice_state: String,
    is_speaking: bool,
    last_activity: Instant,
    visual_state: OrbVisualState,
    is_hovering: bool,
    is_hidden_for_chat_mode: bool,
    is_hidden_for_fullscreen: bool,
    /// More than zero while on-screen actions run (a counter, because an aborted
    /// task's guard may drop after the next task's guard was made).
    computer_actions_in_progress: u32,
    /// The chat window the orb sits under while it is open, if any.
    window_label_to_stay_below: Option<String>,
    orb_frame: PhysicalRect,
    drag: Option<OrbDrag>,
    /// What the page last got, so the 250 ms watch never re-sends the same thing.
    applied_page_state_name: Option<&'static str>,
    applied_opacity: Option<f64>,
    applied_ignores_cursor_events: Option<bool>,
}

pub struct OrbWindow {
    /// For callbacks that must not keep the orb alive (the page-load handler).
    weak_self: Weak<OrbWindow>,
    app: AppHandle,
    settings: SharedSettings,
    hotkey_sender: UnboundedSender<HotkeyEvent>,
    state: Mutex<OrbState>,
}

impl OrbWindow {
    /// `hotkey_sender` is the push-to-talk channel: a click in voice mode is a tap on the hotkey.
    pub fn new(app: AppHandle, settings: SharedSettings, hotkey_sender: UnboundedSender<HotkeyEvent>) -> Arc<Self> {
        let orb_window = Arc::new_cyclic(|weak_self| OrbWindow {
            weak_self: weak_self.clone(),
            app,
            settings,
            hotkey_sender,
            state: Mutex::new(OrbState {
                voice_state: "idle".to_string(),
                is_speaking: false,
                last_activity: Instant::now(),
                visual_state: OrbVisualState::Idle,
                is_hovering: false,
                is_hidden_for_chat_mode: false,
                is_hidden_for_fullscreen: false,
                computer_actions_in_progress: 0,
                window_label_to_stay_below: None,
                orb_frame: PhysicalRect { x: 0, y: 0, width: 0, height: 0 },
                drag: None,
                applied_page_state_name: None,
                applied_opacity: None,
                applied_ignores_cursor_events: None,
            }),
        });
        // Known before the window exists, so the overlay can place the welcome bubble next to the orb.
        let initial_frame = orb_window.saved_or_default_frame();
        orb_window.state.lock().unwrap().orb_frame = initial_frame;
        orb_window
    }

    /// Follows the brain's events, shows the orb (unless the cursor style is on)
    /// and starts the 250 ms watch. Call once, after the overlays exist, so the
    /// orb is ordered above them.
    pub fn start(self: &Arc<Self>) {
        let weak_orb_window = Arc::downgrade(self);
        self.app.listen_any("buddy-voice-state", move |event| {
            let Some(orb_window) = weak_orb_window.upgrade() else { return };
            let voice_state = serde_json::from_str::<serde_json::Value>(event.payload())
                .ok()
                .and_then(|payload| payload.get("state")?.as_str().map(str::to_string))
                .unwrap_or_else(|| "idle".to_string());
            orb_window.state.lock().unwrap().voice_state = voice_state;
            orb_window.refresh_visual_state();
        });
        let weak_orb_window = Arc::downgrade(self);
        self.app.listen_any("speaking-changed", move |event| {
            let Some(orb_window) = weak_orb_window.upgrade() else { return };
            let is_speaking = serde_json::from_str::<serde_json::Value>(event.payload())
                .ok()
                .and_then(|payload| payload.get("isSpeaking")?.as_bool())
                .unwrap_or(false);
            orb_window.state.lock().unwrap().is_speaking = is_speaking;
            orb_window.refresh_visual_state();
        });

        self.refresh_visual_state();
        self.apply_visibility();

        let weak_orb_window = Arc::downgrade(self);
        tauri::async_runtime::spawn(async move {
            let mut watch_tick_number: u64 = 0;
            loop {
                tokio::time::sleep(WATCH_INTERVAL).await;
                let Some(orb_window) = weak_orb_window.upgrade() else { break };
                watch_tick_number += 1;
                orb_window.watch_tick(watch_tick_number % WATCH_TICKS_BETWEEN_SLOW_CHECKS == 0);
            }
        });
    }

    // ---- Settings ----

    fn voice_buddy_style(&self) -> VoiceBuddyStyle {
        self.settings.read().unwrap().voice_buddy_style
    }

    fn orb_side_length(&self) -> f64 {
        clamped_orb_side_length(self.settings.read().unwrap().orb_side_length)
    }

    fn orb_color(&self) -> u8 {
        self.settings.read().unwrap().orb_color.min(MAXIMUM_ORB_COLOR)
    }

    fn save_orb_origin(&self, orb_origin: (i32, i32)) {
        let updated_settings = {
            let mut settings = self.settings.write().unwrap();
            settings.orb_window_origin = Some(orb_origin);
            settings.clone()
        };
        save_settings(&self.app, &updated_settings);
    }

    pub fn buddy_settings_snapshot(&self) -> BuddySettingsSnapshot {
        BuddySettingsSnapshot {
            voice_buddy_style: self.voice_buddy_style(),
            orb_side_length: self.orb_side_length(),
            minimum_orb_side_length: MINIMUM_ORB_SIDE_LENGTH,
            maximum_orb_side_length: MAXIMUM_ORB_SIDE_LENGTH,
            orb_color: self.orb_color(),
            orb_color_variants: ORB_COLOR_VARIANTS.to_vec(),
        }
    }

    pub fn home_state(&self) -> BuddyHomeState {
        let state = self.state.lock().unwrap();
        BuddyHomeState {
            voice_buddy_style: self.voice_buddy_style(),
            is_chat_mode: state.is_hidden_for_chat_mode,
            orb_frame: state.orb_frame,
        }
    }

    fn emit_home_changed(&self) {
        emit_to_overlays(&self.app, "buddy-home-changed", self.home_state());
    }

    /// The style switched in the Buddy tab: show or hide the orb, re-home the pointer.
    pub fn apply_voice_buddy_style(&self) {
        self.apply_visibility();
        self.emit_home_changed();
    }

    /// Chat mode: the chat sidebar stands in for the orb, and the overlay's
    /// pointer launches from the orb's spot without following the mouse.
    /// Call whenever bhai mode changes, and once at launch.
    pub fn set_hidden_for_chat_mode(&self, is_chat_mode: bool) {
        self.state.lock().unwrap().is_hidden_for_chat_mode = is_chat_mode;
        self.apply_visibility();
        self.emit_home_changed();
    }

    /// While the chat is open the orb sits under it so it can't cover the send
    /// button; `None` when the chat hides. Both are always-on-top, so the chat is
    /// re-raised above the orb (Mac: the orb drops to the chat's level, ordered below).
    pub fn place_below_window(&self, window_to_stay_below: Option<&WebviewWindow>) {
        self.state.lock().unwrap().window_label_to_stay_below = window_to_stay_below.map(|window| window.label().to_string());
        self.keep_window_order();
    }

    fn keep_window_order(&self) {
        let window_label_to_stay_below = self.state.lock().unwrap().window_label_to_stay_below.clone();
        match window_label_to_stay_below.and_then(|window_label| self.app.get_webview_window(&window_label)) {
            Some(window_to_stay_below) if window_to_stay_below.is_visible().unwrap_or(false) => {
                let _ = window_to_stay_below.set_always_on_top(true);
            }
            // Re-asserted every couple of seconds: clicking the taskbar raises it over
            // topmost windows, and the default spot is in the taskbar's row.
            _ => {
                if let Some(orb_window) = self.window_if_visible() {
                    let _ = orb_window.set_always_on_top(true);
                }
            }
        }
    }

    /// Size from the Buddy tab, applied around the orb's centre so it stays where the user put it.
    pub fn apply_orb_size(&self) {
        let current_frame = self.state.lock().unwrap().orb_frame;
        let monitors = monitors_from_tauri(&self.app);
        let center_x = current_frame.x as f64 + current_frame.width as f64 / 2.0;
        let center_y = current_frame.y as f64 + current_frame.height as f64 / 2.0;
        let scale_factor = monitor_nearest_point(&monitors, center_x, center_y).map_or(1.0, |monitor| monitor.scale_factor);
        let new_side = physical_side_length(self.orb_side_length(), scale_factor);
        let proposed_origin = ((center_x - new_side as f64 / 2.0).round() as i32, (center_y - new_side as f64 / 2.0).round() as i32);
        let new_origin = origin_kept_on_screen(proposed_origin, new_side, &monitors);
        self.move_window_to(new_origin, new_side);
        if self.settings.read().unwrap().orb_window_origin.is_some() {
            self.save_orb_origin(new_origin);
        }
        self.emit_home_changed();
    }

    pub fn apply_orb_color(&self) {
        let color_value = self.orb_color();
        if let Some(orb_window) = self.app.get_webview_window(ORB_WINDOW_LABEL) {
            // A plain integer, so nothing user-controlled is ever evaluated.
            let _ = orb_window.eval(format!("hb.setOrbColor({color_value})"));
        }
    }

    // ---- Showing ----

    fn should_be_shown(&self) -> bool {
        self.voice_buddy_style() == VoiceBuddyStyle::Orb && !self.state.lock().unwrap().is_hidden_for_chat_mode
    }

    fn window_if_visible(&self) -> Option<WebviewWindow> {
        self.app.get_webview_window(ORB_WINDOW_LABEL).filter(|orb_window| orb_window.is_visible().unwrap_or(false))
    }

    fn saved_or_default_frame(&self) -> PhysicalRect {
        let monitors = monitors_from_tauri(&self.app);
        let primary_monitor_position = self.app.primary_monitor().ok().flatten().map(|monitor| *monitor.position());
        let Some(primary_monitor) = monitors
            .iter()
            .find(|monitor| primary_monitor_position.is_some_and(|position| position.x == monitor.rect.x && position.y == monitor.rect.y))
            .or_else(|| monitors.first())
        else {
            return self.state.lock().unwrap().orb_frame;
        };
        let saved_origin = self.settings.read().unwrap().orb_window_origin;
        // The saved spot's monitor decides the physical size (monitors can differ in scale).
        let scale_factor = saved_origin
            .and_then(|(saved_x, saved_y)| monitor_nearest_point(&monitors, saved_x as f64, saved_y as f64))
            .unwrap_or(primary_monitor)
            .scale_factor;
        let orb_side = physical_side_length(self.orb_side_length(), scale_factor);
        let origin = saved_or_default_origin(saved_origin, orb_side, &monitors, primary_monitor);
        PhysicalRect { x: origin.0, y: origin.1, width: orb_side, height: orb_side }
    }

    fn move_window_to(&self, origin: (i32, i32), orb_side: u32) {
        self.state.lock().unwrap().orb_frame = PhysicalRect { x: origin.0, y: origin.1, width: orb_side, height: orb_side };
        if let Some(orb_window) = self.app.get_webview_window(ORB_WINDOW_LABEL) {
            let _ = orb_window.set_position(PhysicalPosition::new(origin.0, origin.1));
            let _ = orb_window.set_size(PhysicalSize::new(orb_side, orb_side));
        }
    }

    fn apply_visibility(&self) {
        if !self.should_be_shown() {
            if let Some(orb_window) = self.app.get_webview_window(ORB_WINDOW_LABEL) {
                let _ = orb_window.hide();
            }
            return;
        }
        if self.app.get_webview_window(ORB_WINDOW_LABEL).is_none() && !self.create_window() {
            return;
        }
        let frame = self.saved_or_default_frame();
        self.move_window_to((frame.x, frame.y), frame.width);
        if let Some(orb_window) = self.app.get_webview_window(ORB_WINDOW_LABEL) {
            let _ = orb_window.show();
            let _ = orb_window.set_always_on_top(true);
        }
        self.keep_window_order();
        self.apply_state_to_page_and_window();
    }

    fn create_window(&self) -> bool {
        let weak_orb_window = self.weak_self.clone();
        let build_result = locked_down_window_builder(&self.app, ORB_WINDOW_LABEL, "orb.html")
            .title("hellobhai orb")
            .transparent(true)
            .decorations(false)
            .shadow(false)
            .always_on_top(true)
            .focusable(false)
            .resizable(false)
            .initialization_script(ORB_HOST_BRIDGE_SCRIPT)
            .on_page_load(move |_window, page_load_payload| {
                if page_load_payload.event() == PageLoadEvent::Finished {
                    if let Some(orb_window) = weak_orb_window.upgrade() {
                        orb_window.page_did_load();
                    }
                }
            })
            .build();
        match build_result {
            Ok(_) => true,
            Err(build_error) => {
                eprintln!("couldn't create the orb window: {build_error}");
                false
            }
        }
    }

    /// The page starts at idle with color 8; give it the real state and color.
    fn page_did_load(&self) {
        {
            let mut state = self.state.lock().unwrap();
            state.applied_page_state_name = None;
            state.applied_opacity = None;
        }
        self.apply_orb_color();
        self.apply_state_to_page_and_window();
    }

    // ---- State ----

    fn note_user_interaction(&self) {
        self.state.lock().unwrap().last_activity = Instant::now();
        self.refresh_visual_state();
    }

    fn refresh_visual_state(&self) {
        let has_completed_onboarding = self.settings.read().unwrap().has_completed_onboarding;
        {
            let mut state = self.state.lock().unwrap();
            let visual_state = orb_visual_state(&state.voice_state, state.is_speaking, has_completed_onboarding, state.last_activity.elapsed());
            if !visual_state.is_resting() {
                state.last_activity = Instant::now();
            }
            state.visual_state = visual_state;
        }
        self.apply_state_to_page_and_window();
    }

    fn apply_state_to_page_and_window(&self) {
        let (page_state_name_to_send, opacity_to_send, ignores_cursor_events_to_set) = {
            let state = self.state.lock().unwrap();
            let page_state_name = orb_page_state_name(state.visual_state, state.is_hovering);
            let opacity = orb_opacity(state.visual_state, state.is_hovering, state.is_hidden_for_fullscreen);
            // A hidden orb must not catch clicks meant for the video; during actions clicks reach what is behind it.
            let ignores_cursor_events = state.is_hidden_for_fullscreen || state.computer_actions_in_progress > 0;
            let page_state_name_to_send = (state.applied_page_state_name != Some(page_state_name)).then_some(page_state_name);
            let opacity_to_send = (state.applied_opacity != Some(opacity)).then_some(opacity);
            let ignores_cursor_events_to_set = (state.applied_ignores_cursor_events != Some(ignores_cursor_events)).then_some(ignores_cursor_events);
            (page_state_name_to_send, opacity_to_send, ignores_cursor_events_to_set)
        };
        let Some(orb_window) = self.app.get_webview_window(ORB_WINDOW_LABEL) else { return };
        let mut state_changes_sent = (false, false, false);
        if let Some(page_state_name) = page_state_name_to_send {
            // Fixed names only, never user text.
            state_changes_sent.0 = orb_window.eval(format!("hb.setOrbState('{page_state_name}')")).is_ok();
        }
        if let Some(opacity) = opacity_to_send {
            state_changes_sent.1 = orb_window.eval(format!("document.documentElement.style.opacity = '{opacity}'")).is_ok();
        }
        if let Some(ignores_cursor_events) = ignores_cursor_events_to_set {
            state_changes_sent.2 = orb_window.set_ignore_cursor_events(ignores_cursor_events).is_ok();
        }
        let mut state = self.state.lock().unwrap();
        if state_changes_sent.0 {
            state.applied_page_state_name = page_state_name_to_send;
        }
        if state_changes_sent.1 {
            state.applied_opacity = opacity_to_send;
        }
        if state_changes_sent.2 {
            state.applied_ignores_cursor_events = ignores_cursor_events_to_set;
        }
    }

    /// Fullscreen video check, asleep timer and (every 2 s) window order and monitor changes.
    fn watch_tick(&self, is_slow_check_tick: bool) {
        if self.window_if_visible().is_none() {
            return;
        }
        let is_resting = self.state.lock().unwrap().visual_state.is_resting();
        // The OS also hides the pointer while typing, so a hidden pointer only counts
        // when the foreground window fills its monitor.
        let should_hide_for_fullscreen =
            is_resting && !platform::is_system_pointer_visible() && platform::is_foreground_window_full_screen();
        self.state.lock().unwrap().is_hidden_for_fullscreen = should_hide_for_fullscreen;
        self.refresh_visual_state();

        if is_slow_check_tick {
            self.keep_window_order();
            let current_frame = self.state.lock().unwrap().orb_frame;
            let monitors = monitors_from_tauri(&self.app);
            let is_on_a_connected_monitor = monitors.iter().any(|monitor| rects_intersect(area_orb_may_use(monitor), current_frame));
            if !is_on_a_connected_monitor && !monitors.is_empty() {
                let frame = self.saved_or_default_frame();
                self.move_window_to((frame.x, frame.y), frame.width);
                self.emit_home_changed();
            }
        }
    }

    /// +1 while on-screen actions run; the guard's drop undoes it.
    fn change_computer_actions_in_progress(&self, is_starting: bool) {
        {
            let mut state = self.state.lock().unwrap();
            state.computer_actions_in_progress = if is_starting {
                state.computer_actions_in_progress + 1
            } else {
                state.computer_actions_in_progress.saturating_sub(1)
            };
        }
        self.apply_state_to_page_and_window();
    }

    // ---- Mouse (from ui/orb-host-bridge.js) ----

    fn pointer_pressed(&self) {
        let Ok(cursor_position) = self.app.cursor_position() else { return };
        let origin_at_press = {
            let frame = self.state.lock().unwrap().orb_frame;
            (frame.x, frame.y)
        };
        self.state.lock().unwrap().drag = Some(OrbDrag {
            cursor_at_press: (cursor_position.x, cursor_position.y),
            origin_at_press,
            is_dragging: false,
        });
    }

    fn pointer_moved(&self) {
        let Ok(cursor_position) = self.app.cursor_position() else { return };
        let scale_factor = self
            .app
            .get_webview_window(ORB_WINDOW_LABEL)
            .and_then(|orb_window| orb_window.scale_factor().ok())
            .unwrap_or(1.0);
        let (proposed_origin, orb_side) = {
            let mut state = self.state.lock().unwrap();
            let orb_side = state.orb_frame.width;
            let Some(drag) = state.drag.as_mut() else { return };
            let offset_x = cursor_position.x - drag.cursor_at_press.0;
            let offset_y = cursor_position.y - drag.cursor_at_press.1;
            if !drag.is_dragging && offset_x.hypot(offset_y) < DRAG_START_DISTANCE * scale_factor {
                return;
            }
            drag.is_dragging = true;
            (
                (drag.origin_at_press.0 + offset_x.round() as i32, drag.origin_at_press.1 + offset_y.round() as i32),
                orb_side,
            )
        };
        let monitors = monitors_from_tauri(&self.app);
        let kept_origin = origin_kept_on_screen(proposed_origin, orb_side, &monitors);
        self.state.lock().unwrap().orb_frame = PhysicalRect { x: kept_origin.0, y: kept_origin.1, width: orb_side, height: orb_side };
        if let Some(orb_window) = self.app.get_webview_window(ORB_WINDOW_LABEL) {
            let _ = orb_window.set_position(PhysicalPosition::new(kept_origin.0, kept_origin.1));
        }
        self.emit_home_changed();
    }

    fn pointer_released(&self, was_cancelled: bool) {
        let Some(finished_drag) = self.state.lock().unwrap().drag.take() else { return };
        if finished_drag.is_dragging {
            // Crossing onto a monitor with another scale resizes the window, so the size is re-read.
            let frame = self.state.lock().unwrap().orb_frame;
            self.save_orb_origin((frame.x, frame.y));
            if let Some(window_size) = self.app.get_webview_window(ORB_WINDOW_LABEL).and_then(|orb_window| orb_window.outer_size().ok()) {
                self.state.lock().unwrap().orb_frame.width = window_size.width;
                self.state.lock().unwrap().orb_frame.height = window_size.height;
                self.emit_home_changed();
            }
            self.note_user_interaction();
        } else if !was_cancelled {
            self.handle_click();
        }
    }

    /// Voice mode: tap to talk. (In chat mode the orb is hidden; the chat sidebar stands in for it.)
    fn handle_click(&self) {
        self.note_user_interaction();
        if self.state.lock().unwrap().is_hidden_for_chat_mode {
            return;
        }
        let is_listening = self
            .app
            .try_state::<AppState>()
            .is_some_and(|app_state| app_state.companion_brain.is_listening());
        let _ = self.hotkey_sender.send(tap_to_talk_hotkey_event(is_listening));
    }

    fn hover_changed(&self, is_hovering: bool) {
        {
            let mut state = self.state.lock().unwrap();
            if state.is_hovering == is_hovering {
                return;
            }
            state.is_hovering = is_hovering;
            if is_hovering {
                state.last_activity = Instant::now();
            }
        }
        self.refresh_visual_state();
    }
}

/// Makes the orb click-through for as long as it lives (on-screen actions):
/// screenshots leave the orb out, so Claude may click something behind it.
/// Dropped when the actions finish or the response task is aborted.
pub struct OrbClickThroughWhileActing {
    app: AppHandle,
}

impl OrbClickThroughWhileActing {
    pub fn start(app: &AppHandle) -> Self {
        if let Some(app_state) = app.try_state::<AppState>() {
            app_state.orb_window.change_computer_actions_in_progress(true);
        }
        OrbClickThroughWhileActing { app: app.clone() }
    }
}

impl Drop for OrbClickThroughWhileActing {
    fn drop(&mut self) {
        if let Some(app_state) = self.app.try_state::<AppState>() {
            app_state.orb_window.change_computer_actions_in_progress(false);
        }
    }
}

fn is_orb_window(window: &WebviewWindow) -> bool {
    window.label() == ORB_WINDOW_LABEL
}

#[tauri::command]
pub fn orb_pointer_pressed(window: WebviewWindow, state: State<'_, AppState>) {
    if is_orb_window(&window) {
        state.orb_window.pointer_pressed();
    }
}

#[tauri::command]
pub fn orb_pointer_moved(window: WebviewWindow, state: State<'_, AppState>) {
    if is_orb_window(&window) {
        state.orb_window.pointer_moved();
    }
}

#[tauri::command]
pub fn orb_pointer_released(was_cancelled: bool, window: WebviewWindow, state: State<'_, AppState>) {
    if is_orb_window(&window) {
        state.orb_window.pointer_released(was_cancelled);
    }
}

#[tauri::command]
pub fn orb_hover_changed(is_hovering: bool, window: WebviewWindow, state: State<'_, AppState>) {
    if is_orb_window(&window) {
        state.orb_window.hover_changed(is_hovering);
    }
}

/// Asked by each overlay page when it starts; later changes arrive as `buddy-home-changed`.
#[tauri::command]
pub fn buddy_home_state(state: State<'_, AppState>) -> BuddyHomeState {
    state.orb_window.home_state()
}

/// For the settings page's Buddy tab.
#[tauri::command]
pub fn get_buddy_settings(state: State<'_, AppState>) -> BuddySettingsSnapshot {
    state.orb_window.buddy_settings_snapshot()
}

fn update_and_save_settings(app: &AppHandle, state: &State<'_, AppState>, change_settings: impl FnOnce(&mut crate::settings::Settings)) {
    let updated_settings = {
        let mut settings = state.settings.write().unwrap();
        change_settings(&mut settings);
        settings.clone()
    };
    save_settings(app, &updated_settings);
}

/// "orb" | "cursor", applied live.
#[tauri::command]
pub fn set_voice_buddy_style(style: VoiceBuddyStyle, app: AppHandle, state: State<'_, AppState>) {
    update_and_save_settings(&app, &state, |settings| settings.voice_buddy_style = style);
    state.orb_window.apply_voice_buddy_style();
}

/// 48-160 logical pixels in 2 px steps, applied live around the orb's centre.
#[tauri::command]
pub fn set_orb_side_length(side_length: f64, app: AppHandle, state: State<'_, AppState>) {
    let side_length = clamped_orb_side_length(side_length);
    update_and_save_settings(&app, &state, |settings| settings.orb_side_length = side_length);
    state.orb_window.apply_orb_size();
}

/// orb.riv color variant 0-9 (`ORB_COLOR_VARIANTS`).
#[tauri::command]
pub fn set_orb_color(color_value: u8, app: AppHandle, state: State<'_, AppState>) {
    if color_value > MAXIMUM_ORB_COLOR {
        eprintln!("settings: ignoring unknown orb color {color_value}");
        return;
    }
    update_and_save_settings(&app, &state, |settings| settings.orb_color = color_value);
    state.orb_window.apply_orb_color();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn monitor(monitor_index: usize, rect: PhysicalRect, work_area: PhysicalRect, scale_factor: f64) -> MonitorDescription {
        MonitorDescription { monitor_index, rect, work_area, scale_factor }
    }

    fn rect(x: i32, y: i32, width: u32, height: u32) -> PhysicalRect {
        PhysicalRect { x, y, width, height }
    }

    /// 1920x1080 with a 48 px bottom taskbar.
    fn primary_with_bottom_taskbar() -> MonitorDescription {
        monitor(0, rect(0, 0, 1920, 1080), rect(0, 0, 1920, 1032), 1.0)
    }

    #[test]
    fn visual_state_follows_the_mac_mapping() {
        let short_idle = Duration::from_secs(10);
        assert_eq!(orb_visual_state("idle", true, true, short_idle), OrbVisualState::Speaking);
        assert_eq!(orb_visual_state("listening", false, true, short_idle), OrbVisualState::Listening);
        assert_eq!(orb_visual_state("processing", false, true, short_idle), OrbVisualState::Thinking);
        assert_eq!(orb_visual_state("responding", false, true, short_idle), OrbVisualState::Thinking);
        assert_eq!(orb_visual_state("idle", false, true, short_idle), OrbVisualState::Idle);
        assert_eq!(orb_visual_state("idle", false, false, short_idle), OrbVisualState::Asleep);
        assert_eq!(orb_visual_state("idle", false, true, Duration::from_secs(5 * 60 + 1)), OrbVisualState::Asleep);
        // Listening wins over a long idle stretch.
        assert_eq!(orb_visual_state("listening", false, false, Duration::from_secs(9999)), OrbVisualState::Listening);
    }

    #[test]
    fn hover_only_shows_over_a_resting_orb_and_asleep_is_dimmed_idle() {
        assert_eq!(orb_page_state_name(OrbVisualState::Idle, true), "hover");
        assert_eq!(orb_page_state_name(OrbVisualState::Asleep, true), "hover");
        assert_eq!(orb_page_state_name(OrbVisualState::Speaking, true), "speaking");
        assert_eq!(orb_page_state_name(OrbVisualState::Asleep, false), "idle");
        assert_eq!(orb_page_state_name(OrbVisualState::Thinking, false), "thinking");
        assert_eq!(orb_opacity(OrbVisualState::Asleep, false, false), 0.45);
        assert_eq!(orb_opacity(OrbVisualState::Asleep, true, false), 1.0);
        assert_eq!(orb_opacity(OrbVisualState::Idle, false, true), 0.0);
        assert_eq!(orb_opacity(OrbVisualState::Listening, false, false), 1.0);
    }

    #[test]
    fn a_click_is_a_tap_on_the_hotkey() {
        assert_eq!(tap_to_talk_hotkey_event(false), HotkeyEvent::PushToTalkPressed);
        assert_eq!(tap_to_talk_hotkey_event(true), HotkeyEvent::PushToTalkReleased);
    }

    #[test]
    fn side_length_is_clamped_and_stepped() {
        assert_eq!(clamped_orb_side_length(88.0), 88.0);
        assert_eq!(clamped_orb_side_length(89.2), 90.0);
        assert_eq!(clamped_orb_side_length(10.0), 48.0);
        assert_eq!(clamped_orb_side_length(500.0), 160.0);
        assert_eq!(clamped_orb_side_length(f64::NAN), 88.0);
        assert_eq!(clamped_orb_side_length(0.0), 88.0);
        assert_eq!(physical_side_length(88.0, 1.5), 132);
    }

    #[test]
    fn default_spot_is_in_the_taskbar_row_at_the_right_edge() {
        // Orb taller than the 48 px taskbar: 4 px from the bottom edge.
        assert_eq!(default_orb_origin(&primary_with_bottom_taskbar(), 88), (1920 - 88 - 20, 1080 - 88 - 4));
        // A small orb is centred on the taskbar's height.
        assert_eq!(default_orb_origin(&primary_with_bottom_taskbar(), 40), (1920 - 40 - 20, 1080 - 40 - 4));
        assert_eq!(default_orb_origin(&primary_with_bottom_taskbar(), 32), (1920 - 32 - 20, 1080 - 32 - 8));
        // Taskbar hidden: bottom-right corner, 20 px in; scale applies to the margin.
        let without_taskbar = monitor(0, rect(0, 0, 3840, 2160), rect(0, 0, 3840, 2160), 2.0);
        assert_eq!(default_orb_origin(&without_taskbar, 176), (3840 - 176 - 40, 2160 - 176 - 40));
    }

    #[test]
    fn the_orb_may_use_the_bottom_taskbar_row_but_not_a_top_taskbar() {
        assert_eq!(area_orb_may_use(&primary_with_bottom_taskbar()), rect(0, 0, 1920, 1080));
        let top_taskbar = monitor(0, rect(0, 0, 1920, 1080), rect(0, 48, 1920, 1032), 1.0);
        assert_eq!(area_orb_may_use(&top_taskbar), rect(0, 48, 1920, 1032));
    }

    #[test]
    fn drags_are_kept_on_the_monitor_under_the_orb() {
        let monitors = vec![
            monitor(0, rect(-1920, 0, 1920, 1080), rect(-1920, 0, 1920, 1040), 1.0),
            primary_with_bottom_taskbar(),
        ];
        assert_eq!(origin_kept_on_screen((100, 100), 88, &monitors), (100, 100));
        assert_eq!(origin_kept_on_screen((1900, 1070), 88, &monitors), (1920 - 88, 1080 - 88));
        assert_eq!(origin_kept_on_screen((-50, -40), 88, &monitors), (-88, 0));
        // Far outside: the nearest monitor.
        assert_eq!(origin_kept_on_screen((-5000, 500), 88, &monitors), (-1920, 500));
    }

    #[test]
    fn a_saved_spot_is_reused_only_while_its_monitor_is_connected() {
        let primary = primary_with_bottom_taskbar();
        let monitors = vec![primary.clone()];
        assert_eq!(saved_or_default_origin(Some((300, 400)), 88, &monitors, &primary), (300, 400));
        // Partly off screen: pulled back in.
        assert_eq!(saved_or_default_origin(Some((1900, 400)), 88, &monitors, &primary), (1920 - 88, 400));
        // On a monitor that is gone: the default spot.
        assert_eq!(saved_or_default_origin(Some((-1000, 400)), 88, &monitors, &primary), default_orb_origin(&primary, 88));
        assert_eq!(saved_or_default_origin(None, 88, &monitors, &primary), default_orb_origin(&primary, 88));
    }

    #[test]
    fn color_variants_match_the_mac_names_and_values() {
        let names: Vec<&str> = ORB_COLOR_VARIANTS.iter().map(|variant| variant.display_name).collect();
        assert_eq!(names, ["Rainbow", "Candy", "Red", "Coral", "Sunset", "Green", "Teal", "Blue", "Aurora", "Violet"]);
        assert!(ORB_COLOR_VARIANTS.iter().enumerate().all(|(index, variant)| variant.color_value as usize == index));
        assert_eq!(ORB_COLOR_VARIANTS[DEFAULT_ORB_COLOR as usize].display_name, "Aurora");
        assert_eq!(serde_json::to_value(VoiceBuddyStyle::Cursor).unwrap(), "cursor");
    }
}
