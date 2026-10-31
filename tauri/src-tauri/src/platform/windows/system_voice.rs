//! Fallback voice until Supertonic is loaded or when it fails:
//! WinRT `Windows.Media.SpeechSynthesis.SpeechSynthesizer`.

use windows::core::HSTRING;
use windows::Media::SpeechSynthesis::SpeechSynthesizer;
use windows::Storage::Streams::{Buffer, DataReader, InputStreamOptions};

use crate::platform::PlatformError;

/// WAV bytes. Picks a voice whose language starts with "hi" when `prefer_hindi_voice`.
pub fn synthesize_with_system_voice(text: &str, prefer_hindi_voice: bool) -> Result<Vec<u8>, PlatformError> {
    synthesize(text, prefer_hindi_voice).map_err(|error| PlatformError::Failed(error.message()))
}

// WinRT calls need no explicit init: windows-rs joins the MTA on first use.
fn synthesize(text: &str, prefer_hindi_voice: bool) -> windows::core::Result<Vec<u8>> {
    let speech_synthesizer = SpeechSynthesizer::new()?;
    if prefer_hindi_voice {
        let installed_voices = SpeechSynthesizer::AllVoices()?;
        for voice_index in 0..installed_voices.Size()? {
            let installed_voice = installed_voices.GetAt(voice_index)?;
            if installed_voice.Language()?.to_string().to_lowercase().starts_with("hi") {
                speech_synthesizer.SetVoice(&installed_voice)?;
                break;
            }
        }
    }
    let speech_stream = speech_synthesizer.SynthesizeTextToStreamAsync(&HSTRING::from(text))?.join()?;
    let stream_size = speech_stream.Size()? as u32;
    let read_buffer = Buffer::Create(stream_size)?;
    let filled_buffer = speech_stream.ReadAsync(&read_buffer, stream_size, InputStreamOptions::None)?.join()?;
    let mut wav_bytes = vec![0u8; filled_buffer.Length()? as usize];
    DataReader::FromBuffer(&filled_buffer)?.ReadBytes(&mut wav_bytes)?;
    Ok(wav_bytes)
}
