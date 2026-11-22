//! Crossfade: mixing the end of one track with the start of the next.
//!
//! The engine mixes before the ring buffer, by sample position, so the audio
//! callback does not change. This module holds the curve and the frame
//! bookkeeping; the engine supplies the samples.

use std::f32::consts::FRAC_PI_2;

/// Frames mixed per step once the outgoing track has ended.
pub const TAIL_CHUNK_FRAMES: usize = 1024;

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

    #[test]
    fn an_ended_incoming_track_is_silence() {
        let mut mix = CrossfadeMix::new(4, 1);
        let mut pull = || None;
        let out = mix.mix(Some(&[1.0; 4]), &mut pull);
        assert_eq!(out[0], 1.0);
        assert!(out[3] < 1.0);
    }
}
