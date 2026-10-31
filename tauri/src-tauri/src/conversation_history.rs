//! The last 20 exchanges (agent rounds included, with action summaries), folded
//! into each prompt by `stream_json::fold_history_into_prompt`.

#[derive(Clone, Debug, PartialEq)]
pub struct ConversationEntry {
    pub user_text: String,
    pub assistant_response: String,
}

#[derive(Default)]
pub struct ConversationHistory {
    entries: Vec<ConversationEntry>,
}

impl ConversationHistory {
    pub fn new() -> Self {
        Self::default()
    }

    /// Appends and keeps only the last 20 entries.
    pub fn append(&mut self, entry: ConversationEntry) {
        todo!("E: append, cap at 20")
    }

    pub fn entries(&self) -> Vec<ConversationEntry> {
        todo!("E: clone the entries")
    }

    /// Adds narration (and notes text) of a board/notes/3D panel to the last reply.
    pub fn append_to_last_response(&mut self, extra: &str) {
        todo!("E: append to the last entry's assistant_response")
    }
}
