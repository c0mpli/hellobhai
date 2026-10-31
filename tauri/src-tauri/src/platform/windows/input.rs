//! Synthetic mouse and keyboard input with SetCursorPos + SendInput.
//! Coordinates are global physical pixels (the process is per-monitor DPI aware).

use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::sleep;
use std::time::Duration;

use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::SetCursorPos;

use crate::key_combo::{KeyModifier, NamedKey, ParsedKeyCombo};
use crate::platform::PlatformError;

/// SetCursorPos, 30 ms, left down, 30 ms, left up.
pub fn click_at_global_point(x: i32, y: i32) -> Result<(), PlatformError> {
    unsafe { SetCursorPos(x, y) }.map_err(|error| PlatformError::Failed(format!("SetCursorPos failed: {error}")))?;
    // Some apps drop a mouse-down that arrives in the same instant as the move (same as the Mac).
    sleep(Duration::from_millis(30));
    send_inputs(&[mouse_button_input(MOUSEEVENTF_LEFTDOWN)])?;
    sleep(Duration::from_millis(30));
    send_inputs(&[mouse_button_input(MOUSEEVENTF_LEFTUP)])
}

/// KEYEVENTF_UNICODE per UTF-16 unit (Devanagari works, surrogate pairs kept together),
/// '\n' as Enter, 8 ms between chunks of 20 units. Stops before the next chunk once
/// `should_stop_typing` is set (the hotkey cancelled the reply) or while the user
/// holds ctrl, alt or win: typed letters would arrive as shortcuts (ctrl+w, ctrl+a).
pub fn type_text(text: &str, should_stop_typing: &AtomicBool) -> Result<(), PlatformError> {
    const MAXIMUM_UTF16_UNITS_PER_CHUNK: usize = 20;
    let mut current_chunk_inputs = Vec::new();
    let mut current_chunk_utf16_unit_count = 0;
    let send_chunk_unless_stopped = |chunk_inputs: &[INPUT]| -> Result<(), PlatformError> {
        if should_stop_typing.load(Ordering::SeqCst) {
            return Err(PlatformError::Failed("typing cancelled".into()));
        }
        if is_any_shortcut_modifier_held() {
            return Err(PlatformError::Failed("typing stopped: ctrl, alt or win is held".into()));
        }
        send_inputs(chunk_inputs)
    };

    // Splitting on chars (not UTF-16 units) keeps a surrogate pair in one chunk.
    for character in text.chars() {
        if character == '\r' {
            continue;
        }
        if current_chunk_utf16_unit_count + character.len_utf16() > MAXIMUM_UTF16_UNITS_PER_CHUNK {
            send_chunk_unless_stopped(&current_chunk_inputs)?;
            sleep(Duration::from_millis(8));
            current_chunk_inputs.clear();
            current_chunk_utf16_unit_count = 0;
        }
        if character == '\n' {
            // A Unicode '\n' does nothing in most apps; Enter starts the new line.
            current_chunk_inputs.push(virtual_key_input(VK_RETURN, true));
            current_chunk_inputs.push(virtual_key_input(VK_RETURN, false));
        } else {
            let mut utf16_buffer = [0u16; 2];
            for utf16_unit in character.encode_utf16(&mut utf16_buffer) {
                current_chunk_inputs.push(unicode_unit_input(*utf16_unit, true));
                current_chunk_inputs.push(unicode_unit_input(*utf16_unit, false));
            }
        }
        current_chunk_utf16_unit_count += character.len_utf16();
    }
    if !current_chunk_inputs.is_empty() {
        send_chunk_unless_stopped(&current_chunk_inputs)?;
        sleep(Duration::from_millis(8));
    }
    Ok(())
}

/// Physical state right now (GetAsyncKeyState's high bit), not this thread's queue state.
fn is_any_shortcut_modifier_held() -> bool {
    [VK_CONTROL, VK_MENU, VK_LWIN, VK_RWIN]
        .iter()
        .any(|virtual_key| unsafe { GetAsyncKeyState(virtual_key.0 as i32) } as u16 & 0x8000 != 0)
}

/// Modifiers down, key down/up, modifiers up in reverse.
pub fn press_key_combo(parsed_key_combo: &ParsedKeyCombo) -> Result<(), PlatformError> {
    let modifier_virtual_keys: Vec<VIRTUAL_KEY> =
        parsed_key_combo.modifiers.iter().map(|modifier| virtual_key_for_modifier(*modifier)).collect();
    let main_virtual_key = virtual_key_for_named_key(parsed_key_combo.main_key);

    let mut inputs = Vec::new();
    for modifier_virtual_key in &modifier_virtual_keys {
        inputs.push(virtual_key_input(*modifier_virtual_key, true));
    }
    inputs.push(virtual_key_input(main_virtual_key, true));
    inputs.push(virtual_key_input(main_virtual_key, false));
    for modifier_virtual_key in modifier_virtual_keys.iter().rev() {
        inputs.push(virtual_key_input(*modifier_virtual_key, false));
    }
    send_inputs(&inputs)
}

fn virtual_key_for_modifier(modifier: KeyModifier) -> VIRTUAL_KEY {
    match modifier {
        KeyModifier::Ctrl => VK_CONTROL,
        KeyModifier::Shift => VK_SHIFT,
        KeyModifier::Alt => VK_MENU,
        KeyModifier::Win => VK_LWIN,
    }
}

