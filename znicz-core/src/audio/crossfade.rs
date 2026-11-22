//! Crossfade: mixing the end of one track with the start of the next.
//!
//! The engine mixes before the ring buffer, by sample position, so the audio
//! callback does not change. This module holds the curve and the frame
//! bookkeeping; the engine supplies the samples.

use std::f32::consts::FRAC_PI_2;
use std::time::Duration;

use crate::audio::convert::{RateConverter, adapt_channels};
use crate::audio::feeder::DecodeStep;
use crate::audio::source::AudioDecoder;
use crate::error::Result;

/// Frames mixed per step once the outgoing track has ended.
pub const TAIL_CHUNK_FRAMES: usize = 1024;

/// `frames` at `file_rate`, as frames at `output_rate` once resampled.
pub fn output_frames(frames: u64, file_rate: u32, output_rate: u32) -> u64 {
    if file_rate == 0 || file_rate == output_rate {
        return frames;
    }
    frames * u64::from(output_rate) / u64::from(file_rate)
}

/// The next packet of `decoder`, with channels adapted to `output_channels`
/// and resampled by `converter`; `None` at the end. The main decode path and
/// a crossfade both go through here, so the two cannot drift apart.
pub fn next_packet(
    decoder: &mut AudioDecoder,
    converter: Option<&mut RateConverter>,
    output_channels: usize,
) -> Result<Option<Vec<f32>>> {
    let Some(samples) = decoder.decode_next()? else {
        return Ok(None);
    };
    let source_channels = decoder.channels().max(1) as usize;
    let matched = adapt_channels(&samples, source_channels, output_channels);
    Ok(Some(match converter {
        Some(converter) => converter.process(&matched),
        None => matched,
    }))
}

/// A decoder with its own rate converter: one side of a crossfade.
pub struct DecodedTrack {
    pub decoder: AudioDecoder,
    pub converter: Option<RateConverter>,
}

impl DecodedTrack {
    /// The next packet in the output's channel count and rate; `None` at the end.
    pub fn next_packet(&mut self, output_channels: usize) -> Result<Option<Vec<f32>>> {
        next_packet(&mut self.decoder, self.converter.as_mut(), output_channels)
    }
}

/// A running crossfade: the outgoing track (until it ends), the incoming one,
/// and the mix between them. The engine pumps it in place of the main decoder.
pub struct Crossfade {
    outgoing: Option<DecodedTrack>,
    incoming: DecodedTrack,
    incoming_duration: Option<Duration>,
    mix: CrossfadeMix,
    leftover_sent: bool,
}

impl Crossfade {
    /// `length` is in output frames; `channels` is the output channel count.
    pub fn new(
        outgoing: DecodedTrack,
        incoming: DecodedTrack,
        incoming_duration: Option<Duration>,
        length: u64,
        channels: usize,
    ) -> Self {
        Self {
            outgoing: Some(outgoing),
            incoming,
            incoming_duration,
            mix: CrossfadeMix::new(length, channels),
            leftover_sent: false,
        }
    }

    /// The mix is done and the incoming leftover has been handed on.
    pub fn is_done(&self) -> bool {
        self.mix.is_done() && self.leftover_sent
    }

    /// One pump step: a mixed packet while mixing; then the incoming leftover;
    /// then `Yield`, which hands the crossfade over to the incoming track.
    pub fn next_step(&mut self, output_channels: usize) -> DecodeStep {
        if self.mix.is_done() {
            return self.finish_step();
        }

        let packet = match self
            .outgoing
            .as_mut()
            .map(|t| t.next_packet(output_channels))
        {
            Some(Ok(packet)) => packet,
            Some(Err(e)) => return DecodeStep::Failed(e.to_string()),
            None => None,
        };
        if packet.is_none() {
            self.outgoing = None;
        }

        // The mix pulls incoming packets on demand. A decode error ends the
        // incoming side early; the error is reported once the step is mixed.
        let mut error = None;
        let incoming = &mut self.incoming;
        let mut pull = || match incoming.next_packet(output_channels) {
            Ok(packet) => packet,
            Err(e) => {
                error = Some(e.to_string());
                None
            }
        };
        let samples = self.mix.mix(packet.as_deref(), &mut pull);

        match error {
            Some(message) => DecodeStep::Failed(message),
            None => DecodeStep::Samples(samples),
        }
    }

