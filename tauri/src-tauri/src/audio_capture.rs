//! Microphone capture for push-to-talk with cpal (WASAPI on Windows). The cpal
//! stream is !Send, so it lives on its own thread driven by a command channel.

use std::sync::mpsc::{channel, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample};

const WHISPER_SAMPLE_RATE: u32 = 16_000;
const AUDIO_LEVEL_EMIT_INTERVAL: Duration = Duration::from_millis(50);

enum RecorderCommand {
    Start { reply: Sender<Result<(), String>> },
    Stop { reply: Sender<Vec<f32>> },
}

/// What the cpal callback writes into: the clip so far (mono, device rate) and the waveform level.
struct RecordingBuffer {
    mono_samples_at_device_rate: Vec<f32>,
    device_sample_rate: u32,
    smoothed_audio_level: f32,
    last_audio_level_emit: Instant,
}

/// Records from the default input device between start and stop.
pub struct PushToTalkRecorder {
    command_sender: Sender<RecorderCommand>,
}

impl PushToTalkRecorder {
    /// `on_audio_level` gets an RMS level 0..1 about 20 times a second, for the waveform.
    pub fn new(on_audio_level: Box<dyn Fn(f32) + Send>) -> Self {
        let (command_sender, command_receiver) = channel::<RecorderCommand>();
        let on_audio_level: Arc<Mutex<Box<dyn Fn(f32) + Send>>> = Arc::new(Mutex::new(on_audio_level));

        std::thread::Builder::new()
            .name("push-to-talk-recorder".into())
            .spawn(move || {
                let recording_buffer = Arc::new(Mutex::new(RecordingBuffer {
                    mono_samples_at_device_rate: Vec::new(),
                    device_sample_rate: WHISPER_SAMPLE_RATE,
                    smoothed_audio_level: 0.0,
                    last_audio_level_emit: Instant::now(),
                }));
                let mut active_stream: Option<cpal::Stream> = None;

                for command in command_receiver {
                    match command {
                        RecorderCommand::Start { reply } => {
                            drop(active_stream.take());
                            let start_result = open_default_input_stream(recording_buffer.clone(), on_audio_level.clone());
                            let reply_result = start_result.map(|stream| {
                                active_stream = Some(stream);
                            });
                            let _ = reply.send(reply_result);
                        }
                        RecorderCommand::Stop { reply } => {
                            // Dropping the stream stops capture before the clip is taken.
                            drop(active_stream.take());
                            let mut buffer = recording_buffer.lock().unwrap();
                            let recorded_samples = std::mem::take(&mut buffer.mono_samples_at_device_rate);
                            buffer.smoothed_audio_level = 0.0;
                            let clip_16k = resample_mono_linear(&recorded_samples, buffer.device_sample_rate, WHISPER_SAMPLE_RATE);
                            drop(buffer);
                            (on_audio_level.lock().unwrap())(0.0);
                            let _ = reply.send(clip_16k);
                        }
                    }
                }
            })
            .expect("cannot spawn the recorder thread");

        PushToTalkRecorder { command_sender }
    }

    /// Err when there is no input device or the microphone is blocked in Windows privacy settings.
    pub fn start_recording(&self) -> Result<(), String> {
        let (reply_sender, reply_receiver) = channel();
        self.command_sender
            .send(RecorderCommand::Start { reply: reply_sender })
            .map_err(|_| "the recorder thread stopped".to_string())?;
        reply_receiver.recv().map_err(|_| "the recorder thread stopped".to_string())?
    }

    /// Mono samples resampled to 16 000 Hz.
    pub fn stop_recording(&self) -> Vec<f32> {
        let (reply_sender, reply_receiver) = channel();
        if self.command_sender.send(RecorderCommand::Stop { reply: reply_sender }).is_err() {
            return Vec::new();
        }
        reply_receiver.recv().unwrap_or_default()
    }
}

