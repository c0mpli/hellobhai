//! Parses `[KEY:combo]` payloads like "ctrl+shift+t" into modifiers + a main key.
//! "cmd"/"command" map to Ctrl (Claude's Mac habit). Unknown names -> None.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyModifier {
    Ctrl,
    Shift,
    Alt,
    Win,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NamedKey {
    Return,
    Tab,
    Space,
    Backspace,
    Delete,
    Escape,
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    PageUp,
    PageDown,
    /// a-z, 0-9 and , . / ; [ ] - = `
    Character(char),
    /// F1-F12
    Function(u8),
    /// The Win key pressed on its own (`[KEY:win]` opens Start).
    Win,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParsedKeyCombo {
    pub modifiers: Vec<KeyModifier>,
    pub main_key: NamedKey,
}

pub fn parse_key_combo(key_combo: &str) -> Option<ParsedKeyCombo> {
    todo!("C: key combo parsing")
}
