//! First-launch downloads (whisper-server, Whisper model, ONNX Runtime,
//! Supertonic models and voices) with progress, plus zip (Windows) and .tar.gz
//! (Linux) extraction. Used by the speech-to-text and text-to-speech modules.

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
    extract_into_partial_folder_then_rename(destination_dir, |partial_destination_dir| {
        extract_zip_entries_into(zip_path, partial_destination_dir, keep_entry, flatten)
    })
}

/// `extract_zip_entries` for a `.zip`, `extract_tar_gz_entries` for a `.tar.gz`
/// or `.tgz` (the Linux runtimes), picked by the archive's file name.
pub fn extract_archive_entries(
    archive_path: &Path,
    destination_dir: &Path,
    keep_entry: impl Fn(&str) -> bool,
    flatten: bool,
) -> Result<(), String> {
    let archive_file_name = archive_path.file_name().unwrap_or_default().to_string_lossy().to_ascii_lowercase();
    if archive_file_name.ends_with(".tar.gz") || archive_file_name.ends_with(".tgz") {
        extract_tar_gz_entries(archive_path, destination_dir, keep_entry, flatten)
    } else {
        extract_zip_entries(archive_path, destination_dir, keep_entry, flatten)
    }
}

/// Same contract as `extract_zip_entries`. Symbolic links are kept only when
/// they point at a plain file name in the same folder (the `libwhisper.so.1 ->
/// libwhisper.so.1.9.5` soname links whisper-server loads through its
/// `$ORIGIN` rpath); Unix permission bits are kept so executables stay executable.
pub fn extract_tar_gz_entries(
    tar_gz_path: &Path,
    destination_dir: &Path,
    keep_entry: impl Fn(&str) -> bool,
    flatten: bool,
) -> Result<(), String> {
    extract_into_partial_folder_then_rename(destination_dir, |partial_destination_dir| {
        extract_tar_gz_entries_into(tar_gz_path, partial_destination_dir, keep_entry, flatten)
    })
}

fn extract_into_partial_folder_then_rename(
    destination_dir: &Path,
    extract_into: impl FnOnce(&Path) -> Result<(), String>,
) -> Result<(), String> {
    let partial_destination_dir = partial_path_for(destination_dir);
    let _ = std::fs::remove_dir_all(&partial_destination_dir);
    extract_into(&partial_destination_dir)?;
    let _ = std::fs::remove_dir_all(destination_dir);
    std::fs::rename(&partial_destination_dir, destination_dir)
        .map_err(|error| format!("cannot move {} into place: {error}", partial_destination_dir.display()))
}

/// The entry's path relative to the archive root, or None when it is absolute
/// or climbs out with `..` (a hostile archive must not write outside the folder).
fn safe_relative_entry_path(entry_path: &Path) -> Option<PathBuf> {
    let mut safe_relative_path = PathBuf::new();
    for path_component in entry_path.components() {
        match path_component {
            std::path::Component::Normal(component_name) => safe_relative_path.push(component_name),
            std::path::Component::CurDir => {}
            _ => return None,
        }
    }
    if safe_relative_path.as_os_str().is_empty() {
        None
    } else {
        Some(safe_relative_path)
    }
}

fn extract_tar_gz_entries_into(
    tar_gz_path: &Path,
    destination_dir: &Path,
    keep_entry: impl Fn(&str) -> bool,
    flatten: bool,
) -> Result<(), String> {
    let tar_gz_file = File::open(tar_gz_path).map_err(|error| format!("cannot open {}: {error}", tar_gz_path.display()))?;
    let mut tar_archive = tar::Archive::new(flate2::read::GzDecoder::new(tar_gz_file));
    std::fs::create_dir_all(destination_dir).map_err(|error| format!("cannot create {}: {error}", destination_dir.display()))?;
    let tar_entries = tar_archive.entries().map_err(|error| format!("cannot read {}: {error}", tar_gz_path.display()))?;

    for tar_entry in tar_entries {
        let mut tar_entry = tar_entry.map_err(|error| format!("bad tar entry: {error}"))?;
        let entry_type = tar_entry.header().entry_type();
        if !entry_type.is_file() && !entry_type.is_symlink() {
            continue; // folders are created as needed; devices, fifos and hard links are never wanted
        }
        let entry_path = tar_entry.path().map_err(|error| format!("bad tar entry name: {error}"))?.into_owned();
        let entry_name = entry_path.to_string_lossy().into_owned();
        if !keep_entry(&entry_name) {
            continue;
        }
        let Some(safe_relative_path) = safe_relative_entry_path(&entry_path) else {
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

        if entry_type.is_symlink() {
            let link_target = tar_entry
                .link_name()
                .map_err(|error| format!("bad link in {entry_name}: {error}"))?
                .map(|link_target| link_target.into_owned());
            let Some(link_target) = link_target else {
                continue;
            };
            // Only a sibling file name: a link to "/etc/..." or "../x" could make a later write escape.
            let is_sibling_file_name = link_target.components().count() == 1
                && matches!(link_target.components().next(), Some(std::path::Component::Normal(_)));
            if is_sibling_file_name {
                create_symbolic_link(&link_target, &output_path)?;
            }
            continue;
        }

        // A link of the same name from earlier in the archive must not redirect this write.
        let _ = std::fs::remove_file(&output_path);
        let mut output_file = File::create(&output_path).map_err(|error| format!("cannot create {}: {error}", output_path.display()))?;
        std::io::copy(&mut tar_entry, &mut output_file).map_err(|error| format!("cannot extract {entry_name}: {error}"))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            // rwx bits only: setuid/setgid/sticky from an archive are never applied.
            let permission_bits = tar_entry.header().mode().unwrap_or(0o644) & 0o777;
            std::fs::set_permissions(&output_path, std::fs::Permissions::from_mode(permission_bits))
                .map_err(|error| format!("cannot set permissions on {}: {error}", output_path.display()))?;
        }
    }
    Ok(())
}

