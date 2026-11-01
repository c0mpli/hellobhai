//! The focused window, through the window manager's EWMH properties:
//! - keyboard focus hand-back for the typed chat (the Mac's
//!   `giveKeyboardFocusBackToFrontmostApp`): before [TYPE]/[KEY] or on minimize
//!   the focus goes back to the window the user was working in;
//! - the orb steps aside for fullscreen video (the Mac's
//!   `frontmostAppHasFullScreenWindow`).
//! Wayland apps are invisible to these X11 calls, so there they report nothing.
//! Also the GTK side of keeping the orb and chat in order and sized.

use std::os::raw::c_long;

use x11_dl::xlib;

use super::x11_connection::{intern_atom, is_wayland_session, read_long_list_property, with_x11_connection};
use crate::platform::PlatformError;

/// _NET_ACTIVE_WINDOW on the root window (an X window id as an integer), if any.
/// None on Wayland (it only knows XWayland windows, so it is stale or wrong for
/// a native Wayland app) and when it is one of bhai's own windows (its
/// _NET_WM_PID is ours): focus "handed back" there would type into bhai.
pub fn foreground_window_id() -> Option<isize> {
    if is_wayland_session() {
        return None;
    }
    with_x11_connection(|connection| {
        let active_windows = read_long_list_property(connection, connection.root_window, "_NET_ACTIVE_WINDOW");
        let active_window_id = active_windows.first().copied().filter(|window_id| *window_id != 0)?;
        let owning_process_ids = read_long_list_property(connection, active_window_id as xlib::Window, "_NET_WM_PID");
        if owning_process_ids.first() == Some(&(std::process::id() as u64)) {
            return None;
        }
        Some(active_window_id as isize)
    })
    .ok()
    .flatten()
}

/// Raises an always-on-top window above the other always-on-top ones.
/// gtk_window_set_keep_above(TRUE) on a window that already has it doesn't
/// restack it, so the orb could stay over the chat after being re-shown.
/// gdk_window_raise is XRaiseWindow; GTK calls run on the main thread.
pub fn raise_always_on_top_window(window: &tauri::WebviewWindow) {
    let window_to_raise = window.clone();
    let _ = window.run_on_main_thread(move || {
        let gdk_window = window_to_raise.gtk_window().ok().and_then(|gtk_window| gtk::prelude::WidgetExt::window(&gtk_window));
        if let Some(gdk_window) = gdk_window {
            gdk_window.raise();
        }
    });
}

/// GTK pins a non-resizable window to at least its default size (GTK 3.20+
/// `gtk_window_update_fixed_size`), so a plain resize can grow the orb and chat
/// but never shrink them (the chat's minimized tab, a smaller orb). Moving the
/// default size along with the size lifts that floor. Resizable isn't an
/// option: tao lets the user drag the edges of an undecorated resizable window.
pub fn set_fixed_size_window_size(window: &tauri::WebviewWindow, physical_size: tauri::PhysicalSize<u32>) {
    let scale_factor = window.scale_factor().unwrap_or(1.0);
    let logical_size: tauri::LogicalSize<i32> = physical_size.to_logical(scale_factor);
    let window_to_resize = window.clone();
    let _ = window.run_on_main_thread(move || {
        if let Ok(gtk_window) = window_to_resize.gtk_window() {
            use gtk::prelude::GtkWindowExt;
            gtk_window.set_default_size(logical_size.width, logical_size.height);
            gtk_window.resize(logical_size.width, logical_size.height);
        }
    });
}

/// Asks the window manager to activate the window (a _NET_ACTIVE_WINDOW client
/// message with source "pager", which window managers honour without focus
/// stealing prevention), like SetForegroundWindow on Windows.
pub fn bring_window_to_foreground(window_id: isize) -> Result<(), PlatformError> {
    let target_window = window_id as xlib::Window;
    with_x11_connection(|connection| {
        let mut window_attributes: xlib::XWindowAttributes = unsafe { std::mem::zeroed() };
        let window_exists =
            unsafe { (connection.xlib.XGetWindowAttributes)(connection.display, target_window, &mut window_attributes) } != 0;
        if !window_exists {
            return Err(PlatformError::Failed("that window no longer exists".to_string()));
        }
        let mut client_message = xlib::XClientMessageEvent {
            type_: xlib::ClientMessage,
            serial: 0,
            send_event: xlib::True,
            display: connection.display,
            window: target_window,
            message_type: intern_atom(connection, "_NET_ACTIVE_WINDOW"),
            format: 32,
            data: xlib::ClientMessageData::new(),
        };
        const SOURCE_INDICATION_PAGER: c_long = 2;
        client_message.data.set_long(0, SOURCE_INDICATION_PAGER);
        client_message.data.set_long(1, xlib::CurrentTime as c_long);
        let mut event = xlib::XEvent { client_message };
        unsafe {
            (connection.xlib.XSendEvent)(
                connection.display,
                connection.root_window,
                xlib::False,
                xlib::SubstructureRedirectMask | xlib::SubstructureNotifyMask,
                &mut event,
            );
            (connection.xlib.XFlush)(connection.display);
        }
        Ok(())
    })
    // An X error here means the window closed between the check and the message.
    .map_err(|_| PlatformError::Failed("that window no longer exists".to_string()))?
}

/// True when the focused window is fullscreen (_NET_WM_STATE has
/// _NET_WM_STATE_FULLSCREEN): a fullscreen video or app.
pub fn is_foreground_window_full_screen() -> bool {
    if is_wayland_session() {
        return false;
    }
    let Some(active_window_id) = foreground_window_id() else {
        return false;
    };
    with_x11_connection(|connection| {
        let fullscreen_atom = intern_atom(connection, "_NET_WM_STATE_FULLSCREEN") as u64;
        read_long_list_property(connection, active_window_id as xlib::Window, "_NET_WM_STATE").contains(&fullscreen_atom)
    })
    .unwrap_or(false)
}
