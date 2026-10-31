//! Push-to-talk shortcut options and the pure state machine that turns raw key
//! events from the platform keyboard hook into press/release transitions
//! (mirror of the Mac's `BuddyPushToTalkShortcut`).
//!
//! Default is ctrl + win, not ctrl + alt: Windows reports AltGr as left Ctrl +
//! right Alt, so ctrl + alt would fire while typing ₹ (AltGr+4 on English (India)).
//! `CtrlLeftAlt` needs the LEFT alt, so AltGr never triggers it.

use std::collections::HashSet;

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
pub struct PushToTalkShortcutTracker {
    held_keys: HashSet<PhysicalKey>,
    is_shortcut_pressed: bool,
}

impl PushToTalkShortcutTracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Modifier-only options: Pressed when all required modifiers are held (either side
    /// counts for ctrl/shift/win), Released when any is let go. Space options: Pressed on
    /// Space down with the modifiers held, Released on Space up.
    pub fn handle_key_event(&mut self, option: PushToTalkShortcutOption, event: RawKeyEvent) -> ShortcutTransition {
        if event.is_injected {
            return ShortcutTransition::None;
        }
        if event.is_key_down {
            self.held_keys.insert(event.physical_key);
        } else {
            self.held_keys.remove(&event.physical_key);
        }

        let is_held = |physical_key: PhysicalKey| self.held_keys.contains(&physical_key);
        let is_ctrl_held = is_held(PhysicalKey::LeftCtrl) || is_held(PhysicalKey::RightCtrl);
        let is_shift_held = is_held(PhysicalKey::LeftShift) || is_held(PhysicalKey::RightShift);
        let is_win_held = is_held(PhysicalKey::LeftWin) || is_held(PhysicalKey::RightWin);
        let is_any_alt_held = is_held(PhysicalKey::LeftAlt) || is_held(PhysicalKey::RightAlt);

        // Like the Mac, extra modifiers do not block the shortcut (a superset still matches).
        let required_modifiers_for_modifier_only_option = match option {
            // LEFT alt only: AltGr arrives as left ctrl + right alt and must never start recording.
            PushToTalkShortcutOption::CtrlLeftAlt => Some(is_ctrl_held && is_held(PhysicalKey::LeftAlt)),
            PushToTalkShortcutOption::CtrlWin => Some(is_ctrl_held && is_win_held),
            PushToTalkShortcutOption::ShiftCtrl => Some(is_shift_held && is_ctrl_held),
            PushToTalkShortcutOption::CtrlAltSpace | PushToTalkShortcutOption::ShiftCtrlSpace => None,
        };

        if let Some(is_shortcut_currently_held) = required_modifiers_for_modifier_only_option {
            return self.transition_to(is_shortcut_currently_held);
        }

        if event.physical_key != PhysicalKey::Space {
            return ShortcutTransition::None;
        }
        let are_space_modifiers_held = match option {
            PushToTalkShortcutOption::CtrlAltSpace => is_ctrl_held && is_any_alt_held,
            _ => is_shift_held && is_ctrl_held,
        };
        if event.is_key_down && are_space_modifiers_held {
            return self.transition_to(true);
        }
        if !event.is_key_down {
            return self.transition_to(false);
        }
        ShortcutTransition::None
    }

    fn transition_to(&mut self, is_shortcut_now_pressed: bool) -> ShortcutTransition {
        let transition = match (self.is_shortcut_pressed, is_shortcut_now_pressed) {
            (false, true) => ShortcutTransition::Pressed,
            (true, false) => ShortcutTransition::Released,
            _ => ShortcutTransition::None,
        };
        self.is_shortcut_pressed = is_shortcut_now_pressed;
        transition
    }
}

fn shortcut_uses_win_key(option: PushToTalkShortcutOption) -> bool {
    option == PushToTalkShortcutOption::CtrlWin
}

