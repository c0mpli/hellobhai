//! Supertonic-3 neural TTS (English and Hindi, one voice for both) on ONNX
//! Runtime. Port of supertone-inc/supertonic `rust/src/helper.rs` (MIT) to the
//! ort rc.15 API: duration predictor -> text encoder -> 8 denoising steps of the
//! vector estimator -> vocoder, 44 100 Hz mono, speed 1.05.

use std::path::Path;
use std::sync::{Mutex, OnceLock};

use ndarray::{Array, Array1, Array2, Array3};
use ort::environment::Environment;
use ort::session::Session;
use ort::value::Tensor;
use rand_distr::{Distribution, Normal};
use regex::Regex;
use serde::{Deserialize, Serialize};
use unicode_normalization::UnicodeNormalization;

/// Same values as FluidAudio's Supertonic-3 on the Mac.
const DENOISING_STEP_COUNT: usize = 8;
const SPEECH_SPEED: f32 = 1.05;
/// helper.rs `call`: longer text is cut into chunks of at most this many bytes
/// joined by this much silence.
const MAXIMUM_CHUNK_LENGTH: usize = 300;
const SILENCE_BETWEEN_CHUNKS_SECONDS: f32 = 0.3;

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq, Hash)]
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

impl SupertonicVoice {
    /// "F2" -> `voice_styles/F2.json` in the Hugging Face repo.
    pub fn file_name(self) -> String {
        format!("{self:?}.json")
    }

    /// Exact sizes in the pinned Hugging Face revision, so downloads can be checked.
    pub fn file_size_bytes(self) -> u64 {
        match self {
            SupertonicVoice::F1 => 292_046,
            SupertonicVoice::F2 => 292_423,
            SupertonicVoice::F3 => 290_794,
            SupertonicVoice::F4 => 291_808,
            SupertonicVoice::F5 => 291_479,
            SupertonicVoice::M1 => 291_748,
            SupertonicVoice::M2 => 292_055,
            SupertonicVoice::M3 => 290_198,
            SupertonicVoice::M4 => 291_522,
            SupertonicVoice::M5 => 291_469,
        }
    }

    /// SHA-256 of the voice file in the pinned revision.
    pub fn file_sha256(self) -> &'static str {
        match self {
            SupertonicVoice::F1 => "bbdec6ee00231c2c742ad05483df5334cab3b52fda3ba38e6a07059c4563dbc2",
            SupertonicVoice::F2 => "7c722c6a72707b1a77f035d67f0d1351ba187738e06f7683e8c72b1df3477fc6",
            SupertonicVoice::F3 => "12f6ef2573baa2defa1128069cb59f203e3ab67c92af77b42df8a0e3a2f7c6ab",
            SupertonicVoice::F4 => "c2fa764c1225a76dfc3e2c73e8aa4f70d9ee48793860eb34c295fff01c2e032b",
            SupertonicVoice::F5 => "45966e73316415626cf41a7d1c6f3b4c70dbc1ba2bee5c1978ef0ce33244fc8d",
            SupertonicVoice::M1 => "e35604687f5d23694b8e91593a93eec0e4eca6c0b02bb8ed69139ab2ea6b0a5b",
            SupertonicVoice::M2 => "b76cbf62bac707c710cf0ae5aba5e31eea1a6339a9734bfae33ab98499534a50",
            SupertonicVoice::M3 => "ea1ac35ccb91b0d7ecad533a2fbd0eec10c91513d8951e3b25fbba99954e159b",
            SupertonicVoice::M4 => "ca8eefad4fcd989c9379032ff3e50738adc547eeb5e221b82593a6d7b3bac303",
            SupertonicVoice::M5 => "dd22b92740314321f8ae11c5e87f8dd60d060f15dd3a632b5adf77f471f77af2",
        }
    }
}

