//! The central state machine, port of the Mac's `CompanionManager`: push-to-talk
//! -> transcribe -> agent loop (up to 8 screenshot -> claude -> actions rounds)
//! -> speak, point, circle; plus the board, notes and 3D side calls and onboarding.
//! The message flow (`send_transcript`) takes plain text so a typed chat panel
//! can reuse it later.
//!
//! Cancelling = aborting a task handle: the futures are dropped at their next
//! await, which also kills a running `claude` (kill_on_drop). No lock is ever
//! held across an await.

use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::json;
use tauri::async_runtime::JoinHandle;
use tauri::{AppHandle, Emitter};
use tokio::sync::mpsc::UnboundedReceiver;

use crate::app_paths;
use crate::audio_capture::PushToTalkRecorder;
use crate::board_panel::{BoardPanel, LiveContentKind};
use crate::claude_cli::{home_directory, run_claude_streaming, ClaudeRequest};
use crate::computer_control;
use crate::conversation_history::{ConversationEntry, ConversationHistory};
use crate::model3d_window::Model3DWindow;
use crate::narrated_step_queue::{NarratedStepQueue, StreamedLineForwarder};
use crate::orb_window::OrbClickThroughWhileActing;
use crate::overlay_windows;
use crate::page_bridge::emit_to_overlays;
use crate::prompts::{self, SystemPromptKind};
use crate::push_to_talk::HotkeyEvent;
use crate::response_tags::{parse_point_tag, parse_reply, CircleTag, ComputerAction};
use crate::screen_capture::{self, ScreenCapture};
use crate::screen_mapping::{self, PhysicalRect};
use crate::settings::{self, SharedSettings};
use crate::speech_player::SpeechPlayer;
use crate::stream_json::LabeledImage;
use crate::tray_and_settings;
use crate::whisper_transcriber::{SpeechModelState, WhisperTranscriber};

/// Most look -> act rounds for one request; also bounds runaway tasks.
const MAXIMUM_AGENT_STEPS: usize = 8;
const CLAUDE_UNREACHABLE_MESSAGE: &str = "couldn't reach claude. make sure claude code is installed and logged in.";
const REPEATED_ACTIONS_MESSAGE: &str = "that didn't seem to work, so i stopped. can you take a look?";
/// No Apple-Speech-style fallback on Windows while Whisper downloads.
const SPEECH_MODEL_NOT_READY_MESSAGE: &str = "my ears are still downloading. give me a minute.";
const DRAWING_WITHOUT_SPEECH_PROMPT: &str = "What is this?";
const ONBOARDING_WELCOME_MESSAGE: &str = "hey! i'm bhai";
const ONBOARDING_USER_PROMPT: &str = "look around my screen and find something interesting to point at";

/// Emitted to overlays and settings as `buddy-voice-state` `{state}`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum VoiceState {
    Idle,
    Listening,
    Processing,
    Responding,
}

/// Everything that changes, behind one lock.
struct BrainState {
    voice_state: VoiceState,
    /// None until the hotkey has tried the microphone once (shown in the settings panel).
    microphone_working: Option<bool>,
    is_recording: bool,
    is_transcribing: bool,
    response_task: Option<JoinHandle<()>>,
    visual_task: Option<JoinHandle<()>>,
    screen_circle_fade_task: Option<JoinHandle<()>>,
    onboarding_prompt_task: Option<JoinHandle<()>>,
    conversation_history: ConversationHistory,
    /// Freehand strokes drawn while the hotkey was held, global physical pixels.
    user_drawn_strokes: Vec<Vec<(i32, i32)>>,
    /// Set on release when strokes exist; a transcript clears it, otherwise "What is this?" is sent.
    user_drawn_strokes_awaiting_request: bool,
}

/// Owns the recorder, response and visual task handles, conversation history,
/// the user's drawn strokes and the voice state.
pub struct CompanionBrain {
    app: AppHandle,
    settings: SharedSettings,
    speech_player: Arc<SpeechPlayer>,
    whisper_transcriber: Arc<WhisperTranscriber>,
    board_panel: Arc<BoardPanel>,
    model3d_window: Arc<Model3DWindow>,
    // In a Mutex so the brain stays Sync whatever channel the recorder holds inside.
    push_to_talk_recorder: Mutex<PushToTalkRecorder>,
    state: Mutex<BrainState>,
}

fn abort_task(task: &mut Option<JoinHandle<()>>) {
    if let Some(running_task) = task.take() {
        running_task.abort();
    }
}

/// The capture for Claude's 1-based screen number, falling back to the cursor screen.
fn screen_capture_for_screen_number(screen_number: Option<usize>, screen_captures: &[ScreenCapture]) -> Option<&ScreenCapture> {
    if let Some(screen_number) = screen_number {
        if screen_number >= 1 && screen_number <= screen_captures.len() {
            return Some(&screen_captures[screen_number - 1]);
        }
    }
    screen_captures.iter().find(|capture| capture.is_cursor_screen)
}