fn open_default_input_stream(
    recording_buffer: Arc<Mutex<RecordingBuffer>>,
    on_audio_level: Arc<Mutex<Box<dyn Fn(f32) + Send>>>,
) -> Result<cpal::Stream, String> {
    let input_device = cpal::default_host()
        .default_input_device()
        .ok_or_else(|| "no microphone found".to_string())?;
    let supported_config = input_device
        .default_input_config()
        .map_err(|error| format!("cannot open the microphone (check the microphone privacy settings): {error}"))?;

    {
        let mut buffer = recording_buffer.lock().unwrap();
        buffer.mono_samples_at_device_rate.clear();
        buffer.device_sample_rate = supported_config.sample_rate();
        buffer.smoothed_audio_level = 0.0;
    }

    let sample_format = supported_config.sample_format();
    let stream_config: cpal::StreamConfig = supported_config.into();
    let stream = match sample_format {
        SampleFormat::F32 => build_input_stream::<f32>(&input_device, &stream_config, recording_buffer, on_audio_level),
        SampleFormat::I16 => build_input_stream::<i16>(&input_device, &stream_config, recording_buffer, on_audio_level),
        SampleFormat::U16 => build_input_stream::<u16>(&input_device, &stream_config, recording_buffer, on_audio_level),
        SampleFormat::I32 => build_input_stream::<i32>(&input_device, &stream_config, recording_buffer, on_audio_level),
        other_format => return Err(format!("unsupported microphone sample format {other_format}")),
    }?;
    stream
        .play()
        .map_err(|error| format!("cannot start the microphone (check the microphone privacy settings): {error}"))?;
    Ok(stream)
}

fn build_input_stream<SampleType>(
    input_device: &cpal::Device,
    stream_config: &cpal::StreamConfig,
    recording_buffer: Arc<Mutex<RecordingBuffer>>,
    on_audio_level: Arc<Mutex<Box<dyn Fn(f32) + Send>>>,
) -> Result<cpal::Stream, String>
where
    SampleType: SizedSample,
    f32: FromSample<SampleType>,
{
    let channel_count = stream_config.channels.max(1) as usize;
    input_device
        .build_input_stream::<SampleType, _, _>(
            *stream_config,
            move |interleaved_samples: &[SampleType], _callback_info| {
                let mono_samples: Vec<f32> = interleaved_samples
                    .chunks(channel_count)
                    .map(|frame| frame.iter().map(|sample| sample.to_sample::<f32>()).sum::<f32>() / frame.len() as f32)
                    .collect();
                if mono_samples.is_empty() {
                    return;
                }

                // Same level math as the Mac's updateAudioPowerLevel: RMS boosted x10.2,
                // clamped, and decayed by 0.72 so the waveform falls smoothly.
                let summed_squares: f32 = mono_samples.iter().map(|sample| sample * sample).sum();
                let root_mean_square = (summed_squares / mono_samples.len() as f32).sqrt();
                let boosted_level = (root_mean_square * 10.2).clamp(0.0, 1.0);

                let mut buffer = recording_buffer.lock().unwrap();
                buffer.mono_samples_at_device_rate.extend_from_slice(&mono_samples);
                buffer.smoothed_audio_level = boosted_level.max(buffer.smoothed_audio_level * 0.72);
                if buffer.last_audio_level_emit.elapsed() >= AUDIO_LEVEL_EMIT_INTERVAL {
                    buffer.last_audio_level_emit = Instant::now();
                    let level_to_emit = buffer.smoothed_audio_level;
                    drop(buffer);
                    (on_audio_level.lock().unwrap())(level_to_emit);
                }
            },
            |stream_error| eprintln!("microphone stream error: {stream_error}"),
            None,
        )
        .map_err(|error| format!("cannot open the microphone (check the microphone privacy settings): {error}"))
}

/// Linear-interpolation resampler for mono audio. When downsampling (48 or 44.1 kHz
/// to 16 kHz) a low-pass filter runs first, so everything above the new Nyquist
/// frequency is removed instead of folding back into the speech band as noise.
pub fn resample_mono_linear(samples: &[f32], from_rate: u32, to_rate: u32) -> Vec<f32> {
    if samples.is_empty() || from_rate == 0 || to_rate == 0 {
        return Vec::new();
    }
    if from_rate == to_rate {
        return samples.to_vec();
    }
    let low_passed_samples;
    let samples = if from_rate > to_rate {
        // Cutoff a little under the new Nyquist (8 kHz for 16 kHz) to leave room for the filter's slope.
        let cutoff_hz = to_rate as f64 * 0.45;
        low_passed_samples = low_pass_filter(samples, cutoff_hz / from_rate as f64);
        &low_passed_samples
    } else {
        samples
    };
    let output_length = (samples.len() as u64 * to_rate as u64 / from_rate as u64) as usize;
    let source_step = from_rate as f64 / to_rate as f64;
    (0..output_length)
        .map(|output_index| {
            let source_position = output_index as f64 * source_step;
            let left_index = source_position.floor() as usize;
            let right_index = (left_index + 1).min(samples.len() - 1);
            let fraction = (source_position - left_index as f64) as f32;
            samples[left_index] + (samples[right_index] - samples[left_index]) * fraction
        })
        .collect()
}

