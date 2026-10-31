//! Commands the settings page calls. They change settings (saved at once) and
//! forward to the speech player, autostart plugin and brain.

use serde::Serialize;
use tauri::{AppHandle, State};

use crate::app_state::AppState;
use crate::push_to_talk::PushToTalkShortcutOption;
use crate::supertonic::SupertonicVoice;

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

#[tauri::command]
pub fn get_settings(app: AppHandle, state: State<'_, AppState>) -> SettingsSnapshot {
    todo!("D: snapshot settings + autostart state")
}

#[tauri::command]
pub fn get_status(app: AppHandle, state: State<'_, AppState>) -> StatusSnapshot {
    todo!("D: snapshot status")
}

/// "fable" | "opus" | "sonnet" | "haiku"
#[tauri::command]
pub fn set_claude_model(model: String, app: AppHandle, state: State<'_, AppState>) {
    todo!("D: save the model alias")
}

#[tauri::command]
pub fn set_supertonic_voice(voice: SupertonicVoice, app: AppHandle, state: State<'_, AppState>) {
    todo!("D: save + speech_player.set_voice")
}

#[tauri::command]
pub fn preview_voice(voice: SupertonicVoice, state: State<'_, AppState>) {
    todo!("D: speech_player.preview_voice")
}

#[tauri::command]
pub fn set_push_to_talk_shortcut(option: PushToTalkShortcutOption, app: AppHandle, state: State<'_, AppState>) {
    todo!("D: save the shortcut")
}

#[tauri::command]
pub fn set_launch_at_login(enabled: bool, app: AppHandle) {
    todo!("D: tauri-plugin-autostart enable/disable")
}

#[tauri::command]
pub fn replay_onboarding(state: State<'_, AppState>) {
    todo!("D: companion_brain.start_onboarding")
}

#[tauri::command]
pub fn open_microphone_settings() {
    todo!("D: platform::open_microphone_privacy_settings")
}

#[tauri::command]
pub fn quit_app(app: AppHandle) {
    todo!("D: app.exit(0)")
}
