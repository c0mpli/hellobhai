//! Port of the Mac's `NarratedStepQueue` + `StreamedLineForwarder`: the board,
//! notes and 3D calls stream JSON lines; `{"t":"say"}` lines start a step, and a
//! step's lines are drawn while its sentence is spoken, in lockstep.

use std::sync::Arc;

use crate::speech_player::SpeechPlayer;

/// Turns the growing accumulated reply into complete lines, each returned once.
#[derive(Default)]
pub struct StreamedLineForwarder {
    number_of_characters_already_forwarded: usize,
}

impl StreamedLineForwarder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Complete lines (ending in "\n") that appeared since the last call.
    pub fn new_complete_lines(&mut self, accumulated_text: &str) -> Vec<String> {
        todo!("E: port StreamedLineForwarder")
    }

    /// Whatever is left once the stream ended (the last line may lack "\n").
    pub fn remaining_lines(&mut self, full_text: &str) -> Vec<String> {
        todo!("E: port StreamedLineForwarder")
    }
}

/// Steps of narration + drawing lines, played in order. Cloneable handle; the
/// state lives behind an `Arc<Mutex<…>>` inside.
pub struct NarratedStepQueue {}

impl NarratedStepQueue {
    /// `draw_command_line` forwards one line to the board or 3D window.
    pub fn new(draw_command_line: Box<dyn Fn(String) + Send + Sync>) -> Self {
        todo!("E: port NarratedStepQueue")
    }

    pub fn receive_line(&self, streamed_line: &str) {
        todo!("E: port NarratedStepQueue")
    }

    pub fn mark_stream_finished(&self) {
        todo!("E: port NarratedStepQueue")
    }

    /// Waits until the voice is free, then for each step: draw its lines, speak and
    /// await; silent steps draw then sleep 300 ms; polls 60 ms for new steps.
    /// Returns the narration sentences spoken.
    pub async fn play_steps(&self, speech_player: Arc<SpeechPlayer>) -> Vec<String> {
        todo!("E: port NarratedStepQueue")
    }

    /// Exact Mac markdown-ish rendering of the notes lines shown so far (for history).
    pub fn plain_text_of_visible_lines(&self) -> String {
        todo!("E: port NarratedStepQueue")
    }
}
