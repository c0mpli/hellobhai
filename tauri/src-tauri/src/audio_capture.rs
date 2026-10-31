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

/// Linear-interpolation resampler for mono audio.
pub fn resample_mono_linear(samples: &[f32], from_rate: u32, to_rate: u32) -> Vec<f32> {
    if samples.is_empty() || from_rate == 0 || to_rate == 0 {
        return Vec::new();
    }
    if from_rate == to_rate {
        return samples.to_vec();
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resampling_48k_to_16k_keeps_every_third_sample_of_a_ramp() {
        let ramp: Vec<f32> = (0..48).map(|index| index as f32).collect();
        let resampled = resample_mono_linear(&ramp, 48_000, 16_000);
        assert_eq!(resampled.len(), 16);
        assert_eq!(resampled[0], 0.0);
        assert_eq!(resampled[5], 15.0);
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
