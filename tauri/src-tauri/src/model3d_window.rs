//! The transparent 3D model overlay (`generated/model3d.html`, three.js), mirror
//! of the Mac's `Model3DOverlayManager`: covers the monitor, click-through except
//! over the model and labels (the page posts hit regions; a 30 Hz task toggles
//! `set_ignore_cursor_events`).

use std::sync::{Arc, Mutex};
use std::time::Duration;

use tauri::webview::PageLoadEvent;
use tauri::{AppHandle, Manager, PhysicalPosition, PhysicalSize, WebviewWindow};

use crate::page_bridge::{
    emit_pen_position, javascript_call_with_string_argument, locked_down_window_builder, MODEL3D_WINDOW_LABEL_PREFIX,
    PAGE_HOST_BRIDGE_SCRIPT,
};
use crate::platform;
use crate::screen_mapping::MonitorDescription;

/// Like the Mac's mouse pass-through timer.
const MOUSE_PASS_THROUGH_POLL_INTERVAL: Duration = Duration::from_millis(33);

type UserClosedCallback = Arc<dyn Fn() + Send + Sync>;

#[derive(Default)]
struct Model3DWindowState {
    window: Option<WebviewWindow>,
    is_page_ready: bool,
    pending_command_lines: Vec<String>,
    /// `hb.configure(...)` for this overlay (numbers only), run once the page has loaded.
    page_configuration_script: Option<String>,
    /// Where the model and its labels are, page CSS px `[x, y, width, height]`.
    hit_region_rects: Vec<[f64; 4]>,
    is_mouse_over_hit_region: bool,
    /// Bumped on every open and close so a previous window's tracking task stops.
    window_generation: u64,
    on_user_closed: Option<UserClosedCallback>,
}

/// Window handle, queued lines, page-ready flag, hit regions and the user-closed callback.
#[derive(Default)]
pub struct Model3DWindow {
    state: Arc<Mutex<Model3DWindowState>>,
}

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
        // A fresh window per model: closing destroys it, which is what stops WebGL and the pen sound.
        self.close();

        let window_generation = {
            let mut model3d_state = self.state.lock().unwrap();
            model3d_state.window_generation += 1;
            model3d_state.is_page_ready = false;
            model3d_state.pending_command_lines.clear();
            model3d_state.hit_region_rects.clear();
            model3d_state.is_mouse_over_hit_region = false;
            model3d_state.page_configuration_script = Some(model3d_configure_script(
                monitor,
                screenshot_width,
                screenshot_height,
                focus_screenshot_x,
                focus_screenshot_y,
            ));
            model3d_state.window_generation
        };

        let state_for_page_load = self.state.clone();
        // A new label per window: destroying the previous one is asynchronous, so its label may not be free yet.
        let model3d_window_label = format!("{MODEL3D_WINDOW_LABEL_PREFIX}{window_generation}");
        let build_result = locked_down_window_builder(app, &model3d_window_label, "model3d.html")
            .title("bhai 3D")
            .transparent(true)
            .decorations(false)
            .shadow(false)
            .always_on_top(true)
            .focusable(false)
            .resizable(false)
            .incognito(true)
            .initialization_script(PAGE_HOST_BRIDGE_SCRIPT)
            .on_page_load(move |model3d_webview_window, page_load_payload| {
                if page_load_payload.event() != PageLoadEvent::Finished {
                    return;
                }
                let (page_configuration_script, queued_command_lines) = {
                    let mut model3d_state = state_for_page_load.lock().unwrap();
                    model3d_state.is_page_ready = true;
                    (
                        model3d_state.page_configuration_script.clone(),
                        std::mem::take(&mut model3d_state.pending_command_lines),
                    )
                };
                if let Some(page_configuration_script) = page_configuration_script {
                    let _ = model3d_webview_window.eval(page_configuration_script);
                }
                for queued_command_line in queued_command_lines {
                    let _ = model3d_webview_window.eval(javascript_call_with_string_argument("hb.add", &queued_command_line));
                }
            })
            .build();

        let model3d_webview_window = match build_result {
            Ok(model3d_webview_window) => model3d_webview_window,
            Err(build_error) => {
                eprintln!("couldn't create the 3D window: {build_error}");
                return;
            }
        };
        // The screenshot covers the whole monitor, so the overlay does too.
        let _ = model3d_webview_window.set_position(PhysicalPosition::new(monitor.rect.x, monitor.rect.y));
        let _ = model3d_webview_window.set_size(PhysicalSize::new(monitor.rect.width, monitor.rect.height));
        let _ = model3d_webview_window.show();
        let _ = model3d_webview_window.set_ignore_cursor_events(true);
        self.state.lock().unwrap().window = Some(model3d_webview_window);

        start_mouse_pass_through_tracking(app.clone(), self.state.clone(), window_generation);
    }

    pub fn add_command_line(&self, line: String) {
        let mut model3d_state = self.state.lock().unwrap();
        // Closed (by the user or a newer request): nothing left to build on.
        let Some(model3d_webview_window) = model3d_state.window.clone() else { return };
        if model3d_state.is_page_ready {
            let _ = model3d_webview_window.eval(javascript_call_with_string_argument("hb.add", &line));
        } else {
            model3d_state.pending_command_lines.push(line);
        }
    }

    /// Destroys the window (stops WebGL and sound); pen-position null.
    pub fn close(&self) {
        let closed_window = {
            let mut model3d_state = self.state.lock().unwrap();
            model3d_state.window_generation += 1;
            model3d_state.is_page_ready = false;
            model3d_state.pending_command_lines.clear();
            model3d_state.hit_region_rects.clear();
            model3d_state.window.take()
        };
        if let Some(closed_window) = closed_window {
            let app = closed_window.app_handle().clone();
            let _ = closed_window.destroy();
            emit_pen_position(&app, None);
        }
    }

    pub fn is_visible(&self) -> bool {
        self.state.lock().unwrap().window.is_some()
    }

    /// Called when the user closes the model (Esc or its close button), not when the app does.
    pub fn set_on_user_closed(&self, callback: Box<dyn Fn() + Send + Sync>) {
        self.state.lock().unwrap().on_user_closed = Some(Arc::from(callback));
    }

    /// On `HotkeyEvent::EscapePressed`: if visible, close and call the user-closed callback.
    pub fn handle_escape_pressed(&self) {
        self.close_because_user_asked_to();
    }

    /// Esc or the page's close button: close, then let the brain stop the narration.
    pub fn close_because_user_asked_to(&self) {
        if !self.is_visible() {
            return;
        }
        self.close();
        let on_user_closed = self.state.lock().unwrap().on_user_closed.clone();
        if let Some(on_user_closed) = on_user_closed {
            on_user_closed();
        }
    }

    /// The page's interactive rects in CSS px, from its `hitRegions` message.
    pub fn set_hit_regions(&self, rects: Vec<[f64; 4]>) {
        self.state.lock().unwrap().hit_region_rects = rects;
    }
}

