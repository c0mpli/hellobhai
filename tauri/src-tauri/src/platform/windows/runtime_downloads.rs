//! Prebuilt Windows runtimes fetched on first launch, pinned to fixed releases.

/// A prebuilt native runtime fetched on first launch (zip archive).
#[derive(Clone, Copy, Debug)]
pub struct RuntimeDownload {
    pub url: &'static str,
    pub size_bytes: u64,
    pub executable_or_library_file_name: &'static str,
}

/// whisper.cpp b5454 `whisper-bin-x64.zip`: keep `Release/whisper-server.exe` and every `Release/*.dll`.
pub fn whisper_server_download() -> Option<RuntimeDownload> {
    Some(RuntimeDownload {
        url: "https://github.com/ggml-org/whisper.cpp/releases/download/b5454/whisper-bin-x64.zip",
        size_bytes: 8_928_640,
        executable_or_library_file_name: "whisper-server.exe",
    })
}

/// ONNX Runtime 1.31.1: keep `*/lib/onnxruntime.dll` and `*/lib/onnxruntime_providers_shared.dll`.
pub fn onnxruntime_download() -> Option<RuntimeDownload> {
    Some(RuntimeDownload {
        url: "https://github.com/microsoft/onnxruntime/releases/download/v1.31.1/onnxruntime-win-x64-1.31.1.zip",
        size_bytes: 82_929_944,
        executable_or_library_file_name: "onnxruntime.dll",
    })
}
