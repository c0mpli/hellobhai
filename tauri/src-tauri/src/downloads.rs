//! First-launch downloads (whisper-server, Whisper model, ONNX Runtime,
//! Supertonic models and voices) with progress, plus zip extraction. Used by
//! the speech-to-text and text-to-speech modules.

use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

use sha2::{Digest, Sha256};

/// https only, gives up on a stalled connection instead of "downloading" forever.
fn download_http_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .https_only(true)
        .connect_timeout(Duration::from_secs(15))
        .read_timeout(Duration::from_secs(30))
        .build()
        .map_err(|error| format!("cannot set up downloads: {error}"))
}

/// Streams to `<destination>.partial`, checks the size and SHA-256, renames.
/// Skips when the file already exists with the expected size: only a verified
/// download is ever renamed to `destination`.
pub async fn download_file_if_missing(
    url: &str,
    destination: &Path,
    expected_size_bytes: u64,
    expected_sha256: &str,
    on_progress: impl Fn(f32),
) -> Result<(), String> {
    let already_downloaded = std::fs::metadata(destination)
        .map(|metadata| metadata.len() == expected_size_bytes)
        .unwrap_or(false);
    if already_downloaded {
        on_progress(1.0);
        return Ok(());
    }

    if let Some(parent_directory) = destination.parent() {
        std::fs::create_dir_all(parent_directory).map_err(|error| format!("cannot create {}: {error}", parent_directory.display()))?;
    }
    let partial_destination = partial_path_for(destination);

    let mut response = download_http_client()?
        .get(url)
        .send()
        .await
        .and_then(|response| response.error_for_status())
        .map_err(|error| format!("download of {url} failed: {error}"))?;

    // ponytail: blocking std::fs writes inside the async task; chunks are small and
    // downloads run on their own spawned task. Switch to tokio::fs if it ever stalls the runtime.
    let mut partial_file = File::create(&partial_destination)
        .map_err(|error| format!("cannot create {}: {error}", partial_destination.display()))?;
    let mut downloaded_size_bytes: u64 = 0;
    let mut content_hasher = Sha256::new();
    while let Some(chunk) = response.chunk().await.map_err(|error| format!("download of {url} failed: {error}"))? {
        downloaded_size_bytes += chunk.len() as u64;
        if downloaded_size_bytes > expected_size_bytes {
            break; // already wrong; reported below
        }
        content_hasher.update(&chunk);
        partial_file
            .write_all(&chunk)
            .map_err(|error| format!("cannot write {}: {error}", partial_destination.display()))?;
        on_progress((downloaded_size_bytes as f32 / expected_size_bytes.max(1) as f32).min(1.0));
    }
    drop(partial_file);

    if downloaded_size_bytes != expected_size_bytes {
        let _ = std::fs::remove_file(&partial_destination);
        return Err(format!(
            "download of {url} has {downloaded_size_bytes} bytes, expected {expected_size_bytes}"
        ));
    }
    let downloaded_sha256 = hex_string(&content_hasher.finalize());
    if !downloaded_sha256.eq_ignore_ascii_case(expected_sha256) {
        let _ = std::fs::remove_file(&partial_destination);
        return Err(format!("download of {url} has SHA-256 {downloaded_sha256}, expected {expected_sha256}"));
    }
    std::fs::rename(&partial_destination, destination)
        .map_err(|error| format!("cannot move {} into place: {error}", partial_destination.display()))
}

