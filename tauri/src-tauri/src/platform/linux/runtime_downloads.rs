//! Prebuilt Linux x64 runtimes fetched on first launch, pinned to the same
//! releases as Windows (sizes and SHA-256 from the GitHub release asset digests).

/// A prebuilt native runtime fetched on first launch (zip or .tar.gz archive).
#[derive(Clone, Copy, Debug)]
pub struct RuntimeDownload {
    pub url: &'static str,
    pub size_bytes: u64,
    /// Checked before the download is moved into place.
    pub sha256: &'static str,
    pub executable_or_library_file_name: &'static str,
    /// Saved under this name next to the models; its extension picks zip or tar.gz extraction.
    pub archive_file_name: &'static str,
    /// Which archive entries to keep (extracted flat into one folder).
    pub keep_archive_entry: fn(&str) -> bool,
}

/// whisper.cpp b5454 `whisper-bin-ubuntu-x64.tar.gz`: keep `whisper-server` and the
/// libwhisper/libggml shared libraries (with their soname links) it loads
/// through its `$ORIGIN` rpath, including the per-CPU `libggml-cpu-*.so` backends.
pub fn whisper_server_download() -> Option<RuntimeDownload> {
    Some(RuntimeDownload {
        url: "https://github.com/ggml-org/whisper.cpp/releases/download/b5454/whisper-bin-ubuntu-x64.tar.gz",
        size_bytes: 10_364_195,
        sha256: "a72becf15d7917f990f6313867a52638b82b7f9ef237fb0c980dac56a135781c",
        executable_or_library_file_name: "whisper-server",
        archive_file_name: "whisper-bin-ubuntu-x64.tar.gz",
        keep_archive_entry: keep_whisper_server_archive_entry,
    })
}

fn keep_whisper_server_archive_entry(entry_name: &str) -> bool {
    let file_name = entry_name.rsplit('/').next().unwrap_or(entry_name);
    file_name == "whisper-server" || (file_name.contains(".so") && (file_name.starts_with("libwhisper") || file_name.starts_with("libggml")))
}

/// ONNX Runtime 1.31.1 `onnxruntime-linux-x64-1.31.1.tgz`: keep only the real
/// library; it is loaded by absolute path, so the soname links are not needed.
pub fn onnxruntime_download() -> Option<RuntimeDownload> {
    Some(RuntimeDownload {
        url: "https://github.com/microsoft/onnxruntime/releases/download/v1.31.1/onnxruntime-linux-x64-1.31.1.tgz",
        size_bytes: 11_413_655,
        sha256: "c849e36a1ef3e22d4ff217b9d1ce4a3cf5b9467b975610913ae959ad5b16bdc9",
        executable_or_library_file_name: "libonnxruntime.so.1.31.1",
        archive_file_name: "onnxruntime-linux-x64-1.31.1.tgz",
        keep_archive_entry: |entry_name| entry_name.ends_with("/lib/libonnxruntime.so.1.31.1"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whisper_archive_keeps_the_server_and_its_libraries_only() {
        let keep = whisper_server_download().unwrap().keep_archive_entry;
        assert!(keep("whisper-bin-ubuntu-x64/whisper-server"));
        assert!(keep("whisper-bin-ubuntu-x64/libwhisper.so.1"));
        assert!(keep("whisper-bin-ubuntu-x64/libggml-base.so.0.26.0"));
        assert!(keep("whisper-bin-ubuntu-x64/libggml-cpu-haswell.so"));
        assert!(!keep("whisper-bin-ubuntu-x64/whisper-cli"));
        assert!(!keep("whisper-bin-ubuntu-x64/libparakeet.so.1"));
        assert!(!keep("whisper-bin-ubuntu-x64/LICENSE"));
    }

    #[test]
    fn onnxruntime_archive_keeps_the_real_library_only() {
        let keep = onnxruntime_download().unwrap().keep_archive_entry;
        assert!(keep("onnxruntime-linux-x64-1.31.1/lib/libonnxruntime.so.1.31.1"));
        assert!(!keep("onnxruntime-linux-x64-1.31.1/lib/libonnxruntime.so"));
        assert!(!keep("onnxruntime-linux-x64-1.31.1/lib/libonnxruntime_providers_shared.so"));
    }
}