    fn finish_step(&mut self) -> DecodeStep {
        if !self.leftover_sent {
            self.leftover_sent = true;
            let rest = self.mix.take_leftover();
            if !rest.is_empty() {
                return DecodeStep::Samples(rest);
            }
        }
        DecodeStep::Yield
    }

    /// The incoming track (with its converter) and its stated duration, for
    /// the engine to take over.
    pub fn into_incoming(self) -> (DecodedTrack, Option<Duration>) {
        (self.incoming, self.incoming_duration)
    }

    /// Sample rate and channel count of the incoming track, which is the one
    /// playing once the crossfade ends.
    pub fn format(&self) -> (u32, u16) {
        let decoder = &self.incoming.decoder;
        (decoder.sample_rate(), decoder.channels())
    }
}

/// Equal-power gains `(outgoing, incoming)` at `t` in 0..=1: the loudness
/// stays even through the crossfade (a² + b² = 1).
pub fn crossfade_gains(t: f32) -> (f32, f32) {
    let t = t.clamp(0.0, 1.0);
    if t == 0.0 {
        return (1.0, 0.0);
    }
    if t >= 1.0 {
        return (0.0, 1.0);
    }
    let angle = t * FRAC_PI_2;
    (angle.cos(), angle.sin())
}

/// One crossfade's progress and the incoming samples not mixed yet.
/// Samples are interleaved with `channels` channels.
pub struct CrossfadeMix {
    length: u64,
    done: u64,
    channels: usize,
    incoming: Vec<f32>,
    incoming_offset: usize,
    incoming_ended: bool,
}

impl CrossfadeMix {
    /// A crossfade of `length` frames (at least 1).
    pub fn new(length: u64, channels: usize) -> Self {
        Self {
            length: length.max(1),
            done: 0,
            channels: channels.max(1),
            incoming: Vec::new(),
            incoming_offset: 0,
            incoming_ended: false,
        }
    }

    /// All frames mixed.
    pub fn is_done(&self) -> bool {
        self.done >= self.length
    }

    /// Mix one step. `outgoing` is the next outgoing packet, or `None` once
    /// that track has ended (then `TAIL_CHUNK_FRAMES` frames are mixed with
    /// silence for the outgoing side). Outgoing frames past the end of the
    /// crossfade are dropped. `pull` yields incoming packets; `None` means the
    /// incoming track has ended and silence stands in for it.
    pub fn mix(
        &mut self,
        outgoing: Option<&[f32]>,
        pull: &mut dyn FnMut() -> Option<Vec<f32>>,
    ) -> Vec<f32> {
        let channels = self.channels;
        let available = match outgoing {
            Some(samples) => samples.len() / channels,
            None => TAIL_CHUNK_FRAMES,
        };
        let frames = available.min(self.length.saturating_sub(self.done) as usize);
        let mut out = Vec::with_capacity(frames * channels);

        for frame in 0..frames {
            let t = self.done as f32 / self.length as f32;
            let (a, b) = crossfade_gains(t);
            for channel in 0..channels {
                let outgoing_sample = outgoing.map_or(0.0, |s| s[frame * channels + channel]);
                let incoming_sample = self.next_incoming(pull);
                out.push(a * outgoing_sample + b * incoming_sample);
            }
            self.done += 1;
        }

        out
    }

