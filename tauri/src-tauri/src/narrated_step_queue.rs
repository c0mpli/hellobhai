//! Port of the Mac's `NarratedStepQueue` + `StreamedLineForwarder`: the board,
//! notes and 3D calls stream JSON lines; `{"t":"say"}` lines start a step, and a
//! step's lines are drawn while its sentence is spoken, in lockstep.

use std::future::Future;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::Value;

use crate::speech_player::SpeechPlayer;

/// Turns the growing accumulated reply into complete lines, each returned once.
#[derive(Default)]
pub struct StreamedLineForwarder {
    number_of_lines_already_forwarded: usize,
}

impl StreamedLineForwarder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Complete lines (ending in "\n") that appeared since the last call.
    pub fn new_complete_lines(&mut self, accumulated_text: &str) -> Vec<String> {
        let lines_so_far: Vec<&str> = accumulated_text.split('\n').collect();
        // The last element is still being written (no newline after it yet).
        let number_of_complete_lines = lines_so_far.len() - 1;
        self.take_lines_up_to(&lines_so_far, number_of_complete_lines)
    }

    /// Whatever is left once the stream ended (the last line may lack "\n").
    pub fn remaining_lines(&mut self, full_text: &str) -> Vec<String> {
        let all_lines: Vec<&str> = full_text.split('\n').collect();
        self.take_lines_up_to(&all_lines, all_lines.len())
    }

    fn take_lines_up_to(&mut self, lines: &[&str], line_count_limit: usize) -> Vec<String> {
        let mut new_lines = Vec::new();
        while self.number_of_lines_already_forwarded < line_count_limit {
            new_lines.push(lines[self.number_of_lines_already_forwarded].to_string());
            self.number_of_lines_already_forwarded += 1;
        }
        new_lines
    }
}

struct NarratedStep {
    narration: Option<String>,
    drawing_command_lines: Vec<String>,
}

struct NarratedStepQueueState {
    narrated_steps: Vec<NarratedStep>,
    is_stream_finished: bool,
    /// Index of the step currently being spoken, or -1 before playback starts.
    playing_step_index: isize,
}

/// Steps of narration + drawing lines, played in order. Cloneable handle; the
/// state lives behind an `Arc<Mutex<…>>` inside.
#[derive(Clone)]
pub struct NarratedStepQueue {
    shared_state: Arc<Mutex<NarratedStepQueueState>>,
    draw_command_line: Arc<dyn Fn(String) + Send + Sync>,
}

impl NarratedStepQueue {
    /// `draw_command_line` forwards one line to the board or 3D window.
    pub fn new(draw_command_line: Box<dyn Fn(String) + Send + Sync>) -> Self {
        NarratedStepQueue {
            shared_state: Arc::new(Mutex::new(NarratedStepQueueState {
                narrated_steps: Vec::new(),
                is_stream_finished: false,
                playing_step_index: -1,
            })),
            draw_command_line: Arc::from(draw_command_line),
        }
    }

    pub fn receive_line(&self, streamed_line: &str) {
        let trimmed_line = streamed_line.trim_matches(|character| character == ' ' || character == '\t');
        if !trimmed_line.starts_with('{') {
            return;
        }

        let should_draw_right_away = {
            let mut state = self.shared_state.lock().unwrap();
            if let Some(narration) = narration_text(trimmed_line) {
                state.narrated_steps.push(NarratedStep { narration: Some(narration), drawing_command_lines: Vec::new() });
                return;
            }
            // Lines before the first "say" (like the title) form a silent first step.
            if state.narrated_steps.is_empty() {
                state.narrated_steps.push(NarratedStep { narration: None, drawing_command_lines: Vec::new() });
            }
            let last_step_index = state.narrated_steps.len() - 1;
            state.narrated_steps[last_step_index].drawing_command_lines.push(trimmed_line.to_string());
            // Shapes for the step that's already playing go straight to the board.
            (last_step_index as isize) <= state.playing_step_index
        };
        if should_draw_right_away {
            (self.draw_command_line)(trimmed_line.to_string());
        }
    }

    pub fn mark_stream_finished(&self) {
        self.shared_state.lock().unwrap().is_stream_finished = true;
    }

    /// Waits until the voice is free, then for each step: draw its lines, speak and
    /// await; silent steps draw then sleep 300 ms; polls 60 ms for new steps.
    /// Returns the narration sentences spoken.
    pub async fn play_steps(&self, speech_player: Arc<SpeechPlayer>) -> Vec<String> {
        let speech_player_for_waiting = speech_player.clone();
        self.play_steps_with(
            move || {
                let speech_player = speech_player_for_waiting.clone();
                async move {
                    // Let the intro sentence finish before the first narrated step.
                    while speech_player.is_speaking() {
                        tokio::time::sleep(Duration::from_millis(80)).await;
                    }
                }
            },
            move |narration| {
                let speech_player = speech_player.clone();
                async move { speech_player.speak_text_and_wait_until_finished(narration).await }
            },
        )
        .await
    }

