//! Shared window settings and the bridge between Rust and the generated Mac
//! pages (board, notes, 3D). Those pages call `window.webkit.messageHandlers`
//! as on the Mac; `PAGE_HOST_BRIDGE_SCRIPT` defines that object and forwards to
//! the `page_message` command, so the pages run unmodified. Rust talks to them
//! with `eval("hb.add(<JSON string literal>)")`, like the Mac.

use serde::Serialize;
use tauri::webview::{NewWindowResponse, WebviewWindowBuilder};
use tauri::{AppHandle, Emitter, EventTarget, Manager, State, Url, WebviewUrl, WebviewWindow};

use crate::app_state::AppState;
use crate::screen_mapping::MonitorDescription;

/// Every window gets the same arguments (WebView2 requires identical arguments for
/// windows sharing one environment). The autoplay flag lets the pen's scratch sound play.
pub const SHARED_BROWSER_ARGUMENTS: &str =
    "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection --autoplay-policy=no-user-gesture-required";

/// Injected with `initialization_script` into the board, notes and 3D windows only.
pub const PAGE_HOST_BRIDGE_SCRIPT: &str = include_str!("../../ui/page-host-bridge.js");

/// Window label prefix of the per-monitor overlays ("overlay-0", "overlay-1", ...).
pub const OVERLAY_WINDOW_LABEL_PREFIX: &str = "overlay-";
pub const BOARD_WINDOW_LABEL: &str = "board";
/// The 3D overlay's label is this plus a counter ("model3d-1", "model3d-2", ...),
/// because destroying the previous window is asynchronous.
pub const MODEL3D_WINDOW_LABEL_PREFIX: &str = "model3d-";
pub const SETTINGS_WINDOW_LABEL: &str = "settings";

/// Only the app's own pages may load: `http://tauri.localhost` on Windows, `tauri://localhost` elsewhere.
pub fn is_app_origin(url: &Url) -> bool {
    match (url.scheme(), url.host_str()) {
        ("http" | "https", Some("tauri.localhost")) => url.port().is_none(),
        ("tauri", Some("localhost")) => true,
        _ => false,
    }
}

/// `function_name("<argument as a JSON string literal>")`, safe for quotes, newlines and `</script>`.
pub fn javascript_call_with_string_argument(function_name: &str, argument: &str) -> String {
    let argument_as_json_string_literal = serde_json::to_string(argument).expect("a string always serializes");
    format!("{function_name}({argument_as_json_string_literal})")
}

/// A window builder with the lockdown every app window shares: same browser
/// arguments, hidden from screenshots (so Claude never sees our own overlays),
/// navigation limited to the app's pages, popups refused, no taskbar button.
/// Starts hidden so callers can place it in physical pixels before showing it.
pub fn locked_down_window_builder<'a>(
    app: &'a AppHandle,
    label: &str,
    page_path: &str,
) -> WebviewWindowBuilder<'a, tauri::Wry, AppHandle> {
    WebviewWindowBuilder::new(app, label, WebviewUrl::App(page_path.into()))
        .additional_browser_args(SHARED_BROWSER_ARGUMENTS)
        .content_protected(true)
        .on_navigation(|url| is_app_origin(url))
        .on_new_window(|_url, _features| NewWindowResponse::Deny)
        .skip_taskbar(true)
        .visible(false)
}

/// Emits only to the per-monitor overlay pages, so 60 Hz pointer updates never
/// reach the heavy board or 3D pages.
pub fn emit_to_overlays<Payload: Serialize + Clone>(app: &AppHandle, event_name: &str, payload: Payload) {
    let emit_result = app.emit_filter(event_name, payload, |event_target| match event_target {
        EventTarget::WebviewWindow { label } | EventTarget::Webview { label } | EventTarget::Window { label } => {
            label.starts_with(OVERLAY_WINDOW_LABEL_PREFIX)
        }
        _ => false,
    });
    if let Err(emit_error) = emit_result {
        eprintln!("emit {event_name} to overlays failed: {emit_error}");
    }
}

/// The monitor containing a global physical point, or the first monitor when none does.
pub fn monitor_containing_point(monitors: &[MonitorDescription], global_x: f64, global_y: f64) -> Option<&MonitorDescription> {
    monitors
        .iter()
        .find(|monitor| {
            let monitor_rect = monitor.rect;
            global_x >= monitor_rect.x as f64
                && global_x < monitor_rect.x as f64 + monitor_rect.width as f64
                && global_y >= monitor_rect.y as f64
                && global_y < monitor_rect.y as f64 + monitor_rect.height as f64
        })
        .or_else(|| monitors.first())
}

/// A pen position posted by a generated page (page CSS px, top-left of the
/// webview) as global physical pixels, or None when the pen rests (`{hidden: true}`).
pub fn global_pen_position_for_page_message(window: &WebviewWindow, message: &serde_json::Value) -> Option<(f64, f64)> {
    let page_x = message.get("x")?.as_f64()?;
    let page_y = message.get("y")?.as_f64()?;
    if !window.is_visible().unwrap_or(false) {
        return None;
    }
    let window_content_origin = window.inner_position().ok()?;
    let scale_factor = window.scale_factor().ok()?;
    Some((
        window_content_origin.x as f64 + page_x * scale_factor,
        window_content_origin.y as f64 + page_y * scale_factor,
    ))
}

