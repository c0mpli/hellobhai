//! The `claude -p` stream-json wire format: parsing output lines, building the
//! one stdin message (screenshots as base64 image blocks + prompt text), and
//! folding conversation history into the prompt text (stream-json input takes
//! one user message per run, so history cannot be sent as separate turns).

use base64::Engine;
use serde_json::{json, Value};

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
    let Ok(output_event) = serde_json::from_str::<Value>(line) else {
        return ClaudeOutputEvent::Other;
    };
    match output_event["type"].as_str() {
        Some("stream_event") => {
            let stream_event = &output_event["event"];
            let delta = &stream_event["delta"];
            if stream_event["type"] == "content_block_delta" && delta["type"] == "text_delta" {
                if let Some(text_chunk) = delta["text"].as_str() {
                    return ClaudeOutputEvent::TextDelta(text_chunk.to_string());
                }
            }
            ClaudeOutputEvent::Other
        }
        Some("result") => {
            if output_event["is_error"] == true {
                let error_text = output_event["result"].as_str().unwrap_or("claude -p returned an error");
                ClaudeOutputEvent::ErrorResult(error_text.to_string())
            } else {
                // The Mac keeps a missing result as nil and falls back to the deltas.
                match output_event["result"].as_str() {
                    Some(result_text) => ClaudeOutputEvent::FinalResult(result_text.to_string()),
                    None => ClaudeOutputEvent::Other,
                }
            }
        }
        _ => ClaudeOutputEvent::Other,
    }
}

/// One JSON line plus "\n": for each image an image block then a text block
/// with its label, then the prompt text block.
pub fn build_stdin_message(images: &[LabeledImage], prompt_text: &str) -> String {
    let mut content_blocks: Vec<Value> = Vec::new();
    for image in images {
        content_blocks.push(json!({
            "type": "image",
            "source": {
                "type": "base64",
                "media_type": "image/jpeg",
                "data": base64::engine::general_purpose::STANDARD.encode(&image.jpeg_bytes),
            }
        }));
        content_blocks.push(json!({ "type": "text", "text": image.label }));
    }
    content_blocks.push(json!({ "type": "text", "text": prompt_text }));

    let stdin_message = json!({
        "type": "user",
        "message": { "role": "user", "content": content_blocks }
    });
    let mut stdin_line = stdin_message.to_string();
    stdin_line.push('\n');
    stdin_line
}

/// Exact Mac format: "conversation so far:\nuser: …\nyou: …\n\nnew message from the user:\n" + prompt.
/// Empty history -> the prompt unchanged.
pub fn fold_history_into_prompt(history: &[ConversationEntry], user_prompt: &str) -> String {
    let mut prompt_text = String::new();
    if !history.is_empty() {
        prompt_text.push_str("conversation so far:\n");
        for entry in history {
            prompt_text.push_str(&format!("user: {}\nyou: {}\n", entry.user_text, entry.assistant_response));
        }
        prompt_text.push_str("\nnew message from the user:\n");
    }
    prompt_text.push_str(user_prompt);
    prompt_text
}

#[cfg(test)]
mod tests {
    use super::*;

    // Shapes copied from a real `claude -p --output-format stream-json --include-partial-messages --verbose` run.
    #[test]
    fn text_deltas_and_results_are_parsed_and_everything_else_is_ignored() {
        let text_delta_line = r#"{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"see that "}},"session_id":"abc","parent_tool_use_id":null,"uuid":"1"}"#;
        assert_eq!(parse_output_line(text_delta_line), ClaudeOutputEvent::TextDelta("see that ".to_string()));

        let thinking_delta_line = r#"{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"the user wants"}},"session_id":"abc"}"#;
        assert_eq!(parse_output_line(thinking_delta_line), ClaudeOutputEvent::Other);

        let signature_delta_line = r#"{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":"EqQB"}}}"#;
        assert_eq!(parse_output_line(signature_delta_line), ClaudeOutputEvent::Other);

        let message_start_line = r#"{"type":"stream_event","event":{"type":"message_start","message":{"id":"msg_1"}}}"#;
        assert_eq!(parse_output_line(message_start_line), ClaudeOutputEvent::Other);

        let system_init_line = r#"{"type":"system","subtype":"init","cwd":"C:\\Users\\me\\AppData\\Local\\Temp","tools":[]}"#;
        assert_eq!(parse_output_line(system_init_line), ClaudeOutputEvent::Other);

        let success_result_line = r#"{"type":"result","subtype":"success","is_error":false,"duration_ms":2310,"result":"see that button? [POINT:100,200:run]","session_id":"abc"}"#;
        assert_eq!(
            parse_output_line(success_result_line),
            ClaudeOutputEvent::FinalResult("see that button? [POINT:100,200:run]".to_string())
        );

        let error_result_line = r#"{"type":"result","subtype":"success","is_error":true,"result":"Invalid API key · Please run /login"}"#;
        assert_eq!(
            parse_output_line(error_result_line),
            ClaudeOutputEvent::ErrorResult("Invalid API key · Please run /login".to_string())
        );

        assert_eq!(parse_output_line("not json at all"), ClaudeOutputEvent::Other);
        assert_eq!(parse_output_line(""), ClaudeOutputEvent::Other);
    }

    #[test]
    fn stdin_message_has_image_then_label_per_screenshot_then_the_prompt() {
        let images = vec![LabeledImage { jpeg_bytes: vec![1, 2, 3], label: "user's screen (cursor is here)".to_string() }];
        let stdin_line = build_stdin_message(&images, "what is this?");
        assert!(stdin_line.ends_with('\n'));
        let stdin_message: Value = serde_json::from_str(stdin_line.trim_end()).unwrap();
        assert_eq!(stdin_message["type"], "user");
        assert_eq!(stdin_message["message"]["role"], "user");
        let content_blocks = stdin_message["message"]["content"].as_array().unwrap();
        assert_eq!(content_blocks.len(), 3);
        assert_eq!(content_blocks[0]["type"], "image");
        assert_eq!(content_blocks[0]["source"]["media_type"], "image/jpeg");
        assert_eq!(content_blocks[0]["source"]["data"], "AQID");
        assert_eq!(content_blocks[1]["text"], "user's screen (cursor is here)");
        assert_eq!(content_blocks[2]["text"], "what is this?");
    }

    #[test]
    fn history_is_folded_in_the_mac_format() {
        assert_eq!(fold_history_into_prompt(&[], "hello"), "hello");
        let history = vec![
            ConversationEntry { user_text: "open chrome".to_string(), assistant_response: "opening it. (did: pressed cmd+space)".to_string() },
            ConversationEntry { user_text: "thanks".to_string(), assistant_response: "anytime".to_string() },
        ];
        assert_eq!(
            fold_history_into_prompt(&history, "now youtube"),
            "conversation so far:\nuser: open chrome\nyou: opening it. (did: pressed cmd+space)\nuser: thanks\nyou: anytime\n\nnew message from the user:\nnow youtube"
        );
    }
}
