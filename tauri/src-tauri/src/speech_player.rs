//! Speaks replies aloud, port of the Mac's `LocalTTSClient`: Supertonic-3
//! sentence by sentence (next sentence synthesized during playback), the system
//! voice until the model is loaded or when synthesis fails. Emits
//! `speaking-changed { isSpeaking }` to every window on each change, which drives
//! the speaking pill.

use std::sync::Arc;

use tauri::AppHandle;

use crate::supertonic::SupertonicVoice;

#[derive(Clone, Debug, PartialEq)]
pub enum NeuralVoiceLoadState {
    NotLoaded,
    Downloading { fraction: f32 },
    Loading,
    Ready,
    Failed(String),
}

pub struct SpeechPlayer {}

impl SpeechPlayer {
    /// Downloads ONNX Runtime, Supertonic and the voice if missing, loads them and warms up with "Hello.".
    pub fn start(app: AppHandle, initial_voice: SupertonicVoice) -> Arc<Self> {
        todo!("B: start the synthesis thread and downloads")
    }

    /// Returns immediately; stops any previous playback first.
    pub fn speak_text(&self, text: String) {
        todo!("B: speak")
    }

    pub async fn speak_text_and_wait_until_finished(&self, text: String) {
        todo!("B: speak and wait")
    }

    pub fn stop_playback(&self) {
        todo!("B: stop")
    }

    pub fn is_speaking(&self) -> bool {
        todo!("B: speaking flag")
    }

    /// Downloads that voice's json if missing.
    pub fn set_voice(&self, voice: SupertonicVoice) {
        todo!("B: switch voice")
    }

    /// Speaks a short fixed line in that voice.
    pub fn preview_voice(&self, voice: SupertonicVoice) {
        todo!("B: preview")
    }

    pub fn load_state(&self) -> NeuralVoiceLoadState {
        todo!("B: load state")
    }
}
