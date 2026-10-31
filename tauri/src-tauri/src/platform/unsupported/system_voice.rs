use crate::platform::PlatformError;

/// WAV bytes.
pub fn synthesize_with_system_voice(text: &str, prefer_hindi_voice: bool) -> Result<Vec<u8>, PlatformError> {
    Err(PlatformError::Unsupported)
}
