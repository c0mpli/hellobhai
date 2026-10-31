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

fn modifier_for_name(key_name: &str) -> Option<KeyModifier> {
    match key_name {
        "cmd" | "command" | "ctrl" | "control" => Some(KeyModifier::Ctrl),
        "shift" => Some(KeyModifier::Shift),
        "alt" | "option" | "opt" => Some(KeyModifier::Alt),
        "win" | "windows" | "super" => Some(KeyModifier::Win),
        _ => None,
    }
}

fn named_key_for_name(key_name: &str) -> Option<NamedKey> {
    let named_key = match key_name {
        "return" | "enter" => NamedKey::Return,
        "tab" => NamedKey::Tab,
        "space" => NamedKey::Space,
        // Like the Mac: "delete" is the backspace key, "forwarddelete" is Del.
        "delete" | "backspace" => NamedKey::Backspace,
        "forwarddelete" | "del" => NamedKey::Delete,
        "escape" | "esc" => NamedKey::Escape,
        "left" => NamedKey::Left,
        "right" => NamedKey::Right,
        "up" => NamedKey::Up,
        "down" => NamedKey::Down,
        "home" => NamedKey::Home,
        "end" => NamedKey::End,
        "pageup" => NamedKey::PageUp,
        "pagedown" => NamedKey::PageDown,
        _ => {
            let mut characters = key_name.chars();
            let first_character = characters.next()?;
            let is_single_character = characters.next().is_none();
            if is_single_character
                && (first_character.is_ascii_lowercase() || first_character.is_ascii_digit() || ",./;[]-=`".contains(first_character))
            {
                return Some(NamedKey::Character(first_character));
            }
            let function_number: u8 = key_name.strip_prefix('f')?.parse().ok()?;
            if !(1..=12).contains(&function_number) {
                return None;
            }
            NamedKey::Function(function_number)
        }
    };
    Some(named_key)
}

pub fn parse_key_combo(key_combo: &str) -> Option<ParsedKeyCombo> {
    let mut modifiers = Vec::new();
    let mut main_key = None;

    for key_name in key_combo.to_lowercase().split('+').map(str::trim) {
        if let Some(modifier) = modifier_for_name(key_name) {
            if !modifiers.contains(&modifier) {
                modifiers.push(modifier);
            }
        } else if let Some(named_key) = named_key_for_name(key_name) {
            main_key = Some(named_key);
        } else {
            eprintln!("computer control: unknown key \"{key_name}\" in \"{key_combo}\"");
            return None;
        }
    }

    // A combo of only modifiers has no key to press, except Win, which on
    // Windows is a key in its own right ("win" opens Start, "ctrl+win" too).
    if main_key.is_none() && modifiers.contains(&KeyModifier::Win) {
        modifiers.retain(|modifier| *modifier != KeyModifier::Win);
        main_key = Some(NamedKey::Win);
    }

    Some(ParsedKeyCombo { modifiers, main_key: main_key? })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_key_combos_like_the_mac() {
        assert_eq!(
            parse_key_combo("cmd+shift+T"),
            Some(ParsedKeyCombo { modifiers: vec![KeyModifier::Ctrl, KeyModifier::Shift], main_key: NamedKey::Character('t') })
        );
        assert_eq!(parse_key_combo(" return "), Some(ParsedKeyCombo { modifiers: vec![], main_key: NamedKey::Return }));
        assert_eq!(parse_key_combo("delete").unwrap().main_key, NamedKey::Backspace);
        assert_eq!(parse_key_combo("forwarddelete").unwrap().main_key, NamedKey::Delete);
        assert_eq!(
            parse_key_combo("option + f4"),
            Some(ParsedKeyCombo { modifiers: vec![KeyModifier::Alt], main_key: NamedKey::Function(4) })
        );
        assert_eq!(parse_key_combo("ctrl+/").unwrap().main_key, NamedKey::Character('/'));
        assert_eq!(parse_key_combo("win"), Some(ParsedKeyCombo { modifiers: vec![], main_key: NamedKey::Win }));
        assert_eq!(
            parse_key_combo("win+e"),
            Some(ParsedKeyCombo { modifiers: vec![KeyModifier::Win], main_key: NamedKey::Character('e') })
        );
        assert_eq!(parse_key_combo("cmd+shift"), None);
        assert_eq!(parse_key_combo("ctrl+banana"), None);
        assert_eq!(parse_key_combo("f13"), None);
        assert_eq!(parse_key_combo(""), None);
    }
}