/// The two style tensors of a voice json: style_ttl [1,50,256], style_dp [1,8,16].
pub struct VoiceStyle {
    pub style_ttl: ndarray::Array3<f32>,
    pub style_dp: ndarray::Array3<f32>,
}

/// The parts of `onnx/tts.json` the pipeline needs.
#[derive(Deserialize)]
struct SupertonicConfig {
    ae: AutoencoderConfig,
    ttl: TextToLatentConfig,
}

#[derive(Deserialize)]
struct AutoencoderConfig {
    sample_rate: u32,
    base_chunk_size: usize,
}

#[derive(Deserialize)]
struct TextToLatentConfig {
    chunk_compress_factor: usize,
    latent_dim: usize,
}

/// The four ONNX sessions plus `tts.json` config and the unicode indexer.
/// Sessions need `&mut self` to run, so the engine lives on one synthesis thread.
pub struct SupertonicEngine {
    config: SupertonicConfig,
    /// Unicode code point -> model token id.
    unicode_indexer: Vec<i64>,
    duration_predictor: Session,
    text_encoder: Session,
    vector_estimator: Session,
    vocoder: Session,
}

/// ort allows one environment per process, and the DLL can only be loaded once.
static ONNX_RUNTIME_ENVIRONMENT: OnceLock<Environment> = OnceLock::new();
static ONNX_RUNTIME_ENVIRONMENT_INIT_LOCK: Mutex<()> = Mutex::new(());

fn onnx_runtime_environment(onnxruntime_dll: &Path) -> Result<&'static Environment, String> {
    let _init_guard = ONNX_RUNTIME_ENVIRONMENT_INIT_LOCK.lock().unwrap();
    if let Some(environment) = ONNX_RUNTIME_ENVIRONMENT.get() {
        return Ok(environment);
    }
    // Absolute path on purpose: System32 ships an older onnxruntime.dll for Windows ML.
    let environment = ort::init_from(onnxruntime_dll)
        .map_err(|error| format!("could not load {}: {error}", onnxruntime_dll.display()))?
        .build()
        .map_err(|error| error.to_string())?;
    Ok(ONNX_RUNTIME_ENVIRONMENT.get_or_init(|| environment))
}

fn read_json_file<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T, String> {
    let file_contents = std::fs::read(path).map_err(|error| format!("{}: {error}", path.display()))?;
    serde_json::from_slice(&file_contents).map_err(|error| format!("{}: {error}", path.display()))
}

impl SupertonicEngine {
    /// `ort::init_from(onnxruntime_dll)` (absolute path, once per process) then loads the models.
    pub fn load(models_dir: &Path, onnxruntime_dll: &Path) -> Result<Self, String> {
        let environment = onnx_runtime_environment(onnxruntime_dll)?;
        let load_session = |file_name: &str| -> Result<Session, String> {
            Session::builder(environment)
                .and_then(|mut session_builder| session_builder.commit_from_file(models_dir.join(file_name)))
                .map_err(|error| format!("{file_name}: {error}"))
        };
        Ok(SupertonicEngine {
            config: read_json_file(&models_dir.join("tts.json"))?,
            unicode_indexer: read_json_file(&models_dir.join("unicode_indexer.json"))?,
            duration_predictor: load_session("duration_predictor.onnx")?,
            text_encoder: load_session("text_encoder.onnx")?,
            vector_estimator: load_session("vector_estimator.onnx")?,
            vocoder: load_session("vocoder.onnx")?,
        })
    }

    pub fn sample_rate(&self) -> u32 {
        self.config.ae.sample_rate
    }

