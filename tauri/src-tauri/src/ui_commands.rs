//! Commands the settings page calls. They change settings (saved at once and
//! announced to every window as `settings-changed`) and forward to the speech
//! player, autostart plugin, shortcut recorder and brain.

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, State};
use tauri_plugin_autostart::ManagerExt;

use crate::app_state::AppState;
use crate::brain_engine::{BrainEngine, CODEX_REASONING_EFFORTS};
use crate::claude_cli::find_claude_executable;
use crate::codex_cli::find_codex_executable;
use crate::platform;
use crate::push_to_talk::{set_recording_new_shortcut, PushToTalkShortcutOption, ShortcutRecorderUpdate};
use crate::chat_panel::ChatPanelSide;
use crate::orb_window::{clamped_orb_side_length, VoiceBuddyStyle, MAXIMUM_ORB_COLOR};
use crate::settings::{save_settings, Settings};
use crate::speech_player::NeuralVoiceLoadState;
use crate::supertonic::SupertonicVoice;
use crate::tray_and_settings::hide_settings_window;
use crate::whisper_transcriber::SpeechModelState;

/// The model aliases `claude -p --model` accepts, as on the Mac.
pub const CLAUDE_MODEL_ALIASES: [&str; 4] = ["fable", "opus", "sonnet", "haiku"];

/// Every setting plus what the page derives from them. Returned by
/// `get_settings` and sent to every window as `settings-changed`, so the orb,
/// chat and overlay pages can follow bhai mode, voice style, orb size/colour,
/// chat side and reply settings live.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsSnapshot {
    #[serde(flatten)]
    pub settings: Settings,
    pub push_to_talk_display_text: String,
    pub push_to_talk_key_capsule_labels: Vec<String>,
    pub is_default_push_to_talk_shortcut: bool,
    /// For the recorder's reset button ("Back to ctrl + win").
    pub default_push_to_talk_shortcut: PushToTalkShortcutOption,
    pub default_push_to_talk_display_text: String,
    pub launch_at_login: bool,
}

/// Also pushed to the settings page as `status-changed` whenever a part changes.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StatusSnapshot {
    /// Whether the CLI of the selected brain (claude or codex) was found.
    pub brain_cli_found: bool,
    /// Human-readable speech-to-text model state, e.g. "downloading 42%" or "ready".
    pub speech_model: String,
    /// Human-readable voice state.
    pub voice: String,
    /// None until the microphone has been tried once.
    pub microphone_working: Option<bool>,
    pub is_listening: bool,
    pub is_speaking: bool,
    pub version: String,
    /// Platform-specific limits the panel should explain (e.g. Linux on
    /// Wayland), or None when everything works.
    pub platform_limitation_note: Option<String>,
}

fn percent_text(fraction: f32) -> String {
    format!("{}%", (fraction.clamp(0.0, 1.0) * 100.0).round() as u32)
}

pub fn speech_model_status_text(speech_model_state: &SpeechModelState) -> String {
    match speech_model_state {
        SpeechModelState::Downloading { fraction } => format!("downloading {}", percent_text(*fraction)),
        SpeechModelState::Starting => "starting".to_string(),
        SpeechModelState::Ready => "ready".to_string(),
        SpeechModelState::Failed(failure_reason) => format!("failed: {failure_reason}"),
    }
}

pub fn voice_status_text(neural_voice_load_state: &NeuralVoiceLoadState) -> String {
    match neural_voice_load_state {
        NeuralVoiceLoadState::NotLoaded => "system voice".to_string(),
        NeuralVoiceLoadState::Downloading { fraction } => format!("downloading {}", percent_text(*fraction)),
        NeuralVoiceLoadState::Loading => "loading".to_string(),
        NeuralVoiceLoadState::Ready => "Supertonic-3".to_string(),
        NeuralVoiceLoadState::Failed(failure_reason) => format!("system voice (Supertonic failed: {failure_reason})"),
    }
}

/// One setting the panel changes, as `{name, value}` from the page.
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(tag = "name", content = "value", rename_all = "camelCase")]
pub enum SettingChange {
    ClaudeModel(String),
    BrainEngine(BrainEngine),
    /// null = the model's default.
    CodexReasoningEffort(Option<String>),
    SupertonicVoice(SupertonicVoice),
    IsChatInputMode(bool),
    SpeaksVoiceReplies(bool),
    SpeaksChatReplies(bool),
    ChatPanelSide(ChatPanelSide),
    VoiceBuddyStyle(VoiceBuddyStyle),
    OrbSideLength(f64),
    SelectedOrbColor(u8),
    PushToTalkShortcut(PushToTalkShortcutOption),
}