    /// `play_steps` with the voice passed in, so the ordering can be tested without audio.
    /// Cancelling = dropping this future (the owning task is aborted).
    async fn play_steps_with<WaitFuture, SpeakFuture>(
        &self,
        wait_until_voice_is_free: impl Fn() -> WaitFuture,
        speak: impl Fn(String) -> SpeakFuture,
    ) -> Vec<String>
    where
        WaitFuture: Future<Output = ()>,
        SpeakFuture: Future<Output = ()>,
    {
        let mut spoken_narrations = Vec::new();
        let mut next_step_index = 0usize;

        loop {
            let next_step_narration = {
                let state = self.shared_state.lock().unwrap();
                if next_step_index >= state.narrated_steps.len() {
                    if state.is_stream_finished {
                        break;
                    }
                    None
                } else {
                    Some(state.narrated_steps[next_step_index].narration.clone())
                }
            };
            let Some(narration) = next_step_narration else {
                tokio::time::sleep(Duration::from_millis(60)).await;
                continue;
            };

            if let Some(narration) = narration {
                wait_until_voice_is_free().await;
                for drawing_command_line in self.start_playing_step(next_step_index) {
                    (self.draw_command_line)(drawing_command_line);
                }
                spoken_narrations.push(narration.clone());
                speak(narration).await;
            } else {
                for drawing_command_line in self.start_playing_step(next_step_index) {
                    (self.draw_command_line)(drawing_command_line);
                }
                tokio::time::sleep(Duration::from_millis(300)).await;
            }
            next_step_index += 1;
        }
        spoken_narrations
    }

    /// Marks the step as playing (so later lines for it draw at once) and returns its lines so far.
    fn start_playing_step(&self, step_index: usize) -> Vec<String> {
        let mut state = self.shared_state.lock().unwrap();
        state.playing_step_index = step_index as isize;
        state.narrated_steps[step_index].drawing_command_lines.clone()
    }

    /// Exact Mac markdown-ish rendering of the notes lines shown so far (for history).
    pub fn plain_text_of_visible_lines(&self) -> String {
        let state = self.shared_state.lock().unwrap();
        let plain_lines: Vec<String> = state
            .narrated_steps
            .iter()
            .flat_map(|narrated_step| narrated_step.drawing_command_lines.iter())
            .filter_map(|json_line| plain_text_of_notes_line(json_line))
            .collect();
        plain_lines.join("\n")
    }
}

fn plain_text_of_notes_line(json_line: &str) -> Option<String> {
    let line_object: Value = serde_json::from_str(json_line).ok()?;
    let line_type = line_object["t"].as_str()?;
    let plain_line_text = line_object["text"].as_str().unwrap_or("").replace("==", "");
    match line_type {
        "title" => Some(format!("# {plain_line_text}")),
        "heading" => Some(format!("## {plain_line_text}")),
        "bullet" => {
            let bullet_prefix = if line_object["level"].as_f64() == Some(2.0) { "    - " } else { "- " };
            Some(format!("{bullet_prefix}{plain_line_text}"))
        }
        "analogy" => Some(format!("Think of it like: {plain_line_text}")),
        "remember" => Some(format!("Remember: {plain_line_text}")),
        "todo" => Some(format!("[ ] {plain_line_text}")),
        "quote" => Some(format!("> {plain_line_text}")),
        "code" => Some(format!("```\n{plain_line_text}\n```")),
        "table" => {
            let table_columns: Vec<String> = line_object["columns"]
                .as_array()
                .map(|columns| columns.iter().filter_map(|column| column.as_str().map(str::to_string)).collect())
                .unwrap_or_default();
            let mut table_lines = vec![table_columns.join(" | ")];
            if let Some(table_rows) = line_object["rows"].as_array() {
                for table_row in table_rows {
                    let cells: Vec<String> = table_row
                        .as_array()
                        .map(|row_cells| row_cells.iter().map(table_cell_text).collect())
                        .unwrap_or_default();
                    table_lines.push(cells.join(" | "));
                }
            }
            Some(table_lines.join("\n"))
        }
        _ => None,
    }
}

/// Strings as they are (Swift's "\(value)"), numbers and others as JSON text.
fn table_cell_text(cell: &Value) -> String {
    match cell {
        Value::String(cell_text) => cell_text.clone(),
        other_value => other_value.to_string(),
    }
}

