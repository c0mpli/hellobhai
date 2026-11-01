//! Where claude.exe (and codex) lives on Windows and which tools a voice turn may use: the
//! Windows counterpart of the Mac's osascript/open/mdfind/shortcuts/ls allowlist
//! (read-only tools here; opening things is the `[OPEN]` tag). The prompt still
//! demands a spoken yes before anything that sends, posts, deletes or buys.

use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};

/// No console window flashes up for `where.exe`.
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// In order: %USERPROFILE%\.local\bin\claude.exe (native installer),
/// %LOCALAPPDATA%\Microsoft\WinGet\Links\claude.exe, then `where.exe claude`
/// (preferring .exe over .cmd).
pub fn claude_executable_candidates() -> Vec<PathBuf> {
    command_line_tool_candidates("claude", Vec::new())
}

/// The same places for OpenAI's codex CLI, plus %APPDATA%\npm\codex.cmd (npm's
/// global folder, the usual install, which may not be on the app's PATH).
pub fn codex_executable_candidates() -> Vec<PathBuf> {
    let npm_global_candidates = std::env::var_os("APPDATA")
        .map(|app_data| vec![Path::new(&app_data).join("npm").join("codex.cmd")])
        .unwrap_or_default();
    command_line_tool_candidates("codex", npm_global_candidates)
}

/// .local\bin and WinGet Links first, then `extra_candidates`, then `where.exe`.
fn command_line_tool_candidates(command_name: &str, extra_candidates: Vec<PathBuf>) -> Vec<PathBuf> {
    let executable_file_name = format!("{command_name}.exe");
    let mut candidates = Vec::new();
    if let Some(user_profile) = std::env::var_os("USERPROFILE") {
        candidates.push(Path::new(&user_profile).join(".local").join("bin").join(&executable_file_name));
    }
    if let Some(local_app_data) = std::env::var_os("LOCALAPPDATA") {
        candidates.push(Path::new(&local_app_data).join("Microsoft").join("WinGet").join("Links").join(&executable_file_name));
    }
    candidates.extend(extra_candidates);

    let where_output = std::process::Command::new("where.exe").arg(command_name).creation_flags(CREATE_NO_WINDOW).output();
    if let Ok(where_output) = where_output {
        let found_paths: Vec<PathBuf> = String::from_utf8_lossy(&where_output.stdout)
            .lines()
            .map(|line| PathBuf::from(line.trim()))
            .collect();
        // npm installs a .cmd shim next to an extensionless sh script; only .exe and .cmd can be spawned.
        for wanted_extension in ["exe", "cmd"] {
            for found_path in &found_paths {
                let has_wanted_extension = found_path
                    .extension()
                    .is_some_and(|extension| extension.eq_ignore_ascii_case(wanted_extension));
                if has_wanted_extension && !candidates.contains(found_path) {
                    candidates.push(found_path.clone());
                }
            }
        }
    }
    candidates
}

/// `--tools Read,Glob,Grep` (all read-only, allowed without asking) plus `--add-dir`
/// for the home folder and the shared Start menu folder, so Glob can list
/// installed apps. No shell tool on purpose: a PowerShell allowlist can't be
/// made tight (`Start-Process` runs any program, `Where-Object { }` runs any
/// script block). Opening apps, files and links is the `[OPEN:target]` tag,
/// checked in Rust (`computer_control::allowed_open_target`).
pub fn voice_turn_tool_arguments(home_directory: &Path) -> Vec<String> {
    let mut arguments: Vec<String> = ["--tools", "Read,Glob,Grep", "--allowedTools", "Read", "Glob", "Grep", "--add-dir"]
        .iter()
        .map(|argument| argument.to_string())
        .collect();
    arguments.push(home_directory.to_string_lossy().into_owned());
    for start_menu_directory in start_menu_shortcut_directories() {
        if !start_menu_directory.starts_with(home_directory) {
            arguments.push("--add-dir".to_string());
            arguments.push(start_menu_directory.to_string_lossy().into_owned());
        }
    }
    arguments
}

/// The user's and the shared Start menu Programs folders; their .lnk files are
/// the installed desktop apps `[OPEN:app name]` may start.
pub fn start_menu_shortcut_directories() -> Vec<PathBuf> {
    let start_menu_programs_path = Path::new("Microsoft").join("Windows").join("Start Menu").join("Programs");
    ["APPDATA", "ProgramData"]
        .iter()
        .filter_map(|environment_variable_name| std::env::var_os(environment_variable_name))
        .map(|base_directory| Path::new(&base_directory).join(&start_menu_programs_path))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn voice_turns_get_only_read_only_tools() {
        let arguments = voice_turn_tool_arguments(Path::new(r"C:\Users\bhai"));
        assert_eq!(&arguments[..8], ["--tools", "Read,Glob,Grep", "--allowedTools", "Read", "Glob", "Grep", "--add-dir", r"C:\Users\bhai"]);
        assert!(!arguments.iter().any(|argument| argument.contains("PowerShell")));
    }
}
