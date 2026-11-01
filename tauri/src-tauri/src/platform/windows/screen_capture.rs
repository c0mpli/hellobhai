//! GDI screen capture: GetDC(None) + BitBlt into a 32-bit DIB section.
//! Windows.Graphics.Capture can replace it if some app captures black.

use windows::Win32::Graphics::Gdi::*;

use crate::platform::PlatformError;
use crate::screen_mapping::PhysicalRect;

/// Every window is built with `content_protected` (WDA_EXCLUDEFROMCAPTURE),
/// so BitBlt never sees our own windows.
pub const CAPTURE_EXCLUDES_OWN_WINDOWS: bool = true;

/// (width, height, RGBA bytes); BGRA from GDI converted to RGBA.
/// `global_rect` is in virtual-screen physical pixels, which is what the
/// screen DC uses in a per-monitor DPI aware process.
pub fn capture_screen_rect_rgba(global_rect: PhysicalRect) -> Result<(u32, u32, Vec<u8>), PlatformError> {
    let width = global_rect.width as i32;
    let height = global_rect.height as i32;
    if width <= 0 || height <= 0 {
        return Err(PlatformError::Failed("empty capture rect".to_string()));
    }

    unsafe {
        let screen_device_context = GetDC(None);
        if screen_device_context.is_invalid() {
            return Err(PlatformError::Failed("GetDC failed".to_string()));
        }
        let memory_device_context = CreateCompatibleDC(Some(screen_device_context));

        let bitmap_info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width,
                // Negative height: top-down rows, so the buffer reads like an image.
                biHeight: -height,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut dib_pixel_pointer: *mut core::ffi::c_void = std::ptr::null_mut();
        let capture_result =
            CreateDIBSection(Some(screen_device_context), &bitmap_info, DIB_RGB_COLORS, &mut dib_pixel_pointer, None, 0)
                .map_err(|error| PlatformError::Failed(format!("CreateDIBSection failed: {error}")))
                .and_then(|dib_bitmap| {
                    let previously_selected_object = SelectObject(memory_device_context, dib_bitmap.into());
                    // CAPTUREBLT includes layered windows (menus, tooltips, other apps' overlays).
                    // Our own windows stay out through WDA_EXCLUDEFROMCAPTURE.
                    let blit_result = BitBlt(
                        memory_device_context,
                        0,
                        0,
                        width,
                        height,
                        Some(screen_device_context),
                        global_rect.x,
                        global_rect.y,
                        SRCCOPY | CAPTUREBLT,
                    )
                    .map_err(|error| PlatformError::Failed(format!("BitBlt failed: {error}")));

                    let pixel_bytes = blit_result.map(|()| {
                        let byte_count = width as usize * height as usize * 4;
                        let mut pixel_bytes = std::slice::from_raw_parts(dib_pixel_pointer as *const u8, byte_count).to_vec();
                        for pixel in pixel_bytes.chunks_exact_mut(4) {
                            pixel.swap(0, 2); // BGRA -> RGBA
                            pixel[3] = 255; // GDI leaves alpha undefined
                        }
                        pixel_bytes
                    });

                    SelectObject(memory_device_context, previously_selected_object);
                    let _ = DeleteObject(dib_bitmap.into());
                    pixel_bytes
                });

        let _ = DeleteDC(memory_device_context);
        ReleaseDC(None, screen_device_context);
        capture_result.map(|pixel_bytes| (global_rect.width, global_rect.height, pixel_bytes))
    }
}
