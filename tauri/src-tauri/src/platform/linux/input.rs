//! Synthetic mouse and keyboard input with the XTest extension. Coordinates are
//! global physical pixels, which is what the X root window uses.
//!
//! Wayland: XTest goes through XWayland, so only X11 apps receive it (the
//! settings panel says so, see `platform_limitation_note`).
//!
//! Typing: a character the keyboard layout has is typed with its own key (plus
//! Shift when it sits on the shifted level). Anything else (Devanagari, emoji,
//! ₹) is typed by mapping a spare keycode to that character's keysym for one
//! press, the way xdotool does; the spare keycode is cleared again afterwards.

use std::os::raw::{c_int, c_uint, c_ulong};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::sleep;
use std::time::Duration;

use x11_dl::keysym;
use x11_dl::xlib;

use super::x11_connection::{with_x11_connection, xtest, X11Connection};
use crate::key_combo::{KeyModifier, NamedKey, ParsedKeyCombo};
use crate::platform::PlatformError;

/// Move, 30 ms, left down, 30 ms, left up (same timing as Windows and the Mac).
pub fn click_at_global_point(x: i32, y: i32) -> Result<(), PlatformError> {
    let xtest = xtest().ok_or_else(|| PlatformError::Failed("libXtst is not installed".into()))?;
    with_x11_connection(|connection| unsafe {
        // Screen -1: the screen the pointer is on (there is one per display today).
        (xtest.XTestFakeMotionEvent)(connection.display, -1, x, y, 0);
        (connection.xlib.XFlush)(connection.display);
    })?;
    // Some apps drop a button press that arrives in the same instant as the move.
    sleep(Duration::from_millis(30));
    with_x11_connection(|connection| unsafe {
        (xtest.XTestFakeButtonEvent)(connection.display, 1, xlib::True, 0);
        (connection.xlib.XFlush)(connection.display);
    })?;
    sleep(Duration::from_millis(30));
    with_x11_connection(|connection| unsafe {
        (xtest.XTestFakeButtonEvent)(connection.display, 1, xlib::False, 0);
        (connection.xlib.XFlush)(connection.display);
    })
}

/// The keysym X uses for a character: Latin-1 characters are their own code,
/// everything else is 0x01000000 + the Unicode code point.
pub fn keysym_for_character(character: char) -> c_ulong {
    let code_point = character as u32;
    if (0x20..=0x7e).contains(&code_point) || (0xa0..=0xff).contains(&code_point) {
        code_point as c_ulong
    } else {
        0x0100_0000 + code_point as c_ulong
    }
}

/// Characters in chunks of 20 with 8 ms between chunks, '\n' as Return, '\r'
/// skipped. Stops before the next chunk once `should_stop_typing` is set (the
/// hotkey cancelled the reply) or while the user holds ctrl, alt or super:
/// typed letters would arrive as shortcuts.
pub fn type_text(text: &str, should_stop_typing: &AtomicBool) -> Result<(), PlatformError> {
    const MAXIMUM_CHARACTERS_PER_CHUNK: usize = 20;
    let xtest = xtest().ok_or_else(|| PlatformError::Failed("libXtst is not installed".into()))?;
    let characters: Vec<char> = text.chars().filter(|character| *character != '\r').collect();

    for character_chunk in characters.chunks(MAXIMUM_CHARACTERS_PER_CHUNK) {
        if should_stop_typing.load(Ordering::SeqCst) {
            return Err(PlatformError::Failed("typing cancelled".into()));
        }
        if is_any_shortcut_modifier_held() {
            return Err(PlatformError::Failed("typing stopped: ctrl, alt or super is held".into()));
        }
        with_x11_connection(|connection| {
            let mut spare_keycode: Option<u8> = None;
            for character in character_chunk {
                let keysym = if *character == '\n' { keysym::XK_Return as c_ulong } else { keysym_for_character(*character) };
                tap_keysym(connection, xtest, keysym, &mut spare_keycode);
            }
            if let Some(spare_keycode) = spare_keycode {
                clear_keycode_mapping(connection, spare_keycode);
            }
        })?;
        sleep(Duration::from_millis(8));
    }
    Ok(())
}

