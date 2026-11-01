//! User settings, stored as JSON at `app_config_dir/settings.json`.
//! A missing or corrupt file means defaults, the Mac's: Claude sonnet, voice F2,
//! voice mode with the Aurora orb at 88 px, spoken questions get voice + text,
//! typed ones text, chat on the right; shortcut ctrl + win. Every change made
//! from the settings panel is saved at once and announced to every window as
//! `settings-changed` (payload: `ui_commands::SettingsSnapshot`).

use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

use crate::brain_engine::BrainEngine;
use crate::push_to_talk::PushToTalkShortcutOption;
use crate::supertonic::SupertonicVoice;

/// How bhai looks in voice mode (Mac: `VoiceBuddyStyle`): the draggable orb the
/// pointer launches from, or the classic blue triangle following the mouse.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum VoiceBuddyStyle {
    #[default]
    Orb,
    Cursor,
}

/// Which screen edge the chat panel slides in from (Mac: `ChatPanelSide`).
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ChatPanelSide {
    Left,
    #[default]
    Right,
}

/// The orb's side length in logical pixels: 48-160 in steps of 2, default 88 (Mac: `OrbSize`).
pub const ORB_SIDE_LENGTH_DEFAULT: f64 = 88.0;
pub const ORB_SIDE_LENGTH_MINIMUM: f64 = 48.0;
pub const ORB_SIDE_LENGTH_MAXIMUM: f64 = 160.0;

/// Rounds to the slider's 2 px steps and keeps it in range.
pub fn clamped_orb_side_length(requested_side_length: f64) -> f64 {
    if !requested_side_length.is_finite() {
        return ORB_SIDE_LENGTH_DEFAULT;
    }
    ((requested_side_length / 2.0).round() * 2.0).clamp(ORB_SIDE_LENGTH_MINIMUM, ORB_SIDE_LENGTH_MAXIMUM)
}

/// orb.riv's colour input, 0-9 (Rainbow, Candy, Red, Coral, Sunset, Green,
/// Teal, Blue, Aurora, Violet); 8 = Aurora.
pub const ORB_COLOR_DEFAULT: u8 = 8;
pub const ORB_COLOR_MAXIMUM: u8 = 9;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    /// Claude model alias passed to `claude -p --model`: "fable", "opus", "sonnet" or "haiku".
    pub claude_model: String,
    pub supertonic_voice: SupertonicVoice,
    pub push_to_talk_shortcut: PushToTalkShortcutOption,
    pub has_completed_onboarding: bool,
    /// Which local CLI answers: claude (default) or codex.
    pub brain_engine: BrainEngine,
    /// "low", "medium" or "high"; None = the model's own default.
    pub codex_reasoning_effort: Option<String>,
    /// bhai mode: false = voice (hold the shortcut and talk; orb or cursor on
    /// screen), true = chat (the shortcut and the orb open the chat sidebar).
    pub is_chat_input_mode: bool,
    /// Spoken questions: voice + text (true) or text only, in the chat.
    pub speaks_voice_replies: bool,
    /// Typed questions: text only (false) or text + voice.
    pub speaks_chat_replies: bool,
    pub chat_panel_side: ChatPanelSide,
    pub voice_buddy_style: VoiceBuddyStyle,
    pub orb_side_length: f64,
    pub selected_orb_color: u8,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            claude_model: "sonnet".to_string(),
            supertonic_voice: SupertonicVoice::default(),
            push_to_talk_shortcut: PushToTalkShortcutOption::default(),
            has_completed_onboarding: false,
            brain_engine: BrainEngine::default(),
            codex_reasoning_effort: None,
            is_chat_input_mode: false,
            speaks_voice_replies: true,
            speaks_chat_replies: false,
            chat_panel_side: ChatPanelSide::default(),
            voice_buddy_style: VoiceBuddyStyle::default(),
            orb_side_length: ORB_SIDE_LENGTH_DEFAULT,
            selected_orb_color: ORB_COLOR_DEFAULT,
        }
    }
}

pub type SharedSettings = Arc<RwLock<Settings>>;

fn settings_file_path(app: &AppHandle) -> Option<PathBuf> {
    app.path().app_config_dir().ok().map(|config_directory| config_directory.join("settings.json"))
}

/// Missing, unreadable or corrupt -> defaults.
fn settings_from_json_text(settings_json_text: Option<String>) -> Settings {
    settings_json_text
        .and_then(|json_text| serde_json::from_str(&json_text).ok())
        .unwrap_or_default()
}

/// Reads settings.json; missing or unreadable -> defaults.
pub fn load_settings(app: &AppHandle) -> SharedSettings {
    let settings_json_text = settings_file_path(app).and_then(|settings_path| std::fs::read_to_string(settings_path).ok());
    Arc::new(RwLock::new(settings_from_json_text(settings_json_text)))
}

/// Writes settings.json (creating the folder); errors are logged, not fatal.
pub fn save_settings(app: &AppHandle, settings: &Settings) {
    let Some(settings_path) = settings_file_path(app) else {
        eprintln!("settings: no config folder, not saving");
        return;
    };
    let write_result = settings_path
        .parent()
        .map(std::fs::create_dir_all)
        .unwrap_or(Ok(()))
        .and_then(|_| std::fs::write(&settings_path, serde_json::to_string_pretty(settings).unwrap_or_default()));
    if let Err(write_error) = write_result {
        eprintln!("settings: couldn't save {}: {write_error}", settings_path.display());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_json_uses_the_page_contract_names_and_fills_missing_fields_with_defaults() {
        let default_settings_json = serde_json::to_value(Settings::default()).unwrap();
        assert_eq!(
            default_settings_json,
            serde_json::json!({
                "claudeModel": "sonnet",
                "supertonicVoice": "F2",
                "pushToTalkShortcut": "ctrlWin",
                "hasCompletedOnboarding": false,
                "brainEngine": "claude",
                "codexReasoningEffort": null,
                "isChatInputMode": false,
                "speaksVoiceReplies": true,
                "speaksChatReplies": false,
                "chatPanelSide": "right",
                "voiceBuddyStyle": "orb",
                "orbSideLength": 88.0,
                "selectedOrbColor": 8,
            })
        );

        // A settings file from before these fields existed keeps the Mac defaults for them.
        let partially_saved_settings: Settings = serde_json::from_str(r#"{"claudeModel":"opus"}"#).unwrap();
        assert_eq!(partially_saved_settings.claude_model, "opus");
        assert_eq!(partially_saved_settings.push_to_talk_shortcut, PushToTalkShortcutOption::CtrlWin);
        assert!(partially_saved_settings.speaks_voice_replies);
        assert_eq!(partially_saved_settings.selected_orb_color, ORB_COLOR_DEFAULT);
    }

    #[test]
    fn orb_size_snaps_to_two_pixel_steps_inside_48_to_160() {
        assert_eq!(clamped_orb_side_length(89.2), 90.0);
        assert_eq!(clamped_orb_side_length(10.0), 48.0);
        assert_eq!(clamped_orb_side_length(500.0), 160.0);
        assert_eq!(clamped_orb_side_length(f64::NAN), 88.0);
    }

    #[test]
    fn missing_or_corrupt_settings_file_means_defaults() {
        assert_eq!(settings_from_json_text(None), Settings::default());
        assert_eq!(settings_from_json_text(Some("{not json".to_string())), Settings::default());
        assert_eq!(settings_from_json_text(Some(r#"{"claudeModel":"haiku"}"#.to_string())).claude_model, "haiku");
    }
}
