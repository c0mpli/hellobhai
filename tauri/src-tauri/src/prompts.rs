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

const ALL_SYSTEM_PROMPT_KINDS: [SystemPromptKind; 5] = [
    SystemPromptKind::VoiceResponse,
    SystemPromptKind::VisualExplanation,
    SystemPromptKind::LiveNotes,
    SystemPromptKind::Model3D,
    SystemPromptKind::OnboardingDemo,
];

fn system_prompt_text(kind: SystemPromptKind) -> &'static str {
    match kind {
        SystemPromptKind::VoiceResponse => VOICE_RESPONSE_SYSTEM_PROMPT,
        SystemPromptKind::VisualExplanation => VISUAL_EXPLANATION_SYSTEM_PROMPT,
        SystemPromptKind::LiveNotes => LIVE_NOTES_SYSTEM_PROMPT,
        SystemPromptKind::Model3D => MODEL3D_SYSTEM_PROMPT,
        SystemPromptKind::OnboardingDemo => ONBOARDING_DEMO_SYSTEM_PROMPT,
    }
}

/// Writes every system prompt to its own file so claude gets `--system-prompt-file`.
pub fn write_prompt_files(prompts_directory: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(prompts_directory)?;
    for kind in ALL_SYSTEM_PROMPT_KINDS {
        std::fs::write(prompt_file_path(prompts_directory, kind), prompt_file_text(kind))?;
    }
    Ok(())
}

/// The prompt as written to its file: Linux swaps the Windows wording for its own.
fn prompt_file_text(kind: SystemPromptKind) -> String {
    #[cfg(target_os = "linux")]
    {
        let prompt_file_name = prompt_file_path(Path::new(""), kind);
        crate::platform::linux_prompt_wording::adapt_prompt_for_linux(&prompt_file_name.to_string_lossy(), system_prompt_text(kind))
    }
    #[cfg(not(target_os = "linux"))]
    {
        system_prompt_text(kind).to_string()
    }
}

pub fn prompt_file_path(prompts_directory: &Path, kind: SystemPromptKind) -> PathBuf {
    let file_name = match kind {
        SystemPromptKind::VoiceResponse => "voice_response.txt",
        SystemPromptKind::VisualExplanation => "visual_explanation.txt",
        SystemPromptKind::LiveNotes => "live_notes.txt",
        SystemPromptKind::Model3D => "model3d.txt",
        SystemPromptKind::OnboardingDemo => "onboarding_demo.txt",
    };
    prompts_directory.join(file_name)
}

/// Exact Mac strings. Whisper writes Hindi in Devanagari, so a transcript without
/// any Devanagari means the user spoke English; saying so stops Claude carrying on
/// in Hindi just because earlier turns were Hindi.
pub fn reply_language_note(transcript: &str) -> &'static str {
    let transcript_contains_devanagari = transcript.chars().any(|character| ('\u{0900}'..='\u{097F}').contains(&character));
    if transcript_contains_devanagari {
        "(the user spoke hindi or hinglish this turn: reply and narrate in natural hinglish written in devanagari.)"
    } else {
        "(the user spoke english this turn: reply and narrate in english only, even if earlier turns were in hindi.)"
    }
}

/// Exact Mac string (`typedReplyLanguageNote`). Typed Hinglish is usually in Latin
/// letters, so the Devanagari check can't tell it from English; Claude reads the
/// language itself, and the script depends on whether the reply is read aloud.
pub fn typed_reply_language_note(replies_are_spoken: bool) -> String {
    let hinglish_script = if replies_are_spoken {
        "natural hinglish written in devanagari, because the reply is read aloud"
    } else {
        "natural hinglish in the same script the user typed in"
    };
    format!("(the user typed this turn instead of speaking. reply in english if they wrote english; if they wrote hindi or hinglish in any script, reply in {hinglish_script}. the reply is shown in a chat panel, so when something needs the user's yes, ask in the reply and wait for their next message. they cannot circle things on screen while typing.)")
}

