use crate::platform::PlatformError;

pub fn foreground_window_id() -> Option<isize> {
    None
}

pub fn bring_window_to_foreground(window_id: isize) -> Result<(), PlatformError> {
    Err(PlatformError::Unsupported)
}
