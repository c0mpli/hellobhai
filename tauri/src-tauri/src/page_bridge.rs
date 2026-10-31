//! Shared window settings and the bridge between Rust and the generated Mac
//! pages (board, notes, 3D). Those pages call `window.webkit.messageHandlers`
//! as on the Mac; `PAGE_HOST_BRIDGE_SCRIPT` defines that object and forwards to
//! the `page_message` command, so the pages run unmodified. Rust talks to them
//! with `eval("hb.add(<JSON string literal>)")`, like the Mac.

use tauri::{State, Url, WebviewWindow};

use crate::app_state::AppState;

/// Every window gets the same arguments (WebView2 requires identical arguments for
/// windows sharing one environment). The autoplay flag lets the pen's scratch sound play.
pub const SHARED_BROWSER_ARGUMENTS: &str =
    "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection --autoplay-policy=no-user-gesture-required";

/// Injected with `initialization_script` into the board, notes and 3D windows only.
pub const PAGE_HOST_BRIDGE_SCRIPT: &str = include_str!("../../ui/page-host-bridge.js");

/// Only the app's own pages may load: `http://tauri.localhost` on Windows, `tauri://localhost` elsewhere.
pub fn is_app_origin(url: &Url) -> bool {
    todo!("D: origin check")
}

/// `function_name("<argument as a JSON string literal>")`, safe for quotes, newlines and `</script>`.
pub fn javascript_call_with_string_argument(function_name: &str, argument: &str) -> String {
    todo!("D: serde_json::to_string the argument")
}

/// Messages from the generated pages: board/notes `pen` positions; 3D `model3D`
/// messages (`hitRegions`, `close`, `pen`).
#[tauri::command]
pub fn page_message(window: WebviewWindow, channel: String, message: serde_json::Value, state: State<'_, AppState>) {
    todo!("D: route page messages")
}
