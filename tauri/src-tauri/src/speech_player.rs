//! Speaks replies aloud, port of the Mac's `LocalTTSClient`: Supertonic-3
//! sentence by sentence (next sentence synthesized during playback), the system
//! voice until the model is loaded or when synthesis fails. Emits
//! `speaking-changed { isSpeaking }` to every window on each change, which drives
//! the speaking pill, and `voice-load-state-changed` (no payload) whenever
//! `load_state()` changes, so the settings page can refresh its status.

use std::collections::HashMap;
use std::num::NonZero;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rodio::buffer::SamplesBuffer;
use rodio::{DeviceSinkBuilder, Player};
use tauri::{AppHandle, Emitter};

use crate::app_paths;
use crate::downloads::{download_file_if_missing, extract_zip_entries};
use crate::platform;
use crate::sentence_splitter::split_into_sentences;
use crate::supertonic::{language_code_for_sentence, load_voice_style, SupertonicEngine, SupertonicVoice, VoiceStyle};

/// Pinned Hugging Face revision of Supertone/supertonic-3.
const SUPERTONIC_DOWNLOAD_BASE_URL: &str =
    "https://huggingface.co/Supertone/supertonic-3/resolve/3cadd1ee6394adea1bd021217a0e650ede09a323/";
/// `onnx/` files and their exact sizes.
const SUPERTONIC_MODEL_FILES: [(&str, u64); 6] = [
    ("duration_predictor.onnx", 3_700_147),
    ("text_encoder.onnx", 36_416_150),
    ("vector_estimator.onnx", 256_534_781),
    ("vocoder.onnx", 101_424_195),
    ("tts.json", 8_253),
    ("unicode_indexer.json", 277_676),
];
/// Same line as the Mac panel's preview button.
const PREVIEW_SENTENCE: &str = "Hi, I'm bhai. Hold the shortcut and ask me anything.";
const PLAYBACK_POLL_INTERVAL: Duration = Duration::from_millis(30);

#[derive(Clone, Debug, PartialEq)]
pub enum NeuralVoiceLoadState {
    NotLoaded,
    Downloading { fraction: f32 },
    Loading,
    Ready,
    Failed(String),
}

/// Work for the synthesis thread, which owns the engine (ort sessions run with `&mut`).
enum SynthesisThreadCommand {
    Speak { text: String, playback_generation: u64, voice: SupertonicVoice },
    NeuralEngineLoaded { engine: SupertonicEngine, supertonic_directory: PathBuf },
}

pub struct SpeechPlayer {
    app: AppHandle,
    synthesis_thread_sender: Sender<SynthesisThreadCommand>,
    /// Bumped on every speak and stop so a cancelled playback finishing late
    /// can't clear the speaking flag for the playback that replaced it.
    playback_generation: AtomicU64,
    is_speaking: AtomicBool,
    selected_voice: Mutex<SupertonicVoice>,
    load_state: Mutex<NeuralVoiceLoadState>,
    /// The playback in progress, so `stop_playback` can silence it from any thread.
    current_player: Mutex<Option<Arc<Player>>>,
}

fn supertonic_directory(models_directory: &Path) -> PathBuf {
    models_directory.join("supertonic-3")
}

fn voice_style_path(supertonic_directory: &Path, voice: SupertonicVoice) -> PathBuf {
    supertonic_directory.join("voice_styles").join(voice.file_name())
}