fn hex_string(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn partial_path_for(destination: &Path) -> PathBuf {
    let mut partial_file_name = destination.file_name().unwrap_or_default().to_os_string();
    partial_file_name.push(".partial");
    destination.with_file_name(partial_file_name)
}

/// Extracts the entries for which `keep_entry(entry_name)` is true into
/// `<destination_dir>.partial`, then renames it to `destination_dir`, so a crash
/// or full disk midway never leaves a half-filled folder that looks finished.
/// `flatten` drops the entries' folders.
pub fn extract_zip_entries(
    zip_path: &Path,
    destination_dir: &Path,
    keep_entry: impl Fn(&str) -> bool,
    flatten: bool,
) -> Result<(), String> {
    let partial_destination_dir = partial_path_for(destination_dir);
    let _ = std::fs::remove_dir_all(&partial_destination_dir);
    extract_zip_entries_into(zip_path, &partial_destination_dir, keep_entry, flatten)?;
    let _ = std::fs::remove_dir_all(destination_dir);
    std::fs::rename(&partial_destination_dir, destination_dir)
        .map_err(|error| format!("cannot move {} into place: {error}", partial_destination_dir.display()))
}

fn extract_zip_entries_into(
    zip_path: &Path,
    destination_dir: &Path,
    keep_entry: impl Fn(&str) -> bool,
    flatten: bool,
) -> Result<(), String> {
    let zip_file = File::open(zip_path).map_err(|error| format!("cannot open {}: {error}", zip_path.display()))?;
    let mut zip_archive = zip::ZipArchive::new(zip_file).map_err(|error| format!("cannot read {}: {error}", zip_path.display()))?;
    std::fs::create_dir_all(destination_dir).map_err(|error| format!("cannot create {}: {error}", destination_dir.display()))?;

    for entry_index in 0..zip_archive.len() {
        let mut zip_entry = zip_archive.by_index(entry_index).map_err(|error| format!("bad zip entry: {error}"))?;
        if zip_entry.is_dir() {
            continue;
        }
        let entry_name = zip_entry.name().map_err(|error| format!("bad zip entry name: {error}"))?.to_string();
        if !keep_entry(&entry_name) {
            continue;
        }
        // enclosed_name rejects absolute paths and `..`, so a hostile archive cannot write outside destination_dir.
        let Some(safe_relative_path) = zip_entry.enclosed_name() else {
            continue;
        };
        let output_path = if flatten {
            match safe_relative_path.file_name() {
                Some(file_name) => destination_dir.join(file_name),
                None => continue,
            }
        } else {
            destination_dir.join(&safe_relative_path)
        };
        if let Some(parent_directory) = output_path.parent() {
            std::fs::create_dir_all(parent_directory).map_err(|error| format!("cannot create {}: {error}", parent_directory.display()))?;
        }
        let mut output_file = File::create(&output_path).map_err(|error| format!("cannot create {}: {error}", output_path.display()))?;
        std::io::copy(&mut zip_entry, &mut output_file).map_err(|error| format!("cannot extract {entry_name}: {error}"))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh_temporary_directory(test_name: &str) -> PathBuf {
        let directory = std::env::temp_dir().join(format!("hellobhai-downloads-test-{test_name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).unwrap();
        directory
    }

    #[test]
    fn existing_file_with_the_expected_size_is_not_downloaded_again() {
        let directory = fresh_temporary_directory("skip");
        let destination = directory.join("model.bin");
        std::fs::write(&destination, b"12345").unwrap();

        let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        // An unroutable URL: reaching the network would fail the test.
        let result = runtime.block_on(download_file_if_missing("http://127.0.0.1:9/never", &destination, 5, "unused", |_| {}));
        assert_eq!(result, Ok(()));
        assert_eq!(std::fs::read(&destination).unwrap(), b"12345");
    }

    #[test]
    fn plain_http_downloads_are_refused() {
        let directory = fresh_temporary_directory("http");
        let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let result = runtime.block_on(download_file_if_missing("http://127.0.0.1:9/never", &directory.join("x.bin"), 5, "unused", |_| {}));
        assert!(result.is_err());
    }

    #[test]
    fn sha256_is_lowercase_hex() {
        assert_eq!(hex_string(&Sha256::digest(b"abc")), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    }

    #[test]
    fn extract_keeps_only_matching_entries_and_flattens_folders() {
        let directory = fresh_temporary_directory("zip");
        let zip_path = directory.join("archive.zip");
        {
            let mut zip_writer = zip::ZipWriter::new(File::create(&zip_path).unwrap());
            let options = zip::write::SimpleFileOptions::default();
            zip_writer.start_file("Release/whisper-server.exe", options).unwrap();
            zip_writer.write_all(b"exe").unwrap();
            zip_writer.start_file("Release/readme.txt", options).unwrap();
            zip_writer.write_all(b"txt").unwrap();
            zip_writer.finish().unwrap();
        }
        let output_directory = directory.join("out");
        extract_zip_entries(&zip_path, &output_directory, |name| name.ends_with(".exe"), true).unwrap();
        assert_eq!(std::fs::read(output_directory.join("whisper-server.exe")).unwrap(), b"exe");
        assert!(!output_directory.join("readme.txt").exists());
        assert!(!output_directory.join("Release").exists());
    }
}