/// Applies a change after checking it; an invalid value changes nothing and
/// returns false. The page is trusted less than Rust: every value is checked.
pub fn apply_setting_change(settings: &mut Settings, setting_change: SettingChange) -> bool {
    match setting_change {
        SettingChange::ClaudeModel(model) => {
            if !CLAUDE_MODEL_ALIASES.contains(&model.as_str()) {
                return false;
            }
            settings.claude_model = model;
        }
        SettingChange::BrainEngine(brain_engine) => settings.brain_engine = brain_engine,
        SettingChange::CodexReasoningEffort(reasoning_effort) => {
            if reasoning_effort.as_deref().is_some_and(|effort| !CODEX_REASONING_EFFORTS.contains(&effort)) {
                return false;
            }
            settings.codex_reasoning_effort = reasoning_effort;
        }
        SettingChange::SupertonicVoice(voice) => settings.supertonic_voice = voice,
        SettingChange::IsChatInputMode(is_chat_input_mode) => settings.is_chat_input_mode = is_chat_input_mode,
        SettingChange::SpeaksVoiceReplies(speaks_voice_replies) => settings.speaks_voice_replies = speaks_voice_replies,
        SettingChange::SpeaksChatReplies(speaks_chat_replies) => settings.speaks_chat_replies = speaks_chat_replies,
        SettingChange::ChatPanelSide(chat_panel_side) => settings.chat_panel_side = chat_panel_side,
        SettingChange::VoiceBuddyStyle(voice_buddy_style) => settings.voice_buddy_style = voice_buddy_style,
        SettingChange::OrbSideLength(orb_side_length) => settings.orb_side_length = clamped_orb_side_length(orb_side_length),
        SettingChange::SelectedOrbColor(orb_color) => {
            if orb_color > MAXIMUM_ORB_COLOR {
                return false;
            }
            settings.selected_orb_color = orb_color;
        }
        SettingChange::PushToTalkShortcut(push_to_talk_shortcut) => {
            // Only modifier-only combinations of two or more keys can be recorded.
            if let PushToTalkShortcutOption::CustomModifiers(custom_modifiers) = push_to_talk_shortcut {
                if custom_modifiers.key_count() < 2 {
                    return false;
                }
            }
            settings.push_to_talk_shortcut = push_to_talk_shortcut;
        }
    }
    true
}

pub fn settings_snapshot(app: &AppHandle, settings: Settings) -> SettingsSnapshot {
    let push_to_talk_shortcut = settings.push_to_talk_shortcut;
    SettingsSnapshot {
        push_to_talk_display_text: push_to_talk_shortcut.display_text(),
        push_to_talk_key_capsule_labels: push_to_talk_shortcut.key_capsule_labels().iter().map(|key_label| key_label.to_string()).collect(),
        is_default_push_to_talk_shortcut: push_to_talk_shortcut == PushToTalkShortcutOption::default(),
        default_push_to_talk_shortcut: PushToTalkShortcutOption::default(),
        default_push_to_talk_display_text: PushToTalkShortcutOption::default().display_text(),
        launch_at_login: app.autolaunch().is_enabled().unwrap_or(false),
        settings,
    }
}

/// Saves the settings file and tells every window. Every settings change goes
/// through here (the settings page, the chat's own buttons, the shortcut recorder).
pub fn save_and_announce_settings(app: &AppHandle, updated_settings: Settings) {
    save_settings(app, &updated_settings);
    let _ = app.emit("settings-changed", settings_snapshot(app, updated_settings));
}

#[tauri::command]
pub fn get_settings(app: AppHandle, state: State<'_, AppState>) -> SettingsSnapshot {
    let settings = state.settings.read().unwrap().clone();
    settings_snapshot(&app, settings)
}

