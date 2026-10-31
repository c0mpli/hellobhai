//! Opens the Windows microphone privacy page from the settings panel.

use crate::platform::PlatformError;

/// `explorer ms-settings:privacy-microphone`
pub fn open_microphone_privacy_settings() -> Result<(), PlatformError> {
    // explorer.exe hands the ms-settings: URI to the Settings app and exits;
    // its exit code is meaningless, so it is not waited for.
    std::process::Command::new("explorer.exe")
        .arg("ms-settings:privacy-microphone")
        .spawn()
        .map(|_explorer_process| ())
        .map_err(|spawn_error| PlatformError::Failed(format!("couldn't start explorer: {spawn_error}")))
}