#[cfg(unix)]
fn create_symbolic_link(link_target: &Path, link_path: &Path) -> Result<(), String> {
    let _ = std::fs::remove_file(link_path);
    std::os::unix::fs::symlink(link_target, link_path).map_err(|error| format!("cannot create link {}: {error}", link_path.display()))
}

/// Tar archives are only used on Linux; elsewhere their links are skipped.
#[cfg(not(unix))]
fn create_symbolic_link(_link_target: &Path, _link_path: &Path) -> Result<(), String> {
    Ok(())
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

    fn append_tar_file(tar_builder: &mut tar::Builder<flate2::write::GzEncoder<File>>, path: &str, contents: &[u8], mode: u32) {
        let mut tar_header = tar::Header::new_gnu();
        tar_header.set_size(contents.len() as u64);
        tar_header.set_mode(mode);
        tar_header.set_entry_type(tar::EntryType::Regular);
        tar_builder.append_data(&mut tar_header, path, contents).unwrap();
    }

    fn append_tar_symlink(tar_builder: &mut tar::Builder<flate2::write::GzEncoder<File>>, path: &str, link_target: &str) {
        let mut tar_header = tar::Header::new_gnu();
        tar_header.set_size(0);
        tar_header.set_entry_type(tar::EntryType::Symlink);
        tar_builder.append_link(&mut tar_header, path, link_target).unwrap();
    }

    #[test]
    fn tar_gz_extract_keeps_matching_files_sibling_links_and_modes() {
        let directory = fresh_temporary_directory("targz");
        let tar_gz_path = directory.join("whisper-bin-ubuntu-x64.tar.gz");
        {
            let gzip_encoder = flate2::write::GzEncoder::new(File::create(&tar_gz_path).unwrap(), flate2::Compression::fast());
            let mut tar_builder = tar::Builder::new(gzip_encoder);
            append_tar_file(&mut tar_builder, "whisper-bin-ubuntu-x64/whisper-server", b"elf", 0o4755);
            append_tar_file(&mut tar_builder, "whisper-bin-ubuntu-x64/libwhisper.so.1.9.5", b"so", 0o755);
            append_tar_symlink(&mut tar_builder, "whisper-bin-ubuntu-x64/libwhisper.so.1", "libwhisper.so.1.9.5");
            append_tar_symlink(&mut tar_builder, "whisper-bin-ubuntu-x64/libescape.so", "../../etc/passwd");
            append_tar_file(&mut tar_builder, "whisper-bin-ubuntu-x64/README.md", b"txt", 0o644);
            tar_builder.into_inner().unwrap().finish().unwrap();
        }
        let output_directory = directory.join("out");
        extract_archive_entries(&tar_gz_path, &output_directory, |name| name.ends_with("/whisper-server") || name.contains(".so"), true)
            .unwrap();
        assert_eq!(std::fs::read(output_directory.join("whisper-server")).unwrap(), b"elf");
        assert!(!output_directory.join("README.md").exists());
        assert!(std::fs::symlink_metadata(output_directory.join("libescape.so")).is_err());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let server_mode = std::fs::metadata(output_directory.join("whisper-server")).unwrap().permissions().mode();
            assert_eq!(server_mode & 0o7777, 0o755, "setuid is dropped, rwx kept");
            assert_eq!(std::fs::read(output_directory.join("libwhisper.so.1")).unwrap(), b"so");
        }
    }

    #[test]
    fn archive_entry_paths_that_escape_are_rejected() {
        assert_eq!(safe_relative_entry_path(Path::new("a/./b.so")), Some(PathBuf::from("a/b.so")));
        assert_eq!(safe_relative_entry_path(Path::new("../b.so")), None);
        assert_eq!(safe_relative_entry_path(Path::new("/etc/passwd")), None);
        assert_eq!(safe_relative_entry_path(Path::new(".")), None);
    }
}
