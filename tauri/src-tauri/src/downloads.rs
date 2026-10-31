//! First-launch downloads (whisper-server, Whisper model, ONNX Runtime,
//! Supertonic models and voices) with progress, plus zip extraction. Used by
//! the speech-to-text and text-to-speech modules.

use std::path::Path;

/// Streams to `<destination>.partial`, checks the size, renames. Skips when the
/// file already exists with the expected size.
pub async fn download_file_if_missing(
    url: &str,
    destination: &Path,
    expected_size_bytes: u64,
    on_progress: impl Fn(f32),
) -> Result<(), String> {
    todo!("A: reqwest stream download")
}

/// Extracts the entries for which `keep_entry(entry_name)` is true; `flatten`
/// drops their folders.
pub fn extract_zip_entries(
    zip_path: &Path,
    destination_dir: &Path,
    keep_entry: impl Fn(&str) -> bool,
    flatten: bool,
) -> Result<(), String> {
    todo!("A: zip extraction")
}