    /// The next incoming sample, pulling a packet when the buffer is used up.
    /// Silence once the incoming track has ended.
    fn next_incoming(&mut self, pull: &mut dyn FnMut() -> Option<Vec<f32>>) -> f32 {
        if self.incoming_offset >= self.incoming.len() && !self.incoming_ended {
            match pull() {
                Some(packet) => {
                    self.incoming = packet;
                    self.incoming_offset = 0;
                }
                None => self.incoming_ended = true,
            }
        }

        if self.incoming_offset < self.incoming.len() {
            let sample = self.incoming[self.incoming_offset];
            self.incoming_offset += 1;
            sample
        } else {
            0.0
        }
    }

    /// Incoming samples decoded but not mixed; emptied by the call. Pushed
    /// before the incoming decoder takes over, so nothing is lost.
    pub fn take_leftover(&mut self) -> Vec<f32> {
        let rest = self.incoming.split_off(self.incoming_offset);
        self.incoming.clear();
        self.incoming_offset = 0;
        rest
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-4
    }

    #[test]
    fn the_gains_keep_equal_power() {
        assert_eq!(crossfade_gains(0.0), (1.0, 0.0));
        let (a, b) = crossfade_gains(1.0);
        assert!(close(a, 0.0) && close(b, 1.0));
        for t in [0.1, 0.25, 0.5, 0.9] {
            let (a, b) = crossfade_gains(t);
            assert!(close(a * a + b * b, 1.0), "t={t}");
        }
        let (a, b) = crossfade_gains(0.5);
        assert!(close(a, b));
    }

    #[test]
    fn constant_sources_cross_over_the_length() {
        // Mono, 4 frames: out = 1.0 everywhere, in = 0.5 everywhere.
        let mut mix = CrossfadeMix::new(4, 1);
        let mut pull = || Some(vec![0.5; 4]);
        let out = mix.mix(Some(&[1.0; 4]), &mut pull);
        assert_eq!(out.len(), 4);
        assert_eq!(out[0], 1.0); // t = 0: outgoing only
        assert!(out[3] < 1.0 && out[3] > 0.5);
        assert!(mix.is_done());
    }

    #[test]
    fn incoming_packets_of_another_size_are_buffered() {
        // Stereo; outgoing packets of 3 frames, incoming of 5 frames.
        let mut mix = CrossfadeMix::new(6, 2);
        let mut next = 0.0f32;
        let mut pull = || {
            let packet: Vec<f32> = (0..10)
                .map(|_| {
                    next += 1.0;
                    next
                })
                .collect();
            Some(packet)
        };
        let first = mix.mix(Some(&[0.0; 6]), &mut pull);
        let second = mix.mix(Some(&[0.0; 6]), &mut pull);
        assert_eq!(first.len() + second.len(), 12);
        assert!(mix.is_done());
        // 6 frames of the 10 pulled are mixed; 4 frames (8 samples) are left.
        assert_eq!(
            mix.take_leftover(),
            (13..=20).map(|v| v as f32).collect::<Vec<_>>()
        );
        assert!(mix.take_leftover().is_empty());
    }

    #[test]
    fn an_early_end_of_the_outgoing_track_keeps_the_curve() {
        let mut mix = CrossfadeMix::new(2 * TAIL_CHUNK_FRAMES as u64, 1);
        let mut pull = || Some(vec![1.0; 256]);
        let tail = mix.mix(None, &mut pull);
        assert_eq!(tail.len(), TAIL_CHUNK_FRAMES);
        // Incoming rises from silence; the first frame is 0 (t = 0).
        assert_eq!(tail[0], 0.0);
        assert!(tail[TAIL_CHUNK_FRAMES - 1] > tail[1]);
        assert!(!mix.is_done());
        mix.mix(None, &mut pull);
        assert!(mix.is_done());
    }

    #[test]
    fn outgoing_frames_past_the_end_are_dropped() {
        let mut mix = CrossfadeMix::new(2, 1);
        let mut pull = || Some(vec![0.0; 8]);
        let out = mix.mix(Some(&[1.0; 5]), &mut pull);
        assert_eq!(out.len(), 2);
        assert!(mix.is_done());
    }

