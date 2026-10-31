//! Fallback voice until Supertonic is loaded or when it fails:
//! WinRT `Windows.Media.SpeechSynthesis.SpeechSynthesizer`.

use crate::platform::PlatformError;

/// WAV bytes. Picks a voice whose language starts with "hi" when `prefer_hindi_voice`.
pub fn synthesize_with_system_voice(text: &str, prefer_hindi_voice: bool) -> Result<Vec<u8>, PlatformError> {
    todo!("B: SpeechSynthesizer::SynthesizeTextToStreamAsync")
}
