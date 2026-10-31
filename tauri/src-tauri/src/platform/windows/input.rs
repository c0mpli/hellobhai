//! Synthetic mouse and keyboard input with SetCursorPos + SendInput.
//! Coordinates are global physical pixels (the process is per-monitor DPI aware).

use crate::key_combo::ParsedKeyCombo;
use crate::platform::PlatformError;

/// SetCursorPos, 30 ms, left down, 30 ms, left up.
pub fn click_at_global_point(x: i32, y: i32) -> Result<(), PlatformError> {
    todo!("C: SendInput click")
}

/// KEYEVENTF_UNICODE per UTF-16 unit (Devanagari works, surrogate pairs kept together),
/// '\n' as Enter, 8 ms between chunks of 20 units.
pub fn type_text(text: &str) -> Result<(), PlatformError> {
    todo!("C: SendInput unicode typing")
}

/// Modifiers down, key down/up, modifiers up in reverse.
pub fn press_key_combo(parsed_key_combo: &ParsedKeyCombo) -> Result<(), PlatformError> {
    todo!("C: SendInput key combo")
}
