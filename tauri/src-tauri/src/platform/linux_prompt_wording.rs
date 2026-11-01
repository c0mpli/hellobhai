//! The exported prompts carry Windows wording (`tauri/prompt-substitutions.json`:
//! notepad, the Start menu, win+shift+r, C:\ paths). On Linux these edits swap
//! in the Linux equivalents when the prompt files are written. Like the
//! exporter's substitutions, every `find` must match exactly once; the tests
//! fail when a Mac or Windows prompt edit breaks one, so Windows instructions
//! never silently reach Linux.

/// (prompt file name, find, replace)
pub const LINUX_PROMPT_SUBSTITUTIONS: [(&str, &str, &str); 4] = [
    (
        "voice_response.txt",
        "if the user asks to save the notes, open notepad with [OPEN:notepad], then in the next round type them with [TYPE:...]",
        "if the user asks to save the notes, open a text editor with [OPEN:Text Editor] (or another installed editor, like [OPEN:Kate] on kde), then in the next round type them with [TYPE:...]",
    ),
    (
        "voice_response.txt",
        "1. windows tools — prefer these whenever they work, they're reliable:\n- Glob, Grep and Read: find, list and read files yourself before you reply, for example the newest screen recording with Glob \"**/*.mp4\" in the user's Videos folder (newest first). installed apps are the .lnk files in the start menu Programs folders.\n- [OPEN:target] (an action tag, see below): opens an app, file, folder or link. target is an app's start menu name like [OPEN:Spotify] or [OPEN:Visual Studio Code], or notepad, calc, mspaint or explorer; a full path to a file or folder inside the user's home folder; an http or https link; or a whatsapp:, spotify:, slack:, msteams:, discord:, tg:, zoommtg:, mailto: or ms-settings: link, for example [OPEN:whatsapp://send?phone=919876543210&text=hi]. programs and scripts can't be opened.",
        "1. linux tools — prefer these whenever they work, they're reliable:\n- Glob, Grep and Read: find, list and read files yourself before you reply, for example the newest screen recording with Glob \"**/*.webm\" or \"**/*.mp4\" in the user's Videos folder (newest first). installed apps are the .desktop files in the applications folders (~/.local/share/applications, /usr/share/applications and the flatpak and snap ones); their Name= line is the app's name.\n- [OPEN:target] (an action tag, see below): opens an app, file, folder or link. target is an installed app's name like [OPEN:Spotify], [OPEN:Visual Studio Code] or [OPEN:Files], or its .desktop file name like [OPEN:org.gnome.Calculator]; a full path to a file or folder inside the user's home folder; an http or https link; or a whatsapp:, spotify:, slack:, msteams:, discord:, tg:, zoommtg: or mailto: link, for example [OPEN:whatsapp://send?phone=919876543210&text=hi]. programs and scripts can't be opened.",
    ),
    (
        "voice_response.txt",
        "- to start a screen recording: [KEY:win+shift+r] opens the snipping tool recorder. in the next round, drag across the area to record (or the whole screen if they asked for everything), then click \"start\". to stop, click the stop button in the snipping tool bar at the top of the screen.",
        "- to start a screen recording on gnome: [KEY:shift+ctrl+alt+r] starts recording the whole screen right away, and the same keys (or the red stop button in the top bar) stop it. for part of the screen, or on other desktops, open the desktop's screenshot tool from the installed apps and use its record option.",
    ),
    (
        "voice_response.txt",
        "that opens the windows file dialog with the file name box focused — [TYPE:C:\\full\\path\\to\\file] then [KEY:enter].",
        "that opens the file dialog — [KEY:ctrl+l] for the location box, [TYPE:/full/path/to/file], then [KEY:return].",
    ),
];

/// `prompt_text` with every Linux substitution for `prompt_file_name` applied.
pub fn adapt_prompt_for_linux(prompt_file_name: &str, prompt_text: &str) -> String {
    let mut adapted_text = prompt_text.to_string();
    for (substitution_prompt_file_name, find_text, replace_text) in LINUX_PROMPT_SUBSTITUTIONS {
        if substitution_prompt_file_name == prompt_file_name {
            adapted_text = adapted_text.replacen(find_text, replace_text, 1);
        }
    }
    adapted_text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_substitution_matches_its_prompt_exactly_once() {
        for (prompt_file_name, find_text, _) in LINUX_PROMPT_SUBSTITUTIONS {
            let prompt_text = match prompt_file_name {
                "voice_response.txt" => crate::prompts::VOICE_RESPONSE_SYSTEM_PROMPT,
                other => panic!("no prompt called {other}"),
            };
            assert_eq!(prompt_text.matches(find_text).count(), 1, "{prompt_file_name}: {find_text}");
        }
    }

    #[test]
    fn linux_voice_prompt_has_no_windows_instructions_left() {
        let adapted_prompt = adapt_prompt_for_linux("voice_response.txt", crate::prompts::VOICE_RESPONSE_SYSTEM_PROMPT);
        for windows_only_text in ["notepad", "start menu", ".lnk", "win+shift", "C:\\", "ms-settings", "windows"] {
            assert!(!adapted_prompt.contains(windows_only_text), "still mentions {windows_only_text}");
        }
    }
}
