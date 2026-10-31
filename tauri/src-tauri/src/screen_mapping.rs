//! Monitors and coordinate mapping between screenshot pixels (what Claude sees)
//! and global physical pixels (what the OS uses). The process is per-monitor DPI
//! aware, so everything is physical pixels with a top-left origin (no y-flip).

use serde::Serialize;
use tauri::AppHandle;

use crate::screen_capture::ScreenCapture;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PhysicalRect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl PhysicalRect {
    /// Right and bottom edges are exclusive, so a point on a shared edge belongs to one monitor.
    pub fn contains_point(&self, x: f64, y: f64) -> bool {
        x >= self.x as f64
            && x < self.x as f64 + self.width as f64
            && y >= self.y as f64
            && y < self.y as f64 + self.height as f64
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MonitorDescription {
    /// Stable index into the monitors sorted by x then y; also the overlay window label suffix.
    pub monitor_index: usize,
    pub rect: PhysicalRect,
    /// The monitor minus the taskbar.
    pub work_area: PhysicalRect,
    pub scale_factor: f64,
}

/// Tauri's `available_monitors()`, sorted by x then y.
pub fn monitors_from_tauri(app: &AppHandle) -> Vec<MonitorDescription> {
    let mut tauri_monitors = app.available_monitors().unwrap_or_default();
    tauri_monitors.sort_by_key(|monitor| (monitor.position().x, monitor.position().y));
    tauri_monitors
        .iter()
        .enumerate()
        .map(|(monitor_index, monitor)| MonitorDescription {
            monitor_index,
            rect: PhysicalRect {
                x: monitor.position().x,
                y: monitor.position().y,
                width: monitor.size().width,
                height: monitor.size().height,
            },
            work_area: PhysicalRect {
                x: monitor.work_area().position.x,
                y: monitor.work_area().position.y,
                width: monitor.work_area().size.width,
                height: monitor.work_area().size.height,
            },
            scale_factor: monitor.scale_factor(),
        })
        .collect()
}

/// Fits the long side into `maximum_dimension` (1280), keeping the aspect ratio like the Mac.
/// Like the Mac, the long side is always exactly `maximum_dimension` (small monitors scale up).
pub fn downscaled_screenshot_size(width: u32, height: u32, maximum_dimension: u32) -> (u32, u32) {
    let aspect_ratio = width as f64 / height as f64;
    if width >= height {
        (maximum_dimension, (maximum_dimension as f64 / aspect_ratio) as u32)
    } else {
        ((maximum_dimension as f64 * aspect_ratio) as u32, maximum_dimension)
    }
}

/// Clamps to the screenshot, scales to the monitor, adds the monitor origin.
pub fn global_point_for_screenshot_point(screenshot_x: f64, screenshot_y: f64, capture: &ScreenCapture) -> (i32, i32) {
    let monitor_rect = capture.monitor.rect;
    let screenshot_width = capture.screenshot_width_in_pixels as f64;
    let screenshot_height = capture.screenshot_height_in_pixels as f64;
    let monitor_pixels_per_screenshot_pixel_horizontally = monitor_rect.width as f64 / screenshot_width;
    let monitor_pixels_per_screenshot_pixel_vertically = monitor_rect.height as f64 / screenshot_height;

    let clamped_screenshot_x = screenshot_x.clamp(0.0, screenshot_width);
    let clamped_screenshot_y = screenshot_y.clamp(0.0, screenshot_height);
    let monitor_local_x = (clamped_screenshot_x * monitor_pixels_per_screenshot_pixel_horizontally).round() as i32;
    let monitor_local_y = (clamped_screenshot_y * monitor_pixels_per_screenshot_pixel_vertically).round() as i32;

    // The far edge (x == width) would be the first pixel of the next monitor.
    let last_monitor_local_x = monitor_rect.width.saturating_sub(1) as i32;
    let last_monitor_local_y = monitor_rect.height.saturating_sub(1) as i32;
    (
        monitor_rect.x + monitor_local_x.min(last_monitor_local_x),
        monitor_rect.y + monitor_local_y.min(last_monitor_local_y),
    )
}

/// Inverse of `global_point_for_screenshot_point` for a rect, clipped to the screenshot.
/// A rect entirely off this monitor comes back with zero width or height.
pub fn screenshot_pixel_rect_for_global_rect(global_rect: PhysicalRect, capture: &ScreenCapture) -> PhysicalRect {
    let monitor_rect = capture.monitor.rect;
    let screenshot_width = capture.screenshot_width_in_pixels as f64;
    let screenshot_height = capture.screenshot_height_in_pixels as f64;
    let screenshot_pixels_per_monitor_pixel_horizontally = screenshot_width / monitor_rect.width as f64;
    let screenshot_pixels_per_monitor_pixel_vertically = screenshot_height / monitor_rect.height as f64;

    let left = ((global_rect.x - monitor_rect.x) as f64 * screenshot_pixels_per_monitor_pixel_horizontally).clamp(0.0, screenshot_width);
    let top = ((global_rect.y - monitor_rect.y) as f64 * screenshot_pixels_per_monitor_pixel_vertically).clamp(0.0, screenshot_height);
    let right = ((global_rect.x - monitor_rect.x + global_rect.width as i32) as f64 * screenshot_pixels_per_monitor_pixel_horizontally)
        .clamp(0.0, screenshot_width);
    let bottom = ((global_rect.y - monitor_rect.y + global_rect.height as i32) as f64 * screenshot_pixels_per_monitor_pixel_vertically)
        .clamp(0.0, screenshot_height);

    let left = left.floor();
    let top = top.floor();
    PhysicalRect {
        x: left as i32,
        y: top as i32,
        width: (right.ceil() - left).max(0.0) as u32,
        height: (bottom.ceil() - top).max(0.0) as u32,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 2x-scaled 4K monitor to the left of the primary one, as Windows numbers it.
    fn capture_of_left_4k_monitor() -> ScreenCapture {
        let (screenshot_width_in_pixels, screenshot_height_in_pixels) = downscaled_screenshot_size(3840, 2160, 1280);
        ScreenCapture {
            jpeg_bytes: Vec::new(),
            label: String::new(),
            is_cursor_screen: false,
            monitor: MonitorDescription {
                monitor_index: 0,
                rect: PhysicalRect { x: -3840, y: -200, width: 3840, height: 2160 },
                work_area: PhysicalRect { x: -3840, y: -200, width: 3840, height: 2100 },
                scale_factor: 2.0,
            },
            screenshot_width_in_pixels,
            screenshot_height_in_pixels,
        }
    }

    #[test]
    fn downscales_like_the_mac() {
        assert_eq!(downscaled_screenshot_size(3840, 2160, 1280), (1280, 720));
        assert_eq!(downscaled_screenshot_size(1920, 1200, 1280), (1280, 800));
        assert_eq!(downscaled_screenshot_size(1080, 1920, 1280), (720, 1280));
        assert_eq!(downscaled_screenshot_size(1366, 768, 1280), (1280, 719));
    }

    #[test]
    fn maps_screenshot_points_to_global_points() {
        let capture = capture_of_left_4k_monitor();
        assert_eq!(global_point_for_screenshot_point(0.0, 0.0, &capture), (-3840, -200));
        assert_eq!(global_point_for_screenshot_point(640.0, 360.0, &capture), (-1920, 880));
        // Clamped to the monitor, never onto its neighbour.
        assert_eq!(global_point_for_screenshot_point(5000.0, -10.0, &capture), (-1, -200));
        assert_eq!(global_point_for_screenshot_point(1280.0, 720.0, &capture), (-1, 1959));
    }

    #[test]
    fn maps_global_rects_back_to_screenshot_pixels() {
        let capture = capture_of_left_4k_monitor();
        let (global_x, global_y) = global_point_for_screenshot_point(100.0, 50.0, &capture);
        let round_trip = screenshot_pixel_rect_for_global_rect(PhysicalRect { x: global_x, y: global_y, width: 300, height: 150 }, &capture);
        assert_eq!(round_trip, PhysicalRect { x: 100, y: 50, width: 100, height: 50 });

        // Hanging off the right edge: clipped to the screenshot.
        let clipped = screenshot_pixel_rect_for_global_rect(PhysicalRect { x: -30, y: -200, width: 300, height: 30 }, &capture);
        assert_eq!(clipped, PhysicalRect { x: 1270, y: 0, width: 10, height: 10 });

        // Entirely on another monitor: empty.
        let elsewhere = screenshot_pixel_rect_for_global_rect(PhysicalRect { x: 100, y: 100, width: 50, height: 50 }, &capture);
        assert_eq!(elsewhere.width, 0);
    }

    #[test]
    fn shared_monitor_edges_belong_to_one_monitor() {
        let rect = PhysicalRect { x: 0, y: 0, width: 1920, height: 1080 };
        assert!(rect.contains_point(0.0, 0.0));
        assert!(rect.contains_point(1919.5, 1079.0));
        assert!(!rect.contains_point(1920.0, 10.0));
        assert!(!rect.contains_point(-0.5, 10.0));
    }
}
