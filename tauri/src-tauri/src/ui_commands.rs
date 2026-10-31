//! Commands the settings page calls. They change settings (saved at once) and
//! forward to the speech player, autostart plugin and brain.

use serde::Serialize;
use tauri::{AppHandle, State};
use tauri_plugin_autostart::ManagerExt;

use crate::app_state::AppState;
use crate::claude_cli::find_claude_executable;
use crate::push_to_talk::PushToTalkShortcutOption;
use crate::settings::{save_settings, Settings};
use crate::speech_player::NeuralVoiceLoadState;
use crate::supertonic::SupertonicVoice;
use crate::tray_and_settings::hide_settings_window;
use crate::whisper_transcriber::SpeechModelState;
use crate::platform;

/// The model aliases `claude -p --model` accepts, as on the Mac.
pub const CLAUDE_MODEL_ALIASES: [&str; 4] = ["fable", "opus", "sonnet", "haiku"];

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsSnapshot {
    pub claude_model: String,
    pub supertonic_voice: SupertonicVoice,
    pub push_to_talk_shortcut: PushToTalkShortcutOption,
    pub push_to_talk_display_text: String,
    pub push_to_talk_key_capsule_labels: Vec<String>,
    pub launch_at_login: bool,
}

/// Also pushed to the settings page as `status-changed` whenever a part changes.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StatusSnapshot {
    pub claude_found: bool,
    /// Human-readable speech-to-text model state, e.g. "downloading 42%" or "ready".
    pub speech_model: String,
    /// Human-readable voice state.
    pub voice: String,
    /// None until the microphone has been tried once.
    pub microphone_working: Option<bool>,
    pub is_listening: bool,
    pub version: String,
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

/// Changes one setting, then saves the file.
fn update_and_save_settings(app: &AppHandle, state: &State<'_, AppState>, change_settings: impl FnOnce(&mut Settings)) {
    let updated_settings = {
        let mut settings = state.settings.write().unwrap();
        change_settings(&mut settings);
        settings.clone()
    };
    save_settings(app, &updated_settings);
}

#[tauri::command]
pub fn get_settings(app: AppHandle, state: State<'_, AppState>) -> SettingsSnapshot {
    let settings = state.settings.read().unwrap().clone();
    SettingsSnapshot {
        claude_model: settings.claude_model,
        supertonic_voice: settings.supertonic_voice,
        push_to_talk_shortcut: settings.push_to_talk_shortcut,
        push_to_talk_display_text: settings.push_to_talk_shortcut.display_text().to_string(),
        push_to_talk_key_capsule_labels: settings
            .push_to_talk_shortcut
            .key_capsule_labels()
            .iter()
            .map(|key_label| key_label.to_string())
            .collect(),
        launch_at_login: app.autolaunch().is_enabled().unwrap_or(false),
    }
}

#[tauri::command]
pub fn get_status(app: AppHandle, state: State<'_, AppState>) -> StatusSnapshot {
    StatusSnapshot {
        claude_found: find_claude_executable().is_some(),
        speech_model: speech_model_status_text(&state.whisper_transcriber.state()),
        voice: voice_status_text(&state.speech_player.load_state()),
        // Nothing reports microphone results yet; the page shows "not tried yet".
        microphone_working: None,
        // The page follows `buddy-voice-state` for this; the snapshot starts idle.
        is_listening: false,
        version: app.package_info().version.to_string(),
    }
}

/// "fable" | "opus" | "sonnet" | "haiku"
#[tauri::command]
pub fn set_claude_model(model: String, app: AppHandle, state: State<'_, AppState>) {
    if !CLAUDE_MODEL_ALIASES.contains(&model.as_str()) {
        eprintln!("settings: ignoring unknown claude model {model:?}");
        return;
    }
    update_and_save_settings(&app, &state, |settings| settings.claude_model = model);
}

#[tauri::command]
pub fn set_supertonic_voice(voice: SupertonicVoice, app: AppHandle, state: State<'_, AppState>) {
    update_and_save_settings(&app, &state, |settings| settings.supertonic_voice = voice);
    state.speech_player.set_voice(voice);
}

#[tauri::command]
pub fn preview_voice(voice: SupertonicVoice, state: State<'_, AppState>) {
    state.speech_player.preview_voice(voice);
}

#[tauri::command]
pub fn set_push_to_talk_shortcut(option: PushToTalkShortcutOption, app: AppHandle, state: State<'_, AppState>) {
    update_and_save_settings(&app, &state, |settings| settings.push_to_talk_shortcut = option);
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

    #[test]
    fn status_texts_are_short_and_human_readable() {
        assert_eq!(speech_model_status_text(&SpeechModelState::Downloading { fraction: 0.416 }), "downloading 42%");
        assert_eq!(speech_model_status_text(&SpeechModelState::Ready), "ready");
        assert_eq!(speech_model_status_text(&SpeechModelState::Failed("no disk".into())), "failed: no disk");
        assert_eq!(voice_status_text(&NeuralVoiceLoadState::Downloading { fraction: 1.2 }), "downloading 100%");
        assert_eq!(voice_status_text(&NeuralVoiceLoadState::Ready), "Supertonic-3");
        assert_eq!(voice_status_text(&NeuralVoiceLoadState::NotLoaded), "system voice");
    }
}
