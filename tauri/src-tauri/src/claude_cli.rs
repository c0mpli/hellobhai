//! Spawns the local `claude -p` CLI with stream-json in and out, the same flags
//! and lockdown as the Mac (`ClaudeCLI.swift`), except `--system-prompt-file`
//! instead of `--system-prompt` (Windows command-line length and quoting).
//! Dropping/aborting the owning task kills the child (`kill_on_drop`).

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::OnceLock;

use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};

use crate::conversation_history::ConversationEntry;
use crate::platform;
use crate::stream_json::{build_stdin_message, fold_history_into_prompt, parse_output_line, ClaudeOutputEvent, LabeledImage};

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

const CLAUDE_NOT_FOUND_MESSAGE: &str = "couldn't find the claude CLI. install claude code and run `claude` once to log in.";

/// Every argument after the executable, in the Mac's order.
fn claude_arguments(request: &ClaudeRequest<'_>, home_directory: &Path) -> Vec<String> {
    let mut arguments: Vec<String> = [
        "-p",
        "--input-format",
        "stream-json",
        "--output-format",
        "stream-json",
        "--include-partial-messages",
        "--verbose",
        "--model",
        &request.model,
        "--system-prompt-file",
        &request.system_prompt_file.to_string_lossy(),
        // No MCP servers, slash commands, or user settings: faster startup and no unrelated context.
        "--strict-mcp-config",
        "--disable-slash-commands",
        "--setting-sources",
        "project",
        "--no-session-persistence",
    ]
    .iter()
    .map(|argument| argument.to_string())
    .collect();
    if request.allows_computer_tools {
        arguments.extend(platform::voice_turn_tool_arguments(home_directory));
    } else {
        arguments.extend(["--tools".to_string(), String::new()]);
    }
    if let Some(effort) = request.effort {
        arguments.extend(["--effort".to_string(), effort.to_string()]);
    }
    arguments
}

fn home_directory() -> PathBuf {
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
}

/// Runs one claude call. `on_accumulated_text` gets the whole reply so far after
/// every text delta. Returns the final result text (or the accumulated deltas).
pub async fn run_claude_streaming(
    request: ClaudeRequest<'_>,
    mut on_accumulated_text: impl FnMut(&str),
) -> Result<String, String> {
    let claude_executable = find_claude_executable().ok_or_else(|| CLAUDE_NOT_FOUND_MESSAGE.to_string())?;

    // stream-json input takes one user message, so prior turns are folded into the prompt text.
    let prompt_text = fold_history_into_prompt(&request.history, &request.user_prompt);
    let stdin_message = build_stdin_message(&request.images, &prompt_text);

    let mut command = tokio::process::Command::new(&claude_executable);
    command
        .args(claude_arguments(&request, &home_directory()))
        // Run from the temp dir so no project CLAUDE.md gets picked up.
        .current_dir(std::env::temp_dir())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        // Aborting the owning task drops the child, which kills claude (Mac: process.terminate()).
        .kill_on_drop(true);
    if request.allows_computer_tools {
        command.envs(platform::voice_turn_environment());
    }
    platform::configure_background_command(&mut command);

    let mut child = command.spawn().map_err(|spawn_error| format!("couldn't start claude: {spawn_error}"))?;
    platform::attach_child_to_app_lifetime(&child);
    eprintln!(
        "claude -p: sent {:.1}MB, {} image(s), model {}",
        stdin_message.len() as f64 / 1_048_576.0,
        request.images.len(),
        request.model
    );

    // Write stdin and drain stderr on their own tasks so neither pipe can fill up
    // and block claude while stdout is read.
    let mut child_stdin = child.stdin.take().ok_or("claude stdin missing")?;
    tokio::spawn(async move {
        let _ = child_stdin.write_all(stdin_message.as_bytes()).await;
        let _ = child_stdin.shutdown().await;
    });
    let mut child_stderr = child.stderr.take().ok_or("claude stderr missing")?;
    let stderr_task = tokio::spawn(async move {
        let mut stderr_text = String::new();
        let _ = child_stderr.read_to_string(&mut stderr_text).await;
        stderr_text
    });

    let child_stdout = child.stdout.take().ok_or("claude stdout missing")?;
    let mut stdout_lines = BufReader::new(child_stdout).lines();
    let mut accumulated_response_text = String::new();
    let mut final_result_text: Option<String> = None;
    let mut result_error_text: Option<String> = None;
    while let Some(line) = stdout_lines.next_line().await.map_err(|read_error| format!("reading claude output failed: {read_error}"))? {
        match parse_output_line(&line) {
            ClaudeOutputEvent::TextDelta(text_chunk) => {
                accumulated_response_text.push_str(&text_chunk);
                on_accumulated_text(&accumulated_response_text);
            }
            ClaudeOutputEvent::FinalResult(result_text) => final_result_text = Some(result_text),
            ClaudeOutputEvent::ErrorResult(error_text) => result_error_text = Some(error_text),
            ClaudeOutputEvent::Other => {}
        }
    }

    let exit_status = child.wait().await.map_err(|wait_error| format!("waiting for claude failed: {wait_error}"))?;
    if let Some(result_error_text) = result_error_text {
        return Err(result_error_text);
    }
    if !exit_status.success() {
        let stderr_text = stderr_task.await.unwrap_or_default();
        let stderr_start: String = stderr_text.chars().take(500).collect();
        let exit_code_text = exit_status.code().map(|code| code.to_string()).unwrap_or_else(|| "a signal".to_string());
        return Err(format!("claude -p exited with {exit_code_text}: {stderr_start}"));
    }
    Ok(final_result_text.unwrap_or(accumulated_response_text))
}

/// First existing path from `platform::claude_executable_candidates()`, cached
/// for the app's lifetime like the Mac's static lookup.
pub fn find_claude_executable() -> Option<PathBuf> {
    static CLAUDE_EXECUTABLE: OnceLock<Option<PathBuf>> = OnceLock::new();
    CLAUDE_EXECUTABLE
        .get_or_init(|| platform::claude_executable_candidates().into_iter().find(|candidate| candidate.is_file()))
        .clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arguments_follow_the_mac_order_with_a_prompt_file() {
        let system_prompt_file = PathBuf::from("prompts").join("model3d.txt");
        let request = ClaudeRequest {
            images: Vec::new(),
            system_prompt_file: &system_prompt_file,
            history: Vec::new(),
            user_prompt: "build it".to_string(),
            allows_computer_tools: false,
            effort: Some("low"),
            model: "sonnet".to_string(),
        };
        let arguments = claude_arguments(&request, Path::new("home"));
        let mut expected_arguments: Vec<String> = [
            "-p",
            "--input-format",
            "stream-json",
            "--output-format",
            "stream-json",
            "--include-partial-messages",
            "--verbose",
            "--model",
            "sonnet",
            "--system-prompt-file",
        ]
        .iter()
        .map(|argument| argument.to_string())
        .collect();
        expected_arguments.push(system_prompt_file.to_string_lossy().to_string());
        expected_arguments.extend(
            [
                "--strict-mcp-config",
                "--disable-slash-commands",
                "--setting-sources",
                "project",
                "--no-session-persistence",
                "--tools",
                "",
                "--effort",
                "low",
            ]
            .iter()
            .map(|argument| argument.to_string()),
        );
        assert_eq!(arguments, expected_arguments);
    }
}