/// Punctuation uses the US-layout OEM keys, like the Mac's kVK_ANSI_* codes.
fn virtual_key_for_named_key(named_key: NamedKey) -> VIRTUAL_KEY {
    match named_key {
        NamedKey::Return => VK_RETURN,
        NamedKey::Tab => VK_TAB,
        NamedKey::Space => VK_SPACE,
        NamedKey::Backspace => VK_BACK,
        NamedKey::Delete => VK_DELETE,
        NamedKey::Escape => VK_ESCAPE,
        NamedKey::Left => VK_LEFT,
        NamedKey::Right => VK_RIGHT,
        NamedKey::Up => VK_UP,
        NamedKey::Down => VK_DOWN,
        NamedKey::Home => VK_HOME,
        NamedKey::End => VK_END,
        NamedKey::PageUp => VK_PRIOR,
        NamedKey::PageDown => VK_NEXT,
        NamedKey::Win => VK_LWIN,
        NamedKey::Function(function_number) => VIRTUAL_KEY(VK_F1.0 + function_number as u16 - 1),
        NamedKey::Character(character) => match character {
            // VK codes for A-Z and 0-9 are their uppercase ASCII values.
            'a'..='z' => VIRTUAL_KEY(character.to_ascii_uppercase() as u16),
            '0'..='9' => VIRTUAL_KEY(character as u16),
            ',' => VK_OEM_COMMA,
            '.' => VK_OEM_PERIOD,
            '/' => VK_OEM_2,
            ';' => VK_OEM_1,
            '[' => VK_OEM_4,
            ']' => VK_OEM_6,
            '-' => VK_OEM_MINUS,
            '=' => VK_OEM_PLUS,
            _ => VK_OEM_3, // '`', the only other character `parse_key_combo` accepts
        },
    }
}

/// Arrows, Home/End, PageUp/PageDown, Delete and Win sit on the extended keypad;
/// without the flag Windows reads them as numpad keys.
fn is_extended_key(virtual_key: VIRTUAL_KEY) -> bool {
    [VK_LEFT, VK_RIGHT, VK_UP, VK_DOWN, VK_HOME, VK_END, VK_PRIOR, VK_NEXT, VK_DELETE, VK_LWIN].contains(&virtual_key)
}

fn virtual_key_input(virtual_key: VIRTUAL_KEY, is_key_down: bool) -> INPUT {
    let mut flags = KEYBD_EVENT_FLAGS(0);
    if is_extended_key(virtual_key) {
        flags |= KEYEVENTF_EXTENDEDKEY;
    }
    if !is_key_down {
        flags |= KEYEVENTF_KEYUP;
    }
    keyboard_input(KEYBDINPUT { wVk: virtual_key, wScan: 0, dwFlags: flags, time: 0, dwExtraInfo: 0 })
}

fn unicode_unit_input(utf16_unit: u16, is_key_down: bool) -> INPUT {
    let mut flags = KEYEVENTF_UNICODE;
    if !is_key_down {
        flags |= KEYEVENTF_KEYUP;
    }
    keyboard_input(KEYBDINPUT { wVk: VIRTUAL_KEY(0), wScan: utf16_unit, dwFlags: flags, time: 0, dwExtraInfo: 0 })
}

fn keyboard_input(keyboard_input_data: KEYBDINPUT) -> INPUT {
    INPUT { r#type: INPUT_KEYBOARD, Anonymous: INPUT_0 { ki: keyboard_input_data } }
}

/// A button event at the current pointer position (no MOUSEEVENTF_MOVE).
fn mouse_button_input(button_flags: MOUSE_EVENT_FLAGS) -> INPUT {
    INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 { mi: MOUSEINPUT { dx: 0, dy: 0, mouseData: 0, dwFlags: button_flags, time: 0, dwExtraInfo: 0 } },
    }
}

/// SendInput returns how many events it inserted; fewer means UIPI blocked them
/// (the target window runs elevated) or another thread's input interleaved.
fn send_inputs(inputs: &[INPUT]) -> Result<(), PlatformError> {
    if inputs.is_empty() {
        return Ok(());
    }
    let inserted_event_count = unsafe { SendInput(inputs, std::mem::size_of::<INPUT>() as i32) };
    if inserted_event_count as usize != inputs.len() {
        return Err(PlatformError::Failed(format!(
            "SendInput inserted {inserted_event_count} of {} events (is the target app running as administrator?)",
            inputs.len()
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::key_combo::parse_key_combo;

    #[test]
    fn maps_named_keys_to_virtual_keys() {
        assert_eq!(virtual_key_for_named_key(parse_key_combo("s").unwrap().main_key), VIRTUAL_KEY(0x53));
        assert_eq!(virtual_key_for_named_key(parse_key_combo("7").unwrap().main_key), VIRTUAL_KEY(0x37));
        assert_eq!(virtual_key_for_named_key(parse_key_combo("f12").unwrap().main_key), VK_F12);
        assert_eq!(virtual_key_for_named_key(parse_key_combo("`").unwrap().main_key), VK_OEM_3);
        assert!(is_extended_key(virtual_key_for_named_key(parse_key_combo("win").unwrap().main_key)));
    }
}
