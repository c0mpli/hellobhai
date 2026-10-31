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
    todo!("C: map tauri monitors")
}

/// Fits the long side into `maximum_dimension` (1280), keeping the aspect ratio like the Mac.
pub fn downscaled_screenshot_size(width: u32, height: u32, maximum_dimension: u32) -> (u32, u32) {
    todo!("C: downscale")
}

/// Clamps to the screenshot, scales to the monitor, adds the monitor origin.
pub fn global_point_for_screenshot_point(screenshot_x: f64, screenshot_y: f64, capture: &ScreenCapture) -> (i32, i32) {
    todo!("C: screenshot -> global")
}

/// Inverse of `global_point_for_screenshot_point` for a rect, clipped to the screenshot.
pub fn screenshot_pixel_rect_for_global_rect(global_rect: PhysicalRect, capture: &ScreenCapture) -> PhysicalRect {
    todo!("C: global -> screenshot")
}