    /// 44 100 Hz mono samples. helper.rs `call`: chunks of at most 300 bytes,
    /// each run through `_infer`, trimmed to its predicted duration, joined by 0.3 s of silence.
    pub fn synthesize(&mut self, text: &str, language_code: &str, voice_style: &VoiceStyle) -> Result<Vec<f32>, String> {
        let sample_rate = self.sample_rate() as f32;
        let mut all_samples = Vec::new();
        for (chunk_index, chunk) in split_into_chunks_of_maximum_length(text, MAXIMUM_CHUNK_LENGTH).iter().enumerate() {
            let (chunk_samples, chunk_duration_seconds) = self.infer_one_chunk(chunk, language_code, voice_style)?;
            let chunk_sample_count = ((sample_rate * chunk_duration_seconds) as usize).min(chunk_samples.len());
            if chunk_index > 0 {
                let silence_sample_count = (SILENCE_BETWEEN_CHUNKS_SECONDS * sample_rate) as usize;
                all_samples.extend(std::iter::repeat_n(0.0f32, silence_sample_count));
            }
            all_samples.extend_from_slice(&chunk_samples[..chunk_sample_count]);
        }
        Ok(all_samples)
    }

    /// helper.rs `_infer` with a batch of one. Returns the vocoder samples and
    /// the predicted duration in seconds.
    fn infer_one_chunk(&mut self, text: &str, language_code: &str, voice_style: &VoiceStyle) -> Result<(Vec<f32>, f32), String> {
        let to_message = |error: ort::Error| error.to_string();

        let processed_text = preprocess_text(text, language_code);
        let text_ids: Vec<i64> = processed_text
            .chars()
            .map(|character| self.unicode_indexer.get(character as usize).copied().unwrap_or(-1))
            .collect();
        let text_length = text_ids.len();
        let text_ids_tensor = Tensor::from_array(Array2::from_shape_vec((1, text_length), text_ids).map_err(|error| error.to_string())?)
            .map_err(to_message)?;
        let text_mask_tensor = Tensor::from_array(Array3::<f32>::ones((1, 1, text_length))).map_err(to_message)?;
        let style_dp_tensor = Tensor::from_array(voice_style.style_dp.clone()).map_err(to_message)?;
        let style_ttl_tensor = Tensor::from_array(voice_style.style_ttl.clone()).map_err(to_message)?;

        let duration_outputs = self
            .duration_predictor
            .run(ort::inputs! {
                "text_ids" => &text_ids_tensor,
                "style_dp" => &style_dp_tensor,
                "text_mask" => &text_mask_tensor
            })
            .map_err(to_message)?;
        let (_, duration_data) = duration_outputs["duration"].try_extract_tensor::<f32>().map_err(to_message)?;
        let duration_seconds = duration_data.first().copied().ok_or("empty duration")? / SPEECH_SPEED;
        drop(duration_outputs);

        let text_encoder_outputs = self
            .text_encoder
            .run(ort::inputs! {
                "text_ids" => &text_ids_tensor,
                "style_ttl" => &style_ttl_tensor,
                "text_mask" => &text_mask_tensor
            })
            .map_err(to_message)?;
        let (text_embedding_shape, text_embedding_data) =
            text_encoder_outputs["text_emb"].try_extract_tensor::<f32>().map_err(to_message)?;
        let text_embedding_tensor = Tensor::from_array((text_embedding_shape.to_vec(), text_embedding_data.to_vec())).map_err(to_message)?;
        drop(text_encoder_outputs);

        let (mut noisy_latent, latent_mask) = sample_noisy_latent(
            duration_seconds,
            self.config.ae.sample_rate,
            self.config.ae.base_chunk_size,
            self.config.ttl.chunk_compress_factor,
            self.config.ttl.latent_dim,
        );
        let latent_mask_tensor = Tensor::from_array(latent_mask).map_err(to_message)?;
        let total_step_tensor = Tensor::from_array(Array1::from_elem(1, DENOISING_STEP_COUNT as f32)).map_err(to_message)?;

        for denoising_step in 0..DENOISING_STEP_COUNT {
            let noisy_latent_tensor = Tensor::from_array(noisy_latent).map_err(to_message)?;
            let current_step_tensor = Tensor::from_array(Array1::from_elem(1, denoising_step as f32)).map_err(to_message)?;
            let vector_estimator_outputs = self
                .vector_estimator
                .run(ort::inputs! {
                    "noisy_latent" => &noisy_latent_tensor,
                    "text_emb" => &text_embedding_tensor,
                    "style_ttl" => &style_ttl_tensor,
                    "latent_mask" => &latent_mask_tensor,
                    "text_mask" => &text_mask_tensor,
                    "current_step" => &current_step_tensor,
                    "total_step" => &total_step_tensor
                })
                .map_err(to_message)?;
            let (denoised_shape, denoised_data) =
                vector_estimator_outputs["denoised_latent"].try_extract_tensor::<f32>().map_err(to_message)?;
            noisy_latent = Array3::from_shape_vec(
                (denoised_shape[0] as usize, denoised_shape[1] as usize, denoised_shape[2] as usize),
                denoised_data.to_vec(),
            )
            .map_err(|error| error.to_string())?;
        }

        let final_latent_tensor = Tensor::from_array(noisy_latent).map_err(to_message)?;
        let vocoder_outputs = self.vocoder.run(ort::inputs! { "latent" => &final_latent_tensor }).map_err(to_message)?;
        let (_, waveform_data) = vocoder_outputs["wav_tts"].try_extract_tensor::<f32>().map_err(to_message)?;
        Ok((waveform_data.to_vec(), duration_seconds))
    }
}

