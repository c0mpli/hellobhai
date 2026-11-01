//! The typed chat's conversation and reply rules, ports of the Mac's
//! `CompanionChatMessage`, `chatDisplayText`, `finishStreamingChatMessages` and
//! the reply combinations in `sendTranscriptToClaudeWithScreenshot`. Pure logic,
//! no windows, so it is unit-tested here; `chat_panel.rs` shows it.

use crate::brain_engine::BrainEngine;
use crate::conversation_history::ConversationHistory;
use std::collections::HashMap;
use std::ops::Range;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::LazyLock;

use regex::Regex;
use serde::{Deserialize, Serialize};

/// Message ids are unique across every tab: a cancelled request's cleanup can
/// run after a tab switch and must not touch the new tab's rows.
static NEXT_CHAT_MESSAGE_ID: AtomicU64 = AtomicU64::new(1);

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
}

impl ChatConversation {
    pub fn messages(&self) -> &[ChatMessage] {
        &self.messages
    }

    /// Adds a row and returns its id.
    pub fn append(&mut self, is_from_user: bool, text: String, is_streaming: bool) -> u64 {
        let message_id = NEXT_CHAT_MESSAGE_ID.fetch_add(1, Ordering::Relaxed);
        self.messages.push(ChatMessage { id: message_id, is_from_user, text, is_streaming });
        message_id
    }

    pub fn update(&mut self, message_id: u64, text: String, is_streaming: bool) {
        if let Some(message) = self.messages.iter_mut().find(|message| message.id == message_id) {
            message.text = text;
            message.is_streaming = is_streaming;
        }
    }

    pub fn message_text(&self, message_id: u64) -> Option<&str> {
        self.messages.iter().find(|message| message.id == message_id).map(|message| message.text.as_str())
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

pub const NEW_CHAT_TAB_TITLE: &str = "New chat";
const MAXIMUM_CHAT_TAB_TITLE_CHARACTERS: usize = 24;

/// A tab is named after its first question, on one line and cut to fit a tab
/// (Mac `ChatTab.title(forChatMessages:)`).
pub fn chat_tab_title(chat_messages: &[ChatMessage]) -> String {
    let Some(first_question) = chat_messages.iter().find(|message| message.is_from_user) else {
        return NEW_CHAT_TAB_TITLE.to_string();
    };
    let single_line_question = first_question.text.lines().filter(|line| !line.is_empty()).collect::<Vec<_>>().join(" ");
    if single_line_question.chars().count() > MAXIMUM_CHAT_TAB_TITLE_CHARACTERS {
        single_line_question.chars().take(MAXIMUM_CHAT_TAB_TITLE_CHARACTERS).collect::<String>() + "..."
    } else {
        single_line_question
    }
}

/// One tab in the strip under the chat header, as the page draws it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatTabSummary {
    pub id: u64,
    pub title: String,
}

/// A tab that isn't shown: its rows and the history sent to the brain.
#[derive(Default)]
struct ParkedChatTab {
    conversation: ChatConversation,
    conversation_history: ConversationHistory,
}

/// The chat's tabs, like browser tabs (Mac `chatTabs` and
/// `parkedChatTabConversations`). The active tab's rows and history live with
/// their owners (`ChatPanel` and the brain); the other tabs are parked here.
pub struct ChatTabs {
    tab_ids_in_order: Vec<u64>,
    active_tab_id: u64,
    parked_tabs: HashMap<u64, ParkedChatTab>,
    next_tab_id: u64,
}

impl Default for ChatTabs {
    fn default() -> Self {
        ChatTabs { tab_ids_in_order: vec![1], active_tab_id: 1, parked_tabs: HashMap::new(), next_tab_id: 2 }
    }
}

impl ChatTabs {
    pub fn active_tab_id(&self) -> u64 {
        self.active_tab_id
    }

    pub fn tab_count(&self) -> usize {
        self.tab_ids_in_order.len()
    }

    pub fn contains_tab(&self, tab_id: u64) -> bool {
        self.tab_ids_in_order.contains(&tab_id)
    }

    /// Every tab in order, titled from the active rows or the parked ones.
    pub fn summaries(&self, active_conversation: &ChatConversation) -> Vec<ChatTabSummary> {
        self.tab_ids_in_order
            .iter()
            .map(|&tab_id| {
                let tab_messages = if tab_id == self.active_tab_id {
                    active_conversation.messages()
                } else {
                    self.parked_tabs.get(&tab_id).map(|parked_tab| parked_tab.conversation.messages()).unwrap_or(&[])
                };
                ChatTabSummary { id: tab_id, title: chat_tab_title(tab_messages) }
            })
            .collect()
    }

