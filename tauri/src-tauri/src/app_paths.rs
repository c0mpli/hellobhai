//! Where the app keeps its files (all under the per-user local data folder).

use std::path::PathBuf;

use tauri::{AppHandle, Manager};

fn app_local_data_subdirectory(app: &AppHandle, subdirectory_name: &str) -> PathBuf {
    // Falls back to the temp folder only if Windows can't tell us %LOCALAPPDATA%.
    let local_data_directory = app.path().app_local_data_dir().unwrap_or_else(|_| std::env::temp_dir().join("hellobhai"));
    let subdirectory = local_data_directory.join(subdirectory_name);
    if let Err(create_error) = std::fs::create_dir_all(&subdirectory) {
        eprintln!("app paths: couldn't create {}: {create_error}", subdirectory.display());
    }
    subdirectory
}

/// `app_local_data_dir/Models`: whisper-server, Whisper model, ONNX Runtime,
/// Supertonic models and voices (about 1.06 GB after first launch).
pub fn models_directory(app: &AppHandle) -> PathBuf {
    app_local_data_subdirectory(app, "Models")
}

/// `app_local_data_dir/prompts`: system prompt files for `--system-prompt-file`,
/// rewritten on every launch from the embedded generated prompts.
pub fn prompts_directory(app: &AppHandle) -> PathBuf {
    app_local_data_subdirectory(app, "prompts")
}
