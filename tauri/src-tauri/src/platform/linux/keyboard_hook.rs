//! Listen-only global keyboard events for push-to-talk, never grabbing a key.
//!
//! X11: XInput2 raw key events on the root window (XI 2.1+ delivers them even
//! while another client grabs the keyboard, e.g. the window manager's Super
//! binding), so modifier-only shortcuts work. Events from the XTEST device
//! (our own typing) are marked injected.
//!
//! Wayland: XWayland only shows keys typed into X11 windows, so the keyboards
//! are read from /dev/input/event* instead when the user may (member of the
//! `input` group). Otherwise it falls back to XInput2, which then only works
//! while an X11 window is focused; the settings panel explains this.

use std::collections::HashSet;
use std::io::Read;
use std::os::raw::{c_int, c_ulong};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use x11_dl::keysym;
use x11_dl::xinput2::{self, XInput2};
use x11_dl::xlib;

use super::x11_connection::{is_wayland_session, open_display, with_x11_connection};
use crate::platform::PlatformError;
use crate::push_to_talk::{PhysicalKey, RawKeyEvent};

/// How the Super key is written in shortcut labels on this OS.
pub const SUPER_KEY_DISPLAY_NAME: &str = "super";

pub fn start_keyboard_hook(on_key_event: Box<dyn Fn(RawKeyEvent) + Send + 'static>) -> Result<(), PlatformError> {
    if keyboard_hook_reads_input_devices() {
        return start_input_device_reader(on_key_event);
    }
    start_xinput2_raw_key_listener(on_key_event)
}

/// Wayland with at least one /dev/input device the user may read.
pub(in crate::platform) fn keyboard_hook_reads_input_devices() -> bool {
    is_wayland_session() && !readable_input_event_device_paths().is_empty()
}

/// X11 keysym of a key's base level -> the key the tracker cares about.
/// ISO_Level3_Shift (AltGr) is not an alt key, so AltGr never acts as alt.
pub fn physical_key_for_keysym(keysym_value: c_ulong) -> PhysicalKey {
    match keysym_value as u32 {
        keysym::XK_Control_L => PhysicalKey::LeftCtrl,
        keysym::XK_Control_R => PhysicalKey::RightCtrl,
        keysym::XK_Alt_L | keysym::XK_Meta_L => PhysicalKey::LeftAlt,
        keysym::XK_Alt_R | keysym::XK_Meta_R => PhysicalKey::RightAlt,
        keysym::XK_Super_L | keysym::XK_Hyper_L => PhysicalKey::LeftWin,
        keysym::XK_Super_R | keysym::XK_Hyper_R => PhysicalKey::RightWin,
        keysym::XK_Shift_L => PhysicalKey::LeftShift,
        keysym::XK_Shift_R => PhysicalKey::RightShift,
        keysym::XK_space => PhysicalKey::Space,
        keysym::XK_Escape => PhysicalKey::Escape,
        _ => PhysicalKey::Other,
    }
}

/// Linux input event key code (linux/input-event-codes.h) -> tracked key.
/// None for mouse and joystick buttons (BTN_*, 0x100 and up), which must not
/// count as "another key" and cancel a held chord.
pub fn physical_key_for_input_event_code(key_code: u16) -> Option<PhysicalKey> {
    const KEY_ESC: u16 = 1;
    const KEY_LEFTCTRL: u16 = 29;
    const KEY_LEFTSHIFT: u16 = 42;
    const KEY_RIGHTSHIFT: u16 = 54;
    const KEY_LEFTALT: u16 = 56;
    const KEY_SPACE: u16 = 57;
    const KEY_RIGHTCTRL: u16 = 97;
    const KEY_RIGHTALT: u16 = 100;
    const KEY_LEFTMETA: u16 = 125;
    const KEY_RIGHTMETA: u16 = 126;
    const FIRST_BUTTON_CODE: u16 = 0x100;
    if key_code >= FIRST_BUTTON_CODE {
        return None;
    }
    Some(match key_code {
        KEY_LEFTCTRL => PhysicalKey::LeftCtrl,
        KEY_RIGHTCTRL => PhysicalKey::RightCtrl,
        KEY_LEFTALT => PhysicalKey::LeftAlt,
        // KEY_RIGHTALT is AltGr on most layouts, but the tracker only needs left alt to be left alt.
        KEY_RIGHTALT => PhysicalKey::RightAlt,
        KEY_LEFTMETA => PhysicalKey::LeftWin,
        KEY_RIGHTMETA => PhysicalKey::RightWin,
        KEY_LEFTSHIFT => PhysicalKey::LeftShift,
        KEY_RIGHTSHIFT => PhysicalKey::RightShift,
        KEY_SPACE => PhysicalKey::Space,
        KEY_ESC => PhysicalKey::Escape,
        _ => PhysicalKey::Other,
    })
}