    /// A mono 16-bit WAV file of `frames` frames, all at `sample`.
    fn write_wav(path: &std::path::Path, frames: u32, sample: i16) {
        let data_len = frames * 2;
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"RIFF");
        bytes.extend_from_slice(&(36 + data_len).to_le_bytes());
        bytes.extend_from_slice(b"WAVEfmt ");
        bytes.extend_from_slice(&16u32.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&44_100u32.to_le_bytes());
        bytes.extend_from_slice(&88_200u32.to_le_bytes());
        bytes.extend_from_slice(&2u16.to_le_bytes());
        bytes.extend_from_slice(&16u16.to_le_bytes());
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&data_len.to_le_bytes());
        for _ in 0..frames {
            bytes.extend_from_slice(&sample.to_le_bytes());
        }
        std::fs::write(path, bytes).unwrap();
    }

    fn track(path: &std::path::Path) -> DecodedTrack {
        let (decoder, _) = AudioDecoder::open_path(path).unwrap();
        DecodedTrack {
            decoder,
            converter: None,
        }
    }

    /// Run steps until the engine would hand over; returns the samples.
    fn run_to_yield(fade: &mut Crossfade) -> Vec<f32> {
        let mut out = Vec::new();
        loop {
            match fade.next_step(1) {
                DecodeStep::Samples(samples) => out.extend(samples),
                DecodeStep::Yield => return out,
                DecodeStep::End => panic!("a crossfade never ends by itself"),
                DecodeStep::Failed(message) => panic!("crossfade failed: {message}"),
            }
        }
    }

    #[test]
    fn output_frames_follow_the_resampling_ratio() {
        assert_eq!(output_frames(1000, 44_100, 44_100), 1000);
        assert_eq!(output_frames(1000, 0, 48_000), 1000);
        assert_eq!(output_frames(44_100, 44_100, 48_000), 48_000);
    }

    #[test]
    fn a_crossfade_mixes_its_length_then_hands_over() {
        let dir = tempfile::tempdir().unwrap();
        let (a, b) = (dir.path().join("a.wav"), dir.path().join("b.wav"));
        write_wav(&a, 4410, 16_384); // 0.5
        write_wav(&b, 4410, 8_192); // 0.25
        let mut fade = Crossfade::new(track(&a), track(&b), None, 2000, 1);

        let out = run_to_yield(&mut fade);
        assert!(fade.is_done());
        // The mix is 2000 frames; the incoming leftover follows it.
        assert!(out.len() >= 2000, "got {} samples", out.len());
        assert!((out[0] - 0.5).abs() < 0.01, "starts on the outgoing track");
        assert!(
            (out[1999] - 0.25).abs() < 0.05,
            "ends on the incoming track"
        );
        assert!((out[out.len() - 1] - 0.25).abs() < 0.01);
    }

    #[test]
    fn a_short_outgoing_track_fades_out_on_the_curve() {
        let dir = tempfile::tempdir().unwrap();
        let (a, b) = (dir.path().join("a.wav"), dir.path().join("b.wav"));
        write_wav(&a, 300, 16_384);
        write_wav(&b, 8000, 8_192);
        let mut fade = Crossfade::new(track(&a), track(&b), None, 2000, 1);

        let out = run_to_yield(&mut fade);
        assert!(fade.is_done());
        assert!(out.len() >= 2000, "the mix runs its full length");
        // Once the outgoing track is gone the incoming one keeps rising.
        assert!(out[1900] > out[1000], "the incoming track keeps rising");
        assert!((out[1999] - 0.25).abs() < 0.05);
    }

    #[test]
    fn an_ended_incoming_track_is_silence() {
        let mut mix = CrossfadeMix::new(4, 1);
        let mut pull = || None;
        let out = mix.mix(Some(&[1.0; 4]), &mut pull);
        assert_eq!(out[0], 1.0);
        assert!(out[3] < 1.0);
    }
}
