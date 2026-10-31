use crate::platform::PlatformError;
use crate::push_to_talk::RawKeyEvent;

pub fn start_keyboard_hook(_on_key_event: Box<dyn Fn(RawKeyEvent) + Send + 'static>) -> Result<(), PlatformError> {
    Err(PlatformError::Unsupported)
}

/// Taps an unassigned key so releasing Win after the shortcut does not open the Start menu.
pub fn inject_start_menu_suppression_key() {}
