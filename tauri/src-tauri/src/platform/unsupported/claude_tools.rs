use std::path::{Path, PathBuf};

pub fn claude_executable_candidates() -> Vec<PathBuf> {
    Vec::new()
}

/// No tools at all on an unsupported OS.
pub fn voice_turn_tool_arguments(home_directory: &Path) -> Vec<String> {
    vec!["--tools".to_string(), String::new()]
}

pub fn voice_turn_environment() -> Vec<(String, String)> {
    Vec::new()
}
