//! Runs OpenAI's local `codex exec` CLI as the alternative brain to `claude -p`
//! (port of the Mac's `CodexCLI.swift`): the same locked-down flags, read-only
//! sandbox, no shell/apps/plugins, bhai's system prompt as
//! `model_instructions_file`, and only the last `agent_message` as the answer.
//! Uses the CLI's own `codex login`, so no API keys. Dropping/aborting the
//! owning task kills the child and its process tree.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;

use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};

use crate::claude_cli::ClaudeRequest;
use crate::platform;
use crate::stream_json::fold_history_into_prompt;

const CODEX_NOT_FOUND_MESSAGE: &str = "couldn't find the codex CLI. install it, then run `codex login` once in a terminal.";

/// Codex has no command allowlist, so voice turns run without any tools; this
/// note (added to the voice prompt only) steers it to on-screen actions, like
/// the Mac's `codexCannotRunMacToolsNote`.
pub const CODEX_CANNOT_USE_FILE_TOOLS_NOTE: &str = "\n\nimportant: in this session you cannot run commands or read files, so the file tools above (Glob, Grep, Read) are not available. open apps, files and links with [OPEN:...], and do everything else with on-screen actions ([CLICK], [TYPE], [KEY]).";

/// The full `codex exec` argument list, in the Mac's order.
pub fn codex_exec_arguments(
    image_file_paths: &[PathBuf],
    working_directory: &Path,
    instructions_file: &Path,
    reasoning_effort: Option<&str>,
) -> Vec<String> {
    let mut arguments: Vec<String> = vec![
        "exec".into(),
        "--json".into(),
        // Don't save this run as a resumable session.
        "--ephemeral".into(),
        "--skip-git-repo-check".into(),
        // Skips ~/.codex/config.toml (MCP servers, plugins, the user's model) and
        // the user's execpolicy rules, like claude's --strict-mcp-config and
        // --setting-sources project. Auth still comes from ~/.codex.
        "--ignore-user-config".into(),
        "--ignore-rules".into(),
        "--sandbox".into(),
        "read-only".into(),
        "--cd".into(),
        working_directory.to_string_lossy().into_owned(),
        "--config".into(),
        "approval_policy=\"never\"".into(),
        // Don't read AGENTS.md files into the prompt.
        "--config".into(),
        "project_doc_max_bytes=0".into(),
        "--config".into(),
        "web_search=\"disabled\"".into(),
        "--config".into(),
        format!("model_instructions_file={}", toml_basic_string(&instructions_file.to_string_lossy())),
        // No shell: the read-only sandbox still lets commands read the whole disk
        // (ssh keys, ~/.codex/auth.json), and text on screen could ask for that.
        "--config".into(),
        "features.shell_tool=false".into(),
        // ChatGPT connectors and installed plugins load from the login, not
        // config.toml, so --ignore-user-config doesn't drop them.
        "--config".into(),
        "features.apps=false".into(),
        "--config".into(),
        "features.plugins=false".into(),
    ];
    if let Some(reasoning_effort) = reasoning_effort {
        arguments.push("--config".into());
        arguments.push(format!("model_reasoning_effort={}", toml_basic_string(reasoning_effort)));
    }
    if !image_file_paths.is_empty() {
        // --image takes several comma-separated values, so the bare "-" (prompt
        // from stdin) goes after "--" or it would be swallowed as another image.
        let joined_image_paths: Vec<String> = image_file_paths.iter().map(|image_path| image_path.to_string_lossy().into_owned()).collect();
        arguments.push(format!("--image={}", joined_image_paths.join(",")));
    }
    arguments.push("--".into());
    arguments.push("-".into());
    arguments
}

/// --config values are parsed as TOML, so a free-form string must be a quoted
/// TOML basic string with backslashes, quotes and control characters escaped
/// (Windows paths are full of backslashes).
pub fn toml_basic_string(raw_text: &str) -> String {
    let mut escaped_text = String::from("\"");
    for character in raw_text.chars() {
        match character {
            '\\' => escaped_text.push_str("\\\\"),
            '"' => escaped_text.push_str("\\\""),
            '\n' => escaped_text.push_str("\\n"),
            '\r' => escaped_text.push_str("\\r"),
            '\t' => escaped_text.push_str("\\t"),
            control_character if (control_character as u32) < 0x20 || control_character as u32 == 0x7F => {
                escaped_text.push_str(&format!("\\u{:04X}", control_character as u32));
            }
            other_character => escaped_text.push(other_character),
        }
    }
    escaped_text.push('"');
    escaped_text
}