impl SpeechPlayer {
    /// Downloads ONNX Runtime, Supertonic and the voice if missing, loads them and warms up with "Hello.".
    pub fn start(app: AppHandle, initial_voice: SupertonicVoice) -> Arc<Self> {
        let (synthesis_thread_sender, synthesis_thread_receiver) = std::sync::mpsc::channel();
        let speech_player = Arc::new(SpeechPlayer {
            app,
            synthesis_thread_sender,
            playback_generation: AtomicU64::new(0),
            is_speaking: AtomicBool::new(false),
            selected_voice: Mutex::new(initial_voice),
            load_state: Mutex::new(NeuralVoiceLoadState::NotLoaded),
            current_player: Mutex::new(None),
        });

        let speech_player_for_thread = speech_player.clone();
        std::thread::Builder::new()
            .name("speech synthesis".into())
            .spawn(move || speech_player_for_thread.run_synthesis_thread(synthesis_thread_receiver))
            .expect("could not start the speech synthesis thread");

        let speech_player_for_download = speech_player.clone();
        tauri::async_runtime::spawn(async move {
            if let Err(error_message) = speech_player_for_download.download_and_load_neural_voice(initial_voice).await {
                eprintln!("Supertonic-3: failed to load, using the system voice: {error_message}");
                speech_player_for_download.set_load_state(NeuralVoiceLoadState::Failed(error_message));
            }
        });
        speech_player
    }

    /// Returns immediately; stops any previous playback first.
    pub fn speak_text(&self, text: String) {
        let voice = *self.selected_voice.lock().unwrap();
        self.start_speaking(text, voice);
    }

    pub async fn speak_text_and_wait_until_finished(&self, text: String) {
        let voice = *self.selected_voice.lock().unwrap();
        let this_playback_generation = self.start_speaking(text, voice);
        while self.playback_generation.load(Ordering::SeqCst) == this_playback_generation && self.is_speaking() {
            tokio::time::sleep(PLAYBACK_POLL_INTERVAL).await;
        }
    }

    pub fn stop_playback(&self) {
        self.playback_generation.fetch_add(1, Ordering::SeqCst);
        self.silence_current_player();
        self.set_speaking(false);
    }

    pub fn is_speaking(&self) -> bool {
        self.is_speaking.load(Ordering::SeqCst)
    }

    /// Downloads that voice's json if missing.
    pub fn set_voice(&self, voice: SupertonicVoice) {
        *self.selected_voice.lock().unwrap() = voice;
        // Until it has downloaded, the synthesis thread keeps speaking with the previous voice.
        let app = self.app.clone();
        tauri::async_runtime::spawn(async move {
            if let Err(error_message) = download_voice_style_if_missing(&app, voice).await {
                eprintln!("Supertonic-3: failed to download voice {voice:?}: {error_message}");
            }
        });
    }

    /// Speaks a short fixed line in that voice. Like the Mac's preview button,
    /// pressing it while something is playing stops instead.
    pub fn preview_voice(self: &Arc<Self>, voice: SupertonicVoice) {
        if self.is_speaking() {
            self.stop_playback();
            return;
        }
        let speech_player = self.clone();
        tauri::async_runtime::spawn(async move {
            if let Err(error_message) = download_voice_style_if_missing(&speech_player.app, voice).await {
                eprintln!("Supertonic-3: failed to download voice {voice:?}: {error_message}");
            }
            speech_player.start_speaking(PREVIEW_SENTENCE.to_string(), voice);
        });
    }

    pub fn load_state(&self) -> NeuralVoiceLoadState {
        self.load_state.lock().unwrap().clone()
    }

    /// Returns the generation of the new playback.
    fn start_speaking(&self, text: String, voice: SupertonicVoice) -> u64 {
        self.silence_current_player();
        let this_playback_generation = self.playback_generation.fetch_add(1, Ordering::SeqCst) + 1;
        self.set_speaking(true);
        let _ = self.synthesis_thread_sender.send(SynthesisThreadCommand::Speak {
            text,
            playback_generation: this_playback_generation,
            voice,
        });
        this_playback_generation
    }

    fn silence_current_player(&self) {
        if let Some(player) = self.current_player.lock().unwrap().take() {
            player.stop();
        }
    }

    fn set_speaking(&self, is_speaking_now: bool) {
        let was_speaking = self.is_speaking.swap(is_speaking_now, Ordering::SeqCst);
        if was_speaking != is_speaking_now {
            let _ = self.app.emit("speaking-changed", serde_json::json!({ "isSpeaking": is_speaking_now }));
        }
    }

