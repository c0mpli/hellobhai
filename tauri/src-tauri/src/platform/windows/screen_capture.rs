//! GDI screen capture: GetDC(None) + BitBlt into a 32-bit DIB section.
//! Windows.Graphics.Capture can replace it if some app captures black.

use crate::platform::PlatformError;
use crate::screen_mapping::PhysicalRect;

/// (width, height, RGBA bytes); BGRA from GDI converted to RGBA.
pub fn capture_screen_rect_rgba(global_rect: PhysicalRect) -> Result<(u32, u32, Vec<u8>), PlatformError> {
    todo!("C: BitBlt capture")
}