// MARK: - X11 (XInput2 raw events)

fn start_xinput2_raw_key_listener(on_key_event: Box<dyn Fn(RawKeyEvent) + Send + 'static>) -> Result<(), PlatformError> {
    let (start_result_sender, start_result_receiver) = std::sync::mpsc::channel::<Result<(), PlatformError>>();
    std::thread::Builder::new()
        .name("keyboard-hook".into())
        .spawn(move || {
            // Its own connection: XNextEvent blocks, so it can't share the mutex-guarded one.
            let (xlib, display) = match open_display() {
                Ok(opened) => opened,
                Err(open_error) => {
                    let _ = start_result_sender.send(Err(open_error));
                    return;
                }
            };
            let Ok(xinput2) = XInput2::open() else {
                let _ = start_result_sender.send(Err(PlatformError::Failed("libXi is not installed".into())));
                return;
            };
            let mut xinput_opcode: c_int = 0;
            let mut first_event: c_int = 0;
            let mut first_error: c_int = 0;
            let extension_name = c"XInputExtension";
            let has_xinput = unsafe {
                (xlib.XQueryExtension)(display, extension_name.as_ptr(), &mut xinput_opcode, &mut first_event, &mut first_error)
            } != 0;
            // 2.1 is the first version that sends raw events regardless of grabs.
            let mut major_version: c_int = 2;
            let mut minor_version: c_int = 1;
            let version_is_supported = has_xinput
                && unsafe { (xinput2.XIQueryVersion)(display, &mut major_version, &mut minor_version) } == xlib::Success as c_int;
            if !version_is_supported {
                let _ = start_result_sender.send(Err(PlatformError::Failed("the X server has no XInput 2.1".into())));
                return;
            }

            let mut event_mask_bits = [0u8; 4];
            xinput2::XISetMask(&mut event_mask_bits, xinput2::XI_RawKeyPress);
            xinput2::XISetMask(&mut event_mask_bits, xinput2::XI_RawKeyRelease);
            xinput2::XISetMask(&mut event_mask_bits, xinput2::XI_HierarchyChanged);
            let mut event_mask = xinput2::XIEventMask {
                deviceid: xinput2::XIAllMasterDevices,
                mask_len: event_mask_bits.len() as c_int,
                mask: event_mask_bits.as_mut_ptr(),
            };
            unsafe {
                let root_window = (xlib.XDefaultRootWindow)(display);
                (xinput2.XISelectEvents)(display, root_window, &mut event_mask, 1);
                (xlib.XFlush)(display);
            }
            let mut xtest_keyboard_device_ids = xtest_keyboard_device_ids(&xinput2, display);
            let _ = start_result_sender.send(Ok(()));

            loop {
                let mut event: xlib::XEvent = unsafe { std::mem::zeroed() };
                unsafe { (xlib.XNextEvent)(display, &mut event) };
                if event.get_type() != xlib::GenericEvent {
                    continue;
                }
                let mut cookie = unsafe { event.generic_event_cookie };
                if cookie.extension != xinput_opcode || unsafe { (xlib.XGetEventData)(display, &mut cookie) } == 0 {
                    continue;
                }
                match cookie.evtype {
                    xinput2::XI_RawKeyPress | xinput2::XI_RawKeyRelease => {
                        let raw_event = unsafe { &*(cookie.data as *const xinput2::XIRawEvent) };
                        let base_keysym = unsafe { (xlib.XkbKeycodeToKeysym)(display, raw_event.detail as u8, 0, 0) };
                        on_key_event(RawKeyEvent {
                            physical_key: physical_key_for_keysym(base_keysym),
                            is_key_down: cookie.evtype == xinput2::XI_RawKeyPress,
                            is_injected: xtest_keyboard_device_ids.contains(&raw_event.sourceid),
                        });
                    }
                    // A keyboard was plugged in or removed; the XTEST device ids may change.
                    xinput2::XI_HierarchyChanged => xtest_keyboard_device_ids = xtest_keyboard_device_ids_or_keep(&xinput2, display, xtest_keyboard_device_ids),
                    _ => {}
                }
                unsafe { (xlib.XFreeEventData)(display, &mut cookie) };
            }
        })
        .map_err(|spawn_error| PlatformError::Failed(format!("cannot spawn the keyboard hook thread: {spawn_error}")))?;
    start_result_receiver
        .recv()
        .unwrap_or_else(|_| Err(PlatformError::Failed("keyboard hook thread exited".into())))
}