fn narration_text(json_line: &str) -> Option<String> {
    let line_object: Value = serde_json::from_str(json_line).ok()?;
    if line_object["t"] != "say" {
        return None;
    }
    line_object["text"].as_str().map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forwarder_returns_each_complete_line_once_and_the_tail_at_the_end() {
        let mut streamed_line_forwarder = StreamedLineForwarder::new();
        assert!(streamed_line_forwarder.new_complete_lines("{\"t\":\"ti").is_empty());
        assert_eq!(streamed_line_forwarder.new_complete_lines("{\"t\":\"title\"}\n{\"t\":"), vec!["{\"t\":\"title\"}"]);
        assert!(streamed_line_forwarder.new_complete_lines("{\"t\":\"title\"}\n{\"t\":").is_empty());
        assert_eq!(streamed_line_forwarder.remaining_lines("{\"t\":\"title\"}\n{\"t\":\"box\"}"), vec!["{\"t\":\"box\"}"]);
        assert!(streamed_line_forwarder.remaining_lines("{\"t\":\"title\"}\n{\"t\":\"box\"}").is_empty());
    }

    #[tokio::test]
    async fn steps_play_in_lockstep_with_the_silent_title_step_first() {
        let drawn_lines = Arc::new(Mutex::new(Vec::<String>::new()));
        let drawn_lines_for_queue = drawn_lines.clone();
        let narrated_step_queue = NarratedStepQueue::new(Box::new(move |line| drawn_lines_for_queue.lock().unwrap().push(line)));

        narrated_step_queue.receive_line(r#"{"t":"title","text":"DNS"}"#);
        narrated_step_queue.receive_line("not json, ignored");
        narrated_step_queue.receive_line(r#"  {"t":"say","text":"first you ask."}"#);
        narrated_step_queue.receive_line(r#"{"t":"box","id":"a"}"#);
        // Nothing is drawn before playback starts.
        assert!(drawn_lines.lock().unwrap().is_empty());

        let queue_for_speaking = narrated_step_queue.clone();
        let drawn_lines_when_speaking = drawn_lines.clone();
        let events_while_speaking = Arc::new(Mutex::new(Vec::<String>::new()));
        let events_for_speak = events_while_speaking.clone();
        let first_narration_flag = Arc::new(Mutex::new(true));

        let spoken_narrations = narrated_step_queue
            .play_steps_with(
                || async {},
                |narration| {
                    let queue_for_speaking = queue_for_speaking.clone();
                    let drawn_lines_when_speaking = drawn_lines_when_speaking.clone();
                    let events_for_speak = events_for_speak.clone();
                    let first_narration_flag = first_narration_flag.clone();
                    async move {
                        events_for_speak
                            .lock()
                            .unwrap()
                            .push(format!("{narration} drawn={}", drawn_lines_when_speaking.lock().unwrap().len()));
                        // A line that streams in for the step being spoken goes straight to the board;
                        // lines after the next "say" wait for that step.
                        let is_first_narration = std::mem::replace(&mut *first_narration_flag.lock().unwrap(), false);
                        if is_first_narration {
                            queue_for_speaking.receive_line(r#"{"t":"arrow","from":"a"}"#);
                            assert_eq!(drawn_lines_when_speaking.lock().unwrap().len(), 3);
                            queue_for_speaking.receive_line(r#"{"t":"say","text":"then it answers."}"#);
                            queue_for_speaking.receive_line(r#"{"t":"box","id":"b"}"#);
                            assert_eq!(drawn_lines_when_speaking.lock().unwrap().len(), 3);
                            queue_for_speaking.mark_stream_finished();
                        }
                    }
                },
            )
            .await;

        assert_eq!(spoken_narrations, vec!["first you ask.", "then it answers."]);
        assert_eq!(
            *events_while_speaking.lock().unwrap(),
            vec!["first you ask. drawn=2".to_string(), "then it answers. drawn=4".to_string()]
        );
        assert_eq!(
            *drawn_lines.lock().unwrap(),
            vec![
                r#"{"t":"title","text":"DNS"}"#,
                r#"{"t":"box","id":"a"}"#,
                r#"{"t":"arrow","from":"a"}"#,
                r#"{"t":"box","id":"b"}"#,
            ]
        );
    }

    #[test]
    fn notes_lines_render_as_plain_text_like_the_mac() {
        let narrated_step_queue = NarratedStepQueue::new(Box::new(|_| {}));
        for notes_line in [
            r#"{"t":"title","text":"Photosynthesis"}"#,
            r#"{"t":"say","text":"plants make food."}"#,
            r#"{"t":"heading","text":"Inputs"}"#,
            r#"{"t":"bullet","text":"needs ==sunlight=="}"#,
            r#"{"t":"bullet","text":"and water","level":2}"#,
            r#"{"t":"remember","text":"chlorophyll is green"}"#,
            r#"{"t":"todo","text":"revise"}"#,
            r#"{"t":"quote","text":"life"}"#,
            r#"{"t":"code","text":"x = 1"}"#,
            r#"{"t":"table","columns":["a","b"],"rows":[["1",2]]}"#,
            r#"{"t":"chart","kind":"bar"}"#,
        ] {
            narrated_step_queue.receive_line(notes_line);
        }
        assert_eq!(
            narrated_step_queue.plain_text_of_visible_lines(),
            "# Photosynthesis\n## Inputs\n- needs sunlight\n    - and water\nRemember: chlorophyll is green\n[ ] revise\n> life\n```\nx = 1\n```\na | b\n1 | 2"
        );
    }
}