    /// Adds an empty tab after the active one and returns its id (not switched to yet).
    pub fn insert_empty_tab_after_active(&mut self) -> u64 {
        let new_tab_id = self.next_tab_id;
        self.next_tab_id += 1;
        let active_tab_index = self
            .tab_ids_in_order
            .iter()
            .position(|&tab_id| tab_id == self.active_tab_id)
            .unwrap_or(self.tab_ids_in_order.len() - 1);
        self.tab_ids_in_order.insert(active_tab_index + 1, new_tab_id);
        self.parked_tabs.insert(new_tab_id, ParkedChatTab::default());
        new_tab_id
    }

    /// Parks the active tab's rows and history and swaps in the target tab's.
    /// Returns false (nothing changed) for the active tab or an unknown one.
    pub fn switch_to_tab(
        &mut self,
        target_tab_id: u64,
        active_conversation: &mut ChatConversation,
        active_conversation_history: &mut ConversationHistory,
    ) -> bool {
        if target_tab_id == self.active_tab_id {
            return false;
        }
        let Some(target_tab) = self.parked_tabs.remove(&target_tab_id) else { return false };
        self.parked_tabs.insert(
            self.active_tab_id,
            ParkedChatTab {
                conversation: std::mem::replace(active_conversation, target_tab.conversation),
                conversation_history: std::mem::replace(active_conversation_history, target_tab.conversation_history),
            },
        );
        self.active_tab_id = target_tab_id;
        true
    }

    /// The tab to show when the active one closes: the next one, else the previous one.
    pub fn neighbour_of_active_tab(&self) -> Option<u64> {
        let active_tab_index = self.tab_ids_in_order.iter().position(|&tab_id| tab_id == self.active_tab_id)?;
        if let Some(next_tab_id) = self.tab_ids_in_order.get(active_tab_index + 1) {
            return Some(*next_tab_id);
        }
        let previous_tab_index = active_tab_index.checked_sub(1)?;
        self.tab_ids_in_order.get(previous_tab_index).copied()
    }

    /// Removes a tab that isn't active (switch away from it first).
    pub fn remove_parked_tab(&mut self, tab_id: u64) {
        if tab_id == self.active_tab_id {
            return;
        }
        self.tab_ids_in_order.retain(|&existing_tab_id| existing_tab_id != tab_id);
        self.parked_tabs.remove(&tab_id);
    }

    /// A cancelled request's rows may have been parked by a tab switch.
    pub fn finish_streaming_in_parked_tabs(&mut self, message_ids: &[u64]) {
        for parked_tab in self.parked_tabs.values_mut() {
            parked_tab.conversation.finish_streaming(message_ids);
        }
    }
}

/// Text size for chat messages and the composer (Mac `ChatTextSize`),
/// saved as `chatTextSize`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ChatTextSize {
    Small,
    #[default]
    Medium,
    Large,
    ExtraLarge,
}

impl ChatTextSize {
    pub fn point_size(self) -> u32 {
        match self {
            ChatTextSize::Small => 12,
            ChatTextSize::Medium => 13,
            ChatTextSize::Large => 15,
            ChatTextSize::ExtraLarge => 17,
        }
    }
}

/// Where a spoken sentence sits in a chat reply: the first match at or after
/// `search_start_byte_offset` (so a repeated sentence lights the right
/// occurrence), as a byte range. Port of the Mac's `advanceChatSpeechProgress`.
pub fn find_spoken_sentence_byte_range(message_text: &str, spoken_sentence: &str, search_start_byte_offset: usize) -> Option<Range<usize>> {
    let trimmed_sentence = spoken_sentence.trim();
    if trimmed_sentence.is_empty() {
        return None;
    }
    let text_after_search_start = message_text.get(search_start_byte_offset..)?;
    let match_start_in_rest = text_after_search_start.find(trimmed_sentence)?;
    let sentence_start_byte_offset = search_start_byte_offset + match_start_in_rest;
    Some(sentence_start_byte_offset..sentence_start_byte_offset + trimmed_sentence.len())
}

