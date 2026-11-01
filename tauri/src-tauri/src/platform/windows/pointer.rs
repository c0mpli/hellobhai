//! The real mouse pointer's state: the buddy hides while the pointer is hidden
//! (videos, games, typing), and the 3D overlay skips hit-testing during drags.
//! Plus the fullscreen check the orb uses to step aside for fullscreen video.

use windows::Win32::Foundation::RECT;
use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONULL};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_LBUTTON};
use windows::Win32::UI::WindowsAndMessaging::{
    GetClassNameW, GetCursorInfo, GetForegroundWindow, GetShellWindow, GetWindowRect, CURSORINFO, CURSOR_SHOWING, CURSOR_SUPPRESSED,
};

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

/// True when the foreground window covers its whole monitor (fullscreen video,
/// a fullscreen app or game). The desktop (shell window, Progman, WorkerW)
/// covers the monitor too but is not an app, so it never counts.
pub fn is_foreground_window_full_screen() -> bool {
    let foreground_window = unsafe { GetForegroundWindow() };
    if foreground_window.is_invalid() || foreground_window == unsafe { GetShellWindow() } {
        return false;
    }
    let mut class_name_buffer = [0u16; 64];
    let class_name_length = unsafe { GetClassNameW(foreground_window, &mut class_name_buffer) }.max(0) as usize;
    let class_name = String::from_utf16_lossy(&class_name_buffer[..class_name_length]);
    if class_name == "Progman" || class_name == "WorkerW" {
        return false;
    }
    let mut window_rect = RECT::default();
    if unsafe { GetWindowRect(foreground_window, &mut window_rect) }.is_err() {
        return false;
    }
    let monitor = unsafe { MonitorFromWindow(foreground_window, MONITOR_DEFAULTTONULL) };
    if monitor.is_invalid() {
        return false;
    }
    let mut monitor_info = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
    if !unsafe { GetMonitorInfoW(monitor, &mut monitor_info) }.as_bool() {
        return false;
    }
    window_rect == monitor_info.rcMonitor
}