fn bounding_box_of_stroke_padded(stroke_points: &[(i32, i32)], padding: i32) -> Option<PhysicalRect> {
    let minimum_x = stroke_points.iter().map(|point| point.0).min()?;
    let maximum_x = stroke_points.iter().map(|point| point.0).max()?;
    let minimum_y = stroke_points.iter().map(|point| point.1).min()?;
    let maximum_y = stroke_points.iter().map(|point| point.1).max()?;
    Some(PhysicalRect {
        x: minimum_x - padding,
        y: minimum_y - padding,
        width: (maximum_x - minimum_x + 2 * padding) as u32,
        height: (maximum_y - minimum_y + 2 * padding) as u32,
    })
}

fn padded_rect(rect: PhysicalRect, padding: i32) -> PhysicalRect {
    PhysicalRect {
        x: rect.x - padding,
        y: rect.y - padding,
        width: (rect.width as i32 + 2 * padding).max(0) as u32,
        height: (rect.height as i32 + 2 * padding).max(0) as u32,
    }
}

fn rect_contains_point(rect: PhysicalRect, point_x: i32, point_y: i32) -> bool {
    point_x >= rect.x && point_x < rect.x + rect.width as i32 && point_y >= rect.y && point_y < rect.y + rect.height as i32
}

fn labeled_images_for_captures(screen_captures: &[ScreenCapture]) -> Vec<LabeledImage> {
    screen_captures
        .iter()
        .map(|capture| LabeledImage {
            jpeg_bytes: capture.jpeg_bytes.clone(),
            label: prompts::image_label_with_dimensions(&capture.label, capture.screenshot_width_in_pixels, capture.screenshot_height_in_pixels),
        })
        .collect()
}

async fn capture_all_screens_off_the_async_threads(app: &AppHandle) -> Result<Vec<ScreenCapture>, String> {
    let app_for_capture = app.clone();
    tokio::task::spawn_blocking(move || screen_capture::capture_all_screens(&app_for_capture))
        .await
        .map_err(|join_error| format!("screen capture task failed: {join_error}"))?
}

impl CompanionBrain {
    /// Also creates the `PushToTalkRecorder` (its audio level goes to the overlays as `audio-level`).
    pub fn new(
        app: AppHandle,
        settings: SharedSettings,
        speech_player: Arc<SpeechPlayer>,
        whisper_transcriber: Arc<WhisperTranscriber>,
        board_panel: Arc<BoardPanel>,
        model3d_window: Arc<Model3DWindow>,
    ) -> Arc<Self> {
        let app_for_audio_level = app.clone();
        let push_to_talk_recorder = PushToTalkRecorder::new(Box::new(move |audio_level| {
            emit_to_overlays(&app_for_audio_level, "audio-level", json!({ "level": audio_level }));
        }));
        Arc::new(CompanionBrain {
            app,
            settings,
            speech_player,
            whisper_transcriber,
            board_panel,
            model3d_window,
            push_to_talk_recorder: Mutex::new(push_to_talk_recorder),
            state: Mutex::new(BrainState {
                voice_state: VoiceState::Idle,
                microphone_working: None,
                is_recording: false,
                is_transcribing: false,
                response_task: None,
                visual_task: None,
                screen_circle_fade_task: None,
                onboarding_prompt_task: None,
                conversation_history: ConversationHistory::new(),
                user_drawn_strokes: Vec::new(),
                user_drawn_strokes_awaiting_request: false,
            }),
        })
    }

    fn set_voice_state(&self, voice_state: VoiceState) {
        self.state.lock().unwrap().voice_state = voice_state;
        let _ = self.app.emit("buddy-voice-state", json!({ "state": voice_state }));
    }

    pub fn microphone_working(&self) -> Option<bool> {
        self.state.lock().unwrap().microphone_working
    }

    pub fn is_listening(&self) -> bool {
        self.state.lock().unwrap().voice_state == VoiceState::Listening
    }

    fn claude_model(&self) -> String {
        self.settings.read().unwrap().claude_model.clone()
    }

    fn system_prompt_file(&self, kind: SystemPromptKind) -> std::path::PathBuf {
        prompts::prompt_file_path(&app_paths::prompts_directory(&self.app), kind)
    }

