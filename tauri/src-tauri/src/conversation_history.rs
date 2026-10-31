//! The last 20 exchanges (agent rounds included, with action summaries), folded
//! into each prompt by `stream_json::fold_history_into_prompt`.

/// Agent rounds add up fast, so this keeps a bit more than a plain chat would need (same as the Mac).
const MAXIMUM_HISTORY_ENTRIES: usize = 20;

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
        self.entries.push(entry);
        if self.entries.len() > MAXIMUM_HISTORY_ENTRIES {
            let number_of_entries_to_drop = self.entries.len() - MAXIMUM_HISTORY_ENTRIES;
            self.entries.drain(..number_of_entries_to_drop);
        }
    }

    pub fn entries(&self) -> Vec<ConversationEntry> {
        self.entries.clone()
    }

    /// Adds narration (and notes text) of a board/notes/3D panel to the last reply.
    pub fn append_to_last_response(&mut self, extra: &str) {
        if let Some(last_entry) = self.entries.last_mut() {
            last_entry.assistant_response.push_str(extra);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_only_the_last_twenty_entries_and_appends_to_the_last_reply() {
        let mut conversation_history = ConversationHistory::new();
        conversation_history.append_to_last_response(" ignored when empty");
        for entry_number in 1..=25 {
            conversation_history.append(ConversationEntry {
                user_text: format!("question {entry_number}"),
                assistant_response: format!("answer {entry_number}"),
            });
        }
        let entries = conversation_history.entries();
        assert_eq!(entries.len(), 20);
        assert_eq!(entries[0].user_text, "question 6");
        assert_eq!(entries[19].user_text, "question 25");

        conversation_history.append_to_last_response(" and a drawing");
        assert_eq!(conversation_history.entries()[19].assistant_response, "answer 25 and a drawing");
    }
}
