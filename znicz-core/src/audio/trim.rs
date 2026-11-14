//! Encoder delay and padding for AAC in MP4 (the iTunes `iTunSMPB` tag).
//!
//! AAC encoders add priming frames at the start and padding at the end. The
//! tag says how many, so the player can drop them and the tracks of a
//! continuous album join without a gap. symphonia trims MP3 and Vorbis itself;
//! AAC is left to us.

use std::ops::Range;

use symphonia::core::units::TimeBase;

/// How much of a decoded AAC stream is real audio.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EncoderGap {
    /// Priming frames at the start.
    pub delay: u64,
    /// Real audio frames after the priming.
    pub total: u64,
}

/// Parse an `iTunSMPB` value: space-separated hex fields
/// `reserved delay padding total …`.
pub fn parse_itunsmpb(value: &str) -> Option<EncoderGap> {
    let fields: Vec<&str> = value.split_whitespace().collect();
    if fields.len() < 4 {
        return None;
    }
    let delay = u64::from_str_radix(fields[1], 16).ok()?;
    let total = u64::from_str_radix(fields[3], 16).ok()?;
    if total == 0 {
        return None;
    }
    Some(EncoderGap { delay, total })
}

/// The frames of one decoded packet to keep. `first_frame` is the stream
/// frame index of the packet's first frame, priming included.
pub fn trim_window(first_frame: u64, frames: usize, gap: EncoderGap) -> Range<usize> {
    let frames = frames as u64;
    let audio_end = gap.delay + gap.total;
    let start = gap.delay.saturating_sub(first_frame).min(frames);
    let end = audio_end.saturating_sub(first_frame).min(frames).max(start);
    start as usize..end as usize
}

/// Drop the priming and padding frames of one interleaved packet.
pub fn trim_packet(
    samples: Vec<f32>,
    channels: usize,
    first_frame: u64,
    gap: EncoderGap,
) -> Vec<f32> {
    let channels = channels.max(1);
    let frames = samples.len() / channels;
    let keep = trim_window(first_frame, frames, gap);
    if keep == (0..frames) {
        return samples;
    }
    samples[keep.start * channels..keep.end * channels].to_vec()
}

/// A packet timestamp in frames. Without a time base the timestamp already
/// counts frames.
pub fn ts_to_frames(ts: u64, time_base: Option<TimeBase>, sample_rate: u32) -> u64 {
    match time_base {
        Some(base) => {
            let frames = ts as u128 * base.numer as u128 * sample_rate as u128 / base.denom as u128;
            frames as u64
        }
        None => ts,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GAP: EncoderGap = EncoderGap {
        delay: 2112,
        total: 10_000,
    };

    #[test]
    fn parses_an_itunes_value() {
        let value = " 00000000 00000840 000001CA 00000000001A8DF6 00000000 00000000";
        assert_eq!(
            parse_itunsmpb(value),
            Some(EncoderGap {
                delay: 0x840,
                total: 0x1A8DF6,
            })
        );
    }

    #[test]
    fn rejects_malformed_values() {
        assert_eq!(parse_itunsmpb(""), None);
        assert_eq!(parse_itunsmpb(" 00000000 00000840"), None);
        assert_eq!(parse_itunsmpb(" 00000000 zz 000001CA 1A8DF6"), None);
        // A zero total says nothing about the real length.
        assert_eq!(parse_itunsmpb(" 00000000 00000840 000001CA 0"), None);
    }

    #[test]
    fn drops_the_priming_frames() {
        assert_eq!(trim_window(0, 1024, GAP), 1024..1024);
        assert_eq!(trim_window(2048, 1024, GAP), 64..1024);
    }

    #[test]
    fn keeps_packets_in_the_middle() {
        assert_eq!(trim_window(4096, 1024, GAP), 0..1024);
    }

    #[test]
    fn drops_the_padding_frames() {
        // Real audio ends at frame 2112 + 10000 = 12112.
        assert_eq!(trim_window(11_264, 1024, GAP), 0..848);
        assert_eq!(trim_window(12_288, 1024, GAP), 0..0);
    }

    #[test]
    fn trims_interleaved_samples_by_frame() {
        let gap = EncoderGap { delay: 1, total: 2 };
        let samples = vec![1.0, 1.5, 2.0, 2.5, 3.0, 3.5, 4.0, 4.5];
        assert_eq!(trim_packet(samples, 2, 0, gap), vec![2.0, 2.5, 3.0, 3.5]);
    }

    #[test]
    fn leaves_untouched_packets_alone() {
        let samples = vec![0.25; 2048];
        assert_eq!(trim_packet(samples.clone(), 2, 4096, GAP), samples);
    }

    #[test]
    fn converts_timestamps_to_frames() {
        assert_eq!(
            ts_to_frames(44_100, Some(TimeBase::new(1, 44_100)), 44_100),
            44_100
        );
        assert_eq!(ts_to_frames(2, Some(TimeBase::new(1, 2)), 48_000), 48_000);
        assert_eq!(ts_to_frames(1234, None, 44_100), 1234);
    }
}