/// Blackman-windowed sinc FIR low-pass with unity gain at 0 Hz. `cutoff_fraction`
/// is the cutoff divided by the sample rate. Edges repeat the first/last sample.
fn low_pass_filter(samples: &[f32], cutoff_fraction: f64) -> Vec<f32> {
    // 65 taps: about 74 dB stopband (Blackman) after a ~4 kHz slope at 48 kHz.
    const HALF_TAP_COUNT: usize = 32;
    let tap_count = 2 * HALF_TAP_COUNT + 1;
    let mut filter_taps: Vec<f64> = (0..tap_count)
        .map(|tap_index| {
            let offset_from_center = tap_index as f64 - HALF_TAP_COUNT as f64;
            let sinc = if offset_from_center == 0.0 {
                2.0 * cutoff_fraction
            } else {
                (2.0 * std::f64::consts::PI * cutoff_fraction * offset_from_center).sin() / (std::f64::consts::PI * offset_from_center)
            };
            let window_phase = 2.0 * std::f64::consts::PI * tap_index as f64 / (tap_count - 1) as f64;
            let blackman_window = 0.42 - 0.5 * window_phase.cos() + 0.08 * (2.0 * window_phase).cos();
            sinc * blackman_window
        })
        .collect();
    let taps_sum: f64 = filter_taps.iter().sum();
    filter_taps.iter_mut().for_each(|tap| *tap /= taps_sum);

    let last_index = samples.len() as isize - 1;
    (0..samples.len() as isize)
        .map(|sample_index| {
            let filtered_sample: f64 = filter_taps
                .iter()
                .enumerate()
                .map(|(tap_index, tap)| {
                    let source_index = (sample_index + tap_index as isize - HALF_TAP_COUNT as isize).clamp(0, last_index);
                    tap * samples[source_index as usize] as f64
                })
                .sum();
            filtered_sample as f32
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root_mean_square(samples: &[f32]) -> f32 {
        (samples.iter().map(|sample| sample * sample).sum::<f32>() / samples.len() as f32).sqrt()
    }

    fn sine_wave(frequency_hz: f32, sample_rate: u32, sample_count: usize) -> Vec<f32> {
        (0..sample_count)
            .map(|index| (2.0 * std::f32::consts::PI * frequency_hz * index as f32 / sample_rate as f32).sin())
            .collect()
    }

    #[test]
    fn resampling_48k_to_16k_keeps_speech_and_drops_what_would_alias() {
        // 1 kHz speech-band tone survives at full level.
        let speech_tone = resample_mono_linear(&sine_wave(1_000.0, 48_000, 48_000), 48_000, 16_000);
        assert_eq!(speech_tone.len(), 16_000);
        assert!((root_mean_square(&speech_tone[1_000..15_000]) - 0.707).abs() < 0.02);
        // 12 kHz would fold back to 4 kHz without the filter; it must be nearly gone.
        let hiss = resample_mono_linear(&sine_wave(12_000.0, 48_000, 48_000), 48_000, 16_000);
        assert!(root_mean_square(&hiss[1_000..15_000]) < 0.01);
        // A constant (DC) level passes unchanged, edges included.
        let constant_level = resample_mono_linear(&[0.25; 4_410], 44_100, 16_000);
        assert!(constant_level.iter().all(|sample| (sample - 0.25).abs() < 1e-4));
    }

    #[test]
    fn upsampling_interpolates_between_neighbours() {
        let resampled = resample_mono_linear(&[0.0, 1.0], 8_000, 16_000);
        assert_eq!(resampled, vec![0.0, 0.5, 1.0, 1.0]);
    }

    #[test]
    fn empty_and_same_rate_inputs() {
        assert!(resample_mono_linear(&[], 44_100, 16_000).is_empty());
        assert_eq!(resample_mono_linear(&[0.25, 0.5], 16_000, 16_000), vec![0.25, 0.5]);
    }
}
