//! The typed chat's conversation and reply rules, ports of the Mac's
//! `CompanionChatMessage`, `chatDisplayText`, `finishStreamingChatMessages` and
//! the reply combinations in `sendTranscriptToClaudeWithScreenshot`. Pure logic,
//! no windows, so it is unit-tested here; `chat_panel.rs` shows it.

use crate::brain_engine::BrainEngine;
use std::sync::LazyLock;

use regex::Regex;
use serde::Serialize;

/// One line in the chat: something the user typed or said, or a reply.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatMessage {
    pub id: u64,
    pub is_from_user: bool,
    pub text: String,
    /// True while Claude's reply is still streaming in.
    pub is_streaming: bool,
}

/// Everything said and typed this session. Voice turns land here too, so the
/// chat reads as one conversation.
#[derive(Default)]
pub struct ChatConversation {
    messages: Vec<ChatMessage>,
    next_message_id: u64,
}

impl ChatConversation {
    pub fn messages(&self) -> &[ChatMessage] {
        &self.messages
    }

    /// Adds a row and returns its id.
    pub fn append(&mut self, is_from_user: bool, text: String, is_streaming: bool) -> u64 {
        self.next_message_id += 1;
        let message_id = self.next_message_id;
        self.messages.push(ChatMessage { id: message_id, is_from_user, text, is_streaming });
        message_id
    }

    pub fn update(&mut self, message_id: u64, text: String, is_streaming: bool) {
        if let Some(message) = self.messages.iter_mut().find(|message| message.id == message_id) {
            message.text = text;
            message.is_streaming = is_streaming;
        }
    }

    pub fn remove(&mut self, message_id: u64) {
        self.messages.retain(|message| message.id != message_id);
    }

    /// Only touches the given rows: a cancelled request's cleanup can run when
    /// the next request's empty placeholder row is already in the list. Empty
    /// placeholders are removed; rows with text stop showing as still typing.
    pub fn finish_streaming(&mut self, message_ids: &[u64]) {
        self.messages
            .retain(|message| !(message_ids.contains(&message.id) && message.is_streaming && message.text.is_empty()));
        for message in self.messages.iter_mut() {
            if message_ids.contains(&message.id) {
                message.is_streaming = false;
            }
        }
    }
}

// Only bhai's own tags, so ordinary brackets like [1] or arr[0] stay.
const KNOWN_TAG_NAMES: &str = "(POINT|CIRCLE|CLICK|TYPE|KEY|OPEN|SHOW|NOTES|MODEL3D|CLOSE_DRAWING)";

static FINISHED_TAG_PATTERN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(&format!(r"\[{KNOWN_TAG_NAMES}(:[^\]]*)?\]")).expect("valid tag pattern"));
static UNFINISHED_TAG_AT_END_PATTERN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(&format!(r"\[([A-Z0-9_]*|{KNOWN_TAG_NAMES}:[^\]]*)$")).expect("valid tag pattern"));

/// What a reply looks like in the chat while it streams: finished tags removed,
/// and a tag that has only half arrived hidden so "[POI" never flashes on screen
/// (it shows again once the `]` arrives if it was ordinary text). `[OPEN]` is the
/// Windows/Linux action tag, hidden like the Mac's other action tags.
pub fn chat_display_text_for_partial_reply(partial_reply: &str) -> String {
    let without_finished_tags = FINISHED_TAG_PATTERN.replace_all(partial_reply, "");
    let without_unfinished_tag = UNFINISHED_TAG_AT_END_PATTERN.replace(&without_finished_tags, "");
    without_unfinished_tag.trim().to_string()
}

/// How one question is answered (all four Mac combinations).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReplyRouting {
    pub replies_are_spoken: bool,
    /// A spoken question answered only in text needs the chat on screen to be
    /// read; it opens without leaving voice mode.
    pub opens_chat_without_switching_mode: bool,
}

pub fn reply_routing(is_typed_chat_message: bool, speaks_voice_replies: bool, speaks_chat_replies: bool) -> ReplyRouting {
    if is_typed_chat_message {
        ReplyRouting { replies_are_spoken: speaks_chat_replies, opens_chat_without_switching_mode: false }
    } else {
        ReplyRouting { replies_are_spoken: speaks_voice_replies, opens_chat_without_switching_mode: !speaks_voice_replies }
    }
}

/// What the hotkey does in chat mode (Mac `toggleChatPanelFromHotkey`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChatHotkeyAction {
    MinimizeChat,
    OpenChat,
}

pub fn chat_hotkey_action(chat_is_open_and_focused: bool) -> ChatHotkeyAction {
    if chat_is_open_and_focused {
        ChatHotkeyAction::MinimizeChat
    } else {
        ChatHotkeyAction::OpenChat
    }
}

