//! Listen-only low-level keyboard hook (WH_KEYBOARD_LL) on its own thread with
//! a message loop. The callback must stay tiny (Windows silently removes slow LL
//! hooks): it maps the key, calls `on_key_event`, and always CallNextHookEx.

use std::cell::RefCell;

use windows::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP, VIRTUAL_KEY, VK_ESCAPE,
    VK_LCONTROL, VK_LMENU, VK_LSHIFT, VK_LWIN, VK_RCONTROL, VK_RMENU, VK_RSHIFT, VK_RWIN, VK_SPACE,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, GetMessageW, SetWindowsHookExW, HC_ACTION, KBDLLHOOKSTRUCT, LLKHF_INJECTED, LLKHF_UP, MSG, WH_KEYBOARD_LL,
};

use crate::platform::PlatformError;
use crate::push_to_talk::{PhysicalKey, RawKeyEvent};

/// Unassigned virtual key used as the Start menu mask.
const START_MENU_SUPPRESSION_VIRTUAL_KEY: VIRTUAL_KEY = VIRTUAL_KEY(0xE8);

thread_local! {
    // The hook procedure runs on the thread that installed it (inside GetMessageW),
    // so the handler lives in that thread's local storage.
    static KEY_EVENT_HANDLER: RefCell<Option<Box<dyn Fn(RawKeyEvent) + Send + 'static>>> = RefCell::new(None);
}

pub fn start_keyboard_hook(on_key_event: Box<dyn Fn(RawKeyEvent) + Send + 'static>) -> Result<(), PlatformError> {
    let (install_result_sender, install_result_receiver) = std::sync::mpsc::channel::<Result<(), PlatformError>>();
    std::thread::Builder::new()
        .name("keyboard-hook".into())
        .spawn(move || {
            KEY_EVENT_HANDLER.with(|handler| *handler.borrow_mut() = Some(on_key_event));
            let install_result = unsafe {
                GetModuleHandleW(None).and_then(|module_handle| {
                    SetWindowsHookExW(WH_KEYBOARD_LL, Some(low_level_keyboard_procedure), Some(module_handle.into()), 0)
                })
            };
            if let Err(install_error) = install_result {
                let _ = install_result_sender.send(Err(PlatformError::Failed(format!("SetWindowsHookExW failed: {install_error}"))));
                return;
            }
            let _ = install_result_sender.send(Ok(()));
            // The hook only fires while this thread pumps messages; it runs for the app's lifetime.
            let mut message = MSG::default();
            while unsafe { GetMessageW(&mut message, None, 0, 0) }.as_bool() {}
        })
        .map_err(|spawn_error| PlatformError::Failed(format!("cannot spawn the keyboard hook thread: {spawn_error}")))?;
    install_result_receiver
        .recv()
        .unwrap_or_else(|_| Err(PlatformError::Failed("keyboard hook thread exited".into())))
}

unsafe extern "system" fn low_level_keyboard_procedure(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        let keyboard_event = unsafe { &*(lparam.0 as *const KBDLLHOOKSTRUCT) };
        let raw_key_event = RawKeyEvent {
            physical_key: physical_key_for_virtual_key(VIRTUAL_KEY(keyboard_event.vkCode as u16)),
            is_key_down: !keyboard_event.flags.contains(LLKHF_UP),
            is_injected: keyboard_event.flags.contains(LLKHF_INJECTED),
        };
        KEY_EVENT_HANDLER.with(|handler| {
            if let Some(on_key_event) = handler.borrow().as_ref() {
                on_key_event(raw_key_event);
            }
        });
    }
    // Listen-only: every key still reaches the focused app.
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

fn physical_key_for_virtual_key(virtual_key: VIRTUAL_KEY) -> PhysicalKey {
    match virtual_key {
        VK_LCONTROL => PhysicalKey::LeftCtrl,
        VK_RCONTROL => PhysicalKey::RightCtrl,
        VK_LMENU => PhysicalKey::LeftAlt,
        VK_RMENU => PhysicalKey::RightAlt,
        VK_LWIN => PhysicalKey::LeftWin,
        VK_RWIN => PhysicalKey::RightWin,
        VK_LSHIFT => PhysicalKey::LeftShift,
        VK_RSHIFT => PhysicalKey::RightShift,
        VK_SPACE => PhysicalKey::Space,
        VK_ESCAPE => PhysicalKey::Escape,
        _ => PhysicalKey::Other,
    }
}

/// Whether the key is physically down right now (GetAsyncKeyState); None for `Other`.
pub fn is_key_held_now(physical_key: PhysicalKey) -> Option<bool> {
    let virtual_key = match physical_key {
        PhysicalKey::LeftCtrl => VK_LCONTROL,
        PhysicalKey::RightCtrl => VK_RCONTROL,
        PhysicalKey::LeftAlt => VK_LMENU,
        PhysicalKey::RightAlt => VK_RMENU,
        PhysicalKey::LeftWin => VK_LWIN,
        PhysicalKey::RightWin => VK_RWIN,
        PhysicalKey::LeftShift => VK_LSHIFT,
        PhysicalKey::RightShift => VK_RSHIFT,
        PhysicalKey::Space => VK_SPACE,
        PhysicalKey::Escape => VK_ESCAPE,
        PhysicalKey::Other => return None,
    };
    Some(unsafe { GetAsyncKeyState(virtual_key.0 as i32) } as u16 & 0x8000 != 0)
}

/// Taps VK 0xE8 (unassigned) once so releasing Win after the shortcut does not
/// open the Start menu (the AutoHotkey "menu mask key" trick).
pub fn inject_start_menu_suppression_key() {
    let keyboard_input = |key_event_flags: KEYBD_EVENT_FLAGS| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT { wVk: START_MENU_SUPPRESSION_VIRTUAL_KEY, wScan: 0, dwFlags: key_event_flags, time: 0, dwExtraInfo: 0 },
        },
    };
    let key_down_and_up = [keyboard_input(KEYBD_EVENT_FLAGS(0)), keyboard_input(KEYEVENTF_KEYUP)];
    unsafe {
        SendInput(&key_down_and_up, std::mem::size_of::<INPUT>() as i32);
    }
}