    fn set_load_state(&self, new_load_state: NeuralVoiceLoadState) {
        // Whole percents only, so a download emits about a hundred events, not thousands.
        let new_load_state = match new_load_state {
            NeuralVoiceLoadState::Downloading { fraction } => NeuralVoiceLoadState::Downloading {
                fraction: (fraction * 100.0).floor() / 100.0,
            },
            other_state => other_state,
        };
        let mut load_state = self.load_state.lock().unwrap();
        if *load_state != new_load_state {
            *load_state = new_load_state;
            drop(load_state);
            let _ = self.app.emit("voice-load-state-changed", ());
        }
    }

    async fn download_and_load_neural_voice(&self, initial_voice: SupertonicVoice) -> Result<(), String> {
        let Some(onnxruntime_download) = platform::onnxruntime_download() else {
            return Err("the neural voice is not available on this operating system yet".into());
        };
        let models_directory = app_paths::models_directory(&self.app);
        let onnxruntime_directory = models_directory.join("onnxruntime");
        let onnxruntime_dll = onnxruntime_directory.join(onnxruntime_download.executable_or_library_file_name);
        let onnxruntime_zip = models_directory.join("onnxruntime.zip");
        let supertonic_directory = supertonic_directory(&models_directory);

        let mut files_to_download: Vec<(String, PathBuf, u64)> = Vec::new();
        if !onnxruntime_dll.exists() {
            files_to_download.push((onnxruntime_download.url.to_string(), onnxruntime_zip.clone(), onnxruntime_download.size_bytes));
        }
        for (model_file_name, size_bytes) in SUPERTONIC_MODEL_FILES {
            files_to_download.push((
                format!("{SUPERTONIC_DOWNLOAD_BASE_URL}onnx/{model_file_name}"),
                supertonic_directory.join("onnx").join(model_file_name),
                size_bytes,
            ));
        }
        files_to_download.push((
            format!("{SUPERTONIC_DOWNLOAD_BASE_URL}voice_styles/{}", initial_voice.file_name()),
            voice_style_path(&supertonic_directory, initial_voice),
            initial_voice.file_size_bytes(),
        ));

        // One after another: the connection is the limit, and progress stays simple.
        let total_bytes: u64 = files_to_download.iter().map(|(_, _, size_bytes)| size_bytes).sum();
        let mut finished_bytes = 0u64;
        self.set_load_state(NeuralVoiceLoadState::Downloading { fraction: 0.0 });
        for (url, destination, size_bytes) in &files_to_download {
            download_file_if_missing(url, destination, *size_bytes, |file_fraction| {
                let overall_fraction = (finished_bytes as f32 + file_fraction * *size_bytes as f32) / total_bytes as f32;
                self.set_load_state(NeuralVoiceLoadState::Downloading { fraction: overall_fraction });
            })
            .await?;
            finished_bytes += size_bytes;
        }
        if !onnxruntime_dll.exists() {
            extract_zip_entries(
                &onnxruntime_zip,
                &onnxruntime_directory,
                |entry_name| entry_name.ends_with("/lib/onnxruntime.dll") || entry_name.ends_with("/lib/onnxruntime_providers_shared.dll"),
                true,
            )?;
            let _ = std::fs::remove_file(&onnxruntime_zip);
        }

        self.set_load_state(NeuralVoiceLoadState::Loading);
        let supertonic_directory_for_loading = supertonic_directory.clone();
        let engine = tauri::async_runtime::spawn_blocking(move || -> Result<SupertonicEngine, String> {
            let mut engine = SupertonicEngine::load(&supertonic_directory_for_loading.join("onnx"), &onnxruntime_dll)?;
            // The first synthesis after loading is much slower than the rest; pay it now.
            let warm_up_voice_style = load_voice_style(&voice_style_path(&supertonic_directory_for_loading, initial_voice))?;
            engine.synthesize("Hello.", "en", &warm_up_voice_style)?;
            Ok(engine)
        })
        .await
        .map_err(|error| error.to_string())??;

        let _ = self.synthesis_thread_sender.send(SynthesisThreadCommand::NeuralEngineLoaded { engine, supertonic_directory });
        self.set_load_state(NeuralVoiceLoadState::Ready);
        eprintln!("Supertonic-3: ready");
        Ok(())
    }

