//! On-device speech-to-text: whisper.cpp's `whisper-server` (downloaded on first
//! launch) runs on 127.0.0.1 with `ggml-large-v3-turbo-q5_0.bin` loaded. Each clip
//! is posted as a 16 kHz WAV with auto language detection; clips detected as Urdu
//! are re-sent as Hindi (they sound alike).

use std::io::Cursor;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tauri::AppHandle;

use crate::downloads::{download_file_if_missing, extract_zip_entries};

const WHISPER_MODEL_URL: &str =
    "https://huggingface.co/ggerganov/whisper.cpp/resolve/5359861c739e955e79d9a303bcbc70fb988958b1/ggml-large-v3-turbo-q5_0.bin";
const WHISPER_MODEL_FILE_NAME: &str = "ggml-large-v3-turbo-q5_0.bin";
const WHISPER_MODEL_SIZE_BYTES: u64 = 574_041_195;
const WHISPER_MODEL_SHA256: &str = "394221709cd5ad1f40c46e6031ca61bce88931e6e088c188294c6d5a55ffa7e2";
const WHISPER_SERVER_FOLDER_NAME: &str = "whisper-server";
const WHISPER_SAMPLE_RATE: u32 = 16_000;
/// Whisper hallucinates on near-empty clips; skip anything under 0.3 s (same as the Mac).
const MINIMUM_SAMPLE_COUNT: usize = (WHISPER_SAMPLE_RATE as usize * 3) / 10;
/// Loading the 574 MB model on a slow CPU can take a while.
const SERVER_STARTUP_TIMEOUT: Duration = Duration::from_secs(300);

#[derive(Clone, Debug, PartialEq)]
pub enum SpeechModelState {
    Downloading { fraction: f32 },
    Starting,
    Ready,
    Failed(String),
}

pub struct WhisperTranscriber {
    state: Mutex<SpeechModelState>,
    /// Set once the server answers; `http://127.0.0.1:<port>`.
    server_base_url: Mutex<Option<String>>,
    /// Held so `kill_on_drop` only fires when the app exits.
    server_process: Mutex<Option<tokio::process::Child>>,
    /// Held for the app's lifetime: closing it (at exit) kills whisper-server.
    server_process_tree: Mutex<Option<crate::platform::ProcessTreeKillGuard>>,
    http_client: reqwest::Client,
}

impl WhisperTranscriber {
    /// Downloads the runtime + model if missing, starts whisper-server, polls until it answers.
    pub fn start(app: AppHandle) -> Arc<Self> {
        let whisper_transcriber = Arc::new(WhisperTranscriber {
            state: Mutex::new(SpeechModelState::Downloading { fraction: 0.0 }),
            server_base_url: Mutex::new(None),
            server_process: Mutex::new(None),
            server_process_tree: Mutex::new(None),
            http_client: local_http_client(),
        });
        let models_directory = crate::app_paths::models_directory(&app);
        let transcriber_for_startup = whisper_transcriber.clone();
        tauri::async_runtime::spawn(async move {
            if let Err(startup_error) = transcriber_for_startup.download_and_launch_server(&models_directory).await {
                eprintln!("whisper: {startup_error}");
                transcriber_for_startup.set_state(SpeechModelState::Failed(startup_error));
            }
        });
        whisper_transcriber
    }

    pub fn state(&self) -> SpeechModelState {
        self.state.lock().unwrap().clone()
    }

    fn set_state(&self, new_state: SpeechModelState) {
        *self.state.lock().unwrap() = new_state;
    }

