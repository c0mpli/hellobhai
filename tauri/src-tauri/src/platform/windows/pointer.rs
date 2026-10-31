//! The real mouse pointer's state: the buddy hides while the pointer is hidden
//! (videos, games, typing), and the 3D overlay skips hit-testing during drags.

use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_LBUTTON};
use windows::Win32::UI::WindowsAndMessaging::{GetCursorInfo, CURSORINFO, CURSOR_SHOWING, CURSOR_SUPPRESSED};

/// GetCursorInfo: CURSOR_SHOWING set and CURSOR_SUPPRESSED not set.
/// CURSOR_SUPPRESSED means touch or pen input is hiding the pointer.
pub fn is_system_pointer_visible() -> bool {
    let mut cursor_info = CURSORINFO { cbSize: std::mem::size_of::<CURSORINFO>() as u32, ..Default::default() };
    if unsafe { GetCursorInfo(&mut cursor_info) }.is_err() {
        // Fails on the secure desktop (UAC prompt, lock screen); keep the buddy shown.
        return true;
    }
    let is_showing = cursor_info.flags.0 & CURSOR_SHOWING.0 != 0;
    let is_suppressed = cursor_info.flags.0 & CURSOR_SUPPRESSED.0 != 0;
    is_showing && !is_suppressed
}

/// GetAsyncKeyState(VK_LBUTTON) < 0
pub fn is_left_mouse_button_down() -> bool {
    unsafe { GetAsyncKeyState(VK_LBUTTON.0 as i32) < 0 }
}