/// Async because switching to chat mode may create the chat window, which
/// deadlocks inside a synchronous command on Windows.
#[tauri::command]
pub async fn change_setting(change: SettingChange, app: AppHandle) {
    let state: State<'_, AppState> = tauri::Manager::state(&app);
    let updated_settings = {
        let mut settings = state.settings.write().unwrap();
        if !apply_setting_change(&mut settings, change.clone()) {
            eprintln!("settings: ignoring invalid change {change:?}");
            return;
        }
        settings.clone()
    };
    save_and_announce_settings(&app, updated_settings);
    // Apply the change live where it shows (Mac: the managers observe their settings).
    match change {
        SettingChange::SupertonicVoice(voice) => state.speech_player.set_voice(voice),
        SettingChange::IsChatInputMode(is_chat_input_mode) => state.chat_panel.apply_chat_input_mode(is_chat_input_mode),
        SettingChange::ChatPanelSide(_) => state.chat_panel.apply_chat_panel_side(),
        // The chat shows the model name and the replies toggle.
        SettingChange::ClaudeModel(_) | SettingChange::BrainEngine(_) | SettingChange::SpeaksChatReplies(_) => {
            state.chat_panel.emit_chat_state()
        }
        SettingChange::VoiceBuddyStyle(_) => state.orb_window.apply_voice_buddy_style(),
        SettingChange::OrbSideLength(_) => state.orb_window.apply_orb_size(),
        SettingChange::SelectedOrbColor(_) => state.orb_window.apply_orb_color(),
        SettingChange::CodexReasoningEffort(_) | SettingChange::SpeaksVoiceReplies(_) | SettingChange::PushToTalkShortcut(_) => {}
    }
}

#[tauri::command]
pub fn get_status(app: AppHandle, state: State<'_, AppState>) -> StatusSnapshot {
    let brain_engine = state.settings.read().unwrap().brain_engine;
    let brain_cli_found = match brain_engine {
        BrainEngine::Claude => find_claude_executable().is_some(),
        BrainEngine::Codex => find_codex_executable().is_some(),
    };
    StatusSnapshot {
        brain_cli_found,
        speech_model: speech_model_status_text(&state.whisper_transcriber.state()),
        voice: voice_status_text(&state.speech_player.load_state()),
        microphone_working: state.companion_brain.microphone_working(),
        is_listening: state.companion_brain.is_listening(),
        is_speaking: state.speech_player.is_speaking(),
        version: app.package_info().version.to_string(),
        platform_limitation_note: platform::platform_limitation_note(),
    }
}

/// Plays a voice sample, or stops whatever is speaking (the Mac's preview button).
#[tauri::command]
pub fn preview_voice(voice: SupertonicVoice, state: State<'_, AppState>) {
    if state.speech_player.is_speaking() {
        state.speech_player.stop_playback();
    } else {
        state.speech_player.preview_voice(voice);
    }
}

/// The page's recorder: the keyboard hook feeds keys to a `ShortcutRecorder`
/// until a combination is recorded, Esc is pressed or this is called with false.
#[tauri::command]
pub fn set_shortcut_recording(is_recording: bool) {
    set_recording_new_shortcut(is_recording);
}

/// Called on the push-to-talk thread for each recorder update: saves a recorded
/// shortcut and tells the page (`shortcut-recorder {state, keyLabels}`).
pub fn handle_shortcut_recorder_update(app: &AppHandle, recorder_update: ShortcutRecorderUpdate) {
    let recorder_event_payload = match recorder_update {
        ShortcutRecorderUpdate::Holding(held_modifiers) => {
            serde_json::json!({ "state": "holding", "keyLabels": held_modifiers.key_labels() })
        }
        ShortcutRecorderUpdate::NeedsTwoOrMoreKeys => serde_json::json!({ "state": "needsTwoOrMoreKeys", "keyLabels": [] }),
        ShortcutRecorderUpdate::Cancelled => serde_json::json!({ "state": "stopped", "keyLabels": [] }),
        ShortcutRecorderUpdate::Recorded(recorded_shortcut) => {
            let state: State<'_, AppState> = tauri::Manager::state(app);
            let updated_settings = {
                let mut settings = state.settings.write().unwrap();
                apply_setting_change(&mut settings, SettingChange::PushToTalkShortcut(recorded_shortcut));
                settings.clone()
            };
            save_and_announce_settings(app, updated_settings);
            serde_json::json!({ "state": "stopped", "keyLabels": [] })
        }
    };
    let _ = app.emit("shortcut-recorder", recorder_event_payload);
}