/// The model name under the composer, like the Mac ("Sonnet", "Opus", ...,
/// or "Codex" when Codex is the brain).
pub fn chat_model_label(brain_engine: BrainEngine, claude_model_alias: &str) -> String {
    if brain_engine == BrainEngine::Codex {
        return "Codex".to_string();
    }
    let mut characters = claude_model_alias.chars();
    match characters.next() {
        Some(first_character) => first_character.to_uppercase().collect::<String>() + characters.as_str(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finished_tags_are_hidden_but_ordinary_brackets_stay() {
        assert_eq!(chat_display_text_for_partial_reply("click save [POINT:100,200:save:screen1]"), "click save");
        assert_eq!(chat_display_text_for_partial_reply("done [CLICK:1,2:ok] [TYPE:hello] [KEY:ctrl+s]"), "done");
        assert_eq!(chat_display_text_for_partial_reply("look [SHOW:how dns works]"), "look");
        assert_eq!(chat_display_text_for_partial_reply("closing [CLOSE_DRAWING]"), "closing");
        assert_eq!(chat_display_text_for_partial_reply("opening [OPEN:notepad]"), "opening");
        assert_eq!(chat_display_text_for_partial_reply("see note [1] and arr[0]"), "see note [1] and arr[0]");
        assert_eq!(chat_display_text_for_partial_reply("[POINT:none]"), "");
    }

    #[test]
    fn a_half_arrived_tag_is_hidden_until_it_closes() {
        assert_eq!(chat_display_text_for_partial_reply("here it is [POI"), "here it is");
        assert_eq!(chat_display_text_for_partial_reply("here it is ["), "here it is");
        assert_eq!(chat_display_text_for_partial_reply("here [POINT:100,2"), "here");
        assert_eq!(chat_display_text_for_partial_reply("here [TYPE:hello wor"), "here");
        // Lowercase text after a bracket is ordinary text, not a tag.
        assert_eq!(chat_display_text_for_partial_reply("array [i"), "array [i");
        // Once it closes as something other than a known tag, it shows.
        assert_eq!(chat_display_text_for_partial_reply("see [AB]"), "see [AB]");
    }

    #[test]
    fn finishing_only_touches_this_requests_rows() {
        let mut chat_conversation = ChatConversation::default();
        let question_id = chat_conversation.append(true, "hi".into(), false);
        let old_reply_with_text_id = chat_conversation.append(false, "half a rep".into(), true);
        let old_empty_placeholder_id = chat_conversation.append(false, String::new(), true);
        let next_request_placeholder_id = chat_conversation.append(false, String::new(), true);

        chat_conversation.finish_streaming(&[old_reply_with_text_id, old_empty_placeholder_id]);

        let remaining_ids: Vec<u64> = chat_conversation.messages().iter().map(|message| message.id).collect();
        assert_eq!(remaining_ids, vec![question_id, old_reply_with_text_id, next_request_placeholder_id]);
        assert!(!chat_conversation.messages()[1].is_streaming);
        assert!(chat_conversation.messages()[2].is_streaming);
    }

    #[test]
    fn update_and_remove_find_rows_by_id() {
        let mut chat_conversation = ChatConversation::default();
        let reply_id = chat_conversation.append(false, String::new(), true);
        chat_conversation.update(reply_id, "hello".into(), false);
        assert_eq!(chat_conversation.messages()[0].text, "hello");
        assert!(!chat_conversation.messages()[0].is_streaming);
        chat_conversation.remove(reply_id);
        assert!(chat_conversation.messages().is_empty());
        assert_ne!(chat_conversation.append(true, "a".into(), false), reply_id);
    }

    #[test]
    fn all_four_reply_combinations_match_the_mac() {
        // Spoken question, default: voice + text, chat stays as it is.
        assert_eq!(reply_routing(false, true, false), ReplyRouting { replies_are_spoken: true, opens_chat_without_switching_mode: false });
        // Spoken question, text only: the chat opens without leaving voice mode.
        assert_eq!(reply_routing(false, false, false), ReplyRouting { replies_are_spoken: false, opens_chat_without_switching_mode: true });
        // Typed question, default: text.
        assert_eq!(reply_routing(true, true, false), ReplyRouting { replies_are_spoken: false, opens_chat_without_switching_mode: false });
        // Typed question, text + voice.
        assert_eq!(reply_routing(true, false, true), ReplyRouting { replies_are_spoken: true, opens_chat_without_switching_mode: false });
    }

    #[test]
    fn chat_mode_hotkey_minimizes_an_open_focused_chat_and_otherwise_opens_it() {
        assert_eq!(chat_hotkey_action(true), ChatHotkeyAction::MinimizeChat);
        assert_eq!(chat_hotkey_action(false), ChatHotkeyAction::OpenChat);
    }

    #[test]
    fn model_label_is_the_capitalized_alias() {
        assert_eq!(chat_model_label(BrainEngine::Claude, "sonnet"), "Sonnet");
        assert_eq!(chat_model_label(BrainEngine::Claude, "fable"), "Fable");
        assert_eq!(chat_model_label(BrainEngine::Claude, ""), "");
        assert_eq!(chat_model_label(BrainEngine::Codex, "sonnet"), "Codex");
    }
}
