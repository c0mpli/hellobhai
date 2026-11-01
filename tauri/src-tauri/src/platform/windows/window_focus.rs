//! Keyboard focus hand-back for the typed chat: the chat takes focus so its text
//! box gets typing, and before [TYPE]/[KEY] or on minimize the focus goes back to
//! the window the user was working in (the Mac's `giveKeyboardFocusBackToFrontmostApp`).

use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, IsWindow, SetForegroundWindow};

use crate::platform::PlatformError;

/// The window that has keyboard focus now (an HWND as an integer), if any.
pub fn foreground_window_id() -> Option<isize> {
    let foreground_window = unsafe { GetForegroundWindow() };
    if foreground_window.0.is_null() {
        None
    } else {
        Some(foreground_window.0 as isize)
    }
}

/// SetForegroundWindow is allowed here because our own window is in front when this runs.
pub fn bring_window_to_foreground(window_id: isize) -> Result<(), PlatformError> {
    let target_window = HWND(window_id as *mut std::ffi::c_void);
    if !unsafe { IsWindow(Some(target_window)) }.as_bool() {
        return Err(PlatformError::Failed("that window no longer exists".to_string()));
    }
    if unsafe { SetForegroundWindow(target_window) }.as_bool() {
        Ok(())
    } else {
        Err(PlatformError::Failed("SetForegroundWindow refused".to_string()))
    }
}
