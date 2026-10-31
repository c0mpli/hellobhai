//! Screenshots of every monitor (cursor screen first) as downscaled JPEGs, and
//! native-resolution crops of the regions the user draws around. Our own
//! windows are hidden from capture by `content_protected(true)`.

use tauri::AppHandle;

use crate::screen_mapping::{MonitorDescription, PhysicalRect};

#[derive(Clone, Debug)]
pub struct ScreenCapture {
    pub jpeg_bytes: Vec<u8>,
    /// Exact Mac label, see `screen_label`.
    pub label: String,
    pub is_cursor_screen: bool,
    pub monitor: MonitorDescription,
    pub screenshot_width_in_pixels: u32,
    pub screenshot_height_in_pixels: u32,
}

/// Every monitor, cursor screen first; long side at most 1280 px, JPEG quality 80.
pub fn capture_all_screens(app: &AppHandle) -> Result<Vec<ScreenCapture>, String> {
    todo!("C: platform capture + resize + encode")
}

/// Native resolution, JPEG quality 85.
pub fn capture_region_jpeg(global_rect: PhysicalRect) -> Result<Vec<u8>, String> {
    todo!("C: platform capture + encode")
}

/// One screen: "user's screen (cursor is here)". Several: "screen N of M — cursor is on
/// this screen (primary focus)" or "screen N of M — secondary screen".
pub fn screen_label(screen_index: usize, screen_count: usize, is_cursor_screen: bool) -> String {
    todo!("C: exact Mac labels")
}
