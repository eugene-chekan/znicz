//! Gain changes as short ramps, applied per frame in the audio callback.
//!
//! Instant gain jumps click; a mute that cuts the sound is abrupt. The output
//! keeps a target gain and a ramp time; each stream callback walks its own
//! `GainRamp` toward that target and lands on it exactly, so a gain of 1.0
//! leaves samples untouched (bit-perfect).

use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::time::Duration;

/// Fade for mute and unmute.
pub const MUTE_FADE: Duration = Duration::from_millis(500);
/// Ramp for every other gain change.
pub const VOLUME_RAMP: Duration = Duration::from_millis(20);

/// Shared between the engine (writer) and the stream callback (reader).
#[derive(Debug)]
pub struct GainControl {
    target_bits: AtomicU32,
    ramp_ms: AtomicU32,
    generation: AtomicU64,
}

impl GainControl {
    pub fn new(gain: f32) -> Self {
        Self {
            target_bits: AtomicU32::new(gain.clamp(0.0, 1.0).to_bits()),
            ramp_ms: AtomicU32::new(0),
            generation: AtomicU64::new(0),
        }
    }

    /// Move toward `target` over `ramp`.
    pub fn set(&self, target: f32, ramp: Duration) {
        self.target_bits
            .store(target.clamp(0.0, 1.0).to_bits(), Ordering::Relaxed);
        self.ramp_ms.store(
            u32::try_from(ramp.as_millis()).unwrap_or(u32::MAX),
            Ordering::Relaxed,
        );
        self.generation.fetch_add(1, Ordering::Release);
    }

    /// `(target, ramp_ms, generation)`.
    pub fn snapshot(&self) -> (f32, u32, u64) {
        let generation = self.generation.load(Ordering::Acquire);
        let target = f32::from_bits(self.target_bits.load(Ordering::Relaxed));
        let ramp_ms = self.ramp_ms.load(Ordering::Relaxed);
        (target, ramp_ms, generation)
    }
}

/// Frames in a ramp of `ramp_ms` at `sample_rate`; at least 1.
pub fn ramp_frames(ramp_ms: u32, sample_rate: u32) -> u32 {
    (u64::from(ramp_ms) * u64::from(sample_rate) / 1000).clamp(1, u64::from(u32::MAX)) as u32
}

/// One stream callback's gain, moving linearly toward a target.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GainRamp {
    current: f32,
    target: f32,
    step: f32,
    frames_left: u32,
    generation: u64,
}

impl GainRamp {
    /// Start at `gain` with nothing to do (a new stream does not fade in).
    pub fn new(gain: f32, generation: u64) -> Self {
        Self {
            current: gain,
            target: gain,
            step: 0.0,
            frames_left: 0,
            generation,
        }
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Start a new ramp from wherever the gain is now.
    pub fn retarget(&mut self, target: f32, frames: u32, generation: u64) {
        let frames = frames.max(1);
        self.generation = generation;
        self.target = target;
        self.frames_left = frames;
        self.step = (target - self.current) / frames as f32;
    }

    /// Pick up a change from `control` if there is one.
    pub fn follow(&mut self, control: &GainControl, sample_rate: u32) {
        let (target, ramp_ms, generation) = control.snapshot();
        if generation != self.generation {
            self.retarget(target, ramp_frames(ramp_ms, sample_rate), generation);
        }
    }

    /// The gain for the next frame; exactly the target once the ramp ends.
    pub fn next_frame_gain(&mut self) -> f32 {
        if self.frames_left > 0 {
            self.frames_left -= 1;
            self.current = if self.frames_left == 0 {
                self.target
            } else {
                self.current + self.step
            };
        }
        self.current
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_mute_fade_at_48k_takes_24000_frames_and_lands_on_zero() {
        let frames = ramp_frames(500, 48_000);
        assert_eq!(frames, 24_000);
        let mut ramp = GainRamp::new(1.0, 0);
        ramp.retarget(0.0, frames, 1);
        let mut last = 1.0;
        for i in 0..frames {
            let g = ramp.next_frame_gain();
            assert!(
                g <= last + f32::EPSILON,
                "never rises on the way down at {i}"
            );
            last = g;
        }
        assert_eq!(last, 0.0);
        assert_eq!(ramp.next_frame_gain(), 0.0, "stays there");
    }

    #[test]
    fn unmuting_to_full_ends_at_exactly_one() {
        let mut ramp = GainRamp::new(0.0, 0);
        ramp.retarget(1.0, 1_000, 1);
        let mut g = 0.0;
        for _ in 0..1_000 {
            g = ramp.next_frame_gain();
        }
        assert_eq!(g, 1.0, "bit-perfect needs exactly 1.0");
    }

    #[test]
    fn reversing_mid_fade_starts_from_the_current_gain() {
        let mut ramp = GainRamp::new(1.0, 0);
        ramp.retarget(0.0, 100, 1);
        for _ in 0..50 {
            ramp.next_frame_gain();
        }
        let halfway = ramp.next_frame_gain();
        ramp.retarget(1.0, 100, 2);
        let next = ramp.next_frame_gain();
        assert!(
            next > halfway && next < halfway + 0.02,
            "{halfway} -> {next}"
        );
    }

    #[test]
    fn without_a_change_the_gain_holds() {
        let mut ramp = GainRamp::new(0.7, 3);
        for _ in 0..10 {
            assert_eq!(ramp.next_frame_gain(), 0.7);
        }
    }

    #[test]
    fn follow_picks_up_a_new_target_from_the_control() {
        let control = GainControl::new(1.0);
        let (_, _, generation) = control.snapshot();
        let mut ramp = GainRamp::new(1.0, generation);
        control.set(0.0, MUTE_FADE);
        ramp.follow(&control, 1_000);
        let mut g = 1.0;
        for _ in 0..500 {
            g = ramp.next_frame_gain();
        }
        assert_eq!(g, 0.0);
    }
}
