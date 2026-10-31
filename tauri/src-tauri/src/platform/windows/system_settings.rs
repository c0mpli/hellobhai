//! Opens the Windows microphone privacy page from the settings panel.

use crate::platform::PlatformError;

/// `explorer ms-settings:privacy-microphone`
pub fn open_microphone_privacy_settings() -> Result<(), PlatformError> {
    todo!("D: open ms-settings:privacy-microphone")
}
