//! Spawns the local `claude -p` CLI with stream-json in and out, the same flags
//! and lockdown as the Mac (`ClaudeCLI.swift`), except `--system-prompt-file`
//! instead of `--system-prompt` (Windows command-line length and quoting).
//! Dropping/aborting the owning task kills the child (`kill_on_drop`).

use std::path::{Path, PathBuf};

use crate::conversation_history::ConversationEntry;
use crate::stream_json::LabeledImage;

pub struct ClaudeRequest<'a> {
    pub images: Vec<LabeledImage>,
    pub system_prompt_file: &'a Path,
    pub history: Vec<ConversationEntry>,
    pub user_prompt: String,
    /// Voice turns: platform tool allowlist (`platform::voice_turn_tool_arguments`). Otherwise `--tools ""`.
    pub allows_computer_tools: bool,
    /// `Some("low")` for the board, notes and 3D calls so the first line streams fast.
    pub effort: Option<&'static str>,
    /// Model alias: "fable", "opus", "sonnet" or "haiku".
    pub model: String,
}

/// Runs one claude call. `on_accumulated_text` gets the whole reply so far after
/// every text delta. Returns the final result text (or the accumulated deltas).
pub async fn run_claude_streaming(
    request: ClaudeRequest<'_>,
    on_accumulated_text: impl FnMut(&str),
) -> Result<String, String> {
    todo!("E: spawn claude -p, write stdin, stream stdout, map errors like the Mac")
}

/// First existing path from `platform::claude_executable_candidates()`, cached.
pub fn find_claude_executable() -> Option<PathBuf> {
    todo!("E: cache the first existing candidate")
}
