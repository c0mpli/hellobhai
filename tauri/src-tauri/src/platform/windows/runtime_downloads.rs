//! Prebuilt Windows runtimes fetched on first launch, pinned to fixed releases
//! (sizes and SHA-256 from the GitHub release asset digests).

/// A prebuilt native runtime fetched on first launch (zip archive).
#[derive(Clone, Copy, Debug)]
pub struct RuntimeDownload {
    pub url: &'static str,
    pub size_bytes: u64,
    /// Checked before the download is moved into place.
    pub sha256: &'static str,
    pub executable_or_library_file_name: &'static str,
}

/// whisper.cpp b5454 `whisper-bin-x64.zip`: keep `Release/whisper-server.exe` and every `Release/*.dll`.
pub fn whisper_server_download() -> Option<RuntimeDownload> {
    Some(RuntimeDownload {
        url: "https://github.com/ggml-org/whisper.cpp/releases/download/b5454/whisper-bin-x64.zip",
        size_bytes: 8_928_640,
        sha256: "6ba69e3482d7826214f90a6a9c84ca07782aec1e1d0c6a7c30c994fd5d816ccb",
        executable_or_library_file_name: "whisper-server.exe",
    })
}

/// ONNX Runtime 1.31.1: keep `*/lib/onnxruntime.dll` and `*/lib/onnxruntime_providers_shared.dll`.
pub fn onnxruntime_download() -> Option<RuntimeDownload> {
    Some(RuntimeDownload {
        url: "https://github.com/microsoft/onnxruntime/releases/download/v1.31.1/onnxruntime-win-x64-1.31.1.zip",
        size_bytes: 82_929_944,
        sha256: "5869e26bb25314375f2d5ef1877e3ea06b1543d6c710e6e4a1ec2f7177ab4b2c",
        executable_or_library_file_name: "onnxruntime.dll",
    })
}
