//! User settings, stored as JSON at `app_config_dir/settings.json`.
//! A missing or corrupt file means defaults (sonnet, voice F2, ctrl + win).

use std::sync::{Arc, RwLock};

use serde::{Deserialize, Serialize};
use tauri::AppHandle;

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
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            claude_model: "sonnet".to_string(),
            supertonic_voice: SupertonicVoice::default(),
            push_to_talk_shortcut: PushToTalkShortcutOption::default(),
            has_completed_onboarding: false,
        }
    }
}

pub type SharedSettings = Arc<RwLock<Settings>>;

/// Reads settings.json; missing or unreadable -> defaults.
pub fn load_settings(app: &AppHandle) -> SharedSettings {
    todo!("E: read app_config_dir/settings.json, fall back to Settings::default()")
}

/// Writes settings.json (creating the folder); errors are logged, not fatal.
pub fn save_settings(app: &AppHandle, settings: &Settings) {
    todo!("E: write app_config_dir/settings.json")
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
            })
        );

        let partially_saved_settings: Settings = serde_json::from_str(r#"{"claudeModel":"opus"}"#).unwrap();
        assert_eq!(partially_saved_settings.claude_model, "opus");
        assert_eq!(partially_saved_settings.push_to_talk_shortcut, PushToTalkShortcutOption::CtrlWin);
    }
}