    fn run_synthesis_thread(self: Arc<Self>, commands: Receiver<SynthesisThreadCommand>) {
        let mut neural_engine: Option<(SupertonicEngine, PathBuf)> = None;
        let mut loaded_voice_styles: HashMap<SupertonicVoice, VoiceStyle> = HashMap::new();
        for command in commands {
            match command {
                SynthesisThreadCommand::NeuralEngineLoaded { engine, supertonic_directory } => {
                    neural_engine = Some((engine, supertonic_directory));
                }
                SynthesisThreadCommand::Speak { text, playback_generation, voice } => {
                    self.speak_on_synthesis_thread(&text, playback_generation, voice, &mut neural_engine, &mut loaded_voice_styles);
                }
            }
        }
    }

    // ponytail: a stop lands between sentences, so a newer request can wait for
    // the one sentence being synthesized (well under a second); ort run options
    // with terminate() would cut that if it ever matters.
    fn speak_on_synthesis_thread(
        &self,
        text: &str,
        playback_generation: u64,
        voice: SupertonicVoice,
        neural_engine: &mut Option<(SupertonicEngine, PathBuf)>,
        loaded_voice_styles: &mut HashMap<SupertonicVoice, VoiceStyle>,
    ) {
        let is_cancelled = || self.playback_generation.load(Ordering::SeqCst) != playback_generation;
        if is_cancelled() {
            return;
        }

        // Opened per reply so a newly plugged-in headset is picked up.
        let mut device_sink = match DeviceSinkBuilder::open_default_sink() {
            Ok(device_sink) => device_sink,
            Err(error) => {
                eprintln!("speech: no audio output device: {error}");
                if !is_cancelled() {
                    self.set_speaking(false);
                }
                return;
            }
        };
        device_sink.log_on_drop(false);
        let player = Arc::new(Player::connect_new(device_sink.mixer()));
        *self.current_player.lock().unwrap() = Some(player.clone());
        // A stop that ran before the player was stored would have missed it.
        if is_cancelled() {
            player.stop();
            return;
        }

        let sentences = split_into_sentences(text);
        let voice_style = match neural_engine {
            Some((_, supertonic_directory)) => voice_style_for(voice, supertonic_directory, loaded_voice_styles),
            None => None,
        };
        match (neural_engine.as_mut(), voice_style) {
            (Some((engine, _)), Some(voice_style)) => {
                let sample_rate = NonZero::new(engine.sample_rate()).expect("tts.json sample rate is not zero");
                for (sentence_index, sentence) in sentences.iter().enumerate() {
                    if is_cancelled() {
                        break;
                    }
                    // Appending returns at once, so the next sentence is synthesized
                    // while this one plays and the queue plays them back to back.
                    match engine.synthesize(sentence, language_code_for_sentence(sentence), voice_style) {
                        Ok(samples) => player.append(SamplesBuffer::new(NonZero::<u16>::MIN, sample_rate, samples)),
                        Err(error_message) => {
                            eprintln!("Supertonic-3 failed, finishing with the system voice: {error_message}");
                            append_system_voice_clip(&player, &sentences[sentence_index..].join(" "));
                            break;
                        }
                    }
                }
            }
            _ => append_system_voice_clip(&player, text),
        }

        while !player.empty() && !is_cancelled() {
            std::thread::sleep(PLAYBACK_POLL_INTERVAL);
        }
        let mut current_player = self.current_player.lock().unwrap();
        if current_player.as_ref().is_some_and(|stored_player| Arc::ptr_eq(stored_player, &player)) {
            *current_player = None;
        }
        drop(current_player);
        if !is_cancelled() {
            self.set_speaking(false);
        }
    }
}

