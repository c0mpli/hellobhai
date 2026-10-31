//! System prompts (exported from the Mac app into `tauri/generated/prompts/`
//! by `scripts/export-mac-web-assets-for-tauri.sh`, never edited by hand) and
//! the small per-request prompt texts built in Swift on the Mac.

use std::path::{Path, PathBuf};

use crate::board_panel::LiveContentKind;
use crate::screen_mapping::PhysicalRect;

pub const VOICE_RESPONSE_SYSTEM_PROMPT: &str = include_str!("../../generated/prompts/voice_response.txt");
pub const VISUAL_EXPLANATION_SYSTEM_PROMPT: &str = include_str!("../../generated/prompts/visual_explanation.txt");
pub const LIVE_NOTES_SYSTEM_PROMPT: &str = include_str!("../../generated/prompts/live_notes.txt");
pub const MODEL3D_SYSTEM_PROMPT: &str = include_str!("../../generated/prompts/model3d.txt");
pub const ONBOARDING_DEMO_SYSTEM_PROMPT: &str = include_str!("../../generated/prompts/onboarding_demo.txt");
/// Sent instead of a transcript on every agent round after the first.
pub const AGENT_CONTINUATION_PROMPT: &str = include_str!("../../generated/prompts/agent_continuation.txt");

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SystemPromptKind {
    VoiceResponse,
    VisualExplanation,
    LiveNotes,
    Model3D,
    OnboardingDemo,
}

/// Writes every system prompt to its own file so claude gets `--system-prompt-file`.
pub fn write_prompt_files(prompts_directory: &Path) -> std::io::Result<()> {
    todo!("E: write each *_SYSTEM_PROMPT to prompt_file_path")
}

pub fn prompt_file_path(prompts_directory: &Path, kind: SystemPromptKind) -> PathBuf {
    todo!("E: one file name per kind")
}

/// Exact Mac strings: tells Claude to reply in English when the transcript has no Devanagari.
pub fn reply_language_note(transcript: &str) -> &'static str {
    todo!("E: copy the Mac language note")
}

/// capture label + " (image dimensions: WxH pixels)"
pub fn image_label_with_dimensions(capture_label: &str, width: u32, height: u32) -> String {
    todo!("E: copy the Mac label format")
}

/// Exact Mac text describing the region the user drew around (screen number + screenshot pixels).
pub fn user_circled_region_note(screen_number: usize, rect: PhysicalRect) -> String {
    todo!("E: copy the Mac region note")
}

pub fn user_circled_region_crop_label(screen_number: usize) -> String {
    todo!("E: copy the Mac crop label")
}

/// The user prompt for the second claude call that writes a board or notes page.
pub fn narrated_panel_request_prompt(
    kind: LiveContentKind,
    description: &str,
    user_request: &str,
    spoken_intro: &str,
) -> String {
    todo!("E: copy startNarratedPanel's prompt")
}

/// The user prompt for the second claude call that builds a 3D model.
#[allow(clippy::too_many_arguments)]
pub fn model3d_request_prompt(
    description: &str,
    capture_label: &str,
    width: u32,
    height: u32,
    focus_x: f64,
    focus_y: f64,
    user_request: &str,
    spoken_intro: &str,
) -> String {
    todo!("E: copy startNarratedModel3D's prompt")
}
