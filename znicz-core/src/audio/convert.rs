//! Small helpers that keep playback speed correct.
//!
//! Two things change speed if they are wrong:
//! * channel count (mono samples sent to a stereo stream play twice as fast)
//! * sample rate (44.1 kHz audio in a 96 kHz stream plays about twice as fast)

/// Convert interleaved audio from `in_channels` to `out_channels`.
///
/// * mono to many: copy the sample to every channel
/// * many to mono: average the channels
/// * otherwise: copy the channels we have, fill the rest with silence
pub fn adapt_channels(input: &[f32], in_channels: usize, out_channels: usize) -> Vec<f32> {
    if in_channels == 0 || out_channels == 0 || in_channels == out_channels {
        return input.to_vec();
    }

    let frames = input.len() / in_channels;
    let mut out = Vec::with_capacity(frames * out_channels);

    for frame in 0..frames {
        let start = frame * in_channels;
        let source = &input[start..start + in_channels];

        if in_channels == 1 {
            for _ in 0..out_channels {
                out.push(source[0]);
            }
        } else if out_channels == 1 {
            let sum: f32 = source.iter().sum();
            out.push(sum / in_channels as f32);
        } else {
            for channel in 0..out_channels {
                out.push(source.get(channel).copied().unwrap_or(0.0));
            }
        }
    }

    out
}

/// Simple linear resampler used only when the device refuses the file's rate.
///
/// Linear interpolation is not audiophile grade. It exists so playback keeps
/// the right speed and pitch instead of racing. A proper sinc resampler
/// (for example `rubato`) is the planned replacement.
pub struct RateConverter {
    in_rate: u32,
    out_rate: u32,
    channels: usize,
    carry: Vec<f32>,
    position: f64,
}

impl RateConverter {
    pub fn new(in_rate: u32, out_rate: u32, channels: usize) -> Self {
        Self {
            in_rate,
            out_rate,
            channels: channels.max(1),
            carry: Vec::new(),
            position: 0.0,
        }
    }

    pub fn is_passthrough(&self) -> bool {
        self.in_rate == self.out_rate || self.in_rate == 0 || self.out_rate == 0
    }

    pub fn reset(&mut self) {
        self.carry.clear();
        self.position = 0.0;
    }

    /// Resample one block. Returns interleaved samples at the output rate.
    pub fn process(&mut self, input: &[f32]) -> Vec<f32> {
        if self.is_passthrough() {
            return input.to_vec();
        }

        let channels = self.channels;
        let mut work = std::mem::take(&mut self.carry);
        work.extend_from_slice(input);

        let frames = work.len() / channels;
        if frames < 2 {
            self.carry = work;
            return Vec::new();
        }

        let step = self.in_rate as f64 / self.out_rate as f64;
        let mut out = Vec::new();

        while self.position + 1.0 < frames as f64 {
            let index = self.position.floor() as usize;
            let fraction = (self.position - index as f64) as f32;

            for channel in 0..channels {
                let a = work[index * channels + channel];
                let b = work[(index + 1) * channels + channel];
                out.push(a + (b - a) * fraction);
            }

            self.position += step;
        }

        // Keep the frames we still need for the next block.
        let keep_from = (self.position.floor() as usize).min(frames - 1);
        self.carry = work[keep_from * channels..].to_vec();
        self.position -= keep_from as f64;

        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mono_to_stereo_doubles_each_sample() {
        let out = adapt_channels(&[1.0, 2.0], 1, 2);
        assert_eq!(out, vec![1.0, 1.0, 2.0, 2.0]);
    }

    #[test]
    fn stereo_to_mono_averages() {
        let out = adapt_channels(&[1.0, 3.0, -1.0, 1.0], 2, 1);
        assert_eq!(out, vec![2.0, 0.0]);
    }

    #[test]
    fn same_channel_count_is_unchanged() {
        let input = vec![0.1, 0.2, 0.3, 0.4];
        assert_eq!(adapt_channels(&input, 2, 2), input);
    }

    #[test]
    fn passthrough_when_rates_match() {
        let mut converter = RateConverter::new(44_100, 44_100, 2);
        assert!(converter.is_passthrough());
        let input = vec![0.5, -0.5];
        assert_eq!(converter.process(&input), input);
    }

    #[test]
    fn upsampling_produces_more_frames() {
        let mut converter = RateConverter::new(44_100, 48_000, 1);
        // One second of input.
        let input: Vec<f32> = (0..44_100).map(|i| (i as f32 * 0.001).sin()).collect();
        let out = converter.process(&input);

        // Expect close to 48000 frames, allow a small edge tolerance.
        let diff = (out.len() as i64 - 48_000).abs();
        assert!(diff < 100, "expected about 48000 frames, got {}", out.len());
    }

    #[test]
    fn downsampling_produces_fewer_frames() {
        let mut converter = RateConverter::new(96_000, 48_000, 2);
        let input: Vec<f32> = (0..96_000 * 2).map(|i| i as f32 * 0.0001).collect();
        let out = converter.process(&input);

        let frames = out.len() / 2;
        let diff = (frames as i64 - 48_000).abs();
        assert!(diff < 100, "expected about 48000 frames, got {frames}");
    }
}
