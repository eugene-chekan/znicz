use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, Stream, StreamConfig, SupportedStreamConfigRange};
use rtrb::{Consumer, Producer};

use crate::audio::buffer;
use crate::audio::feeder::SampleSink;
use crate::audio::gain::{GainControl, GainRamp, VOLUME_RAMP};
use crate::error::{Result, ZniczError};
use crate::player::state::AudioDeviceInfo;

pub struct AudioOutput {
    stream: Option<Stream>,
    producer: Option<Producer<f32>>,
    sample_rate: u32,
    channels: u16,
    sample_format: Option<String>,
    device_name: Option<String>,
    paused: Arc<AtomicBool>,
    gain: Arc<GainControl>,
    fade: Arc<GainControl>,
    flush: Arc<AtomicBool>,
}

/// The two gains the callback applies: the user's volume and the fade.
struct Gains {
    volume: Arc<GainControl>,
    fade: Arc<GainControl>,
    volume_ramp: GainRamp,
    fade_ramp: GainRamp,
    sample_rate: u32,
}

impl Gains {
    fn new(volume: Arc<GainControl>, fade: Arc<GainControl>, sample_rate: u32) -> Self {
        let volume_ramp = GainRamp::following(&volume, sample_rate);
        let fade_ramp = GainRamp::following(&fade, sample_rate);
        Self {
            volume,
            fade,
            volume_ramp,
            fade_ramp,
            sample_rate,
        }
    }

    /// Pick up changes made since the last callback.
    fn follow(&mut self) {
        self.volume_ramp.follow(&self.volume, self.sample_rate);
        self.fade_ramp.follow(&self.fade, self.sample_rate);
    }

    /// The gain for the next frame: volume times fade.
    fn next_frame_gain(&mut self) -> f32 {
        self.volume_ramp.next_frame_gain() * self.fade_ramp.next_frame_gain()
    }

    /// Tell the engine which ramps reached their target.
    fn report_landed(&self) {
        self.volume_ramp.report_landed(&self.volume);
        self.fade_ramp.report_landed(&self.fade);
    }
}

impl Default for AudioOutput {
    fn default() -> Self {
        Self::new()
    }
}

