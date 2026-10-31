//! The real mouse pointer's state: the buddy hides while the pointer is hidden
//! (videos, games, typing), and the 3D overlay skips hit-testing during drags.

/// GetCursorInfo: CURSOR_SHOWING set and CURSOR_SUPPRESSED not set.
pub fn is_system_pointer_visible() -> bool {
    todo!("C: GetCursorInfo")
}

/// GetAsyncKeyState(VK_LBUTTON) < 0
pub fn is_left_mouse_button_down() -> bool {
    todo!("C: GetAsyncKeyState")
}