    async fn download_and_launch_server(&self, models_directory: &Path) -> Result<(), String> {
        let Some(whisper_server_download) = crate::platform::whisper_server_download() else {
            return Err("speech-to-text is not supported on this operating system".into());
        };
        let whisper_server_directory = models_directory.join(WHISPER_SERVER_FOLDER_NAME);
        let whisper_server_executable = whisper_server_directory.join(whisper_server_download.executable_or_library_file_name);
        let whisper_model_path = models_directory.join(WHISPER_MODEL_FILE_NAME);

        // One progress bar over both downloads, weighted by size.
        let total_download_size_bytes = (whisper_server_download.size_bytes + WHISPER_MODEL_SIZE_BYTES) as f32;
        let runtime_share = whisper_server_download.size_bytes as f32 / total_download_size_bytes;

        // The folder only appears once extraction has finished (extract_zip_entries renames it into place).
        if !whisper_server_executable.exists() {
            let runtime_zip_path = models_directory.join("whisper-bin-x64.zip");
            download_file_if_missing(
                whisper_server_download.url,
                &runtime_zip_path,
                whisper_server_download.size_bytes,
                whisper_server_download.sha256,
                |fraction| self.set_state(SpeechModelState::Downloading { fraction: fraction * runtime_share }),
            )
            .await?;
            // Keep Release/whisper-server.exe and every Release/*.dll (unused DLLs are harmless).
            extract_zip_entries(
                &runtime_zip_path,
                &whisper_server_directory,
                |entry_name| {
                    entry_name.starts_with("Release/")
                        && (entry_name.ends_with(whisper_server_download.executable_or_library_file_name) || entry_name.ends_with(".dll"))
                },
                true,
            )?;
            let _ = std::fs::remove_file(&runtime_zip_path);
        }

        download_file_if_missing(WHISPER_MODEL_URL, &whisper_model_path, WHISPER_MODEL_SIZE_BYTES, WHISPER_MODEL_SHA256, |fraction| {
            self.set_state(SpeechModelState::Downloading { fraction: runtime_share + fraction * (1.0 - runtime_share) })
        })
        .await?;

        self.set_state(SpeechModelState::Starting);
        let server_port = pick_free_local_port()?;
        let mut server_command = tokio::process::Command::new(&whisper_server_executable);
        server_command
            .arg("-m")
            .arg(&whisper_model_path)
            .args(["--host", "127.0.0.1", "--port", &server_port.to_string(), "-nt", "-l", "auto"])
            .current_dir(&whisper_server_directory)
            .env("PATH", path_with_app_directory_first())
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true);
        crate::platform::configure_background_command(&mut server_command);
        let server_child = server_command
            .spawn()
            .map_err(|spawn_error| format!("cannot start whisper-server: {spawn_error}"))?;
        *self.server_process_tree.lock().unwrap() = Some(crate::platform::kill_process_tree_when_dropped(&server_child));
        *self.server_process.lock().unwrap() = Some(server_child);