/// Turns auth failures into a "sign in" hint; other errors pass through (cut to 500 characters).
pub fn user_facing_error_message(codex_error_text: &str) -> String {
    let lowercased_error_text = codex_error_text.to_lowercase();
    let looks_like_sign_in_problem = ["401", "unauthorized", "not logged in", "login", "log in", "sign in", "authentication"]
        .iter()
        .any(|sign_in_hint| lowercased_error_text.contains(sign_in_hint));
    if looks_like_sign_in_problem {
        let error_start: String = codex_error_text.chars().take(300).collect();
        return format!("codex isn't signed in. run `codex login` in a terminal, then try again. ({error_start})");
    }
    codex_error_text.chars().take(500).collect()
}

/// Reads the `codex exec --json` JSONL stream. Agent text only arrives in
/// `item.completed` events whose item type is `agent_message`.
#[derive(Default)]
pub struct CodexEventStreamParser {
    /// The answer: codex treats the turn's last agent message as final.
    pub last_agent_message_text: Option<String>,
    /// Set by `turn.failed`; the run has no usable answer.
    pub turn_failed_message: Option<String>,
    /// The latest top-level `error` event. Codex also sends these for retries
    /// ("Reconnecting..."), so they only matter if the run fails.
    pub last_error_message: Option<String>,
}

impl CodexEventStreamParser {
    /// Returns the agent message text when this line completed one.
    pub fn handle_json_line(&mut self, json_line: &str) -> Option<String> {
        let thread_event: Value = serde_json::from_str(json_line).ok()?;
        match thread_event["type"].as_str()? {
            "item.completed" => {
                let item = &thread_event["item"];
                if item["type"] != "agent_message" {
                    return None;
                }
                let agent_message_text = item["text"].as_str().filter(|text| !text.is_empty())?.to_string();
                self.last_agent_message_text = Some(agent_message_text.clone());
                Some(agent_message_text)
            }
            "turn.failed" => {
                let failure_message = thread_event["error"]["message"]
                    .as_str()
                    .map(str::to_string)
                    .or_else(|| self.last_error_message.clone())
                    .unwrap_or_else(|| "codex turn failed".to_string());
                self.turn_failed_message = Some(failure_message);
                None
            }
            "error" => {
                self.last_error_message = thread_event["message"].as_str().map(str::to_string);
                None
            }
            _ => None,
        }
    }
}

/// A fresh empty folder per call: holds the screenshots and the instructions
/// file and is the working directory, so no project AGENTS.md is nearby.
/// Removed when dropped, including when the request is cancelled.
struct CodexCallFolder {
    path: PathBuf,
}

impl CodexCallFolder {
    fn create() -> std::io::Result<Self> {
        static CALL_COUNTER: AtomicU64 = AtomicU64::new(0);
        let unique_suffix = format!(
            "{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|elapsed| elapsed.as_nanos()).unwrap_or(0),
            CALL_COUNTER.fetch_add(1, Ordering::Relaxed)
        );
        let path = std::env::temp_dir().join(format!("hellobhai-codex-{unique_suffix}"));
        std::fs::create_dir_all(&path)?;
        Ok(CodexCallFolder { path })
    }
}

