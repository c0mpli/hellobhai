//! Microphone capture for push-to-talk with cpal (WASAPI on Windows). The cpal
//! stream is !Send, so it lives on its own thread driven by a command channel.

/// Records from the default input device between start and stop.
pub struct PushToTalkRecorder {}

impl PushToTalkRecorder {
    /// `on_audio_level` gets an RMS level 0..1 about 20 times a second, for the waveform.
    pub fn new(on_audio_level: Box<dyn Fn(f32) + Send>) -> Self {
        todo!("A: spawn the capture thread")
    }

    /// Err when there is no input device or the microphone is blocked in Windows privacy settings.
    pub fn start_recording(&self) -> Result<(), String> {
        todo!("A: start the cpal stream")
    }

    /// Mono samples resampled to 16 000 Hz.
    pub fn stop_recording(&self) -> Vec<f32> {
        todo!("A: stop and return the clip")
    }
}

/// Linear-interpolation resampler for mono audio.
pub fn resample_mono_linear(samples: &[f32], from_rate: u32, to_rate: u32) -> Vec<f32> {
    todo!("A: linear resample")
}
