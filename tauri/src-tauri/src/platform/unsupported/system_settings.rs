use crate::platform::PlatformError;

pub fn open_microphone_privacy_settings() -> Result<(), PlatformError> {
    Err(PlatformError::Unsupported)
}
