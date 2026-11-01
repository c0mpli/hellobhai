//! One shared X11 connection for the short calls (pointer, clicks, typing,
//! screenshots, focus), behind a mutex because Xlib connections are not
//! thread-safe. The libraries are loaded with dlopen (x11-dl), so the app still
//! starts on a system without libXtst or libXfixes; those features then fail
//! with a message instead.
//!
//! X errors: Xlib's default handler exits the process, and GTK replaces it with
//! one that aborts on errors it didn't expect. Every call here runs inside
//! `with_x11_connection`, which installs a handler that only records the error,
//! syncs, and puts the previous handler back.

use std::os::raw::c_int;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};

use x11_dl::xfixes::Xlib as XFixesLibrary;
use x11_dl::xlib::{self, Display, Xlib};
use x11_dl::xtest::Xf86vmode as XTestLibrary;

use crate::platform::PlatformError;

pub struct X11Connection {
    pub xlib: &'static Xlib,
    pub display: *mut Display,
    pub root_window: xlib::Window,
}

// The display pointer is only used while the mutex is held.
unsafe impl Send for X11Connection {}

static XLIB: OnceLock<Option<Xlib>> = OnceLock::new();
static XTEST: OnceLock<Option<XTestLibrary>> = OnceLock::new();
static XFIXES: OnceLock<Option<XFixesLibrary>> = OnceLock::new();
static SHARED_CONNECTION: Mutex<Option<X11Connection>> = Mutex::new(None);
static X11_ERROR_HAPPENED: AtomicBool = AtomicBool::new(false);

pub fn xlib() -> Option<&'static Xlib> {
    XLIB.get_or_init(|| Xlib::open().ok()).as_ref()
}

pub fn xtest() -> Option<&'static XTestLibrary> {
    XTEST.get_or_init(|| XTestLibrary::open().ok()).as_ref()
}

pub fn xfixes() -> Option<&'static XFixesLibrary> {
    XFIXES.get_or_init(|| XFixesLibrary::open().ok()).as_ref()
}

/// True in a Wayland session (bhai then runs through XWayland, see
/// `prepare_process_environment`).
pub fn is_wayland_session() -> bool {
    let session_type_is_wayland = std::env::var("XDG_SESSION_TYPE").is_ok_and(|session_type| session_type.eq_ignore_ascii_case("wayland"));
    session_type_is_wayland || std::env::var_os("WAYLAND_DISPLAY").is_some()
}

/// Opens a new connection to $DISPLAY (the keyboard listener keeps its own).
pub fn open_display() -> Result<(&'static Xlib, *mut Display), PlatformError> {
    let xlib = xlib().ok_or_else(|| PlatformError::Failed("libX11 is not installed".into()))?;
    let display = unsafe { (xlib.XOpenDisplay)(std::ptr::null()) };
    if display.is_null() {
        return Err(PlatformError::Failed("cannot connect to the X display (is DISPLAY set?)".into()));
    }
    Ok((xlib, display))
}

unsafe extern "C" fn record_x11_error(_display: *mut Display, _error_event: *mut xlib::XErrorEvent) -> c_int {
    X11_ERROR_HAPPENED.store(true, Ordering::SeqCst);
    0
}

/// Runs `x11_calls` on the shared connection with X errors recorded instead of
/// fatal. Returns Err when the calls raised an X error (e.g. a window that
/// closed in the meantime) or when there is no X display.
pub fn with_x11_connection<T>(x11_calls: impl FnOnce(&X11Connection) -> T) -> Result<T, PlatformError> {
    let mut shared_connection = SHARED_CONNECTION.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if shared_connection.is_none() {
        let (xlib, display) = open_display()?;
        let root_window = unsafe { (xlib.XDefaultRootWindow)(display) };
        *shared_connection = Some(X11Connection { xlib, display, root_window });
    }
    let connection = shared_connection.as_ref().expect("connection was just opened");
    let xlib = connection.xlib;

    X11_ERROR_HAPPENED.store(false, Ordering::SeqCst);
    let previous_error_handler = unsafe { (xlib.XSetErrorHandler)(Some(record_x11_error)) };
    let result = x11_calls(connection);
    // Errors arrive asynchronously; XSync makes sure every one of ours has been handled.
    unsafe {
        (xlib.XSync)(connection.display, xlib::False);
        (xlib.XSetErrorHandler)(previous_error_handler);
    }
    if X11_ERROR_HAPPENED.swap(false, Ordering::SeqCst) {
        return Err(PlatformError::Failed("the X server reported an error".into()));
    }
    Ok(result)
}

/// The atom for `name` (created if missing).
pub fn intern_atom(connection: &X11Connection, name: &str) -> xlib::Atom {
    let atom_name = std::ffi::CString::new(name).expect("atom names have no NUL");
    unsafe { (connection.xlib.XInternAtom)(connection.display, atom_name.as_ptr(), xlib::False) }
}

/// A window property made of 32-bit items (windows, atoms), as returned by
/// XGetWindowProperty (each item in a C long).
pub fn read_long_list_property(connection: &X11Connection, window: xlib::Window, property_name: &str) -> Vec<u64> {
    let property_atom = intern_atom(connection, property_name);
    let mut actual_type: xlib::Atom = 0;
    let mut actual_format: c_int = 0;
    let mut item_count: std::os::raw::c_ulong = 0;
    let mut bytes_after: std::os::raw::c_ulong = 0;
    let mut property_data: *mut u8 = std::ptr::null_mut();
    let status = unsafe {
        (connection.xlib.XGetWindowProperty)(
            connection.display,
            window,
            property_atom,
            0,
            1024,
            xlib::False,
            xlib::AnyPropertyType as xlib::Atom,
            &mut actual_type,
            &mut actual_format,
            &mut item_count,
            &mut bytes_after,
            &mut property_data,
        )
    };
    if status != xlib::Success as c_int || property_data.is_null() {
        return Vec::new();
    }
    let items = if actual_format == 32 {
        let long_items = unsafe { std::slice::from_raw_parts(property_data as *const std::os::raw::c_ulong, item_count as usize) };
        long_items.iter().map(|item| *item as u64).collect()
    } else {
        Vec::new()
    };
    unsafe { (connection.xlib.XFree)(property_data as *mut std::ffi::c_void) };
    items
}
