//! Keyboard focus hand-back for the typed chat: the chat takes focus so its text
//! box gets typing, and before [TYPE]/[KEY] or on minimize the focus goes back to
//! the window the user was working in (the Mac's `giveKeyboardFocusBackToFrontmostApp`).

use windows::core::PWSTR;
use windows::Win32::Foundation::{CloseHandle, HWND};
use windows::Win32::System::Threading::{OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION};
use windows::Win32::UI::WindowsAndMessaging::{
    GetForegroundWindow, GetWindowThreadProcessId, IsWindow, SetForegroundWindow, SetWindowPos, HWND_TOPMOST, SWP_ASYNCWINDOWPOS,
    SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
};

use crate::platform::PlatformError;

/// The window that has keyboard focus now (an HWND as an integer), if any.
/// None when it is one of bhai's own windows: handing focus "back" to it would
/// leave [TYPE] typing into bhai.
pub fn foreground_window_id() -> Option<isize> {
    let foreground_window = unsafe { GetForegroundWindow() };
    if foreground_window.0.is_null() {
        return None;
    }
    let mut owning_process_id: u32 = 0;
    unsafe { GetWindowThreadProcessId(foreground_window, Some(&mut owning_process_id)) };
    if owning_process_id == std::process::id() {
        return None;
    }
    Some(foreground_window.0 as isize)
}

/// The program behind the focused window, for bhai's memory episodes (the
/// Mac's frontmost app name): the executable's name without ".exe", like
/// "chrome" or "Code". None for bhai's own windows or when it can't be read.
pub fn foreground_app_name() -> Option<String> {
    let foreground_window = unsafe { GetForegroundWindow() };
    if foreground_window.0.is_null() {
        return None;
    }
    let mut owning_process_id: u32 = 0;
    unsafe { GetWindowThreadProcessId(foreground_window, Some(&mut owning_process_id)) };
    if owning_process_id == 0 || owning_process_id == std::process::id() {
        return None;
    }
    let process_handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, owning_process_id) }.ok()?;
    let mut executable_path_buffer = [0u16; 1024];
    let mut executable_path_length = executable_path_buffer.len() as u32;
    let query_result = unsafe {
        QueryFullProcessImageNameW(process_handle, PROCESS_NAME_WIN32, PWSTR(executable_path_buffer.as_mut_ptr()), &mut executable_path_length)
    };
    let _ = unsafe { CloseHandle(process_handle) };
    query_result.ok()?;
    let executable_path = String::from_utf16_lossy(&executable_path_buffer[..executable_path_length as usize]);
    std::path::Path::new(&executable_path).file_stem().map(|file_stem| file_stem.to_string_lossy().to_string())
}

/// Puts an always-on-top window back at the very top of the topmost band.
/// Setting always-on-top again through tao does nothing when the flag is
/// already on, so the taskbar (which raises itself over topmost windows when
/// clicked) would otherwise stay over the orb. Async so a call from a worker
/// thread never waits on the UI thread.
pub fn raise_always_on_top_window(window: &tauri::WebviewWindow) {
    let Ok(window_handle) = window.hwnd() else { return };
    let _ = unsafe {
        SetWindowPos(window_handle, Some(HWND_TOPMOST), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_ASYNCWINDOWPOS)
    };
}

/// Windows resizes a non-resizable window when asked; nothing special needed.
pub fn set_fixed_size_window_size(window: &tauri::WebviewWindow, physical_size: tauri::PhysicalSize<u32>) {
    let _ = window.set_size(physical_size);
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
