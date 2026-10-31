//! Splits a reply into sentences so speech can start after the first one and
//! the next is synthesized while the current one plays.

fn is_sentence_terminator(character: char) -> bool {
    matches!(character, '.' | '!' | '?' | '।' | '॥')
}

/// Splits after . ! ? । ॥ followed by whitespace or the end; trims; drops empties.
/// Line breaks also end a sentence (the Mac's NLTokenizer does the same), so
/// list items without punctuation are spoken one by one.
// ponytail: "e.g. this" splits after "e.g."; harmless for speech, add an abbreviation list if it ever sounds off.
pub fn split_into_sentences(text: &str) -> Vec<String> {
    let mut sentences = Vec::new();
    let mut current_sentence = String::new();
    let mut characters = text.chars().peekable();
    while let Some(character) = characters.next() {
        current_sentence.push(character);
        let next_character = characters.peek().copied();
        let ends_after_terminator = is_sentence_terminator(character)
            && next_character.is_none_or(|next| next.is_whitespace());
        if ends_after_terminator || character == '\n' {
            sentences.push(std::mem::take(&mut current_sentence));
        }
    }
    sentences.push(current_sentence);
    sentences
        .into_iter()
        .map(|sentence| sentence.trim().to_string())
        .filter(|sentence| !sentence.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_english_sentences() {
        assert_eq!(
            split_into_sentences("Hi there. Pi is 3.14, right?  Yes!"),
            vec!["Hi there.", "Pi is 3.14, right?", "Yes!"]
        );
        assert_eq!(split_into_sentences("Wait... really?! ok"), vec!["Wait...", "really?!", "ok"]);
    }

    #[test]
    fn splits_hindi_and_mixed_sentences() {
        assert_eq!(
            split_into_sentences("नमस्ते, मैं भाई हूँ। शॉर्टकट दबाकर कुछ भी पूछो।"),
            vec!["नमस्ते, मैं भाई हूँ।", "शॉर्टकट दबाकर कुछ भी पूछो।"]
        );
        assert_eq!(
            split_into_sentences("ये settings खोलो। Then click Save.\nfirst item\n\nsecond item"),
            vec!["ये settings खोलो।", "Then click Save.", "first item", "second item"]
        );
    }

    #[test]
    fn empty_text_has_no_sentences() {
        assert!(split_into_sentences("  \n ").is_empty());
    }
}
