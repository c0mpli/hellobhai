use crate::platform::PlatformError;

pub fn foreground_window_id() -> Option<isize> {
    None
}

pub fn bring_window_to_foreground(window_id: isize) -> Result<(), PlatformError> {
    Err(PlatformError::Unsupported)
}

pub fn raise_always_on_top_window(_window: &tauri::WebviewWindow) {}

pub fn set_fixed_size_window_size(window: &tauri::WebviewWindow, physical_size: tauri::PhysicalSize<u32>) {
    let _ = window.set_size(physical_size);
}