/// `hb.configure(sw, sh, fx, fy, top, right, bottom, left)` with `%.1f` numbers.
/// The insets keep the model and labels off the taskbar: monitor rect minus work
/// area, in the page's CSS px (physical / scale factor), Mac order top-right-bottom-left.
pub fn model3d_configure_script(
    monitor: &MonitorDescription,
    screenshot_width: u32,
    screenshot_height: u32,
    focus_screenshot_x: f64,
    focus_screenshot_y: f64,
) -> String {
    let monitor_rect = monitor.rect;
    let work_area = monitor.work_area;
    let scale_factor = if monitor.scale_factor > 0.0 { monitor.scale_factor } else { 1.0 };
    let monitor_right = monitor_rect.x as f64 + monitor_rect.width as f64;
    let monitor_bottom = monitor_rect.y as f64 + monitor_rect.height as f64;
    let work_area_right = work_area.x as f64 + work_area.width as f64;
    let work_area_bottom = work_area.y as f64 + work_area.height as f64;
    let inset_top = (work_area.y - monitor_rect.y) as f64 / scale_factor;
    let inset_right = (monitor_right - work_area_right) / scale_factor;
    let inset_bottom = (monitor_bottom - work_area_bottom) / scale_factor;
    let inset_left = (work_area.x - monitor_rect.x) as f64 / scale_factor;
    format!(
        "hb.configure({:.1}, {:.1}, {:.1}, {:.1}, {:.1}, {:.1}, {:.1}, {:.1})",
        screenshot_width as f64,
        screenshot_height as f64,
        focus_screenshot_x,
        focus_screenshot_y,
        inset_top.max(0.0),
        inset_right.max(0.0),
        inset_bottom.max(0.0),
        inset_left.max(0.0),
    )
}