/// The requested voice, read from disk the first time; while its file is still
/// downloading, any voice already loaded (the Mac keeps the old voice too).
fn voice_style_for<'styles>(
    voice: SupertonicVoice,
    supertonic_directory: &Path,
    loaded_voice_styles: &'styles mut HashMap<SupertonicVoice, VoiceStyle>,
) -> Option<&'styles VoiceStyle> {
    if !loaded_voice_styles.contains_key(&voice) {
        let path = voice_style_path(supertonic_directory, voice);
        if path.exists() {
            match load_voice_style(&path) {
                Ok(voice_style) => {
                    loaded_voice_styles.insert(voice, voice_style);
                }
                Err(error_message) => eprintln!("Supertonic-3: could not read voice {voice:?}: {error_message}"),
            }
        }
    }
    if loaded_voice_styles.contains_key(&voice) {
        loaded_voice_styles.get(&voice)
    } else {
        loaded_voice_styles.values().next()
    }
}

async fn download_voice_style_if_missing(app: &AppHandle, voice: SupertonicVoice) -> Result<(), String> {
    if platform::onnxruntime_download().is_none() {
        return Ok(()); // no neural voice on this OS, nothing to fetch
    }
    let destination = voice_style_path(&supertonic_directory(&app_paths::models_directory(app)), voice);
    let url = format!("{SUPERTONIC_DOWNLOAD_BASE_URL}voice_styles/{}", voice.file_name());
    download_file_if_missing(&url, &destination, voice.file_size_bytes(), |_| {}).await
}

/// Queues the whole text in the system voice (Hindi voice when it has Devanagari).
fn append_system_voice_clip(player: &Player, text: &str) {
    let prefer_hindi_voice = language_code_for_sentence(text) == "hi";
    let wav_bytes = match platform::synthesize_with_system_voice(text, prefer_hindi_voice) {
        Ok(wav_bytes) => wav_bytes,
        Err(error) => {
            eprintln!("speech: system voice failed: {error}");
            return;
        }
    };
    match decode_wav(&wav_bytes) {
        Ok(samples_buffer) => player.append(samples_buffer),
        Err(error_message) => eprintln!("speech: could not read the system voice audio: {error_message}"),
    }
}

/// hound instead of rodio's decoder, which would need rodio's `wav` feature.
fn decode_wav(wav_bytes: &[u8]) -> Result<SamplesBuffer, String> {
    let mut wav_reader = hound::WavReader::new(std::io::Cursor::new(wav_bytes)).map_err(|error| error.to_string())?;
    let wav_spec = wav_reader.spec();
    let samples: Vec<f32> = match wav_spec.sample_format {
        hound::SampleFormat::Float => wav_reader.samples::<f32>().collect::<Result<_, _>>(),
        hound::SampleFormat::Int => {
            let full_scale = (1i64 << (wav_spec.bits_per_sample - 1)) as f32;
            wav_reader.samples::<i32>().map(|sample| sample.map(|value| value as f32 / full_scale)).collect()
        }
    }
    .map_err(|error| error.to_string())?;
    let channel_count = NonZero::new(wav_spec.channels).ok_or("wav has no channels")?;
    let sample_rate = NonZero::new(wav_spec.sample_rate).ok_or("wav has no sample rate")?;
    Ok(SamplesBuffer::new(channel_count, sample_rate, samples))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_sixteen_bit_wav() {
        let mut wav_bytes = std::io::Cursor::new(Vec::new());
        let wav_spec = hound::WavSpec { channels: 1, sample_rate: 22_050, bits_per_sample: 16, sample_format: hound::SampleFormat::Int };
        let mut wav_writer = hound::WavWriter::new(&mut wav_bytes, wav_spec).unwrap();
        for sample in [0i16, 16_384, -32_768] {
            wav_writer.write_sample(sample).unwrap();
        }
        wav_writer.finalize().unwrap();
        let samples_buffer = decode_wav(wav_bytes.get_ref()).unwrap();
        let decoded_samples: Vec<f32> = samples_buffer.collect();
        assert_eq!(decoded_samples, vec![0.0, 0.5, -1.0]);
    }
}