/// A byte offset as a UTF-16 offset, which is how the chat page's JavaScript
/// strings count (for `String.prototype.slice`).
pub fn utf16_offset_for_byte_offset(text: &str, byte_offset: usize) -> usize {
    text[..byte_offset].encode_utf16().count()
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
    fn tab_titles_come_from_the_first_question_cut_to_24_characters() {
        let mut chat_conversation = ChatConversation::default();
        assert_eq!(chat_tab_title(chat_conversation.messages()), "New chat");
        chat_conversation.append(false, "a reply first".into(), false);
        assert_eq!(chat_tab_title(chat_conversation.messages()), "New chat");
        chat_conversation.append(true, "what is\nthis".into(), false);
        assert_eq!(chat_tab_title(chat_conversation.messages()), "what is this");
        let mut long_conversation = ChatConversation::default();
        long_conversation.append(true, "explain how dns resolution works step by step".into(), false);
        assert_eq!(chat_tab_title(long_conversation.messages()), "explain how dns resoluti...");
    }

    #[test]
    fn tabs_park_rows_and_history_and_close_to_a_neighbour() {
        let mut chat_tabs = ChatTabs::default();
        let mut active_conversation = ChatConversation::default();
        let mut active_history = ConversationHistory::default();
        let first_tab_id = chat_tabs.active_tab_id();
        active_conversation.append(true, "first question".into(), false);
        active_history.append(crate::conversation_history::ConversationEntry { user_text: "q".into(), assistant_response: "a".into() });

        let second_tab_id = chat_tabs.insert_empty_tab_after_active();
        assert!(chat_tabs.switch_to_tab(second_tab_id, &mut active_conversation, &mut active_history));
        assert!(active_conversation.messages().is_empty());
        assert!(active_history.entries().is_empty());
        let titles: Vec<String> = chat_tabs.summaries(&active_conversation).into_iter().map(|summary| summary.title).collect();
        assert_eq!(titles, vec!["first question", "New chat"]);

        assert_eq!(chat_tabs.neighbour_of_active_tab(), Some(first_tab_id));
        assert!(chat_tabs.switch_to_tab(first_tab_id, &mut active_conversation, &mut active_history));
        assert_eq!(active_history.entries().len(), 1);
        chat_tabs.remove_parked_tab(second_tab_id);
        assert_eq!(chat_tabs.tab_count(), 1);
        assert!(!chat_tabs.switch_to_tab(second_tab_id, &mut active_conversation, &mut active_history));
    }

    #[test]
    fn spoken_sentences_are_found_in_order_even_when_repeated() {
        let reply = "Yes. Click save. Yes.";
        let first_match = find_spoken_sentence_byte_range(reply, "Yes.", 0).unwrap();
        assert_eq!(first_match, 0..4);
        let second_match = find_spoken_sentence_byte_range(reply, " Click save. ", first_match.end).unwrap();
        assert_eq!(&reply[second_match.clone()], "Click save.");
        let third_match = find_spoken_sentence_byte_range(reply, "Yes.", second_match.end).unwrap();
        assert_eq!(third_match, 17..21);
        assert_eq!(find_spoken_sentence_byte_range(reply, "Nope.", 0), None);
        assert_eq!(find_spoken_sentence_byte_range(reply, "  ", 0), None);
        assert_eq!(find_spoken_sentence_byte_range(reply, "Yes.", 99), None);
        // The page slices JavaScript strings, which count UTF-16 units.
        let hindi_reply = "हाँ भाई। ठीक है।";
        let hindi_match = find_spoken_sentence_byte_range(hindi_reply, "ठीक है।", 0).unwrap();
        assert_eq!(utf16_offset_for_byte_offset(hindi_reply, hindi_match.start), "हाँ भाई। ".encode_utf16().count());
    }

    #[test]
    fn text_sizes_match_the_mac_and_unknown_names_are_rejected() {
        assert_eq!(ChatTextSize::default(), ChatTextSize::Medium);
        let point_sizes = [ChatTextSize::Small, ChatTextSize::Medium, ChatTextSize::Large, ChatTextSize::ExtraLarge].map(ChatTextSize::point_size);
        assert_eq!(point_sizes, [12, 13, 15, 17]);
        assert_eq!(serde_json::from_str::<ChatTextSize>(r#""extraLarge""#).unwrap(), ChatTextSize::ExtraLarge);
        assert!(serde_json::from_str::<ChatTextSize>(r#""huge""#).is_err());
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
