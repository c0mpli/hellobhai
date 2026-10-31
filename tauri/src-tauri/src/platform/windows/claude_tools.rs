//! Where claude.exe lives on Windows and which tools a voice turn may use: the
//! Windows counterpart of the Mac's osascript/open/mdfind/shortcuts/ls allowlist.
//! Anything not allowed is denied in print mode; the prompt still demands a
//! spoken yes before anything that sends, posts, deletes or buys.

use std::path::{Path, PathBuf};

/// In order: %USERPROFILE%\.local\bin\claude.exe (native installer),
/// %LOCALAPPDATA%\Microsoft\WinGet\Links\claude.exe, then `where.exe claude`
/// (preferring .exe over .cmd).
pub fn claude_executable_candidates() -> Vec<PathBuf> {
    todo!("C: candidate paths")
}

/// `--tools PowerShell,Read`, the PowerShell allowlist (Start-Process, Get-ChildItem,
/// Get-StartApps, Sort-Object, Select-Object, Where-Object, Read), the Start-Process
/// deny list for shell interpreters, and `--add-dir <home>`. One argv element per rule.
pub fn voice_turn_tool_arguments(home_directory: &Path) -> Vec<String> {
    todo!("C: tool allowlist argv")
}

/// CLAUDE_CODE_USE_POWERSHELL_TOOL=1 keeps the PowerShell tool on in every session type.
pub fn voice_turn_environment() -> Vec<(String, String)> {
    todo!("C: environment")
}
