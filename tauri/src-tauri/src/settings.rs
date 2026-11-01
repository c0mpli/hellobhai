//! User settings, stored as JSON at `app_config_dir/settings.json`.
//! A missing or corrupt file means defaults (sonnet, voice F2, ctrl + win, orb
//! style, 88 px Aurora orb in its default spot).

use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

use crate::orb_window::{VoiceBuddyStyle, DEFAULT_ORB_COLOR, DEFAULT_ORB_SIDE_LENGTH};
use crate::chat_panel::ChatPanelSide;
use crate::push_to_talk::PushToTalkShortcutOption;
use crate::supertonic::SupertonicVoice;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    /// Claude model alias passed to `claude -p --model`: "fable", "opus", "sonnet" or "haiku".
    pub claude_model: String,
    pub supertonic_voice: SupertonicVoice,
    pub push_to_talk_shortcut: PushToTalkShortcutOption,
    pub has_completed_onboarding: bool,
    pub voice_buddy_style: VoiceBuddyStyle,
    /// Logical pixels, 48-160 (`orb_window::clamped_orb_side_length` on use).
    pub orb_side_length: f64,
    /// orb.riv color variant 0-9.
    pub orb_color: u8,
    /// Global physical pixels of the orb's top-left corner after the user dragged it; None = default spot.
    pub orb_window_origin: Option<(i32, i32)>,
    /// bhai mode: false = Voice (hold the hotkey to talk), true = Chat (the hotkey
    /// opens the typed chat, which replaces the orb on screen). Mac `isChatInputMode`.
    pub is_chat_input_mode: bool,
    /// Spoken questions get voice + text (true) or text only in the chat (false).
    pub speaks_voice_replies: bool,
    /// Typed questions get text (false) or text + voice (true).
    pub speaks_chat_replies: bool,
    pub chat_panel_side: ChatPanelSide,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            claude_model: "sonnet".to_string(),
            supertonic_voice: SupertonicVoice::default(),
            push_to_talk_shortcut: PushToTalkShortcutOption::default(),
            has_completed_onboarding: false,
            voice_buddy_style: VoiceBuddyStyle::Orb,
            orb_side_length: DEFAULT_ORB_SIDE_LENGTH,
            orb_color: DEFAULT_ORB_COLOR,
            orb_window_origin: None,
            is_chat_input_mode: false,
            speaks_voice_replies: true,
            speaks_chat_replies: false,
            chat_panel_side: ChatPanelSide::Right,
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
                "voiceBuddyStyle": "orb",
                "orbSideLength": 88.0,
                "orbColor": 8,
                "orbWindowOrigin": null,
                "isChatInputMode": false,
                "speaksVoiceReplies": true,
                "speaksChatReplies": false,
                "chatPanelSide": "right",
            })
        );

        let partially_saved_settings: Settings = serde_json::from_str(r#"{"claudeModel":"opus"}"#).unwrap();
        assert_eq!(partially_saved_settings.claude_model, "opus");
        assert_eq!(partially_saved_settings.push_to_talk_shortcut, PushToTalkShortcutOption::CtrlWin);
    }

    #[test]
    fn missing_or_corrupt_settings_file_means_defaults() {
        assert_eq!(settings_from_json_text(None), Settings::default());
        assert_eq!(settings_from_json_text(Some("{not json".to_string())), Settings::default());
        assert_eq!(settings_from_json_text(Some(r#"{"claudeModel":"haiku"}"#.to_string())).claude_model, "haiku");
    }
}
