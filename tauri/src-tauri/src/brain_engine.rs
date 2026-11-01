//! Which local CLI answers: `claude -p` (default) or `codex exec`, picked in the
//! settings panel (General -> Brain). Every call (voice, board/notes, 3D,
//! onboarding) goes through `ask_selected_brain`, like the Mac's
//! `CompanionManager.askSelectedBrain`.

use serde::{Deserialize, Serialize};

use crate::claude_cli::{run_claude_streaming, ClaudeRequest};
use crate::codex_cli::run_codex_streaming;
use crate::settings::SharedSettings;

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum BrainEngine {
    #[default]
    Claude,
    Codex,
}

/// Reasoning efforts the settings panel and chat offer, for both brains;
/// `None` (shown as "Default") means the model's own default.
pub const REASONING_EFFORTS: [&str; 3] = ["low", "medium", "high"];

/// Claude gets the request untouched. Codex ignores `model` (it uses the CLI's
/// default model) and, on voice turns, gets a note that it has no file tools.
pub async fn ask_selected_brain(
    settings: &SharedSettings,
    request: ClaudeRequest<'_>,
    on_accumulated_text: impl FnMut(&str),
) -> Result<String, String> {
    let (brain_engine, reasoning_effort) = {
        let current_settings = settings.read().unwrap();
        (current_settings.brain_engine, current_settings.reasoning_effort.clone())
    };
    match brain_engine {
        // The request's own effort ("low" for panels) wins over the setting.
        BrainEngine::Claude => {
            let request = ClaudeRequest {
                effort: request.effort.or_else(|| reasoning_effort.as_deref().and_then(static_reasoning_effort)),
                ..request
            };
            run_claude_streaming(request, on_accumulated_text).await
        }
        BrainEngine::Codex => run_codex_streaming(request, reasoning_effort, on_accumulated_text).await,
    }
}

/// `ClaudeRequest.effort` is `&'static str`; maps a saved effort onto it.
fn static_reasoning_effort(reasoning_effort: &str) -> Option<&'static str> {
    REASONING_EFFORTS.into_iter().find(|known_effort| *known_effort == reasoning_effort)
}
