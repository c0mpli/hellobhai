//! Screenshots of every monitor (cursor screen first) as downscaled JPEGs, and
//! native-resolution crops of the regions the user draws around. Our own
//! windows are hidden from capture by `content_protected(true)`.

use image::codecs::jpeg::JpegEncoder;
use image::imageops::FilterType;
use image::{DynamicImage, RgbaImage};
use tauri::AppHandle;

use crate::platform;
use crate::screen_mapping::{downscaled_screenshot_size, monitors_from_tauri, MonitorDescription, PhysicalRect};

const MAXIMUM_SCREENSHOT_DIMENSION: u32 = 1280;
const SCREENSHOT_JPEG_QUALITY: u8 = 80;
const REGION_CROP_JPEG_QUALITY: u8 = 85;

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
/// A monitor that fails to capture is skipped; an error only when none worked.
pub fn capture_all_screens(app: &AppHandle) -> Result<Vec<ScreenCapture>, String> {
    let mut monitors = monitors_from_tauri(app);
    if monitors.is_empty() {
        return Err("no display available for capture".to_string());
    }

    let cursor_position = app.cursor_position().ok();
    let monitor_contains_cursor = |monitor: &MonitorDescription| {
        cursor_position.is_some_and(|position| monitor.rect.contains_point(position.x, position.y))
    };
    // Stable sort: the cursor screen moves to the front, the rest keep their x-then-y order.
    monitors.sort_by_key(|monitor| !monitor_contains_cursor(monitor));

    let screen_count = monitors.len();
    let mut captured_screens = Vec::new();
    for (screen_index, monitor) in monitors.into_iter().enumerate() {
        let is_cursor_screen = monitor_contains_cursor(&monitor);
        let (screenshot_width_in_pixels, screenshot_height_in_pixels) =
            downscaled_screenshot_size(monitor.rect.width, monitor.rect.height, MAXIMUM_SCREENSHOT_DIMENSION);

        let jpeg_bytes = match capture_rect_as_jpeg(monitor.rect, Some((screenshot_width_in_pixels, screenshot_height_in_pixels)), SCREENSHOT_JPEG_QUALITY) {
            Ok(jpeg_bytes) => jpeg_bytes,
            Err(error) => {
                eprintln!("screen capture: monitor {} failed: {error}", monitor.monitor_index);
                continue;
            }
        };

        captured_screens.push(ScreenCapture {
            jpeg_bytes,
            label: screen_label(screen_index, screen_count, is_cursor_screen),
            is_cursor_screen,
            monitor,
            screenshot_width_in_pixels,
            screenshot_height_in_pixels,
        });
    }

    if captured_screens.is_empty() {
        return Err("failed to capture any screen".to_string());
    }
    Ok(captured_screens)
}

/// Native resolution, JPEG quality 85.
pub fn capture_region_jpeg(global_rect: PhysicalRect) -> Result<Vec<u8>, String> {
    if global_rect.width == 0 || global_rect.height == 0 {
        return Err("empty region".to_string());
    }
    capture_rect_as_jpeg(global_rect, None, REGION_CROP_JPEG_QUALITY)
}

/// Captures `global_rect`, optionally resizes to `resized_size`, and encodes a JPEG.
fn capture_rect_as_jpeg(global_rect: PhysicalRect, resized_size: Option<(u32, u32)>, jpeg_quality: u8) -> Result<Vec<u8>, String> {
    let (captured_width, captured_height, rgba_bytes) =
        platform::capture_screen_rect_rgba(global_rect).map_err(|error| error.to_string())?;
    let mut captured_image = RgbaImage::from_raw(captured_width, captured_height, rgba_bytes)
        .ok_or_else(|| "captured pixel buffer has the wrong size".to_string())?;

    if let Some((resized_width, resized_height)) = resized_size {
        captured_image = image::imageops::resize(&captured_image, resized_width, resized_height, FilterType::Triangle);
    }

    // The JPEG encoder takes no alpha channel.
    let rgb_image = DynamicImage::ImageRgba8(captured_image).to_rgb8();
    let mut jpeg_bytes = Vec::new();
    JpegEncoder::new_with_quality(&mut jpeg_bytes, jpeg_quality)
        .encode_image(&rgb_image)
        .map_err(|error| error.to_string())?;
    Ok(jpeg_bytes)
}

/// One screen: "user's screen (cursor is here)". Several: "screen N of M — cursor is on
/// this screen (primary focus)" or "screen N of M — secondary screen".
pub fn screen_label(screen_index: usize, screen_count: usize, is_cursor_screen: bool) -> String {
    let screen_number = screen_index + 1;
    if screen_count == 1 {
        "user's screen (cursor is here)".to_string()
    } else if is_cursor_screen {
        format!("screen {screen_number} of {screen_count} — cursor is on this screen (primary focus)")
    } else {
        format!("screen {screen_number} of {screen_count} — secondary screen")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_screens_with_the_exact_mac_strings() {
        assert_eq!(screen_label(0, 1, true), "user's screen (cursor is here)");
        assert_eq!(screen_label(0, 2, true), "screen 1 of 2 — cursor is on this screen (primary focus)");
        assert_eq!(screen_label(1, 2, false), "screen 2 of 2 — secondary screen");
    }
}
