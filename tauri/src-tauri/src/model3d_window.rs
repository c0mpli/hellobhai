//! The transparent 3D model overlay (`generated/model3d.html`, three.js), mirror
//! of the Mac's `Model3DOverlayManager`: covers the monitor, click-through except
//! over the model and labels (the page posts hit regions; a 30 Hz task toggles
//! `set_ignore_cursor_events`).

use tauri::AppHandle;

use crate::screen_mapping::MonitorDescription;

/// Window handle, queued lines, page-ready flag, hit regions and the user-closed callback.
#[derive(Default)]
pub struct Model3DWindow {}

impl Model3DWindow {
    pub fn new() -> Self {
        Self::default()
    }

    /// Opens over `monitor`; after load calls `hb.configure(sw, sh, fx, fy, top, right, bottom, left)`.
    pub fn begin(
        &self,
        app: &AppHandle,
        monitor: &MonitorDescription,
        screenshot_width: u32,
        screenshot_height: u32,
        focus_screenshot_x: f64,
        focus_screenshot_y: f64,
    ) {
        todo!("D: create the 3D window")
    }

    pub fn add_command_line(&self, line: String) {
        todo!("D: queue or eval")
    }

    /// Destroys the window (stops WebGL and sound); pen-position null.
    pub fn close(&self) {
        todo!("D: destroy the 3D window")
    }

    pub fn is_visible(&self) -> bool {
        todo!("D: visibility")
    }

    /// Called when the user closes the model (Esc or its close button), not when the app does.
    pub fn set_on_user_closed(&self, callback: Box<dyn Fn() + Send + Sync>) {
        todo!("D: store the callback")
    }

    /// On `HotkeyEvent::EscapePressed`: if visible, close and call the user-closed callback.
    pub fn handle_escape_pressed(&self) {
        todo!("D: Esc closes the model")
    }

    /// The page's interactive rects in CSS px, from its `hitRegions` message.
    pub fn set_hit_regions(&self, rects: Vec<[f64; 4]>) {
        todo!("D: store hit regions")
    }
}