/// capture label + " (image dimensions: WxH pixels)"
pub fn image_label_with_dimensions(capture_label: &str, width: u32, height: u32) -> String {
    format!("{capture_label} (image dimensions: {width}x{height} pixels)")
}

/// Exact Mac text describing the region the user drew around (screen number + screenshot pixels).
/// The caller puts it on its own line after the transcript.
pub fn user_circled_region_note(screen_number: usize, rect: PhysicalRect) -> String {
    format!(
        "(the user circled a region on screen {screen_number} at {},{},{},{} (top-left x,y then width,height, in screenshot pixels) while speaking. \"this\", \"that\", or \"here\" refers to it.)",
        rect.x, rect.y, rect.width, rect.height
    )
}

pub fn user_circled_region_crop_label(screen_number: usize) -> String {
    format!("zoomed crop of the region the user circled on screen {screen_number} (for reading only, not a coordinate space)")
}

/// The user prompt for the second claude call that writes a board or notes page.
pub fn narrated_panel_request_prompt(
    kind: LiveContentKind,
    description: &str,
    user_request: &str,
    spoken_intro: &str,
) -> String {
    let request_verb = match kind {
        LiveContentKind::Drawing => "explain",
        LiveContentKind::Notes => "write notes on",
    };
    format!(
        "{request_verb}: {description}\n\nthe user asked: {user_request}\nyou already said, as an intro: {spoken_intro}\n{}",
        reply_language_note(user_request)
    )
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
    // Swift's Int(...) truncates toward zero, as `as i64` does.
    format!(
        "build and explain: {description}\n\nthe model goes on the screenshot labeled \"{capture_label}\" ({width}x{height} pixels); the anchor box is in that image's pixels. the thing is around pixel {},{} there.\n\nthe user asked: {user_request}\nyou already said, as an intro: {spoken_intro}\n{}",
        focus_x as i64,
        focus_y as i64,
        reply_language_note(user_request)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn language_note_follows_the_script_of_the_transcript() {
        assert_eq!(
            reply_language_note("ये क्या है"),
            "(the user spoke hindi or hinglish this turn: reply and narrate in natural hinglish written in devanagari.)"
        );
        assert_eq!(
            reply_language_note("what is this"),
            "(the user spoke english this turn: reply and narrate in english only, even if earlier turns were in hindi.)"
        );
    }

    #[test]
    fn small_prompt_texts_match_the_mac() {
        assert_eq!(
            image_label_with_dimensions("user's screen (cursor is here)", 1280, 720),
            "user's screen (cursor is here) (image dimensions: 1280x720 pixels)"
        );
        assert_eq!(
            user_circled_region_note(2, PhysicalRect { x: 10, y: 20, width: 30, height: 40 }),
            "(the user circled a region on screen 2 at 10,20,30,40 (top-left x,y then width,height, in screenshot pixels) while speaking. \"this\", \"that\", or \"here\" refers to it.)"
        );
        assert_eq!(
            narrated_panel_request_prompt(LiveContentKind::Notes, "this page", "summarise", "here you go."),
            "write notes on: this page\n\nthe user asked: summarise\nyou already said, as an intro: here you go.\n(the user spoke english this turn: reply and narrate in english only, even if earlier turns were in hindi.)"
        );
        assert!(model3d_request_prompt("engine", "screen 1 of 2", 1280, 800, 640.9, 400.2, "show it", "sure.").starts_with(
            "build and explain: engine\n\nthe model goes on the screenshot labeled \"screen 1 of 2\" (1280x800 pixels); the anchor box is in that image's pixels. the thing is around pixel 640,400 there.\n\n"
        ));
    }

    #[test]
    fn prompt_files_are_written_one_per_kind() {
        let prompts_directory = std::env::temp_dir().join(format!("hellobhai-prompts-test-{}", std::process::id()));
        write_prompt_files(&prompts_directory).unwrap();
        for kind in ALL_SYSTEM_PROMPT_KINDS {
            assert_eq!(std::fs::read_to_string(prompt_file_path(&prompts_directory, kind)).unwrap(), prompt_file_text(kind));
        }
        std::fs::remove_dir_all(&prompts_directory).unwrap();
    }
}