        let server_base_url = format!("http://127.0.0.1:{server_port}");
        self.wait_until_server_answers(&server_base_url).await?;
        *self.server_base_url.lock().unwrap() = Some(server_base_url);
        self.set_state(SpeechModelState::Ready);
        Ok(())
    }

    /// GET / returns 503 while the model loads and 200 once it is ready.
    async fn wait_until_server_answers(&self, server_base_url: &str) -> Result<(), String> {
        let polling_started = Instant::now();
        loop {
            if let Some(server_child) = self.server_process.lock().unwrap().as_mut() {
                if let Ok(Some(exit_status)) = server_child.try_wait() {
                    return Err(format!("whisper-server exited during startup ({exit_status})"));
                }
            }
            let server_answered = self
                .http_client
                .get(format!("{server_base_url}/"))
                .send()
                .await
                .map(|response| response.status().is_success())
                .unwrap_or(false);
            if server_answered {
                return Ok(());
            }
            if polling_started.elapsed() > SERVER_STARTUP_TIMEOUT {
                return Err("whisper-server did not start in time".into());
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }

    /// Clips under 0.3 s return "" without a request.
    pub async fn transcribe(&self, samples_16k: Vec<f32>) -> Result<String, String> {
        if samples_16k.len() < MINIMUM_SAMPLE_COUNT {
            return Ok(String::new());
        }
        let Some(server_base_url) = self.server_base_url.lock().unwrap().clone() else {
            return Err("speech model is not ready yet".into());
        };
        let wav_bytes = encode_16_bit_mono_wav(&samples_16k)?;

        let transcription_started = Instant::now();
        let mut transcription = self.request_transcription(&server_base_url, &wav_bytes, "auto").await?;
        // Spoken Hindi and Urdu sound almost identical, so detection sometimes picks
        // Urdu and writes Urdu script. Redo it as Hindi.
        if language_needs_hindi_retry(&transcription.language) {
            transcription = self.request_transcription(&server_base_url, &wav_bytes, "hi").await?;
        }
        eprintln!(
            "whisper: [{}] in {:.2}s",
            transcription.language,
            transcription_started.elapsed().as_secs_f32()
        );
        Ok(transcription.text.trim().to_string())
    }

    async fn request_transcription(&self, server_base_url: &str, wav_bytes: &[u8], language: &str) -> Result<ServerTranscription, String> {
        let audio_part = reqwest::multipart::Part::bytes(wav_bytes.to_vec())
            .file_name("clip.wav")
            .mime_str("audio/wav")
            .map_err(|error| error.to_string())?;
        let request_form = reqwest::multipart::Form::new()
            .part("file", audio_part)
            .text("response_format", "verbose_json")
            .text("language", language.to_string())
            .text("no_timestamps", "true");
        let response_body = self
            .http_client
            .post(format!("{server_base_url}/inference"))
            .multipart(request_form)
            .send()
            .await
            .and_then(|response| response.error_for_status())
            .map_err(|error| format!("whisper-server request failed: {error}"))?
            .bytes()
            .await
            .map_err(|error| format!("whisper-server response failed: {error}"))?;
        parse_server_transcription(&response_body)
    }
}

#[derive(Debug, PartialEq)]
struct ServerTranscription {
    text: String,
    language: String,
}

fn parse_server_transcription(response_body: &[u8]) -> Result<ServerTranscription, String> {
    let response_json: serde_json::Value =
        serde_json::from_slice(response_body).map_err(|error| format!("whisper-server sent invalid JSON: {error}"))?;
    if let Some(server_error) = response_json.get("error").and_then(|value| value.as_str()) {
        return Err(format!("whisper-server: {server_error}"));
    }
    Ok(ServerTranscription {
        text: response_json.get("text").and_then(|value| value.as_str()).unwrap_or_default().to_string(),
        language: response_json.get("language").and_then(|value| value.as_str()).unwrap_or_default().to_string(),
    })
}

fn encode_16_bit_mono_wav(samples_16k: &[f32]) -> Result<Vec<u8>, String> {
    let wav_spec = hound::WavSpec {
        channels: 1,
        sample_rate: WHISPER_SAMPLE_RATE,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut wav_bytes = Cursor::new(Vec::new());
    let mut wav_writer = hound::WavWriter::new(&mut wav_bytes, wav_spec).map_err(|error| error.to_string())?;
    for sample in samples_16k {
        let sample_as_i16 = (sample.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
        wav_writer.write_sample(sample_as_i16).map_err(|error| error.to_string())?;
    }
    wav_writer.finalize().map_err(|error| error.to_string())?;
    Ok(wav_bytes.into_inner())
}

/// Never the system proxy: corporate proxy settings without a `<local>` bypass
/// would send requests for 127.0.0.1 to the proxy.
fn local_http_client() -> reqwest::Client {
    reqwest::Client::builder().no_proxy().build().unwrap_or_default()
}

/// whisper-server.exe needs the Visual C++ runtime DLLs (msvcp140, vcruntime140).
/// The installer puts app-local copies next to hellobhai.exe; Windows searches
/// PATH for a child's DLLs, so a PC without the VC++ redistributable still works.
fn path_with_app_directory_first() -> std::ffi::OsString {
    let app_directory = std::env::current_exe().ok().and_then(|app_executable| app_executable.parent().map(Path::to_path_buf));
    let existing_path = std::env::var_os("PATH").unwrap_or_default();
    let directories = app_directory.into_iter().chain(std::env::split_paths(&existing_path));
    std::env::join_paths(directories).unwrap_or(existing_path)
}

/// Binds port 0 to let the OS pick, then frees it for whisper-server.
fn pick_free_local_port() -> Result<u16, String> {
    std::net::TcpListener::bind("127.0.0.1:0")
        .and_then(|listener| listener.local_addr())
        .map(|address| address.port())
        .map_err(|error| format!("no free local port: {error}"))
}

/// whisper-server reports the language as a full name ("urdu") or a code ("ur").
pub fn language_needs_hindi_retry(server_language: &str) -> bool {
    let normalized_language = server_language.trim().to_ascii_lowercase();
    normalized_language == "urdu" || normalized_language == "ur"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urdu_in_either_form_triggers_the_hindi_retry() {
        assert!(language_needs_hindi_retry("urdu"));
        assert!(language_needs_hindi_retry("ur"));
        assert!(language_needs_hindi_retry("Urdu "));
        assert!(!language_needs_hindi_retry("hindi"));
        assert!(!language_needs_hindi_retry("english"));
        assert!(!language_needs_hindi_retry(""));
    }

    #[test]
    fn verbose_json_response_is_parsed() {
        let parsed = parse_server_transcription(br#"{"task":"transcribe","language":"hindi","duration":1.2,"text":" namaste bhai\n","segments":[]}"#).unwrap();
        assert_eq!(parsed, ServerTranscription { text: " namaste bhai\n".into(), language: "hindi".into() });
        assert!(parse_server_transcription(br#"{"error":"failed to read audio data"}"#).is_err());
    }

    #[test]
    fn short_clips_return_empty_without_a_server() {
        let whisper_transcriber = WhisperTranscriber {
            state: Mutex::new(SpeechModelState::Starting),
            server_base_url: Mutex::new(None),
            server_process: Mutex::new(None),
            server_process_tree: Mutex::new(None),
            http_client: local_http_client(),
        };
        let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        assert_eq!(runtime.block_on(whisper_transcriber.transcribe(vec![0.0; 4_000])), Ok(String::new()));
        assert!(runtime.block_on(whisper_transcriber.transcribe(vec![0.0; 16_000])).is_err());
    }

    #[test]
    fn wav_has_a_16_khz_mono_16_bit_header() {
        let wav_bytes = encode_16_bit_mono_wav(&[0.0, 0.5, -1.0]).unwrap();
        let wav_reader = hound::WavReader::new(Cursor::new(wav_bytes)).unwrap();
        assert_eq!(wav_reader.spec().sample_rate, 16_000);
        assert_eq!(wav_reader.spec().channels, 1);
        assert_eq!(wav_reader.len(), 3);
    }
}