/// helper.rs `sample_noisy_latent` for a batch of one: N(0,1) noise shaped
/// [1, latent_dim * compress, ceil(samples / (base_chunk * compress))], plus its all-ones mask.
fn sample_noisy_latent(
    duration_seconds: f32,
    sample_rate: u32,
    base_chunk_size: usize,
    chunk_compress_factor: usize,
    latent_dim: usize,
) -> (Array3<f32>, Array3<f32>) {
    let waveform_length = (duration_seconds * sample_rate as f32) as usize;
    let latent_chunk_size = base_chunk_size * chunk_compress_factor;
    let latent_length = waveform_length.div_ceil(latent_chunk_size);
    let latent_channel_count = latent_dim * chunk_compress_factor;

    let standard_normal = Normal::new(0.0f32, 1.0f32).unwrap();
    let mut random_generator = rand::rng();
    let noisy_latent = Array::from_shape_simple_fn((1, latent_channel_count, latent_length), || {
        standard_normal.sample(&mut random_generator)
    });
    // With one item the mask covers every latent frame, so masking the noise changes nothing.
    let latent_mask = Array3::<f32>::ones((1, 1, latent_length));
    (noisy_latent, latent_mask)
}

/// helper.rs `chunk_text`, simplified: our input is already one sentence, so
/// only an over-long sentence is cut, at commas and then at spaces.
fn split_into_chunks_of_maximum_length(text: &str, maximum_length: usize) -> Vec<String> {
    let text = text.trim();
    if text.len() <= maximum_length {
        return vec![text.to_string()];
    }
    let mut chunks = Vec::new();
    let mut current_chunk = String::new();
    for comma_part in text.split(',').map(str::trim).filter(|part| !part.is_empty()) {
        let pieces: Vec<String> = if comma_part.len() <= maximum_length {
            vec![comma_part.to_string()]
        } else {
            // Last resort, like helper.rs: pack whole words.
            let mut word_pieces = vec![String::new()];
            for word in comma_part.split_whitespace() {
                let last_piece = word_pieces.last_mut().unwrap();
                if !last_piece.is_empty() && last_piece.len() + word.len() + 1 > maximum_length {
                    word_pieces.push(word.to_string());
                } else {
                    if !last_piece.is_empty() {
                        last_piece.push(' ');
                    }
                    last_piece.push_str(word);
                }
            }
            word_pieces
        };
        for piece in pieces {
            if !current_chunk.is_empty() && current_chunk.len() + piece.len() + 2 > maximum_length {
                chunks.push(std::mem::take(&mut current_chunk));
            }
            if !current_chunk.is_empty() {
                current_chunk.push_str(", ");
            }
            current_chunk.push_str(&piece);
        }
    }
    if !current_chunk.is_empty() {
        chunks.push(current_chunk);
    }
    chunks
}

