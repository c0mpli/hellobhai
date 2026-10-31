//! Supertonic-3 neural TTS (English and Hindi, one voice for both) on ONNX
//! Runtime. Port of supertone-inc/supertonic `rust/src/helper.rs` (MIT) to the
//! ort rc.15 API: duration predictor -> text encoder -> 8 denoising steps of the
//! vector estimator -> vocoder, 44 100 Hz mono, speed 1.05.

use std::path::Path;

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum SupertonicVoice {
    F1,
    #[default]
    F2,
    F3,
    F4,
    F5,
    M1,
    M2,
    M3,
    M4,
    M5,
}

/// The two style tensors of a voice json: style_ttl [1,50,256], style_dp [1,8,16].
pub struct VoiceStyle {
    pub style_ttl: ndarray::Array3<f32>,
    pub style_dp: ndarray::Array3<f32>,
}

/// The four ONNX sessions plus `tts.json` config and the unicode indexer.
/// Sessions need `&mut self` to run, so the engine lives on one synthesis thread.
pub struct SupertonicEngine {}

impl SupertonicEngine {
    /// `ort::init_from(onnxruntime_dll)` (absolute path, once per process) then loads the models.
    pub fn load(models_dir: &Path, onnxruntime_dll: &Path) -> Result<Self, String> {
        todo!("B: init ort and load the 4 sessions")
    }

    /// 44 100 Hz mono samples.
    pub fn synthesize(&mut self, text: &str, language_code: &str, voice_style: &VoiceStyle) -> Result<Vec<f32>, String> {
        todo!("B: port helper.rs _infer")
    }
}

pub fn load_voice_style(path: &Path) -> Result<VoiceStyle, String> {
    todo!("B: parse the voice json")
}

/// helper.rs preprocessing: NFKD, emoji strip, punctuation fixes, wraps in "<hi>…</hi>" style language tags.
pub fn preprocess_text(text: &str, language_code: &str) -> String {
    todo!("B: port helper.rs preprocess")
}

/// "hi" when the sentence has any Devanagari (U+0900..U+097F), else "en".
pub fn language_code_for_sentence(sentence: &str) -> &'static str {
    todo!("B: route by script")
}
