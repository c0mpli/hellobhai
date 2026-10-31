//! Push-to-talk shortcut options and the pure state machine that turns raw key
//! events from the platform keyboard hook into press/release transitions
//! (mirror of the Mac's `BuddyPushToTalkShortcut`).
//!
//! Default is ctrl + win, not ctrl + alt: Windows reports AltGr as left Ctrl +
//! right Alt, so ctrl + alt would fire while typing ₹ (AltGr+4 on English (India)).
//! `CtrlLeftAlt` needs the LEFT alt, so AltGr never triggers it.

use serde::{Deserialize, Serialize};
use tokio::sync::mpsc::UnboundedSender;

use crate::settings::SharedSettings;

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum PushToTalkShortcutOption {
    #[default]
    CtrlWin,
    CtrlLeftAlt,
    ShiftCtrl,
    CtrlAltSpace,
    ShiftCtrlSpace,
}

impl PushToTalkShortcutOption {
    /// Shown in the settings panel and the onboarding hint ("press ctrl + win and introduce yourself").
    pub fn display_text(self) -> &'static str {
        match self {
            PushToTalkShortcutOption::CtrlWin => "ctrl + win",
            PushToTalkShortcutOption::CtrlLeftAlt => "ctrl + left alt",
            PushToTalkShortcutOption::ShiftCtrl => "shift + ctrl",
            PushToTalkShortcutOption::CtrlAltSpace => "ctrl + alt + space",
            PushToTalkShortcutOption::ShiftCtrlSpace => "shift + ctrl + space",
        }
    }

    /// One label per key capsule in the settings panel's hint.
    pub fn key_capsule_labels(self) -> &'static [&'static str] {
        match self {
            PushToTalkShortcutOption::CtrlWin => &["ctrl", "win"],
            PushToTalkShortcutOption::CtrlLeftAlt => &["ctrl", "left alt"],
            PushToTalkShortcutOption::ShiftCtrl => &["shift", "ctrl"],
            PushToTalkShortcutOption::CtrlAltSpace => &["ctrl", "alt", "space"],
            PushToTalkShortcutOption::ShiftCtrlSpace => &["shift", "ctrl", "space"],
        }
    }

    pub fn all() -> &'static [Self] {
        &[
            PushToTalkShortcutOption::CtrlWin,
            PushToTalkShortcutOption::CtrlLeftAlt,
            PushToTalkShortcutOption::ShiftCtrl,
            PushToTalkShortcutOption::CtrlAltSpace,
            PushToTalkShortcutOption::ShiftCtrlSpace,
        ]
    }
}

/// The keys the tracker cares about, already mapped from OS key codes by the platform hook.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PhysicalKey {
    LeftCtrl,
    RightCtrl,
    LeftAlt,
    RightAlt,
    LeftWin,
    RightWin,
    LeftShift,
    RightShift,
    Space,
    Escape,
    Other,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RawKeyEvent {
    pub physical_key: PhysicalKey,
    pub is_key_down: bool,
    /// Synthetic input (including our own SendInput); always ignored.
    pub is_injected: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShortcutTransition {
    None,
    Pressed,
    Released,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HotkeyEvent {
    PushToTalkPressed,
    PushToTalkReleased,
    EscapePressed,
}

/// Tracks held keys and whether the shortcut is currently pressed.
#[derive(Default)]
pub struct PushToTalkShortcutTracker {}

impl PushToTalkShortcutTracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Modifier-only options: Pressed when all required modifiers are held (either side
    /// counts for ctrl/shift/win), Released when any is let go. Space options: Pressed on
    /// Space down with the modifiers held, Released on Space up.
    pub fn handle_key_event(&mut self, option: PushToTalkShortcutOption, event: RawKeyEvent) -> ShortcutTransition {
        todo!("A: shortcut state machine")
    }
}

/// Starts the platform keyboard hook and sends `HotkeyEvent`s for the shortcut in
/// `settings` (read on every event so a changed shortcut applies at once).
pub fn start_push_to_talk_listener(settings: SharedSettings, hotkey_sender: UnboundedSender<HotkeyEvent>) {
    todo!("A: platform::start_keyboard_hook + tracker")
}
