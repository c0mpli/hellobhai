//! One transparent, always-on-top, click-through, non-focusable overlay window
//! per monitor (`ui/overlay.html`): the blue buddy, response bubble, waveform,
//! spinner, gold circles, user drawing and the speaking pill.

use serde::Serialize;
use tauri::{AppHandle, State};

use crate::app_state::AppState;

/// What an overlay page needs to turn global physical pixels into its own CSS px:
/// `(x - originX) / scaleFactor`.
#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OverlayMonitorConfig {
    pub origin_x: i32,
    pub origin_y: i32,
    pub width: u32,
    pub height: u32,
    pub scale_factor: f64,
}

/// Label "overlay-<index>", url "overlay.html?monitor=<index>"; recreated when the monitor set changes.
pub fn create_overlay_windows(app: &AppHandle) {
    todo!("D: one overlay per monitor")
}

/// While the hotkey is held the overlays take the mouse so the user can draw.
pub fn set_overlays_accept_user_drawing(app: &AppHandle, accepts_user_drawing: bool) {
    todo!("D: set_ignore_cursor_events + emit drawing-enabled")
}

/// About 60 Hz: emits `pointer-state {x, y, visible}` when the pointer moves or hides.
pub fn start_pointer_state_emitter(app: AppHandle) {
    todo!("D: cursor_position + platform::is_system_pointer_visible")
}

#[tauri::command]
pub fn overlay_ready(monitor_index: usize, app: AppHandle) -> OverlayMonitorConfig {
    todo!("D: monitor config for the overlay page")
}

/// A finished freehand stroke in global physical pixels.
#[tauri::command]
pub fn user_stroke_added(points: Vec<(i32, i32)>, state: State<'_, AppState>) {
    todo!("D: state.companion_brain.add_user_stroke(points)")
}
