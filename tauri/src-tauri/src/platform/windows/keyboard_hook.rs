//! Listen-only low-level keyboard hook (WH_KEYBOARD_LL) on its own thread with
//! a message loop. The callback must stay tiny (Windows silently removes slow LL
//! hooks): it maps the key, calls `on_key_event`, and always CallNextHookEx.

use crate::platform::PlatformError;
use crate::push_to_talk::RawKeyEvent;

pub fn start_keyboard_hook(on_key_event: Box<dyn Fn(RawKeyEvent) + Send + 'static>) -> Result<(), PlatformError> {
    todo!("A: SetWindowsHookExW + GetMessageW loop")
}

/// Taps VK 0xE8 (unassigned) once so releasing Win after the shortcut does not
/// open the Start menu (the AutoHotkey "menu mask key" trick).
pub fn inject_start_menu_suppression_key() {
    todo!("A: SendInput VK 0xE8 down/up")
}
