/// A prebuilt native runtime fetched on first launch (zip archive).
#[derive(Clone, Copy, Debug)]
pub struct RuntimeDownload {
    pub url: &'static str,
    pub size_bytes: u64,
    pub sha256: &'static str,
    pub executable_or_library_file_name: &'static str,
    pub archive_file_name: &'static str,
    pub keep_archive_entry: fn(&str) -> bool,
}

pub fn whisper_server_download() -> Option<RuntimeDownload> {
    None
}

pub fn onnxruntime_download() -> Option<RuntimeDownload> {
    None
}