/// Starts the platform keyboard hook and sends `HotkeyEvent`s for the shortcut in
/// `settings` (read on every event so a changed shortcut applies at once).
pub fn start_push_to_talk_listener(settings: SharedSettings, hotkey_sender: UnboundedSender<HotkeyEvent>) {
    // The hook callback only forwards into this channel (Windows drops slow LL hooks);
    // the tracker runs on its own thread.
    let (raw_key_event_sender, raw_key_event_receiver) = std::sync::mpsc::channel::<RawKeyEvent>();
    let hook_start_result = crate::platform::start_keyboard_hook(Box::new(move |raw_key_event| {
        let _ = raw_key_event_sender.send(raw_key_event);
    }));
    if let Err(hook_error) = hook_start_result {
        eprintln!("push-to-talk keyboard hook not started: {hook_error}");
        return;
    }

    std::thread::Builder::new()
        .name("push-to-talk-tracker".into())
        .spawn(move || {
            let mut shortcut_tracker = PushToTalkShortcutTracker::new();
            for raw_key_event in raw_key_event_receiver {
                if raw_key_event.physical_key == PhysicalKey::Escape && raw_key_event.is_key_down && !raw_key_event.is_injected {
                    let _ = hotkey_sender.send(HotkeyEvent::EscapePressed);
                }
                let current_shortcut_option = settings.read().unwrap().push_to_talk_shortcut;
                let hotkey_event = match shortcut_tracker.handle_key_event(current_shortcut_option, raw_key_event) {
                    ShortcutTransition::Pressed => {
                        if shortcut_uses_win_key(current_shortcut_option) {
                            crate::platform::inject_start_menu_suppression_key();
                        }
                        HotkeyEvent::PushToTalkPressed
                    }
                    ShortcutTransition::Released => HotkeyEvent::PushToTalkReleased,
                    ShortcutTransition::None => continue,
                };
                if hotkey_sender.send(hotkey_event).is_err() {
                    return;
                }
            }
        })
        .expect("cannot spawn the push-to-talk tracker thread");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key_down(physical_key: PhysicalKey) -> RawKeyEvent {
        RawKeyEvent { physical_key, is_key_down: true, is_injected: false }
    }

    fn key_up(physical_key: PhysicalKey) -> RawKeyEvent {
        RawKeyEvent { physical_key, is_key_down: false, is_injected: false }
    }

    fn run_sequence(option: PushToTalkShortcutOption, events: &[RawKeyEvent]) -> Vec<ShortcutTransition> {
        let mut shortcut_tracker = PushToTalkShortcutTracker::new();
        events.iter().map(|event| shortcut_tracker.handle_key_event(option, *event)).collect()
    }

    use PhysicalKey::*;
    use ShortcutTransition::{None as NoChange, Pressed, Released};

    #[test]
    fn ctrl_win_presses_with_either_side_and_releases_when_one_is_let_go() {
        assert_eq!(
            run_sequence(PushToTalkShortcutOption::CtrlWin, &[key_down(RightCtrl), key_down(LeftWin), key_down(LeftWin), key_up(RightCtrl), key_up(LeftWin)]),
            vec![NoChange, Pressed, NoChange, Released, NoChange]
        );
    }

    #[test]
    fn altgr_left_ctrl_plus_right_alt_never_fires_ctrl_left_alt() {
        assert_eq!(
            run_sequence(PushToTalkShortcutOption::CtrlLeftAlt, &[key_down(LeftCtrl), key_down(RightAlt), key_down(Other), key_up(Other), key_up(RightAlt), key_up(LeftCtrl)]),
            vec![NoChange; 6]
        );
        assert_eq!(
            run_sequence(PushToTalkShortcutOption::CtrlLeftAlt, &[key_down(LeftAlt), key_down(LeftCtrl), key_up(LeftAlt)]),
            vec![NoChange, Pressed, Released]
        );
    }

    #[test]
    fn shift_ctrl_modifier_only() {
        assert_eq!(
            run_sequence(PushToTalkShortcutOption::ShiftCtrl, &[key_down(LeftShift), key_down(RightCtrl), key_up(LeftShift)]),
            vec![NoChange, Pressed, Released]
        );
    }

    #[test]
    fn space_options_press_on_space_down_and_release_on_space_up() {
        assert_eq!(
            run_sequence(
                PushToTalkShortcutOption::CtrlAltSpace,
                &[key_down(LeftCtrl), key_down(LeftAlt), key_down(Space), key_down(Space), key_up(LeftAlt), key_up(Space)]
            ),
            vec![NoChange, NoChange, Pressed, NoChange, NoChange, Released]
        );
        assert_eq!(
            run_sequence(PushToTalkShortcutOption::ShiftCtrlSpace, &[key_down(Space), key_up(Space), key_down(LeftShift), key_down(LeftCtrl), key_down(Space), key_up(Space)]),
            vec![NoChange, NoChange, NoChange, NoChange, Pressed, Released]
        );
    }

    #[test]
    fn injected_events_are_ignored() {
        let injected_win_down = RawKeyEvent { physical_key: LeftWin, is_key_down: true, is_injected: true };
        assert_eq!(run_sequence(PushToTalkShortcutOption::CtrlWin, &[key_down(LeftCtrl), injected_win_down]), vec![NoChange, NoChange]);
    }

    #[test]
    fn escape_does_not_disturb_a_held_shortcut() {
        assert_eq!(
            run_sequence(PushToTalkShortcutOption::CtrlWin, &[key_down(LeftCtrl), key_down(LeftWin), key_down(Escape), key_up(Escape), key_up(LeftWin)]),
            vec![NoChange, Pressed, NoChange, NoChange, Released]
        );
    }
}
