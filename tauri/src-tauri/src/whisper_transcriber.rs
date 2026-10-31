//! On-device speech-to-text: whisper.cpp's `whisper-server` (downloaded on first
//! launch) runs on 127.0.0.1 with `ggml-large-v3-turbo-q5_0.bin` loaded. Each clip
//! is posted as a 16 kHz WAV with auto language detection; clips detected as Urdu
//! are re-sent as Hindi (they sound alike).

use std::sync::Arc;

use tauri::AppHandle;

#[derive(Clone, Debug, PartialEq)]
pub enum SpeechModelState {
    Downloading { fraction: f32 },
    Starting,
    Ready,
    Failed(String),
}

pub struct WhisperTranscriber {}

impl WhisperTranscriber {
    /// Downloads the runtime + model if missing, starts whisper-server, polls until it answers.
    pub fn start(app: AppHandle) -> Arc<Self> {
        todo!("A: download, launch whisper-server, poll")
    }

    pub fn state(&self) -> SpeechModelState {
        todo!("A: current state")
    }

    /// Clips under 0.3 s return "" without a request.
    pub async fn transcribe(&self, samples_16k: Vec<f32>) -> Result<String, String> {
        todo!("A: POST /inference, Urdu -> retry as Hindi")
    }
}

/// whisper-server reports the language as a full name ("urdu") or a code ("ur").
pub fn language_needs_hindi_retry(server_language: &str) -> bool {
    todo!("A: urdu | ur")
}