    async fn wait_until_not_speaking(&self) {
        while self.speech_player.is_speaking() {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    fn emit_point_at(&self, global_point: (i32, i32), bubble_text: Option<String>) {
        emit_to_overlays(&self.app, "point-at", json!({ "x": global_point.0, "y": global_point.1, "bubbleText": bubble_text }));
    }

    fn hide_onboarding_prompt(&self) {
        let onboarding_prompt_was_showing = {
            let mut state = self.state.lock().unwrap();
            let was_showing = state.onboarding_prompt_task.is_some();
            abort_task(&mut state.onboarding_prompt_task);
            was_showing
        };
        if onboarding_prompt_was_showing {
            emit_to_overlays(&self.app, "onboarding-text", json!({ "text": "", "visible": false }));
        }
    }

    /// Handles press/release/Escape until the sender is dropped.
    pub async fn run_hotkey_loop(self: Arc<Self>, mut hotkey_receiver: UnboundedReceiver<HotkeyEvent>) {
        while let Some(hotkey_event) = hotkey_receiver.recv().await {
            match hotkey_event {
                HotkeyEvent::PushToTalkPressed => self.handle_push_to_talk_pressed(),
                HotkeyEvent::PushToTalkReleased => self.handle_push_to_talk_released(),
                HotkeyEvent::PushToTalkCancelled => self.handle_push_to_talk_cancelled(),
                HotkeyEvent::EscapePressed => self.model3d_window.handle_escape_pressed(),
            }
        }
    }

    fn handle_push_to_talk_pressed(self: &Arc<Self>) {
        {
            let mut state = self.state.lock().unwrap();
            if state.is_recording || state.is_transcribing {
                return;
            }
            // Cancel any in-progress response, narrated explanation and fades so nothing talks over the user.
            abort_task(&mut state.response_task);
            abort_task(&mut state.visual_task);
            abort_task(&mut state.screen_circle_fade_task);
            state.user_drawn_strokes.clear();
            state.user_drawn_strokes_awaiting_request = false;
            state.is_recording = true;
        }
        // The settings panel would cover the screen.
        tray_and_settings::hide_settings_window(&self.app);
        self.speech_player.stop_playback();
        self.board_panel.finish_live_content();
        self.board_panel.stop_drawing();
        self.model3d_window.close();
        emit_to_overlays(&self.app, "clear-annotations", json!({}));
        // While the hotkey is held, a mouse drag draws on the overlay instead of clicking the app underneath.
        overlay_windows::set_overlays_accept_user_drawing(&self.app, true);
        self.hide_onboarding_prompt();

        let start_result = self.push_to_talk_recorder.lock().unwrap().start_recording();
        self.state.lock().unwrap().microphone_working = Some(start_result.is_ok());
        match start_result {
            Ok(()) => self.set_voice_state(VoiceState::Listening),
            Err(recording_error) => {
                eprintln!("push to talk: couldn't start recording: {recording_error}");
                self.state.lock().unwrap().is_recording = false;
                self.set_voice_state(VoiceState::Idle);
            }
        }
    }

    /// The chord turned out to be part of a Windows shortcut (win+ctrl+left, ...):
    /// drop the recording and the strokes, nothing is transcribed or sent.
    fn handle_push_to_talk_cancelled(&self) {
        {
            let mut state = self.state.lock().unwrap();
            if !state.is_recording {
                return;
            }
            state.is_recording = false;
            state.user_drawn_strokes.clear();
            state.user_drawn_strokes_awaiting_request = false;
        }
        let _discarded_samples = self.push_to_talk_recorder.lock().unwrap().stop_recording();
        overlay_windows::set_overlays_accept_user_drawing(&self.app, false);
        emit_to_overlays(&self.app, "clear-annotations", json!({}));
        self.set_voice_state(VoiceState::Idle);
    }

    fn handle_push_to_talk_released(self: &Arc<Self>) {
        let has_user_drawn_strokes = {
            let mut state = self.state.lock().unwrap();
            if !state.is_recording {
                overlay_windows::set_overlays_accept_user_drawing(&self.app, false);
                return;
            }
            state.is_recording = false;
            state.is_transcribing = true;
            // Speech is optional when the user draws: no transcript means "What is this?".
            state.user_drawn_strokes_awaiting_request = !state.user_drawn_strokes.is_empty();
            !state.user_drawn_strokes.is_empty()
        };
        let recorded_samples_16k = self.push_to_talk_recorder.lock().unwrap().stop_recording();
        overlay_windows::set_overlays_accept_user_drawing(&self.app, false);
        self.set_voice_state(VoiceState::Processing);

        let brain = self.clone();
        tauri::async_runtime::spawn(async move {
            let transcript = if brain.whisper_transcriber.state() == SpeechModelState::Ready {
                match brain.whisper_transcriber.transcribe(recorded_samples_16k).await {
                    Ok(transcript) => transcript,
                    Err(transcription_error) => {
                        eprintln!("transcription failed: {transcription_error}");
                        String::new()
                    }
                }
            } else {
                if !has_user_drawn_strokes {
                    brain.speech_player.speak_text(SPEECH_MODEL_NOT_READY_MESSAGE.to_string());
                }
                String::new()
            };

            let should_send_drawing_alone = {
                let mut state = brain.state.lock().unwrap();
                state.is_transcribing = false;
                state.user_drawn_strokes_awaiting_request
            };
            let trimmed_transcript = transcript.trim();
            if !trimmed_transcript.is_empty() {
                eprintln!("transcript: {trimmed_transcript}");
                brain.send_transcript(trimmed_transcript.to_string());
            } else if should_send_drawing_alone {
                brain.send_transcript(DRAWING_WITHOUT_SPEECH_PROMPT.to_string());
            } else {
                brain.set_voice_state(VoiceState::Idle);
            }
        });
    }

    /// Runs the agent loop for one user message (transcript or "What is this?").
    pub fn send_transcript(self: &Arc<Self>, transcript: String) {
        let language_note = prompts::reply_language_note(&transcript);
        let user_circled_regions: Vec<PhysicalRect> = {
            let mut state = self.state.lock().unwrap();
            abort_task(&mut state.response_task);
            abort_task(&mut state.visual_task);
            state.user_drawn_strokes_awaiting_request = false;
            // Strokes are cleared on every hotkey press, so any left belong to this
            // utterance. Pad each by 8 px so a single click still makes a region.
            state
                .user_drawn_strokes
                .iter()
                .filter_map(|stroke_points| bounding_box_of_stroke_padded(stroke_points, 8))
                .collect()
        };
        self.speech_player.stop_playback();

        let brain = self.clone();
        let response_task = tauri::async_runtime::spawn(async move {
            let agent_loop_result = brain.run_agent_loop(&transcript, language_note, &user_circled_regions).await;
            if let Err(claude_error) = agent_loop_result {
                eprintln!("companion response error: {claude_error}");
                brain.speech_player.speak_text(CLAUDE_UNREACHABLE_MESSAGE.to_string());
                brain.set_voice_state(VoiceState::Responding);
            }
            // Only reached when not aborted (an abort drops this future first).
            brain.set_voice_state(VoiceState::Idle);
        });
        self.state.lock().unwrap().response_task = Some(response_task);
    }

    async fn run_agent_loop(
        self: &Arc<Self>,
        transcript: &str,
        language_note: &str,
        user_circled_regions: &[PhysicalRect],
    ) -> Result<(), String> {
        let mut previous_step_action_summaries: Vec<String> = Vec::new();
        for agent_step_number in 1..=MAXIMUM_AGENT_STEPS {
            self.set_voice_state(VoiceState::Processing);
            let screen_captures = capture_all_screens_off_the_async_threads(&self.app).await?;
            let labeled_images = labeled_images_for_captures(&screen_captures);
            let history_entries = self.state.lock().unwrap().conversation_history.entries();

            // Round one carries what the user said; later rounds show Claude the screen after its last actions.
            let mut user_prompt_for_this_step = if agent_step_number == 1 {
                format!("{transcript}\n{language_note}")
            } else {
                prompts::AGENT_CONTINUATION_PROMPT.to_string()
            };

            // Regions the user drew around: tell Claude where they are and attach a native-resolution crop of each.
            let mut images_for_this_step = labeled_images.clone();
            if agent_step_number == 1 {
                for user_circled_region in user_circled_regions {
                    let region_center_x = user_circled_region.x + user_circled_region.width as i32 / 2;
                    let region_center_y = user_circled_region.y + user_circled_region.height as i32 / 2;
                    let Some(screen_index) = screen_captures
                        .iter()
                        .position(|capture| rect_contains_point(capture.monitor.rect, region_center_x, region_center_y))
                    else {
                        continue;
                    };
                    let pixel_rect = screen_mapping::screenshot_pixel_rect_for_global_rect(*user_circled_region, &screen_captures[screen_index]);
                    user_prompt_for_this_step.push('\n');
                    user_prompt_for_this_step.push_str(&prompts::user_circled_region_note(screen_index + 1, pixel_rect));
                    let crop_rect = padded_rect(*user_circled_region, 24);
                    let crop_result = tokio::task::spawn_blocking(move || screen_capture::capture_region_jpeg(crop_rect)).await;
                    if let Ok(Ok(region_crop_jpeg)) = crop_result {
                        images_for_this_step.push(LabeledImage {
                            jpeg_bytes: region_crop_jpeg,
                            label: prompts::user_circled_region_crop_label(screen_index + 1),
                        });
                    }
                }
            }

            let system_prompt_file = self.system_prompt_file(SystemPromptKind::VoiceResponse);
            let full_response_text = run_claude_streaming(
                ClaudeRequest {
                    images: images_for_this_step,
                    system_prompt_file: &system_prompt_file,
                    history: history_entries,
                    user_prompt: user_prompt_for_this_step.clone(),
                    allows_computer_tools: true,
                    effort: None,
                    model: self.claude_model(),
                },
                |_accumulated_reply_text| {},
            )
            .await?;
            eprintln!("claude replied: {}", full_response_text.chars().take(400).collect::<String>());

            let parsed_reply = parse_reply(&full_response_text);
            let spoken_text = parsed_reply.spoken_text.clone();

            // [CLOSE_DRAWING] dismisses the whiteboard (and stops any narration on it).
            if parsed_reply.close_drawing {
                abort_task(&mut self.state.lock().unwrap().visual_task);
                self.board_panel.close();
                self.model3d_window.close();
            }
            if parsed_reply.show_request.is_some() || parsed_reply.notes_request.is_some() {
                self.model3d_window.close();
            }
            if let Some(model3d_request) = &parsed_reply.model3d_request {
                // The model sits over the thing it shows; it needs the screenshots to see what it looks like.
                if let Some(target_screen_capture) = screen_capture_for_screen_number(model3d_request.screen_number, &screen_captures) {
                    let screenshot_center = (
                        target_screen_capture.screenshot_width_in_pixels as f64 / 2.0,
                        target_screen_capture.screenshot_height_in_pixels as f64 / 2.0,
                    );
                    self.start_narrated_model3d(
                        model3d_request.description.clone(),
                        transcript.to_string(),
                        spoken_text.clone(),
                        labeled_images.clone(),
                        target_screen_capture.clone(),
                        model3d_request.screenshot_point.unwrap_or(screenshot_center),
                    );
                }
            } else if let Some(show_request) = &parsed_reply.show_request {
                self.start_narrated_panel(LiveContentKind::Drawing, show_request.clone(), transcript.to_string(), spoken_text.clone(), Vec::new());
            } else if let Some(notes_request) = &parsed_reply.notes_request {
                // Notes summarize what's on screen, so they need the screenshots.
                self.start_narrated_panel(LiveContentKind::Notes, notes_request.clone(), transcript.to_string(), spoken_text.clone(), labeled_images.clone());
            }

            // Switch to idle BEFORE pointing so the buddy is visible and can fly to the target.
            if let Some((point_x, point_y)) = parsed_reply.point.point {
                if let Some(target_screen_capture) = screen_capture_for_screen_number(parsed_reply.point.screen_number, &screen_captures) {
                    self.set_voice_state(VoiceState::Idle);
                    let global_point = screen_mapping::global_point_for_screenshot_point(point_x, point_y, target_screen_capture);
                    self.emit_point_at(global_point, None);
                }
            }

            // Remember what was said and done so later rounds and follow-ups like "yes, send it" have the context.
            let action_summaries: Vec<String> = parsed_reply.actions.iter().map(ComputerAction::summary).collect();
            let action_summary_text = if action_summaries.is_empty() {
                String::new()
            } else {
                format!(" (did: {})", action_summaries.join(", "))
            };
            self.state.lock().unwrap().conversation_history.append(ConversationEntry {
                user_text: user_prompt_for_this_step,
                assistant_response: format!("{spoken_text}{action_summary_text}"),
            });
            eprintln!("step {agent_step_number}: {} on-screen action(s)", parsed_reply.actions.len());

            if !spoken_text.trim().is_empty() {
                // Let the previous round's narration finish rather than cutting it off.
                self.wait_until_not_speaking().await;
                self.speech_player.speak_text(spoken_text.clone());
                self.set_voice_state(VoiceState::Responding);
            }
            self.show_screen_circles_while_speaking(&parsed_reply.circles, &screen_captures);

            // No on-screen actions means the task is finished; any tool work already happened inside claude.
            if parsed_reply.actions.is_empty() {
                break;
            }
            // Same actions as last round means the last attempt didn't change anything. Stop instead of looping.
            if action_summaries == previous_step_action_summaries {
                eprintln!("agent repeated the same actions, stopping");
                self.wait_until_not_speaking().await;
                self.speech_player.speak_text(REPEATED_ACTIONS_MESSAGE.to_string());
                break;
            }
            previous_step_action_summaries = action_summaries;

            self.perform_computer_actions(&parsed_reply.actions, &screen_captures).await;
            // Let the UI settle before the next screenshot.
            tokio::time::sleep(Duration::from_millis(800)).await;
        }
        Ok(())
    }

    /// Replays Claude's [CLICK]/[TYPE]/[KEY] actions in order. Runs inside the
    /// response task, so pressing the hotkey again cancels whatever is left.
    async fn perform_computer_actions(&self, computer_actions: &[ComputerAction], screen_captures: &[ScreenCapture]) {
        // Screenshots leave the orb out, so a click may aim at something behind it.
        let _orb_click_through_guard = OrbClickThroughWhileActing::start(&self.app);
        for computer_action in computer_actions {
            match computer_action {
                ComputerAction::Click { screenshot_x, screenshot_y, label, screen_number } => {
                    let Some(target_screen_capture) = screen_capture_for_screen_number(*screen_number, screen_captures) else {
                        continue;
                    };
                    let click_location = screen_mapping::global_point_for_screenshot_point(*screenshot_x, *screenshot_y, target_screen_capture);
                    // Fly the buddy to the target first so the user sees what's about to be clicked (and can interrupt).
                    self.emit_point_at(click_location, label.clone());
                    tokio::time::sleep(Duration::from_millis(1200)).await;
                    computer_control::perform_click(click_location.0, click_location.1).await;
                }
                ComputerAction::Type { text } => computer_control::perform_type(text).await,
                ComputerAction::PressKey { key_combo } => {
                    computer_control::perform_key_combo(key_combo).await;
                }
                ComputerAction::Open { target } => computer_control::perform_open(target, &home_directory()).await,
            }
            // Give the target app a moment to react before the next action lands.
            tokio::time::sleep(Duration::from_millis(400)).await;
        }
    }

    /// Shows the circles ~1.2 s apart while the reply is spoken, then fades them
    /// (and the user's own strokes) once speech ends and every circle has been drawn.
    fn show_screen_circles_while_speaking(self: &Arc<Self>, circle_tags: &[CircleTag], screen_captures: &[ScreenCapture]) {
        let has_user_drawn_strokes = {
            let mut state = self.state.lock().unwrap();
            abort_task(&mut state.screen_circle_fade_task);
            !state.user_drawn_strokes.is_empty()
        };
        if circle_tags.is_empty() && !has_user_drawn_strokes {
            return;
        }

        let mut circle_annotations = Vec::new();
        for circle_tag in circle_tags {
            let Some(target_screen_capture) = screen_capture_for_screen_number(circle_tag.screen_number, screen_captures) else {
                continue;
            };
            let top_left = screen_mapping::global_point_for_screenshot_point(circle_tag.screenshot_x, circle_tag.screenshot_y, target_screen_capture);
            let bottom_right = screen_mapping::global_point_for_screenshot_point(
                circle_tag.screenshot_x + circle_tag.width,
                circle_tag.screenshot_y + circle_tag.height,
                target_screen_capture,
            );
            // Speech starts a moment after speak_text (the first sentence is synthesized first).
            let appear_delay_seconds = 0.3 + circle_annotations.len() as f64 * 1.2;
            circle_annotations.push(json!({
                "x": top_left.0,
                "y": top_left.1,
                "width": bottom_right.0 - top_left.0,
                "height": bottom_right.1 - top_left.1,
                "appearDelaySeconds": appear_delay_seconds,
            }));
        }
        let last_circle_appear_delay_seconds = if circle_annotations.is_empty() { 0.0 } else { 0.3 + (circle_annotations.len() - 1) as f64 * 1.2 };
        emit_to_overlays(&self.app, "screen-circles", json!({ "circles": circle_annotations }));

        let brain = self.clone();
        let fade_task = tauri::async_runtime::spawn(async move {
            let earliest_fade_time = Instant::now() + Duration::from_secs_f64(last_circle_appear_delay_seconds + 1.5);
            while brain.speech_player.is_speaking() || Instant::now() < earliest_fade_time {
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            brain.state.lock().unwrap().user_drawn_strokes.clear();
            emit_to_overlays(&brain.app, "fade-annotations", json!({}));
        });
        self.state.lock().unwrap().screen_circle_fade_task = Some(fade_task);
    }

    /// Narrates a whiteboard sketch or live notes with a separate claude call. Each
    /// step's lines appear while its sentence is spoken; the next step waits for the speech.
    fn start_narrated_panel(
        self: &Arc<Self>,
        live_content_kind: LiveContentKind,
        content_description: String,
        user_request: String,
        spoken_intro: String,
        screen_images: Vec<LabeledImage>,
    ) {
        abort_task(&mut self.state.lock().unwrap().visual_task);
        self.board_panel.begin_live_content(&self.app, live_content_kind);
        eprintln!("{:?}: {content_description}", live_content_kind);

        let board_panel_for_drawing = self.board_panel.clone();
        let narrated_step_queue = NarratedStepQueue::new(Box::new(move |drawing_command_line| {
            board_panel_for_drawing.add_command_line(drawing_command_line)
        }));
        let system_prompt_kind = match live_content_kind {
            LiveContentKind::Drawing => SystemPromptKind::VisualExplanation,
            LiveContentKind::Notes => SystemPromptKind::LiveNotes,
        };
        let visual_request_prompt = prompts::narrated_panel_request_prompt(live_content_kind, &content_description, &user_request, &spoken_intro);

        let brain = self.clone();
        let visual_task = tauri::async_runtime::spawn(async move {
            let narrations_spoken = brain
                .stream_narrated_script(narrated_step_queue.clone(), system_prompt_kind, visual_request_prompt, screen_images)
                .await;
            brain.board_panel.finish_live_content();

            // Keep what was explained (and the notes text) so follow-ups like "save these notes" have the context.
            if !narrations_spoken.is_empty() {
                let mut explained_content = format!(" {}", narrations_spoken.join(" "));
                if live_content_kind == LiveContentKind::Notes {
                    explained_content.push_str(&format!("\n(notes written:\n{})", narrated_step_queue.plain_text_of_visible_lines()));
                }
                brain.state.lock().unwrap().conversation_history.append_to_last_response(&explained_content);
            }
        });
        self.state.lock().unwrap().visual_task = Some(visual_task);
    }

    /// Builds a narrated 3D model over the screen with a separate claude call.
    fn start_narrated_model3d(
        self: &Arc<Self>,
        model_description: String,
        user_request: String,
        spoken_intro: String,
        screen_images: Vec<LabeledImage>,
        target_screen_capture: ScreenCapture,
        focus_screenshot_point: (f64, f64),
    ) {
        abort_task(&mut self.state.lock().unwrap().visual_task);
        // One visual at a time: the model replaces any whiteboard or notes.
        self.board_panel.close();
        // Esc or the model's close button also stops the narration about it.
        let weak_brain: Weak<CompanionBrain> = Arc::downgrade(self);
        self.model3d_window.set_on_user_closed(Box::new(move || {
            if let Some(brain) = weak_brain.upgrade() {
                abort_task(&mut brain.state.lock().unwrap().visual_task);
                brain.speech_player.stop_playback();
            }
        }));
        self.model3d_window.begin(
            &self.app,
            &target_screen_capture.monitor,
            target_screen_capture.screenshot_width_in_pixels,
            target_screen_capture.screenshot_height_in_pixels,
            focus_screenshot_point.0,
            focus_screenshot_point.1,
        );
        eprintln!("building 3D model: {model_description}");

        let model3d_window_for_drawing = self.model3d_window.clone();
        let narrated_step_queue = NarratedStepQueue::new(Box::new(move |model_command_line| {
            model3d_window_for_drawing.add_command_line(model_command_line)
        }));
        let model3d_request_prompt = prompts::model3d_request_prompt(
            &model_description,
            &target_screen_capture.label,
            target_screen_capture.screenshot_width_in_pixels,
            target_screen_capture.screenshot_height_in_pixels,
            focus_screenshot_point.0,
            focus_screenshot_point.1,
            &user_request,
            &spoken_intro,
        );

        let brain = self.clone();
        let visual_task = tauri::async_runtime::spawn(async move {
            let narrations_spoken = brain
                .stream_narrated_script(narrated_step_queue, SystemPromptKind::Model3D, model3d_request_prompt, screen_images)
                .await;
            if !narrations_spoken.is_empty() {
                let explained_content = format!(" {}", narrations_spoken.join(" "));
                brain.state.lock().unwrap().conversation_history.append_to_last_response(&explained_content);
            }
        });
        self.state.lock().unwrap().visual_task = Some(visual_task);
    }

    /// Streams the side call's JSON lines into the step queue while the queue plays
    /// steps (both at once, like the Mac's `async let`). Returns the spoken narrations.
    async fn stream_narrated_script(
        &self,
        narrated_step_queue: NarratedStepQueue,
        system_prompt_kind: SystemPromptKind,
        request_prompt: String,
        screen_images: Vec<LabeledImage>,
    ) -> Vec<String> {
        let system_prompt_file = self.system_prompt_file(system_prompt_kind);
        let queue_for_streaming = narrated_step_queue.clone();
        let stream_script = async {
            let mut streamed_line_forwarder = StreamedLineForwarder::new();
            let script_result = run_claude_streaming(
                ClaudeRequest {
                    images: screen_images,
                    system_prompt_file: &system_prompt_file,
                    history: Vec::new(),
                    user_prompt: request_prompt,
                    allows_computer_tools: false,
                    // Panels stream structured lines; low effort keeps the first line arriving fast.
                    effort: Some("low"),
                    model: self.claude_model(),
                },
                |accumulated_script| {
                    for complete_line in streamed_line_forwarder.new_complete_lines(accumulated_script) {
                        queue_for_streaming.receive_line(&complete_line);
                    }
                },
            )
            .await;
            match script_result {
                Ok(full_script) => {
                    // The last line has no trailing newline, so it's only complete now.
                    for remaining_line in streamed_line_forwarder.remaining_lines(&full_script) {
                        queue_for_streaming.receive_line(&remaining_line);
                    }
                    eprintln!("narrated script streamed:\n{full_script}");
                }
                Err(script_error) => eprintln!("narrated script failed: {script_error}"),
            }
            queue_for_streaming.mark_stream_finished();
        };
        let (_, narrations_spoken) = tokio::join!(stream_script, narrated_step_queue.play_steps(self.speech_player.clone()));
        narrations_spoken
    }

    /// A freehand stroke the user drew while holding the hotkey, global physical pixels.
    pub fn add_user_stroke(&self, points: Vec<(i32, i32)>) {
        if points.is_empty() {
            return;
        }
        self.state.lock().unwrap().user_drawn_strokes.push(points);
    }

    /// First launch or "replay onboarding": point at something on screen, then show the hotkey hint.
    pub fn start_onboarding(self: &Arc<Self>) {
        {
            let mut current_settings = self.settings.write().unwrap();
            current_settings.has_completed_onboarding = true;
            settings::save_settings(&self.app, &current_settings);
        }
        self.perform_onboarding_demo_interaction();

        let brain = self.clone();
        let onboarding_prompt_task = tauri::async_runtime::spawn(async move {
            // The overlay types the welcome out itself; the demo's claude call runs meanwhile.
            emit_to_overlays(&brain.app, "onboarding-text", json!({ "text": ONBOARDING_WELCOME_MESSAGE, "visible": true }));
            tokio::time::sleep(Duration::from_secs(3)).await;
            emit_to_overlays(&brain.app, "onboarding-text", json!({ "text": "", "visible": false }));
            tokio::time::sleep(Duration::from_secs(5)).await;
            let shortcut_display_text = brain.settings.read().unwrap().push_to_talk_shortcut.display_text();
            let onboarding_message = format!("press {shortcut_display_text} and introduce yourself");
            let mut streamed_text = String::new();
            for character in onboarding_message.chars() {
                streamed_text.push(character);
                emit_to_overlays(&brain.app, "onboarding-text", json!({ "text": streamed_text, "visible": true }));
                tokio::time::sleep(Duration::from_millis(30)).await;
            }
            // Auto-dismiss after 10 seconds.
            tokio::time::sleep(Duration::from_secs(10)).await;
            emit_to_overlays(&brain.app, "onboarding-text", json!({ "text": "", "visible": false }));
            brain.state.lock().unwrap().onboarding_prompt_task = None;
        });
        let mut state = self.state.lock().unwrap();
        abort_task(&mut state.onboarding_prompt_task);
        state.onboarding_prompt_task = Some(onboarding_prompt_task);
    }

    /// Asks Claude to find something interesting on the cursor screen and points at it,
    /// with Claude's comment as the bubble text.
    fn perform_onboarding_demo_interaction(self: &Arc<Self>) {
        let current_voice_state = self.state.lock().unwrap().voice_state;
        // Don't interrupt an active voice response.
        if current_voice_state != VoiceState::Idle && current_voice_state != VoiceState::Responding {
            return;
        }
        let brain = self.clone();
        tauri::async_runtime::spawn(async move {
            let onboarding_result: Result<(), String> = async {
                let screen_captures = capture_all_screens_off_the_async_threads(&brain.app).await?;
                // Only the cursor screen, so Claude can't pick something on a monitor we can't point at.
                let cursor_screen_capture = screen_captures
                    .iter()
                    .find(|capture| capture.is_cursor_screen)
                    .ok_or("no cursor screen found")?;
                let system_prompt_file = brain.system_prompt_file(SystemPromptKind::OnboardingDemo);
                let full_response_text = run_claude_streaming(
                    ClaudeRequest {
                        images: labeled_images_for_captures(std::slice::from_ref(cursor_screen_capture)),
                        system_prompt_file: &system_prompt_file,
                        history: Vec::new(),
                        user_prompt: ONBOARDING_USER_PROMPT.to_string(),
                        allows_computer_tools: false,
                        effort: None,
                        model: brain.claude_model(),
                    },
                    |_accumulated_reply_text| {},
                )
                .await?;
                let point_tag = parse_point_tag(&full_response_text);
                let (point_x, point_y) = point_tag.point.ok_or("no element to point at")?;
                let global_point = screen_mapping::global_point_for_screenshot_point(point_x, point_y, cursor_screen_capture);
                brain.emit_point_at(global_point, Some(point_tag.spoken_text));
                Ok(())
            }
            .await;
            if let Err(onboarding_error) = onboarding_result {
                eprintln!("onboarding demo: {onboarding_error}");
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stroke_bounding_box_is_padded_so_a_single_click_still_makes_a_region() {
        assert_eq!(
            bounding_box_of_stroke_padded(&[(100, 200)], 8),
            Some(PhysicalRect { x: 92, y: 192, width: 16, height: 16 })
        );
        assert_eq!(
            bounding_box_of_stroke_padded(&[(10, 50), (30, 20), (20, 40)], 8),
            Some(PhysicalRect { x: 2, y: 12, width: 36, height: 46 })
        );
        assert_eq!(bounding_box_of_stroke_padded(&[], 8), None);
        assert_eq!(
            padded_rect(PhysicalRect { x: 92, y: 192, width: 16, height: 16 }, 24),
            PhysicalRect { x: 68, y: 168, width: 64, height: 64 }
        );
        assert!(rect_contains_point(PhysicalRect { x: -1920, y: 0, width: 1920, height: 1080 }, -1, 0));
        assert!(!rect_contains_point(PhysicalRect { x: -1920, y: 0, width: 1920, height: 1080 }, 0, 0));
    }
}
