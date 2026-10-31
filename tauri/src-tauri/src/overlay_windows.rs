//! One transparent, always-on-top, click-through, non-focusable overlay window
//! per monitor (`ui/overlay.html`): the blue buddy, response bubble, waveform,
//! spinner, gold circles, user drawing and the speaking pill.

use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Manager, PhysicalPosition, PhysicalSize, State};

use crate::app_state::AppState;
use crate::page_bridge::{emit_to_overlays, locked_down_window_builder, OVERLAY_WINDOW_LABEL_PREFIX};
use crate::platform;
use crate::screen_mapping::{monitors_from_tauri, MonitorDescription};

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

impl OverlayMonitorConfig {
    fn for_monitor(monitor: &MonitorDescription) -> Self {
        OverlayMonitorConfig {
            origin_x: monitor.rect.x,
            origin_y: monitor.rect.y,
            width: monitor.rect.width,
            height: monitor.rect.height,
            scale_factor: monitor.scale_factor,
        }
    }
}

/// How often the monitor set is compared to the overlays. Polling is the
/// simplest way to notice a plugged or unplugged display on every OS.
const MONITOR_SET_POLL_INTERVAL: Duration = Duration::from_secs(2);
/// About 60 Hz, like the Mac's cursor tracking timer.
const POINTER_STATE_POLL_INTERVAL: Duration = Duration::from_millis(16);

/// Label "overlay-<index>", url "overlay.html?monitor=<index>"; recreated when the monitor set changes.
pub fn create_overlay_windows(app: &AppHandle) {
    let mut current_monitors = monitors_from_tauri(app);
    create_one_overlay_per_monitor(app, &current_monitors);

    let app_for_monitor_poll = app.clone();
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(MONITOR_SET_POLL_INTERVAL).await;
            let latest_monitors = monitors_from_tauri(&app_for_monitor_poll);
            if latest_monitors != current_monitors {
                destroy_overlay_windows(&app_for_monitor_poll);
                // Destroying is asynchronous; the labels must be free before they are reused.
                for _ in 0..50 {
                    let any_overlay_still_open = app_for_monitor_poll
                        .webview_windows()
                        .keys()
                        .any(|window_label| window_label.starts_with(OVERLAY_WINDOW_LABEL_PREFIX));
                    if !any_overlay_still_open {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
                create_one_overlay_per_monitor(&app_for_monitor_poll, &latest_monitors);
                current_monitors = latest_monitors;
            }
        }
    });
}

fn create_one_overlay_per_monitor(app: &AppHandle, monitors: &[MonitorDescription]) {
    for monitor in monitors {
        let overlay_label = format!("{OVERLAY_WINDOW_LABEL_PREFIX}{}", monitor.monitor_index);
        let overlay_page_path = format!("overlay.html?monitor={}", monitor.monitor_index);
        let build_result = locked_down_window_builder(app, &overlay_label, &overlay_page_path)
            .title("hellobhai overlay")
            .transparent(true)
            .decorations(false)
            .shadow(false)
            .always_on_top(true)
            .focusable(false)
            .resizable(false)
            .build();
        let overlay_window = match build_result {
            Ok(overlay_window) => overlay_window,
            Err(build_error) => {
                eprintln!("couldn't create overlay {overlay_label}: {build_error}");
                continue;
            }
        };
        // Physical pixels: the monitors can have different scale factors.
        let _ = overlay_window.set_position(PhysicalPosition::new(monitor.rect.x, monitor.rect.y));
        let _ = overlay_window.set_size(PhysicalSize::new(monitor.rect.width, monitor.rect.height));
        let _ = overlay_window.show();
        // Click-through; only the push-to-talk hotkey turns it off, for drawing.
        let _ = overlay_window.set_ignore_cursor_events(true);
    }
}

fn destroy_overlay_windows(app: &AppHandle) {
    for (window_label, window) in app.webview_windows() {
        if window_label.starts_with(OVERLAY_WINDOW_LABEL_PREFIX) {
            let _ = window.destroy();
        }
    }
}

/// While the hotkey is held the overlays take the mouse so the user can draw.
pub fn set_overlays_accept_user_drawing(app: &AppHandle, accepts_user_drawing: bool) {
    for (window_label, window) in app.webview_windows() {
        if window_label.starts_with(OVERLAY_WINDOW_LABEL_PREFIX) {
            let _ = window.set_ignore_cursor_events(!accepts_user_drawing);
        }
    }
    emit_to_overlays(app, "drawing-enabled", serde_json::json!({ "enabled": accepts_user_drawing }));
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
struct PointerState {
    x: i32,
    y: i32,
    visible: bool,
}

/// About 60 Hz: emits `pointer-state {x, y, visible}` when the pointer moves or hides.
pub fn start_pointer_state_emitter(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let mut last_emitted_pointer_state: Option<PointerState> = None;
        let mut poll_interval = tokio::time::interval(POINTER_STATE_POLL_INTERVAL);
        poll_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            poll_interval.tick().await;
            let Ok(cursor_position) = app.cursor_position() else { continue };
            let pointer_state = PointerState {
                x: cursor_position.x.round() as i32,
                y: cursor_position.y.round() as i32,
                visible: platform::is_system_pointer_visible(),
            };
            if last_emitted_pointer_state != Some(pointer_state) {
                emit_to_overlays(&app, "pointer-state", pointer_state);
                last_emitted_pointer_state = Some(pointer_state);
            }
        }
    });
}

#[tauri::command]
pub fn overlay_ready(monitor_index: usize, app: AppHandle) -> OverlayMonitorConfig {
    let monitors = monitors_from_tauri(&app);
    let monitor = monitors
        .iter()
        .find(|monitor| monitor.monitor_index == monitor_index)
        .or_else(|| monitors.first());
    match monitor {
        Some(monitor) => OverlayMonitorConfig::for_monitor(monitor),
        // No monitors reported: a zero-size config draws nothing rather than failing.
        None => OverlayMonitorConfig { origin_x: 0, origin_y: 0, width: 0, height: 0, scale_factor: 1.0 },
    }
}

/// A finished freehand stroke in global physical pixels.
#[tauri::command]
pub fn user_stroke_added(points: Vec<(i32, i32)>, state: State<'_, AppState>) {
    if !points.is_empty() {
        state.companion_brain.add_user_stroke(points);
    }
}