impl Drop for CodexCallFolder {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// The prompt on stdin: image labels in order (images can't carry a caption),
/// then the folded history and the user prompt.
pub fn codex_prompt_text(image_labels: &[String], folded_prompt_text: &str) -> String {
    let mut prompt_text = String::new();
    if !image_labels.is_empty() {
        prompt_text.push_str("attached images, in order:\n");
        for (image_index, image_label) in image_labels.iter().enumerate() {
            prompt_text.push_str(&format!("{}. {image_label}\n", image_index + 1));
        }
        prompt_text.push('\n');
    }
    prompt_text.push_str(folded_prompt_text);
    prompt_text
}

/// Runs one codex call with the same request as claude. `on_accumulated_text`
/// gets each completed agent message (codex exec sends whole messages, not
/// deltas; a later one replaces an earlier one). `default_reasoning_effort` is
/// the panel's choice; the request's own effort ("low" for panels) wins.
pub async fn run_codex_streaming(
    request: ClaudeRequest<'_>,
    default_reasoning_effort: Option<String>,
    mut on_accumulated_text: impl FnMut(&str),
) -> Result<String, String> {
    let codex_executable = find_codex_executable().ok_or_else(|| CODEX_NOT_FOUND_MESSAGE.to_string())?;
    let call_folder = CodexCallFolder::create().map_err(|create_error| format!("couldn't create a temp folder for codex: {create_error}"))?;

    // bhai's system prompt replaces Codex's built-in coding-agent instructions
    // (which ask for Markdown and file references) instead of adding to them.
    let mut system_prompt_text = std::fs::read_to_string(request.system_prompt_file)
        .map_err(|read_error| format!("couldn't read the system prompt: {read_error}"))?;
    if request.allows_computer_tools {
        system_prompt_text.push_str(CODEX_CANNOT_USE_FILE_TOOLS_NOTE);
    }
    let instructions_file = call_folder.path.join("instructions.md");
    std::fs::write(&instructions_file, system_prompt_text).map_err(|write_error| format!("couldn't write codex instructions: {write_error}"))?;

    let mut image_file_paths = Vec::new();
    let mut image_labels = Vec::new();
    for (image_index, labeled_image) in request.images.iter().enumerate() {
        let image_file_path = call_folder.path.join(format!("image-{}.jpg", image_index + 1));
        std::fs::write(&image_file_path, &labeled_image.jpeg_bytes).map_err(|write_error| format!("couldn't write a screenshot for codex: {write_error}"))?;
        image_file_paths.push(image_file_path);
        image_labels.push(labeled_image.label.clone());
    }
    let stdin_text = codex_prompt_text(&image_labels, &fold_history_into_prompt(&request.history, &request.user_prompt));

    let reasoning_effort = request.effort.map(str::to_string).or(default_reasoning_effort);
    let mut command = tokio::process::Command::new(&codex_executable);
    command
        .args(codex_exec_arguments(&image_file_paths, &call_folder.path, &instructions_file, reasoning_effort.as_deref()))
        .current_dir(&call_folder.path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    // An npm install of codex runs on node; put codex's own folder first on PATH
    // so a node next to it is found from an app started with a minimal PATH.
    if let Some(codex_directory) = codex_executable.parent() {
        let existing_path = std::env::var_os("PATH").unwrap_or_default();
        let extended_path = std::env::join_paths(std::iter::once(codex_directory.to_path_buf()).chain(std::env::split_paths(&existing_path)));
        if let Ok(extended_path) = extended_path {
            command.env("PATH", extended_path);
        }
    }
    platform::configure_background_command(&mut command);

    let mut child = command.spawn().map_err(|spawn_error| format!("couldn't start codex: {spawn_error}"))?;
    let _codex_process_tree = platform::kill_process_tree_when_dropped(&child);
    eprintln!(
        "codex exec: sent {} image(s), effort {}",
        image_file_paths.len(),
        reasoning_effort.as_deref().unwrap_or("default")
    );

    // Write stdin and drain stderr on their own tasks so neither pipe can fill up.
    let mut child_stdin = child.stdin.take().ok_or("codex stdin missing")?;
    tokio::spawn(async move {
        let _ = child_stdin.write_all(stdin_text.as_bytes()).await;
        let _ = child_stdin.shutdown().await;
    });
    let mut child_stderr = child.stderr.take().ok_or("codex stderr missing")?;
    let stderr_task = tokio::spawn(async move {
        let mut stderr_text = String::new();
        let _ = child_stderr.read_to_string(&mut stderr_text).await;
        stderr_text
    });

    let child_stdout = child.stdout.take().ok_or("codex stdout missing")?;
    let mut stdout_lines = BufReader::new(child_stdout).lines();
    let mut event_stream_parser = CodexEventStreamParser::default();
    while let Some(line) = stdout_lines.next_line().await.map_err(|read_error| format!("reading codex output failed: {read_error}"))? {
        if let Some(latest_agent_message_text) = event_stream_parser.handle_json_line(&line) {
            on_accumulated_text(&latest_agent_message_text);
        }
    }

    let exit_status = child.wait().await.map_err(|wait_error| format!("waiting for codex failed: {wait_error}"))?;
    if let Some(turn_failed_message) = event_stream_parser.turn_failed_message {
        return Err(user_facing_error_message(&turn_failed_message));
    }
    if !exit_status.success() {
        let stderr_text = stderr_task.await.unwrap_or_default();
        let stderr_tail: String = {
            let stderr_characters: Vec<char> = stderr_text.chars().collect();
            stderr_characters[stderr_characters.len().saturating_sub(500)..].iter().collect()
        };
        let failure_detail = event_stream_parser.last_error_message.unwrap_or(stderr_tail);
        let exit_code_text = exit_status.code().map(|code| code.to_string()).unwrap_or_else(|| "a signal".to_string());
        return Err(user_facing_error_message(&format!("codex exec exited with {exit_code_text}: {failure_detail}")));
    }
    // An interrupted turn, or one that ended on a tool call, exits 0 without an answer.
    event_stream_parser
        .last_agent_message_text
        .ok_or_else(|| "codex returned no reply. try again.".to_string())
}

/// First existing path from `platform::codex_executable_candidates()`, cached
/// for the app's lifetime like the claude lookup.
pub fn find_codex_executable() -> Option<PathBuf> {
    static CODEX_EXECUTABLE: OnceLock<Option<PathBuf>> = OnceLock::new();
    CODEX_EXECUTABLE
        .get_or_init(|| platform::codex_executable_candidates().into_iter().find(|candidate| candidate.is_file()))
        .clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arguments_match_the_mac_with_images_after_the_flags_and_stdin_last() {
        let arguments = codex_exec_arguments(
            &[PathBuf::from("call/image-1.jpg"), PathBuf::from("call/image-2.jpg")],
            Path::new("call"),
            Path::new("call/instructions.md"),
            Some("low"),
        );
        let expected_arguments: Vec<String> = [
            "exec",
            "--json",
            "--ephemeral",
            "--skip-git-repo-check",
            "--ignore-user-config",
            "--ignore-rules",
            "--sandbox",
            "read-only",
            "--cd",
            "call",
            "--config",
            "approval_policy=\"never\"",
            "--config",
            "project_doc_max_bytes=0",
            "--config",
            "web_search=\"disabled\"",
            "--config",
            "model_instructions_file=\"call/instructions.md\"",
            "--config",
            "features.shell_tool=false",
            "--config",
            "features.apps=false",
            "--config",
            "features.plugins=false",
            "--config",
            "model_reasoning_effort=\"low\"",
            "--image=call/image-1.jpg,call/image-2.jpg",
            "--",
            "-",
        ]
        .iter()
        .map(|argument| argument.to_string())
        .collect();
        assert_eq!(arguments, expected_arguments);

        let no_image_arguments = codex_exec_arguments(&[], Path::new("call"), Path::new("call/instructions.md"), None);
        assert!(!no_image_arguments.iter().any(|argument| argument.starts_with("--image") || argument.contains("model_reasoning_effort")));
        assert_eq!(&no_image_arguments[no_image_arguments.len() - 2..], ["--", "-"]);
    }

    #[test]
    fn toml_strings_escape_windows_paths_quotes_and_control_characters() {
        assert_eq!(toml_basic_string(r"C:\Users\bhai\instructions.md"), r#""C:\\Users\\bhai\\instructions.md""#);
        assert_eq!(toml_basic_string("say \"hi\"\n\t"), r#""say \"hi\"\n\t""#);
        assert_eq!(toml_basic_string("\u{1}\u{7F}"), r#""\u0001\u007F""#);
    }

    #[test]
    fn sign_in_failures_get_a_codex_login_hint() {
        assert!(user_facing_error_message("401 Unauthorized").starts_with("codex isn't signed in. run `codex login`"));
        assert_eq!(user_facing_error_message("rate limited"), "rate limited");
    }

    #[test]
    fn only_the_last_agent_message_is_the_answer_and_failures_are_kept() {
        let mut event_stream_parser = CodexEventStreamParser::default();
        assert_eq!(event_stream_parser.handle_json_line(r#"{"type":"thread.started","thread_id":"t"}"#), None);
        assert_eq!(
            event_stream_parser.handle_json_line(r#"{"type":"item.completed","item":{"id":"1","type":"reasoning","text":"thinking"}}"#),
            None
        );
        assert_eq!(
            event_stream_parser.handle_json_line(r#"{"type":"item.completed","item":{"id":"2","type":"agent_message","text":"first"}}"#),
            Some("first".to_string())
        );
        event_stream_parser.handle_json_line(r#"{"type":"item.completed","item":{"id":"3","type":"agent_message","text":"see that button [POINT:10,20:send]"}}"#);
        assert_eq!(event_stream_parser.last_agent_message_text.as_deref(), Some("see that button [POINT:10,20:send]"));
        assert_eq!(event_stream_parser.handle_json_line("not json"), None);
        assert_eq!(event_stream_parser.turn_failed_message, None);

        let mut failing_parser = CodexEventStreamParser::default();
        failing_parser.handle_json_line(r#"{"type":"error","message":"Reconnecting... 1/5"}"#);
        failing_parser.handle_json_line(r#"{"type":"turn.failed"}"#);
        assert_eq!(failing_parser.turn_failed_message.as_deref(), Some("Reconnecting... 1/5"));
        failing_parser.handle_json_line(r#"{"type":"turn.failed","error":{"message":"stream disconnected"}}"#);
        assert_eq!(failing_parser.turn_failed_message.as_deref(), Some("stream disconnected"));
    }

    #[test]
    fn image_labels_come_first_in_the_stdin_prompt() {
        assert_eq!(
            codex_prompt_text(&["screen 1 of 2".to_string(), "screen 2 of 2".to_string()], "hi"),
            "attached images, in order:\n1. screen 1 of 2\n2. screen 2 of 2\n\nhi"
        );
        assert_eq!(codex_prompt_text(&[], "hi"), "hi");
    }
}
