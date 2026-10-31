//! Where the app keeps its files (all under the per-user local data folder).

use std::path::PathBuf;

use tauri::AppHandle;

/// `app_local_data_dir/Models`: whisper-server, Whisper model, ONNX Runtime,
/// Supertonic models and voices (about 1.06 GB after first launch).
pub fn models_directory(app: &AppHandle) -> PathBuf {
    todo!("E: app.path().app_local_data_dir() joined with Models, created if missing")
}

/// `app_local_data_dir/prompts`: system prompt files for `--system-prompt-file`,
/// rewritten on every launch from the embedded generated prompts.
pub fn prompts_directory(app: &AppHandle) -> PathBuf {
    todo!("E: app.path().app_local_data_dir() joined with prompts, created if missing")
}
