//! Where claude.exe lives on Windows and which tools a voice turn may use: the
//! Windows counterpart of the Mac's osascript/open/mdfind/shortcuts/ls allowlist.
//! Anything not allowed is denied in print mode; the prompt still demands a
//! spoken yes before anything that sends, posts, deletes or buys.

use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};

/// No console window flashes up for `where.exe`.
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// In order: %USERPROFILE%\.local\bin\claude.exe (native installer),
/// %LOCALAPPDATA%\Microsoft\WinGet\Links\claude.exe, then `where.exe claude`
/// (preferring .exe over .cmd).
pub fn claude_executable_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(user_profile) = std::env::var_os("USERPROFILE") {
        candidates.push(Path::new(&user_profile).join(".local").join("bin").join("claude.exe"));
    }
    if let Some(local_app_data) = std::env::var_os("LOCALAPPDATA") {
        candidates.push(Path::new(&local_app_data).join("Microsoft").join("WinGet").join("Links").join("claude.exe"));
    }

    let where_output = std::process::Command::new("where.exe").arg("claude").creation_flags(CREATE_NO_WINDOW).output();
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

/// `--tools PowerShell,Read`, the PowerShell allowlist (Start-Process, Get-ChildItem,
/// Get-StartApps, Sort-Object, Select-Object, Where-Object, Read), the Start-Process
/// deny list for shell interpreters, and `--add-dir <home>`. One argv element per rule.
pub fn voice_turn_tool_arguments(home_directory: &Path) -> Vec<String> {
    // Start-Process opens apps, files and URLs (the Mac's `open`/osascript), Get-ChildItem
    // lists folders and searches files (`ls`/`mdfind`), Get-StartApps finds installed apps.
    let allowed_tools = [
        "PowerShell(Start-Process *)",
        "PowerShell(Get-ChildItem *)",
        "PowerShell(Get-StartApps *)",
        "PowerShell(Sort-Object *)",
        "PowerShell(Select-Object *)",
        "PowerShell(Where-Object *)",
        "Read",
    ];
    // Start-Process could launch a shell and run anything; deny the interpreters outright.
    let disallowed_start_process_targets = [
        "powershell", "pwsh", "cmd", "wscript", "cscript", "mshta", "rundll32", "regsvr32", "wsl", "bash",
    ];

    let mut arguments = vec!["--tools".to_string(), "PowerShell,Read".to_string(), "--allowedTools".to_string()];
    arguments.extend(allowed_tools.iter().map(|rule| rule.to_string()));
    arguments.push("--disallowedTools".to_string());
    arguments.extend(
        disallowed_start_process_targets
            .iter()
            .map(|program_name| format!("PowerShell(Start-Process {program_name}*)")),
    );
    arguments.push("--add-dir".to_string());
    arguments.push(home_directory.to_string_lossy().into_owned());
    arguments
}

/// CLAUDE_CODE_USE_POWERSHELL_TOOL=1 keeps the PowerShell tool on in every session type.
pub fn voice_turn_environment() -> Vec<(String, String)> {
    vec![("CLAUDE_CODE_USE_POWERSHELL_TOOL".to_string(), "1".to_string())]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_one_argv_element_per_rule() {
        let arguments = voice_turn_tool_arguments(Path::new(r"C:\Users\bhai"));
        assert_eq!(&arguments[..3], ["--tools", "PowerShell,Read", "--allowedTools"]);
        assert!(arguments.contains(&"PowerShell(Start-Process cmd*)".to_string()));
        assert_eq!(&arguments[arguments.len() - 2..], ["--add-dir", r"C:\Users\bhai"]);
    }
}