impl AudioOutput {
    pub fn new() -> Self {
        Self {
            stream: None,
            producer: None,
            sample_rate: 44_100,
            channels: 2,
            sample_format: None,
            device_name: None,
            paused: Arc::new(AtomicBool::new(false)),
            gain: Arc::new(GainControl::new(1.0)),
            fade: Arc::new(GainControl::new(1.0)),
            flush: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn set_paused(&self, paused: bool) {
        self.paused.store(paused, Ordering::Relaxed);
    }

    /// Move the output gain to `target` over `ramp`.
    pub fn set_gain(&self, target: f32, ramp: Duration) {
        self.gain.set(target, ramp);
    }

    /// A volume change: a short ramp so it does not click.
    pub fn set_volume(&self, volume: f32) {
        self.set_gain(volume, VOLUME_RAMP);
    }

    /// Ramp the fade gain to `target` over `ramp`; returns the generation to
    /// wait for with `fade_landed`.
    pub fn fade_to(&self, target: f32, ramp: Duration) -> u64 {
        self.fade.set(target, ramp)
    }

    /// Fade in from silence over `ramp`.
    pub fn fade_in(&self, ramp: Duration) -> u64 {
        self.fade.set_from(0.0, 1.0, ramp)
    }

    /// Set the fade gain at once, without a ramp.
    pub fn set_fade_now(&self, gain: f32) {
        self.fade.set_from(gain, gain, Duration::ZERO);
    }

    /// The stream has reached the fade target set as `generation`.
    pub fn fade_landed(&self, generation: u64) -> bool {
        self.fade.has_landed(generation)
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    pub fn channels(&self) -> u16 {
        self.channels
    }

    pub fn device_name(&self) -> Option<&str> {
        self.device_name.as_deref()
    }

    /// Sample format of the open stream, such as `f32` or `i16`.
    pub fn sample_format(&self) -> Option<&str> {
        self.sample_format.as_deref()
    }

    /// Samples still waiting to be played. Used to show an honest position.
    pub fn queued_samples(&self) -> usize {
        match self.producer.as_ref() {
            Some(producer) => buffer::RING_BUFFER_SAMPLES.saturating_sub(producer.slots()),
            None => 0,
        }
    }

    /// Ask the audio callback to drop queued samples (used on seek/stop).
    pub fn request_flush(&self) {
        self.flush.store(true, Ordering::Release);
    }

    pub fn wait_flush(&self) {
        for _ in 0..80 {
            if !self.flush.load(Ordering::Acquire) {
                return;
            }
            thread::sleep(Duration::from_millis(1));
        }
        self.flush.store(false, Ordering::Release);
    }

    pub fn list_devices() -> Result<Vec<AudioDeviceInfo>> {
        let host = cpal::default_host();
        let default_name = host.default_output_device().and_then(|d| d.name().ok());

        let devices: Vec<AudioDeviceInfo> = host
            .output_devices()
            .map_err(|e| ZniczError::Audio(e.to_string()))?
            .filter_map(|device| {
                let name = device.name().ok()?;
                let id = name.clone();
                let is_default = default_name.as_ref() == Some(&name);
                Some(AudioDeviceInfo {
                    id,
                    name,
                    is_default,
                })
            })
            .collect();

        Ok(devices)
    }

    pub fn open_stream(
        &mut self,
        sample_rate: u32,
        channels: u16,
        device_id: Option<&str>,
    ) -> Result<()> {
        self.stop_stream();

        let host = cpal::default_host();
        let device = if let Some(id) = device_id {
            host.output_devices()
                .map_err(|e| ZniczError::Audio(e.to_string()))?
                .find(|d| d.name().ok().as_deref() == Some(id))
                .or_else(|| host.default_output_device())
        } else {
            host.default_output_device()
        };

        let device = device.ok_or_else(|| ZniczError::Audio("no output device".into()))?;
        let device_name = device.name().ok();

        let ranges: Vec<SupportedStreamConfigRange> = device
            .supported_output_configs()
            .map_err(|e| ZniczError::Audio(e.to_string()))?
            .collect();

        let (config, sample_format) = select_stream_config(&ranges, sample_rate, channels)
            .or_else(|| {
                device
                    .default_output_config()
                    .ok()
                    .map(|c| (c.config(), c.sample_format()))
            })
            .ok_or_else(|| ZniczError::Audio("no usable output config".into()))?;

        tracing::info!(
            device = device_name.as_deref().unwrap_or("unknown"),
            file_rate = sample_rate,
            stream_rate = config.sample_rate.0,
            file_channels = channels,
            stream_channels = config.channels,
            format = ?sample_format,
            bit_perfect = config.sample_rate.0 == sample_rate && config.channels == channels,
            "opened output stream"
        );

        let (producer, consumer) = buffer::new_pair();
        let paused = self.paused.clone();
        let gains = Gains::new(self.gain.clone(), self.fade.clone(), config.sample_rate.0);
        let flush = self.flush.clone();

        let stream = build_stream(
            &device,
            &config,
            sample_format,
            consumer,
            paused,
            gains,
            flush,
        )?;
        stream
            .play()
            .map_err(|e| ZniczError::Audio(e.to_string()))?;

        self.sample_rate = config.sample_rate.0;
        self.channels = config.channels;
        self.sample_format = Some(format_name(sample_format).to_string());
        self.device_name = device_name;
        self.producer = Some(producer);
        self.stream = Some(stream);
        self.flush.store(false, Ordering::Release);
        Ok(())
    }

    pub fn stop_stream(&mut self) {
        self.request_flush();
        if let Some(stream) = self.stream.take() {
            let _ = stream.pause();
            drop(stream);
        }
        self.producer = None;
        self.flush.store(false, Ordering::Release);
    }
}

impl SampleSink for AudioOutput {
    fn write_slots(&self) -> usize {
        self.producer.as_ref().map(|p| p.slots()).unwrap_or(0)
    }

    /// Push interleaved samples. Returns how many samples were accepted.
    fn push_samples(&mut self, samples: &[f32]) -> usize {
        let Some(producer) = self.producer.as_mut() else {
            return 0;
        };
        let mut written = 0;
        for &sample in samples {
            if producer.push(sample).is_err() {
                break;
            }
            written += 1;
        }
        written
    }
}

/// Short name for a sample format, for showing the signal path in the UI.
fn format_name(format: SampleFormat) -> &'static str {
    match format {
        SampleFormat::I8 => "i8",
        SampleFormat::I16 => "i16",
        SampleFormat::I32 => "i32",
        SampleFormat::I64 => "i64",
        SampleFormat::U8 => "u8",
        SampleFormat::U16 => "u16",
        SampleFormat::U32 => "u32",
        SampleFormat::U64 => "u64",
        SampleFormat::F32 => "f32",
        SampleFormat::F64 => "f64",
        _ => "?",
    }
}

/// Pick a device config that can play this file at the right speed.
/// Prefer an exact sample-rate and channel match. Wrong rate = wrong speed.
pub fn select_stream_config(
    ranges: &[SupportedStreamConfigRange],
    wanted_rate: u32,
    wanted_channels: u16,
) -> Option<(StreamConfig, SampleFormat)> {
    let wanted = cpal::SampleRate(wanted_rate);
    let wanted_ch = wanted_channels.max(1);

    let mut ranked: Vec<(i32, SupportedStreamConfigRange)> = ranges
        .iter()
        .cloned()
        .map(|range| {
            let rate_ok = range.min_sample_rate() <= wanted && wanted <= range.max_sample_rate();
            let ch_ok = range.channels() == wanted_ch;
            let fmt_score = match range.sample_format() {
                SampleFormat::F32 => 3,
                SampleFormat::I16 => 2,
                _ => 1,
            };
            let score = i32::from(rate_ok) * 100 + i32::from(ch_ok) * 10 + fmt_score;
            (score, range)
        })
        .collect();

    ranked.sort_by_key(|(score, _)| std::cmp::Reverse(*score));
    let (_, best) = ranked.into_iter().next()?;

    let rate = if best.min_sample_rate() <= wanted && wanted <= best.max_sample_rate() {
        wanted
    } else {
        best.min_sample_rate()
    };

    let supported = best.try_with_sample_rate(rate)?;
    Some((supported.config(), supported.sample_format()))
}

fn build_stream(
    device: &cpal::Device,
    config: &StreamConfig,
    sample_format: SampleFormat,
    consumer: Consumer<f32>,
    paused: Arc<AtomicBool>,
    mut gains: Gains,
    flush: Arc<AtomicBool>,
) -> Result<Stream> {
    let err_fn = |err| tracing::error!("audio stream error: {err}");

    let channels = usize::from(config.channels.max(1));

    let stream = match sample_format {
        SampleFormat::I16 => {
            let mut consumer = consumer;
            device.build_output_stream(
                config,
                move |data: &mut [i16], _| {
                    fill_i16(data, &mut consumer, &paused, &mut gains, channels, &flush);
                },
                err_fn,
                None,
            )
        }
        SampleFormat::U16 => {
            let mut consumer = consumer;
            device.build_output_stream(
                config,
                move |data: &mut [u16], _| {
                    fill_u16(data, &mut consumer, &paused, &mut gains, channels, &flush);
                },
                err_fn,
                None,
            )
        }
        _ => {
            let mut consumer = consumer;
            device.build_output_stream(
                config,
                move |data: &mut [f32], _| {
                    fill_f32(data, &mut consumer, &paused, &mut gains, channels, &flush);
                },
                err_fn,
                None,
            )
        }
    }
    .map_err(|e| ZniczError::Audio(e.to_string()))?;

    Ok(stream)
}

fn drain_if_flushing(consumer: &mut Consumer<f32>, flush: &AtomicBool) {
    if flush.load(Ordering::Acquire) {
        while consumer.pop().is_ok() {}
        flush.store(false, Ordering::Release);
    }
}

fn next_sample(consumer: &mut Consumer<f32>, volume: f32, paused: bool) -> f32 {
    if paused {
        return 0.0;
    }
    consumer.pop().map(|s| s * volume).unwrap_or(0.0)
}

fn fill_f32(
    data: &mut [f32],
    consumer: &mut Consumer<f32>,
    paused: &AtomicBool,
    gains: &mut Gains,
    channels: usize,
    flush: &AtomicBool,
) {
    drain_if_flushing(consumer, flush);
    let paused = paused.load(Ordering::Relaxed);
    gains.follow();
    for frame in data.chunks_mut(channels) {
        let g = gains.next_frame_gain();
        for out in frame.iter_mut() {
            *out = next_sample(consumer, g, paused);
        }
    }
    gains.report_landed();
}

fn fill_i16(
    data: &mut [i16],
    consumer: &mut Consumer<f32>,
    paused: &AtomicBool,
    gains: &mut Gains,
    channels: usize,
    flush: &AtomicBool,
) {
    drain_if_flushing(consumer, flush);
    let paused = paused.load(Ordering::Relaxed);
    gains.follow();
    for frame in data.chunks_mut(channels) {
        let g = gains.next_frame_gain();
        for out in frame.iter_mut() {
            let sample = next_sample(consumer, g, paused).clamp(-1.0, 1.0);
            *out = (sample * i16::MAX as f32) as i16;
        }
    }
    gains.report_landed();
}

fn fill_u16(
    data: &mut [u16],
    consumer: &mut Consumer<f32>,
    paused: &AtomicBool,
    gains: &mut Gains,
    channels: usize,
    flush: &AtomicBool,
) {
    drain_if_flushing(consumer, flush);
    let paused = paused.load(Ordering::Relaxed);
    gains.follow();
    for frame in data.chunks_mut(channels) {
        let g = gains.next_frame_gain();
        for out in frame.iter_mut() {
            let sample = next_sample(consumer, g, paused).clamp(-1.0, 1.0);
            *out = ((sample * 0.5 + 0.5) * u16::MAX as f32) as u16;
        }
    }
    gains.report_landed();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::gain::MUTE_FADE;
    use cpal::{SampleRate, SupportedBufferSize};

    /// Push `input` into a fresh ring and run one stereo callback over it.
    fn run_gains_with(input: &[f32], gains: &mut Gains) -> Vec<f32> {
        let (mut producer, mut consumer) = buffer::new_pair();
        for s in input {
            producer.push(*s).unwrap();
        }
        let mut out = vec![0.0f32; input.len()];
        fill_f32(
            &mut out,
            &mut consumer,
            &AtomicBool::new(false),
            gains,
            2,
            &AtomicBool::new(false),
        );
        out
    }

    fn run_gains(input: &[f32], mut gains: Gains) -> Vec<f32> {
        run_gains_with(input, &mut gains)
    }

    fn gains(volume: f32, fade: f32, sample_rate: u32) -> Gains {
        Gains::new(
            Arc::new(GainControl::new(volume)),
            Arc::new(GainControl::new(fade)),
            sample_rate,
        )
    }

    #[test]
    fn full_volume_and_full_fade_pass_samples_through_untouched() {
        let input: Vec<f32> = (0..512).map(|i| (i as f32 * 0.37).sin() * 0.9).collect();
        let out = run_gains(&input, gains(1.0, 1.0, 48_000));
        assert_eq!(out, input, "bit-for-bit at gain 1.0");
    }

    #[test]
    fn volume_and_fade_multiply() {
        assert_eq!(run_gains(&[0.8, 0.8], gains(0.5, 0.5, 48_000)), [0.2, 0.2]);
    }

    #[test]
    fn a_fade_out_ends_at_silence_and_reports_it() {
        let fade = Arc::new(GainControl::new(1.0));
        let mut gains = Gains::new(Arc::new(GainControl::new(1.0)), fade.clone(), 1000);
        let generation = fade.set(0.0, Duration::from_millis(4));
        // Stereo, 6 frames: 4 ramp frames, then silence.
        let out = run_gains_with(&[0.5; 12], &mut gains);
        assert_eq!(&out[8..], &[0.0; 4]);
        assert!(fade.has_landed(generation));
    }

    #[test]
    fn a_fade_applies_one_gain_per_frame_and_never_rises() {
        let fade = Arc::new(GainControl::new(1.0));
        let mut gains = Gains::new(Arc::new(GainControl::new(1.0)), fade.clone(), 48_000);
        fade.set(0.0, MUTE_FADE);
        let input = vec![1.0f32; 2 * 2_000];
        let out = run_gains_with(&input, &mut gains);
        let mut last = 1.0f32;
        for frame in out.chunks(2) {
            assert_eq!(frame[0], frame[1], "both channels share the frame gain");
            assert!(frame[0] <= last, "gain never rises while fading down");
            last = frame[0];
        }
        assert!(last < 1.0, "the fade actually moved");
    }

    fn range(
        channels: u16,
        min: u32,
        max: u32,
        format: SampleFormat,
    ) -> SupportedStreamConfigRange {
        SupportedStreamConfigRange::new(
            channels,
            SampleRate(min),
            SampleRate(max),
            SupportedBufferSize::Unknown,
            format,
        )
    }

    #[test]
    fn prefers_matching_sample_rate() {
        let ranges = vec![
            range(2, 48_000, 48_000, SampleFormat::F32),
            range(2, 44_100, 44_100, SampleFormat::F32),
            range(2, 96_000, 96_000, SampleFormat::F32),
        ];
        let (config, _) = select_stream_config(&ranges, 44_100, 2).unwrap();
        assert_eq!(config.sample_rate.0, 44_100);
        assert_eq!(config.channels, 2);
    }

    #[test]
    fn does_not_pick_96k_for_44k_file() {
        let ranges = vec![
            range(2, 96_000, 192_000, SampleFormat::F32),
            range(2, 44_100, 48_000, SampleFormat::I16),
        ];
        let (config, format) = select_stream_config(&ranges, 44_100, 2).unwrap();
        assert_eq!(config.sample_rate.0, 44_100);
        assert_eq!(format, SampleFormat::I16);
    }
}
