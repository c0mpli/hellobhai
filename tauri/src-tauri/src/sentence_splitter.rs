//! Splits a reply into sentences so speech can start after the first one and
//! the next is synthesized while the current one plays.

/// Splits after . ! ? । ॥ followed by whitespace or the end; trims; drops empties.
pub fn split_into_sentences(text: &str) -> Vec<String> {
    todo!("B: sentence splitting")
}
