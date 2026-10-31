//! The `claude -p` stream-json wire format: parsing output lines, building the
//! one stdin message (screenshots as base64 image blocks + prompt text), and
//! folding conversation history into the prompt text (stream-json input takes
//! one user message per run, so history cannot be sent as separate turns).

use crate::conversation_history::ConversationEntry;

/// One parsed line of `--output-format stream-json --include-partial-messages`.
#[derive(Clone, Debug, PartialEq)]
pub enum ClaudeOutputEvent {
    /// A `text_delta` from a `stream_event` content_block_delta.
    TextDelta(String),
    /// The `result` line's text when `is_error` is false.
    FinalResult(String),
    /// The `result` line's text when `is_error` is true.
    ErrorResult(String),
    /// Everything else, including `thinking_delta` and `signature_delta`, which must be ignored.
    Other,
}

/// A JPEG screenshot (or crop) plus the text label sent right after it.
#[derive(Clone, Debug)]
pub struct LabeledImage {
    pub jpeg_bytes: Vec<u8>,
    pub label: String,
}

pub fn parse_output_line(line: &str) -> ClaudeOutputEvent {
    todo!("E: parse one stream-json output line")
}

/// One JSON line plus "\n": for each image an image block then a text block
/// with its label, then the prompt text block.
pub fn build_stdin_message(images: &[LabeledImage], prompt_text: &str) -> String {
    todo!("E: build the stream-json user message")
}

/// Exact Mac format: "conversation so far:\nuser: …\nyou: …\n\nnew message from the user:\n" + prompt.
/// Empty history -> the prompt unchanged.
pub fn fold_history_into_prompt(history: &[ConversationEntry], user_prompt: &str) -> String {
    todo!("E: fold history like ClaudeCLI.swift")
}