/// Presses `keysym` once: with its own key when the layout has it, else
/// through the spare keycode (found on first use and kept in `spare_keycode`).
fn tap_keysym(connection: &X11Connection, xtest: &x11_dl::xtest::Xf86vmode, keysym: c_ulong, spare_keycode: &mut Option<u8>) {
    let xlib = connection.xlib;
    let display = connection.display;
    let layout_keycode = unsafe { (xlib.XKeysymToKeycode)(display, keysym) };
    if layout_keycode != 0 {
        let unshifted_keysym = unsafe { (xlib.XkbKeycodeToKeysym)(display, layout_keycode, 0, 0) };
        let shifted_keysym = unsafe { (xlib.XkbKeycodeToKeysym)(display, layout_keycode, 0, 1) };
        if unshifted_keysym == keysym || shifted_keysym == keysym {
            let needs_shift = unshifted_keysym != keysym;
            let shift_keycode = unsafe { (xlib.XKeysymToKeycode)(display, keysym::XK_Shift_L as c_ulong) };
            unsafe {
                if needs_shift {
                    (xtest.XTestFakeKeyEvent)(display, shift_keycode as c_uint, xlib::True, 0);
                }
                (xtest.XTestFakeKeyEvent)(display, layout_keycode as c_uint, xlib::True, 0);
                (xtest.XTestFakeKeyEvent)(display, layout_keycode as c_uint, xlib::False, 0);
                if needs_shift {
                    (xtest.XTestFakeKeyEvent)(display, shift_keycode as c_uint, xlib::False, 0);
                }
                (xlib.XFlush)(display);
            }
            return;
        }
    }

    if spare_keycode.is_none() {
        *spare_keycode = find_spare_keycode(connection);
    }
    let Some(spare_keycode) = *spare_keycode else {
        eprintln!("typing: no spare keycode to type keysym {keysym:#x}");
        return;
    };
    // Both levels, so a held Shift doesn't matter.
    let mut keysyms_for_spare_keycode = [keysym, keysym];
    unsafe {
        (xlib.XChangeKeyboardMapping)(display, spare_keycode as c_int, 2, keysyms_for_spare_keycode.as_mut_ptr(), 1);
        // Apps must see the new mapping before the key press.
        (xlib.XSync)(display, xlib::False);
        (xtest.XTestFakeKeyEvent)(display, spare_keycode as c_uint, xlib::True, 0);
        (xtest.XTestFakeKeyEvent)(display, spare_keycode as c_uint, xlib::False, 0);
        (xlib.XSync)(display, xlib::False);
    }
    // ponytail: apps read the mapping when they handle the event; 5 ms keeps fast
    // typists' apps in step. Raise it if characters come out wrong in some app.
    sleep(Duration::from_millis(5));
}

/// A keycode with no keysyms at all in the current layout (most layouts leave
/// a few of the 8..255 range unused).
fn find_spare_keycode(connection: &X11Connection) -> Option<u8> {
    let xlib = connection.xlib;
    let mut minimum_keycode: c_int = 0;
    let mut maximum_keycode: c_int = 0;
    unsafe { (xlib.XDisplayKeycodes)(connection.display, &mut minimum_keycode, &mut maximum_keycode) };
    let keycode_count = maximum_keycode - minimum_keycode + 1;
    let mut keysyms_per_keycode: c_int = 0;
    let keyboard_mapping =
        unsafe { (xlib.XGetKeyboardMapping)(connection.display, minimum_keycode as u8, keycode_count, &mut keysyms_per_keycode) };
    if keyboard_mapping.is_null() || keysyms_per_keycode <= 0 {
        return None;
    }
    let all_keysyms =
        unsafe { std::slice::from_raw_parts(keyboard_mapping, (keycode_count * keysyms_per_keycode) as usize) };
    let spare_keycode = all_keysyms
        .chunks(keysyms_per_keycode as usize)
        .enumerate()
        // From the top: high keycodes are the least likely to be real keys.
        .rev()
        .find(|(_, keycode_keysyms)| keycode_keysyms.iter().all(|keysym| *keysym == 0))
        .map(|(keycode_offset, _)| (minimum_keycode as usize + keycode_offset) as u8);
    unsafe { (xlib.XFree)(keyboard_mapping as *mut std::ffi::c_void) };
    spare_keycode
}

fn clear_keycode_mapping(connection: &X11Connection, keycode: u8) {
    let mut no_keysyms: [c_ulong; 2] = [0, 0];
    unsafe {
        (connection.xlib.XChangeKeyboardMapping)(connection.display, keycode as c_int, 2, no_keysyms.as_mut_ptr(), 1);
        (connection.xlib.XFlush)(connection.display);
    }
}

/// Physical keyboard state right now (XQueryKeymap).
fn is_any_shortcut_modifier_held() -> bool {
    let shortcut_modifier_keysyms =
        [keysym::XK_Control_L, keysym::XK_Control_R, keysym::XK_Alt_L, keysym::XK_Alt_R, keysym::XK_Super_L, keysym::XK_Super_R];
    with_x11_connection(|connection| {
        let mut pressed_keys_bitmap = [0 as std::os::raw::c_char; 32];
        unsafe { (connection.xlib.XQueryKeymap)(connection.display, pressed_keys_bitmap.as_mut_ptr()) };
        shortcut_modifier_keysyms.iter().any(|modifier_keysym| {
            let keycode = unsafe { (connection.xlib.XKeysymToKeycode)(connection.display, *modifier_keysym as c_ulong) } as usize;
            keycode != 0 && (pressed_keys_bitmap[keycode / 8] as u8) & (1 << (keycode % 8)) != 0
        })
    })
    .unwrap_or(false)
}