fn xtest_keyboard_device_ids_or_keep(xinput2: &XInput2, display: *mut xlib::Display, previous_ids: Vec<c_int>) -> Vec<c_int> {
    let current_ids = xtest_keyboard_device_ids(xinput2, display);
    if current_ids.is_empty() {
        previous_ids
    } else {
        current_ids
    }
}

/// The slave keyboards named "... XTEST ..." (XTestFakeKeyEvent comes from them).
fn xtest_keyboard_device_ids(xinput2: &XInput2, display: *mut xlib::Display) -> Vec<c_int> {
    let mut device_count: c_int = 0;
    let device_list = unsafe { (xinput2.XIQueryDevice)(display, xinput2::XIAllDevices, &mut device_count) };
    if device_list.is_null() {
        return Vec::new();
    }
    let devices = unsafe { std::slice::from_raw_parts(device_list, device_count.max(0) as usize) };
    let xtest_ids = devices
        .iter()
        .filter(|device| device._use == xinput2::XISlaveKeyboard && !device.name.is_null())
        .filter(|device| unsafe { std::ffi::CStr::from_ptr(device.name) }.to_string_lossy().contains("XTEST"))
        .map(|device| device.deviceid)
        .collect();
    unsafe { (xinput2.XIFreeDeviceInfo)(device_list) };
    xtest_ids
}

/// Whether the key is physically down right now (XQueryKeymap); None when there
/// is no X display (Wayland input devices keep their own state in the tracker).
pub fn is_key_held_now(physical_key: PhysicalKey) -> Option<bool> {
    let candidate_keysyms: &[u32] = match physical_key {
        PhysicalKey::LeftCtrl => &[keysym::XK_Control_L],
        PhysicalKey::RightCtrl => &[keysym::XK_Control_R],
        PhysicalKey::LeftAlt => &[keysym::XK_Alt_L, keysym::XK_Meta_L],
        PhysicalKey::RightAlt => &[keysym::XK_Alt_R, keysym::XK_Meta_R],
        PhysicalKey::LeftWin => &[keysym::XK_Super_L],
        PhysicalKey::RightWin => &[keysym::XK_Super_R],
        PhysicalKey::LeftShift => &[keysym::XK_Shift_L],
        PhysicalKey::RightShift => &[keysym::XK_Shift_R],
        PhysicalKey::Space => &[keysym::XK_space],
        PhysicalKey::Escape => &[keysym::XK_Escape],
        _ => return None,
    };
    if is_wayland_session() {
        // XWayland's keymap only knows about keys typed into X11 windows.
        return None;
    }
    with_x11_connection(|connection| {
        let mut pressed_keys_bitmap = [0 as std::os::raw::c_char; 32];
        unsafe { (connection.xlib.XQueryKeymap)(connection.display, pressed_keys_bitmap.as_mut_ptr()) };
        let keycodes: Vec<usize> = candidate_keysyms
            .iter()
            .map(|candidate_keysym| unsafe { (connection.xlib.XKeysymToKeycode)(connection.display, *candidate_keysym as c_ulong) } as usize)
            .filter(|keycode| *keycode != 0)
            .collect();
        if keycodes.is_empty() {
            return None;
        }
        Some(keycodes.iter().any(|keycode| (pressed_keys_bitmap[keycode / 8] as u8) & (1 << (keycode % 8)) != 0))
    })
    .ok()
    .flatten()
}

/// Windows needs a masking key so releasing Win doesn't open Start. GNOME and
/// KDE only open their launcher when Super is pressed and released alone, and
/// push-to-talk always holds another modifier with it, so nothing to do.
pub fn inject_start_menu_suppression_key() {}

// MARK: - Wayland (/dev/input)

