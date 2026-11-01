use std::path::{Path, PathBuf};

pub fn claude_executable_candidates() -> Vec<PathBuf> {
    Vec::new()
}

pub fn codex_executable_candidates() -> Vec<PathBuf> {
    Vec::new()
}

/// No tools at all on an unsupported OS.
pub fn voice_turn_tool_arguments(home_directory: &Path) -> Vec<String> {
    vec!["--tools".to_string(), String::new()]
}

pub fn start_menu_shortcut_directories() -> Vec<PathBuf> {
    Vec::new()
}
