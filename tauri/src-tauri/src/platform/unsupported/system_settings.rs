use crate::platform::PlatformError;

pub fn open_microphone_privacy_settings() -> Result<(), PlatformError> {
    Err(PlatformError::Unsupported)
}

pub fn open_with_shell(checked_target: &str) -> Result<(), PlatformError> {
    Err(PlatformError::Unsupported)
}