#[derive(Deserialize)]
struct VoiceStyleFile {
    style_ttl: VoiceStyleComponent,
    style_dp: VoiceStyleComponent,
}

#[derive(Deserialize)]
struct VoiceStyleComponent {
    data: Vec<Vec<Vec<f32>>>,
    dims: Vec<usize>,
}

impl VoiceStyleComponent {
    fn into_array(self) -> Result<Array3<f32>, String> {
        let [first_dimension, second_dimension, third_dimension] = self.dims[..] else {
            return Err(format!("voice style dims should have 3 entries, got {:?}", self.dims));
        };
        let flattened_values: Vec<f32> = self.data.into_iter().flatten().flatten().collect();
        Array3::from_shape_vec((first_dimension, second_dimension, third_dimension), flattened_values).map_err(|error| error.to_string())
    }
}

pub fn load_voice_style(path: &Path) -> Result<VoiceStyle, String> {
    let voice_style_file: VoiceStyleFile = read_json_file(path)?;
    Ok(VoiceStyle {
        style_ttl: voice_style_file.style_ttl.into_array()?,
        style_dp: voice_style_file.style_dp.into_array()?,
    })
}

fn compiled_regex(cache_slot: &'static OnceLock<Regex>, pattern: &str) -> &'static Regex {
    cache_slot.get_or_init(|| Regex::new(pattern).unwrap())
}

/// helper.rs preprocessing: NFKD, emoji strip, punctuation fixes, wraps in "<hi>…</hi>" style language tags.
pub fn preprocess_text(text: &str, language_code: &str) -> String {
    static EMOJI_PATTERN: OnceLock<Regex> = OnceLock::new();
    static SPACE_BEFORE_PUNCTUATION_PATTERN: OnceLock<Regex> = OnceLock::new();
    static WHITESPACE_RUN_PATTERN: OnceLock<Regex> = OnceLock::new();
    static ENDS_WITH_PUNCTUATION_PATTERN: OnceLock<Regex> = OnceLock::new();

    let mut text: String = text.nfkd().collect();

    let emoji_pattern = compiled_regex(
        &EMOJI_PATTERN,
        r"[\x{1F600}-\x{1F64F}\x{1F300}-\x{1F5FF}\x{1F680}-\x{1F6FF}\x{1F700}-\x{1F77F}\x{1F780}-\x{1F7FF}\x{1F800}-\x{1F8FF}\x{1F900}-\x{1F9FF}\x{1FA00}-\x{1FA6F}\x{1FA70}-\x{1FAFF}\x{2600}-\x{26FF}\x{2700}-\x{27BF}\x{1F1E6}-\x{1F1FF}]+",
    );
    text = emoji_pattern.replace_all(&text, "").into_owned();

    let character_replacements = [
        ("–", "-"),
        ("‑", "-"),
        ("—", "-"),
        ("_", " "),
        ("\u{201C}", "\""),
        ("\u{201D}", "\""),
        ("\u{2018}", "'"),
        ("\u{2019}", "'"),
        ("´", "'"),
        ("`", "'"),
        ("[", " "),
        ("]", " "),
        ("|", " "),
        ("/", " "),
        ("#", " "),
        ("→", " "),
        ("←", " "),
        ("♥", ""),
        ("☆", ""),
        ("♡", ""),
        ("©", ""),
        ("\\", ""),
        ("@", " at "),
        ("e.g.,", "for example, "),
        ("i.e.,", "that is, "),
    ];
    for (from, to) in character_replacements {
        text = text.replace(from, to);
    }

    // helper.rs removes the space before , . ! ? ; : and '
    let space_before_punctuation_pattern = compiled_regex(&SPACE_BEFORE_PUNCTUATION_PATTERN, r" ([,.!?;:'])");
    text = space_before_punctuation_pattern.replace_all(&text, "$1").into_owned();

    for doubled in ["\"\"", "''", "``"] {
        let single = &doubled[..1];
        while text.contains(doubled) {
            text = text.replace(doubled, single);
        }
    }

    let whitespace_run_pattern = compiled_regex(&WHITESPACE_RUN_PATTERN, r"\s+");
    text = whitespace_run_pattern.replace_all(&text, " ").trim().to_string();

    let ends_with_punctuation_pattern = compiled_regex(
        &ENDS_WITH_PUNCTUATION_PATTERN,
        r#"[.!?;:,'"\u{201C}\u{201D}\u{2018}\u{2019})\]}…。」』】〉》›»]$"#,
    );
    if !text.is_empty() && !ends_with_punctuation_pattern.is_match(&text) {
        text.push('.');
    }

    format!("<{language_code}>{text}</{language_code}>")
}

/// "hi" when the sentence has any Devanagari (U+0900..U+097F), else "en".
pub fn language_code_for_sentence(sentence: &str) -> &'static str {
    if sentence.chars().any(|character| ('\u{0900}'..='\u{097F}').contains(&character)) {
        "hi"
    } else {
        "en"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preprocess_text_matches_helper_rs() {
        assert_eq!(preprocess_text("Hello world", "en"), "<en>Hello world.</en>");
        assert_eq!(preprocess_text("  wait , what ?  ", "en"), "<en>wait, what?</en>");
        assert_eq!(preprocess_text("ping me @ 5 — ok 😀", "en"), "<en>ping me at 5 - ok.</en>");
        assert_eq!(preprocess_text("“quoted”", "en"), "<en>\"quoted\"</en>");
        assert_eq!(preprocess_text("a/b [c]", "en"), "<en>a b c.</en>");
        // Devanagari decomposes under NFKD exactly as the model was trained on.
        let hindi = "नमस्ते, मैं भाई हूँ।";
        assert_eq!(preprocess_text(hindi, "hi"), format!("<hi>{}.</hi>", hindi.nfkd().collect::<String>()));
    }

    #[test]
    fn routes_devanagari_to_hindi() {
        assert_eq!(language_code_for_sentence("Open the settings."), "en");
        assert_eq!(language_code_for_sentence("settings खोलो"), "hi");
    }

    #[test]
    fn parses_voice_style_json() {
        let fixture_path = std::env::temp_dir().join("hellobhai-voice-style-fixture.json");
        std::fs::write(
            &fixture_path,
            r#"{"style_ttl":{"data":[[[1,2,3],[4,5,6]]],"dims":[1,2,3],"type":"float32"},
                "style_dp":{"data":[[[7,8]]],"dims":[1,1,2],"type":"float32"}}"#,
        )
        .unwrap();
        let voice_style = load_voice_style(&fixture_path).unwrap();
        assert_eq!(voice_style.style_ttl.shape(), &[1, 2, 3]);
        assert_eq!(voice_style.style_ttl[[0, 1, 2]], 6.0);
        assert_eq!(voice_style.style_dp.shape(), &[1, 1, 2]);
        std::fs::remove_file(fixture_path).ok();
    }

    #[test]
    fn splits_only_over_long_text_into_chunks() {
        assert_eq!(split_into_chunks_of_maximum_length("short one.", 300), vec!["short one."]);
        let long_text = "one two three, four five six";
        let chunks = split_into_chunks_of_maximum_length(long_text, 14);
        assert_eq!(chunks, vec!["one two three", "four five six"]);
        assert!(split_into_chunks_of_maximum_length("aaaa bbbb cccc dddd", 9).iter().all(|chunk| chunk.len() <= 9));
    }
}
