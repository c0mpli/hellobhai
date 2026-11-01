//! Fallback voice until Supertonic is loaded or when it fails: espeak-ng (or
//! espeak), which most distributions install with speech-dispatcher and which
//! writes WAV to stdout. Without it, bhai stays quiet until Supertonic is ready.

use std::process::{Command, Stdio};

use crate::platform::PlatformError;

/// WAV bytes. Uses espeak's Hindi voice when `prefer_hindi_voice`.
pub fn synthesize_with_system_voice(text: &str, prefer_hindi_voice: bool) -> Result<Vec<u8>, PlatformError> {
    let voice_name = if prefer_hindi_voice { "hi" } else { "en-us" };
    let mut last_error = PlatformError::Failed("no system voice: install espeak-ng".into());
    for espeak_program in ["espeak-ng", "espeak"] {
        // "--" so a reply starting with "-" is never read as an option.
        let espeak_result = Command::new(espeak_program)
            .args(["-v", voice_name, "--stdout", "--", text])
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output();
        match espeak_result {
            Ok(espeak_output) if espeak_output.status.success() && !espeak_output.stdout.is_empty() => return Ok(espeak_output.stdout),
            Ok(espeak_output) => last_error = PlatformError::Failed(format!("{espeak_program} failed ({})", espeak_output.status)),
            Err(_not_installed) => {}
        }
    }
    Err(last_error)
}
