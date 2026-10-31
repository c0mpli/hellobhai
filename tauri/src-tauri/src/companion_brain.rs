//! The central state machine, port of the Mac's `CompanionManager`: push-to-talk
//! -> transcribe -> agent loop (up to 8 screenshot -> claude -> actions rounds)
//! -> speak, point, circle; plus the board, notes and 3D side calls and onboarding.
//! The message flow (`send_transcript`) takes plain text so a typed chat panel
//! can reuse it later.

use std::sync::Arc;

use serde::Serialize;
use tauri::AppHandle;
use tokio::sync::mpsc::UnboundedReceiver;

use crate::board_panel::BoardPanel;
use crate::model3d_window::Model3DWindow;
use crate::push_to_talk::HotkeyEvent;
use crate::settings::SharedSettings;
use crate::speech_player::SpeechPlayer;
use crate::whisper_transcriber::WhisperTranscriber;

/// Emitted to overlays and settings as `buddy-voice-state` `{state}`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum VoiceState {
    Idle,
    Listening,
    Processing,
    Responding,
}

/// Owns the recorder, response and visual task handles, conversation history,
/// the user's drawn strokes and the voice state.
pub struct CompanionBrain {}

impl CompanionBrain {
    /// Also creates the `PushToTalkRecorder` (its audio level goes to the overlays as `audio-level`).
    pub fn new(
        app: AppHandle,
        settings: SharedSettings,
        speech_player: Arc<SpeechPlayer>,
        whisper_transcriber: Arc<WhisperTranscriber>,
        board_panel: Arc<BoardPanel>,
        model3d_window: Arc<Model3DWindow>,
    ) -> Arc<Self> {
        todo!("E: build the brain")
    }

    /// Handles press/release/Escape until the sender is dropped.
    pub async fn run_hotkey_loop(self: Arc<Self>, hotkey_receiver: UnboundedReceiver<HotkeyEvent>) {
        todo!("E: port the Mac press/release handling")
    }

    /// Runs the agent loop for one user message (transcript or "What is this?").
    pub fn send_transcript(self: &Arc<Self>, transcript: String) {
        todo!("E: port sendTranscriptToClaudeWithScreenshot")
    }

    /// A freehand stroke the user drew while holding the hotkey, global physical pixels.
    pub fn add_user_stroke(&self, points: Vec<(i32, i32)>) {
        todo!("E: store the stroke")
    }

    /// First launch or "replay onboarding": point at something on screen, then show the hotkey hint.
    pub fn start_onboarding(&self) {
        todo!("E: port performOnboardingDemoInteraction")
    }
}
