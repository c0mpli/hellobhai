//! Opens things through the Windows shell: the microphone privacy page from the
//! settings panel, and the targets of Claude's `[OPEN]` tags.

use windows::core::{w, HSTRING};
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

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

/// ShellExecuteW "open" on a target `computer_control::allowed_open_target`
/// already checked (link, file, folder, Start menu shortcut or app id).
pub fn open_with_shell(checked_target: &str) -> Result<(), PlatformError> {
    let instance = unsafe { ShellExecuteW(None, w!("open"), &HSTRING::from(checked_target), None, None, SW_SHOWNORMAL) };
    // Values above 32 mean success (documented ShellExecute convention).
    if instance.0 as isize > 32 {
        Ok(())
    } else {
        Err(PlatformError::Failed(format!("ShellExecute failed with code {}", instance.0 as isize)))
    }
}

/// Nothing is limited on Windows.
pub fn platform_limitation_note() -> Option<String> {
    None
}