/// True when a point in page CSS px lies inside any `[x, y, width, height]` rect.
pub fn point_is_inside_any_rect(page_x: f64, page_y: f64, rects: &[[f64; 4]]) -> bool {
    rects.iter().any(|&[rect_x, rect_y, rect_width, rect_height]| {
        page_x >= rect_x && page_x <= rect_x + rect_width && page_y >= rect_y && page_y <= rect_y + rect_height
    })
}

/// Takes the mouse only while it's over the model or a label, so the rest of the
/// overlay never blocks clicks on the screen underneath.
fn start_mouse_pass_through_tracking(app: AppHandle, state: Arc<Mutex<Model3DWindowState>>, window_generation: u64) {
    tauri::async_runtime::spawn(async move {
        let mut poll_interval = tokio::time::interval(MOUSE_PASS_THROUGH_POLL_INTERVAL);
        poll_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            poll_interval.tick().await;
            let (model3d_webview_window, hit_region_rects, was_mouse_over_hit_region) = {
                let model3d_state = state.lock().unwrap();
                if model3d_state.window_generation != window_generation {
                    return;
                }
                let Some(model3d_webview_window) = model3d_state.window.clone() else { return };
                (model3d_webview_window, model3d_state.hit_region_rects.clone(), model3d_state.is_mouse_over_hit_region)
            };
            // Never switch mid-drag, or an orbit drag would be cut off at the model's edge.
            if platform::is_left_mouse_button_down() {
                continue;
            }
            let (Ok(cursor_position), Ok(window_origin), Ok(scale_factor)) = (
                app.cursor_position(),
                model3d_webview_window.inner_position(),
                model3d_webview_window.scale_factor(),
            ) else {
                continue;
            };
            let cursor_page_x = (cursor_position.x - window_origin.x as f64) / scale_factor;
            let cursor_page_y = (cursor_position.y - window_origin.y as f64) / scale_factor;
            let is_mouse_over_hit_region = point_is_inside_any_rect(cursor_page_x, cursor_page_y, &hit_region_rects);
            if is_mouse_over_hit_region == was_mouse_over_hit_region {
                continue;
            }
            state.lock().unwrap().is_mouse_over_hit_region = is_mouse_over_hit_region;
            let _ = model3d_webview_window.set_ignore_cursor_events(!is_mouse_over_hit_region);
            // A non-focused window gets no hover events, so the page is told when to show its close button.
            let _ = model3d_webview_window.eval(format!("hb.setHover({is_mouse_over_hit_region})"));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::screen_mapping::PhysicalRect;

    #[test]
    fn configure_insets_exclude_the_taskbar_in_css_pixels() {
        // 4K monitor at 150% with a 72 px (48 logical) taskbar at the bottom.
        let monitor = MonitorDescription {
            monitor_index: 0,
            rect: PhysicalRect { x: 0, y: 0, width: 3840, height: 2160 },
            work_area: PhysicalRect { x: 0, y: 0, width: 3840, height: 2088 },
            scale_factor: 1.5,
        };
        assert_eq!(
            model3d_configure_script(&monitor, 1280, 720, 640.0, 360.26),
            "hb.configure(1280.0, 720.0, 640.0, 360.3, 0.0, 0.0, 48.0, 0.0)"
        );

        // Second monitor on the left with the taskbar docked on its left edge.
        let left_monitor = MonitorDescription {
            monitor_index: 1,
            rect: PhysicalRect { x: -1920, y: 0, width: 1920, height: 1080 },
            work_area: PhysicalRect { x: -1858, y: 0, width: 1858, height: 1080 },
            scale_factor: 1.0,
        };
        assert_eq!(
            model3d_configure_script(&left_monitor, 1280, 720, 10.0, 20.0),
            "hb.configure(1280.0, 720.0, 10.0, 20.0, 0.0, 0.0, 0.0, 62.0)"
        );
    }

    #[test]
    fn hit_regions_include_their_edges() {
        let rects = [[10.0, 10.0, 100.0, 50.0], [300.0, 0.0, 10.0, 10.0]];
        assert!(point_is_inside_any_rect(10.0, 60.0, &rects));
        assert!(point_is_inside_any_rect(305.0, 5.0, &rects));
        assert!(!point_is_inside_any_rect(200.0, 30.0, &rects));
        assert!(!point_is_inside_any_rect(0.0, 0.0, &[]));
    }
}