fn readable_input_event_device_paths() -> Vec<PathBuf> {
    let Ok(input_directory_entries) = std::fs::read_dir("/dev/input") else {
        return Vec::new();
    };
    let mut device_paths: Vec<PathBuf> = input_directory_entries
        .flatten()
        .map(|directory_entry| directory_entry.path())
        .filter(|device_path| device_path.file_name().is_some_and(|file_name| file_name.to_string_lossy().starts_with("event")))
        .filter(|device_path| std::fs::File::open(device_path).is_ok())
        .collect();
    device_paths.sort();
    device_paths
}

/// One reader thread per input device, plus a thread that looks for newly
/// plugged-in keyboards every 3 seconds.
fn start_input_device_reader(on_key_event: Box<dyn Fn(RawKeyEvent) + Send + 'static>) -> Result<(), PlatformError> {
    let shared_key_event_handler: Arc<Mutex<Box<dyn Fn(RawKeyEvent) + Send + 'static>>> = Arc::new(Mutex::new(on_key_event));
    let open_device_paths: Arc<Mutex<HashSet<PathBuf>>> = Arc::new(Mutex::new(HashSet::new()));
    std::thread::Builder::new()
        .name("keyboard-device-scan".into())
        .spawn(move || loop {
            for device_path in readable_input_event_device_paths() {
                if !open_device_paths.lock().unwrap().insert(device_path.clone()) {
                    continue;
                }
                let key_event_handler = shared_key_event_handler.clone();
                let open_device_paths_for_reader = open_device_paths.clone();
                let spawn_result = std::thread::Builder::new().name("keyboard-device".into()).spawn(move || {
                    read_input_device_until_it_closes(&device_path, &key_event_handler);
                    open_device_paths_for_reader.lock().unwrap().remove(&device_path);
                });
                if let Err(spawn_error) = spawn_result {
                    eprintln!("keyboard: cannot read an input device: {spawn_error}");
                }
            }
            std::thread::sleep(Duration::from_secs(3));
        })
        .map(|_scan_thread| ())
        .map_err(|spawn_error| PlatformError::Failed(format!("cannot spawn the keyboard device thread: {spawn_error}")))
}

/// Reads `struct input_event`s until the device goes away (unplugged).
fn read_input_device_until_it_closes(device_path: &PathBuf, key_event_handler: &Mutex<Box<dyn Fn(RawKeyEvent) + Send + 'static>>) {
    const EV_KEY: u16 = 1;
    let Ok(mut device_file) = std::fs::File::open(device_path) else {
        return;
    };
    let input_event_size = std::mem::size_of::<libc::input_event>();
    let mut event_bytes = vec![0u8; input_event_size];
    while device_file.read_exact(&mut event_bytes).is_ok() {
        let input_event: libc::input_event = unsafe { std::ptr::read_unaligned(event_bytes.as_ptr() as *const libc::input_event) };
        if input_event.type_ != EV_KEY {
            continue;
        }
        let Some(physical_key) = physical_key_for_input_event_code(input_event.code) else {
            continue;
        };
        // value: 0 up, 1 down, 2 auto-repeat (a key-down, like Windows' repeated WM_KEYDOWN).
        let raw_key_event = RawKeyEvent { physical_key, is_key_down: input_event.value != 0, is_injected: false };
        (key_event_handler.lock().unwrap())(raw_key_event);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keysyms_map_to_tracked_keys_and_altgr_is_not_alt() {
        assert_eq!(physical_key_for_keysym(keysym::XK_Control_L as c_ulong), PhysicalKey::LeftCtrl);
        assert_eq!(physical_key_for_keysym(keysym::XK_Super_R as c_ulong), PhysicalKey::RightWin);
        assert_eq!(physical_key_for_keysym(keysym::XK_ISO_Level3_Shift as c_ulong), PhysicalKey::Other);
        assert_eq!(physical_key_for_keysym(keysym::XK_a as c_ulong), PhysicalKey::Other);
    }

    #[test]
    fn input_event_codes_skip_mouse_buttons() {
        assert_eq!(physical_key_for_input_event_code(29), Some(PhysicalKey::LeftCtrl));
        assert_eq!(physical_key_for_input_event_code(125), Some(PhysicalKey::LeftWin));
        assert_eq!(physical_key_for_input_event_code(30), Some(PhysicalKey::Other));
        assert_eq!(physical_key_for_input_event_code(0x110), None); // BTN_LEFT
    }
}
