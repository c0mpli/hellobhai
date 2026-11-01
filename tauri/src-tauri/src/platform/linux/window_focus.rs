//! The focused window, through the window manager's EWMH properties:
//! - keyboard focus hand-back for the typed chat (the Mac's
//!   `giveKeyboardFocusBackToFrontmostApp`): before [TYPE]/[KEY] or on minimize
//!   the focus goes back to the window the user was working in;
//! - the orb steps aside for fullscreen video (the Mac's
//!   `frontmostAppHasFullScreenWindow`).
//! Wayland apps are invisible to these X11 calls, so there they report nothing.

use std::os::raw::c_long;

use x11_dl::xlib;

use super::x11_connection::{intern_atom, is_wayland_session, read_long_list_property, with_x11_connection};
use crate::platform::PlatformError;

/// _NET_ACTIVE_WINDOW on the root window (an X window id as an integer), if any.
pub fn foreground_window_id() -> Option<isize> {
    with_x11_connection(|connection| {
        let active_windows = read_long_list_property(connection, connection.root_window, "_NET_ACTIVE_WINDOW");
        active_windows.first().copied().filter(|window_id| *window_id != 0).map(|window_id| window_id as isize)
    })
    .ok()
    .flatten()
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
