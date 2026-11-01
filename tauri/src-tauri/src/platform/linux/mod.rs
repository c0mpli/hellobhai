//! Linux implementations. Same file and function names as `../windows/`.
//! X11 is the full experience; on Wayland the app runs through XWayland (see
//! `prepare_process_environment`) with the limits `platform_limitation_note`
//! tells the user about.

mod child_process;
mod claude_tools;
mod input;
mod keyboard_hook;
mod pointer;
mod runtime_downloads;
mod screen_capture;
mod system_settings;
mod system_voice;
mod window_focus;
mod x11_connection;

pub use child_process::*;
pub use claude_tools::*;
pub use input::*;
pub use keyboard_hook::*;
pub use pointer::*;
pub use runtime_downloads::*;
pub use screen_capture::*;
pub use system_settings::*;
pub use system_voice::*;
pub use window_focus::*;

/// Called first thing in `main`, before GTK starts or any thread exists.
/// - Wayland gives apps no global window positions, always-on-top or
///   click-through, which the overlays need, so GTK is pointed at XWayland
///   (GDK_BACKEND=x11) unless the user chose a backend themselves.
/// - WebKitGTK's DMA-BUF renderer shows blank windows on some GPU drivers
///   (NVIDIA especially); turning it off costs nothing for these small pages.
pub fn prepare_process_environment() {
    if x11_connection::is_wayland_session() && std::env::var_os("GDK_BACKEND").is_none() {
        std::env::set_var("GDK_BACKEND", "x11");
    }
    if std::env::var_os("WEBKIT_DISABLE_DMABUF_RENDERER").is_none() {
        std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
    }
}
