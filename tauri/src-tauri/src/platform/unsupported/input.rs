use crate::key_combo::ParsedKeyCombo;
use crate::platform::PlatformError;

pub fn click_at_global_point(x: i32, y: i32) -> Result<(), PlatformError> {
    Err(PlatformError::Unsupported)
}

pub fn type_text(text: &str, should_stop_typing: &std::sync::atomic::AtomicBool) -> Result<(), PlatformError> {
    Err(PlatformError::Unsupported)
}

pub fn press_key_combo(parsed_key_combo: &ParsedKeyCombo) -> Result<(), PlatformError> {
    Err(PlatformError::Unsupported)
}
