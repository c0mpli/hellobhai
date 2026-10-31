use crate::platform::PlatformError;
use crate::screen_mapping::PhysicalRect;

/// (width, height, RGBA bytes)
pub fn capture_screen_rect_rgba(global_rect: PhysicalRect) -> Result<(u32, u32, Vec<u8>), PlatformError> {
    Err(PlatformError::Unsupported)
}