/// Tells the overlays where the drawing head is (the buddy becomes the pen), or that it rests.
pub fn emit_pen_position(app: &AppHandle, global_pen_position: Option<(f64, f64)>) {
    let pen_position_payload = global_pen_position.map(|(pen_x, pen_y)| serde_json::json!({ "x": pen_x, "y": pen_y }));
    emit_to_overlays(app, "pen-position", pen_position_payload);
}

/// Messages from the generated pages: board/notes `pen` positions; 3D `model3D`
/// messages (`hitRegions`, `close`, `pen`).
#[tauri::command]
pub fn page_message(window: WebviewWindow, channel: String, message: serde_json::Value, state: State<'_, AppState>) {
    let app = window.app_handle().clone();
    let is_model3d_window = window.label().starts_with(MODEL3D_WINDOW_LABEL_PREFIX);
    match (window.label(), channel.as_str()) {
        (BOARD_WINDOW_LABEL, "pen") => {
            emit_pen_position(&app, global_pen_position_for_page_message(&window, &message));
        }
        (_, "model3D") if is_model3d_window => match message.get("kind").and_then(|kind| kind.as_str()) {
            Some("hitRegions") => {
                let hit_region_rects = message
                    .get("rects")
                    .and_then(|rects| serde_json::from_value::<Vec<Vec<f64>>>(rects.clone()).ok())
                    .unwrap_or_default()
                    .into_iter()
                    .filter_map(|rect_values| <[f64; 4]>::try_from(rect_values).ok())
                    .collect();
                state.model3d_window.set_hit_regions(hit_region_rects);
            }
            Some("close") => {
                // Destroying a webview from inside its own IPC call is unsafe on
                // WebView2, so the close runs on the async runtime instead.
                let model3d_window = state.model3d_window.clone();
                tauri::async_runtime::spawn(async move { model3d_window.close_because_user_asked_to() });
            }
            Some("pen") => {
                emit_pen_position(&app, global_pen_position_for_page_message(&window, &message));
            }
            _ => {}
        },
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::screen_mapping::PhysicalRect;

    #[test]
    fn only_the_app_origins_count_as_app_pages() {
        assert!(is_app_origin(&Url::parse("http://tauri.localhost/board.html").unwrap()));
        assert!(is_app_origin(&Url::parse("https://tauri.localhost/notes.html").unwrap()));
        assert!(is_app_origin(&Url::parse("tauri://localhost/overlay.html?monitor=1").unwrap()));
        assert!(!is_app_origin(&Url::parse("http://tauri.localhost.evil.com/").unwrap()));
        assert!(!is_app_origin(&Url::parse("http://tauri.localhost:8080/").unwrap()));
        assert!(!is_app_origin(&Url::parse("https://example.com/").unwrap()));
        assert!(!is_app_origin(&Url::parse("file:///C:/Windows/").unwrap()));
        assert!(!is_app_origin(&Url::parse("about:blank").unwrap()));
    }

    #[test]
    fn eval_arguments_are_json_string_literals_that_cannot_break_out() {
        assert_eq!(javascript_call_with_string_argument("hb.add", r#"{"t":"box"}"#), r#"hb.add("{\"t\":\"box\"}")"#);
        assert_eq!(javascript_call_with_string_argument("hb.add", "a\nb"), r#"hb.add("a\nb")"#);
        assert_eq!(javascript_call_with_string_argument("hb.add", "\"); alert(1); (\""), r#"hb.add("\"); alert(1); (\"")"#);
        assert_eq!(javascript_call_with_string_argument("hb.add", "</script>"), r#"hb.add("</script>")"#);
        assert_eq!(javascript_call_with_string_argument("hb.add", "नमस्ते"), "hb.add(\"नमस्ते\")");
        assert_eq!(javascript_call_with_string_argument("hb.add", "back\\slash"), r#"hb.add("back\\slash")"#);
    }

    #[test]
    fn the_monitor_under_a_point_is_found_and_falls_back_to_the_first() {
        let monitor_at = |monitor_index: usize, x: i32| MonitorDescription {
            monitor_index,
            rect: PhysicalRect { x, y: 0, width: 1920, height: 1080 },
            work_area: PhysicalRect { x, y: 0, width: 1920, height: 1040 },
            scale_factor: 1.0,
        };
        let monitors = vec![monitor_at(0, -1920), monitor_at(1, 0)];
        assert_eq!(monitor_containing_point(&monitors, -10.0, 500.0).unwrap().monitor_index, 0);
        assert_eq!(monitor_containing_point(&monitors, 0.0, 0.0).unwrap().monitor_index, 1);
        assert_eq!(monitor_containing_point(&monitors, 5000.0, 0.0).unwrap().monitor_index, 0);
        assert!(monitor_containing_point(&[], 0.0, 0.0).is_none());
    }
}