/// Modifiers down, key down/up, modifiers up in reverse.
pub fn press_key_combo(parsed_key_combo: &ParsedKeyCombo) -> Result<(), PlatformError> {
    let xtest = xtest().ok_or_else(|| PlatformError::Failed("libXtst is not installed".into()))?;
    let modifier_keysyms: Vec<c_ulong> = parsed_key_combo.modifiers.iter().map(|modifier| keysym_for_modifier(*modifier)).collect();
    let main_keysym = keysym_for_named_key(parsed_key_combo.main_key);

    with_x11_connection(|connection| {
        let keycode_for = |keysym: c_ulong| unsafe { (connection.xlib.XKeysymToKeycode)(connection.display, keysym) } as c_uint;
        let main_keycode = keycode_for(main_keysym);
        if main_keycode == 0 {
            return Err(PlatformError::Failed(format!("this keyboard layout has no key for keysym {main_keysym:#x}")));
        }
        let modifier_keycodes: Vec<c_uint> = modifier_keysyms.iter().map(|modifier_keysym| keycode_for(*modifier_keysym)).collect();
        unsafe {
            for modifier_keycode in &modifier_keycodes {
                (xtest.XTestFakeKeyEvent)(connection.display, *modifier_keycode, xlib::True, 0);
            }
            (xtest.XTestFakeKeyEvent)(connection.display, main_keycode, xlib::True, 0);
            (xtest.XTestFakeKeyEvent)(connection.display, main_keycode, xlib::False, 0);
            for modifier_keycode in modifier_keycodes.iter().rev() {
                (xtest.XTestFakeKeyEvent)(connection.display, *modifier_keycode, xlib::False, 0);
            }
            (connection.xlib.XFlush)(connection.display);
        }
        Ok(())
    })?
}

fn keysym_for_modifier(modifier: KeyModifier) -> c_ulong {
    (match modifier {
        KeyModifier::Ctrl => keysym::XK_Control_L,
        KeyModifier::Shift => keysym::XK_Shift_L,
        KeyModifier::Alt => keysym::XK_Alt_L,
        KeyModifier::Win => keysym::XK_Super_L,
    }) as c_ulong
}

/// Punctuation by its own keysym (the US-layout keys, like the Mac's kVK_ANSI_* codes).
fn keysym_for_named_key(named_key: NamedKey) -> c_ulong {
    (match named_key {
        NamedKey::Return => keysym::XK_Return,
        NamedKey::Tab => keysym::XK_Tab,
        NamedKey::Space => keysym::XK_space,
        NamedKey::Backspace => keysym::XK_BackSpace,
        NamedKey::Delete => keysym::XK_Delete,
        NamedKey::Escape => keysym::XK_Escape,
        NamedKey::Left => keysym::XK_Left,
        NamedKey::Right => keysym::XK_Right,
        NamedKey::Up => keysym::XK_Up,
        NamedKey::Down => keysym::XK_Down,
        NamedKey::Home => keysym::XK_Home,
        NamedKey::End => keysym::XK_End,
        NamedKey::PageUp => keysym::XK_Page_Up,
        NamedKey::PageDown => keysym::XK_Page_Down,
        NamedKey::Win => keysym::XK_Super_L,
        NamedKey::Function(function_number) => keysym::XK_F1 + function_number as c_uint - 1,
        NamedKey::Character(character) => return keysym_for_character(character),
    }) as c_ulong
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::key_combo::parse_key_combo;

    #[test]
    fn characters_map_to_x_keysyms() {
        assert_eq!(keysym_for_character('a'), 0x61);
        assert_eq!(keysym_for_character('é'), 0xe9);
        assert_eq!(keysym_for_character('न'), 0x0100_0928);
        assert_eq!(keysym_for_character('₹'), 0x0100_20b9);
    }

    #[test]
    fn named_keys_map_to_x_keysyms() {
        assert_eq!(keysym_for_named_key(parse_key_combo("s").unwrap().main_key), 0x73);
        assert_eq!(keysym_for_named_key(parse_key_combo("f12").unwrap().main_key), keysym::XK_F12 as c_ulong);
        assert_eq!(keysym_for_named_key(parse_key_combo("return").unwrap().main_key), keysym::XK_Return as c_ulong);
    }
}
