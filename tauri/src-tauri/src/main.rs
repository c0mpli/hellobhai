// No console window behind the app in release builds on Windows.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // Environment variables GTK and WebKitGTK read at startup (XWayland on Wayland).
    #[cfg(target_os = "linux")]
    hellobhai_lib::platform::prepare_process_environment();
    hellobhai_lib::run()
}
