//! Gain changes as short ramps, applied per frame in the audio callback.
//!
//! Instant gain jumps click; a mute that cuts the sound is abrupt. The output
//! keeps a target gain and a ramp time; each stream callback walks its own
//! `GainRamp` toward that target and lands on it exactly, so a gain of 1.0
//! leaves samples untouched (bit-perfect).

use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::time::Duration;

/// Ramp for every other gain change.
pub const VOLUME_RAMP: Duration = Duration::from_millis(20);

/// Shared between the engine (writer) and the stream callback (reader).
#[derive(Debug)]
pub struct GainControl {
    target_bits: AtomicU32,
    ramp_ms: AtomicU32,
    /// Start of the next ramp, or NaN for "from wherever the gain is now".
    start_bits: AtomicU32,
    generation: AtomicU64,
    /// Highest generation whose ramp the callback has reached.
    landed: AtomicU64,
}

impl GainControl {
    pub fn new(gain: f32) -> Self {
        Self {
            target_bits: AtomicU32::new(gain.clamp(0.0, 1.0).to_bits()),
            ramp_ms: AtomicU32::new(0),
            start_bits: AtomicU32::new(f32::NAN.to_bits()),
            generation: AtomicU64::new(0),
            landed: AtomicU64::new(0),
        }
    }

    /// Move toward `target` over `ramp`, starting from the current gain.
    /// Returns the generation this change created.
    pub fn set(&self, target: f32, ramp: Duration) -> u64 {
        self.store(f32::NAN, target, ramp)
    }

    /// Move toward `target` over `ramp`, starting from `start`.
    /// Returns the generation this change created.
    pub fn set_from(&self, start: f32, target: f32, ramp: Duration) -> u64 {
        self.store(start.clamp(0.0, 1.0), target, ramp)
    }

    fn store(&self, start: f32, target: f32, ramp: Duration) -> u64 {
        self.start_bits.store(start.to_bits(), Ordering::Relaxed);
        self.target_bits
            .store(target.clamp(0.0, 1.0).to_bits(), Ordering::Relaxed);
        self.ramp_ms.store(
            u32::try_from(ramp.as_millis()).unwrap_or(u32::MAX),
            Ordering::Relaxed,
        );
        self.generation.fetch_add(1, Ordering::Release) + 1
    }

    /// `(start, target, ramp_ms, generation)`; `start` is `None` when the
    /// ramp begins from the current gain.
    ///
    /// Read while the engine is writing, the values can belong to the next
    /// generation. That is harmless with one writer: the next callback sees
    /// the newer generation and starts that ramp again.
    pub fn snapshot(&self) -> (Option<f32>, f32, u32, u64) {
        let generation = self.generation.load(Ordering::Acquire);
        let start = f32::from_bits(self.start_bits.load(Ordering::Relaxed));
        let start = (!start.is_nan()).then_some(start);
        let target = f32::from_bits(self.target_bits.load(Ordering::Relaxed));
        let ramp_ms = self.ramp_ms.load(Ordering::Relaxed);
        (start, target, ramp_ms, generation)
    }

    /// Whether the callback has reached the target set as `generation`.
    ///
    /// Reported when the callback writes the samples; the device plays them
    /// one buffer (10–20 ms) later.
    pub fn has_landed(&self, generation: u64) -> bool {
        self.landed.load(Ordering::Acquire) >= generation
    }

    fn mark_landed(&self, generation: u64) {
        self.landed.fetch_max(generation, Ordering::Release);
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

    /// A ramp for a new stream: a pending `set_from` starts from its start
    /// value, a plain `set` does not fade.
    pub fn following(control: &GainControl, sample_rate: u32) -> Self {
        let (start, target, _, generation) = control.snapshot();
        match start {
            Some(_) => {
                let mut ramp = Self::new(target, generation.wrapping_sub(1));
                ramp.follow(control, sample_rate);
                ramp
            }
            None => Self::new(target, generation),
        }
    }

    /// Pick up a change from `control` if there is one.
    pub fn follow(&mut self, control: &GainControl, sample_rate: u32) {
        let (start, target, ramp_ms, generation) = control.snapshot();
        if generation != self.generation {
            if let Some(start) = start {
                self.current = start;
            }
            self.retarget(target, ramp_frames(ramp_ms, sample_rate), generation);
        }
    }

    /// Tell `control` that this ramp reached its target, once it has.
    pub fn report_landed(&self, control: &GainControl) {
        if self.frames_left == 0 {
            control.mark_landed(self.generation);
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
        let (_, _, _, generation) = control.snapshot();
        let mut ramp = GainRamp::new(1.0, generation);
        control.set(0.0, Duration::from_millis(500));
        ramp.follow(&control, 1_000);
        let mut g = 1.0;
        for _ in 0..500 {
            g = ramp.next_frame_gain();
        }
        assert_eq!(g, 0.0);
    }

    #[test]
    fn set_returns_increasing_generations() {
        let control = GainControl::new(1.0);
        let first = control.set(0.5, VOLUME_RAMP);
        let second = control.set(0.2, VOLUME_RAMP);
        assert!(second > first);
    }

    #[test]
    fn a_ramp_from_a_start_value_ignores_the_current_gain() {
        let control = GainControl::new(1.0);
        let (_, _, _, generation) = control.snapshot();
        let mut ramp = GainRamp::new(1.0, generation);
        control.set_from(0.0, 1.0, Duration::from_millis(10));
        ramp.follow(&control, 1000);
        // 10 ms at 1 kHz: 10 frames from 0.0 up to 1.0.
        let first = ramp.next_frame_gain();
        assert!(first < 0.2, "starts near silence, got {first}");
        for _ in 0..9 {
            ramp.next_frame_gain();
        }
        assert_eq!(ramp.next_frame_gain(), 1.0);
    }

    #[test]
    fn landing_is_reported_for_the_generation_reached() {
        let control = GainControl::new(1.0);
        let (_, _, _, generation) = control.snapshot();
        let mut ramp = GainRamp::new(1.0, generation);
        let fade = control.set(0.0, Duration::from_millis(5));
        ramp.follow(&control, 1000);
        ramp.report_landed(&control);
        assert!(!control.has_landed(fade), "not before the ramp ends");
        for _ in 0..5 {
            ramp.next_frame_gain();
        }
        ramp.report_landed(&control);
        assert!(control.has_landed(fade));
        let later = control.set(1.0, Duration::from_millis(5));
        assert!(
            !control.has_landed(later),
            "an older landing does not count"
        );
    }

    #[test]
    fn a_new_stream_starts_a_pending_fade_in_from_its_start_value() {
        let control = GainControl::new(1.0);
        control.set_from(0.0, 1.0, Duration::from_millis(10));
        let mut ramp = GainRamp::following(&control, 1000);
        assert!(ramp.next_frame_gain() < 0.2);
    }

    #[test]
    fn a_new_stream_does_not_fade_a_plain_target() {
        let control = GainControl::new(1.0);
        control.set(0.4, VOLUME_RAMP);
        let mut ramp = GainRamp::following(&control, 48_000);
        assert_eq!(ramp.next_frame_gain(), 0.4);
    }
}