#[tauri::command]
pub fn set_launch_at_login(enabled: bool, app: AppHandle) {
    let autolaunch = app.autolaunch();
    let change_result = if enabled { autolaunch.enable() } else { autolaunch.disable() };
    if let Err(autolaunch_error) = change_result {
        eprintln!("settings: couldn't change launch at login: {autolaunch_error}");
    }
}

#[tauri::command]
pub fn replay_onboarding(app: AppHandle, state: State<'_, AppState>) {
    // The demo points at something on screen, so the panel gets out of the way first.
    hide_settings_window(&app);
    state.companion_brain.start_onboarding();
}

#[tauri::command]
pub fn open_microphone_settings() {
    if let Err(platform_error) = platform::open_microphone_privacy_settings() {
        eprintln!("settings: couldn't open microphone settings: {platform_error}");
    }
}

#[tauri::command]
pub fn quit_app(app: AppHandle) {
    app.exit(0);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::push_to_talk::ShortcutModifiers;

    #[test]
    fn status_texts_are_short_and_human_readable() {
        assert_eq!(speech_model_status_text(&SpeechModelState::Downloading { fraction: 0.416 }), "downloading 42%");
        assert_eq!(speech_model_status_text(&SpeechModelState::Ready), "ready");
        assert_eq!(speech_model_status_text(&SpeechModelState::Failed("no disk".into())), "failed: no disk");
        assert_eq!(voice_status_text(&NeuralVoiceLoadState::Downloading { fraction: 1.2 }), "downloading 100%");
        assert_eq!(voice_status_text(&NeuralVoiceLoadState::Ready), "Supertonic-3");
        assert_eq!(voice_status_text(&NeuralVoiceLoadState::NotLoaded), "system voice");
    }

    #[test]
    fn setting_changes_arrive_as_name_and_value_from_the_page() {
        let change: SettingChange = serde_json::from_str(r#"{"name":"voiceBuddyStyle","value":"cursor"}"#).unwrap();
        assert_eq!(change, SettingChange::VoiceBuddyStyle(VoiceBuddyStyle::Cursor));
        let change: SettingChange = serde_json::from_str(r#"{"name":"codexReasoningEffort","value":null}"#).unwrap();
        assert_eq!(change, SettingChange::CodexReasoningEffort(None));
        let change: SettingChange = serde_json::from_str(r#"{"name":"pushToTalkShortcut","value":"ctrlWin"}"#).unwrap();
        assert_eq!(change, SettingChange::PushToTalkShortcut(PushToTalkShortcutOption::CtrlWin));
    }

    #[test]
    fn invalid_values_from_the_page_change_nothing() {
        let mut settings = Settings::default();
        assert!(!apply_setting_change(&mut settings, SettingChange::ClaudeModel("gpt".into())));
        assert!(!apply_setting_change(&mut settings, SettingChange::CodexReasoningEffort(Some("max".into()))));
        assert!(!apply_setting_change(&mut settings, SettingChange::SelectedOrbColor(10)));
        let single_key = ShortcutModifiers { ctrl: true, ..Default::default() };
        assert!(!apply_setting_change(&mut settings, SettingChange::PushToTalkShortcut(PushToTalkShortcutOption::CustomModifiers(single_key))));
        assert_eq!(settings, Settings::default());
    }

    #[test]
    fn valid_changes_apply_and_the_orb_size_is_clamped() {
        let mut settings = Settings::default();
        assert!(apply_setting_change(&mut settings, SettingChange::ClaudeModel("opus".into())));
        assert!(apply_setting_change(&mut settings, SettingChange::BrainEngine(BrainEngine::Codex)));
        assert!(apply_setting_change(&mut settings, SettingChange::CodexReasoningEffort(Some("high".into()))));
        assert!(apply_setting_change(&mut settings, SettingChange::OrbSideLength(999.0)));
        assert!(apply_setting_change(&mut settings, SettingChange::IsChatInputMode(true)));
        assert!(apply_setting_change(&mut settings, SettingChange::ChatPanelSide(ChatPanelSide::Left)));
        assert_eq!(settings.claude_model, "opus");
        assert_eq!(settings.brain_engine, BrainEngine::Codex);
        assert_eq!(settings.codex_reasoning_effort.as_deref(), Some("high"));
        assert_eq!(settings.orb_side_length, 160.0);
        assert!(settings.is_chat_input_mode);
        assert_eq!(settings.chat_panel_side, ChatPanelSide::Left);
    }
}
