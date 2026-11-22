# Crossfade Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Automatic track changes overlap the outgoing and incoming tracks for a set time with an equal-power crossfade (also for repeat-one); the Playback tab's rows become sliders with a typed value (#90 part 2, v0.19.0).

**Architecture:** The engine mixes the two tracks before the ring buffer: a pure `CrossfadeMix` (curve and frame bookkeeping) inside an engine-level `Crossfade` that owns both decoders. The main decode step yields at the exact start frame. In the TUI, a `Kind::Slider` replaces the fade ladder, with a typed-value prompt built on `line_edit.rs`.

**Tech Stack:** Rust 2024, symphonia, cpal, ratatui, toml/toml_edit.

Spec: `docs/superpowers/specs/2026-11-20-crossfade-design.md` — read it before your task.

Branch: `feat/crossfade`. **Every task starts with `git branch --show-current`; it must print `feat/crossfade`. Stop if not.**

Commit messages end with a blank line and `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`. Before each commit: `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`, and the tests named in the task. **Commit and push in separate commands; do not push at all (the controller pushes).**

---

### Task 1: The mixer (`audio/crossfade.rs`)

**Files:** Create `znicz-core/src/audio/crossfade.rs`; add `pub mod crossfade;` to `audio/mod.rs` (alphabetical).

- [ ] **Step 1: Module with tests, bodies `todo!()`:**

```rust
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
    todo!()
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
        todo!()
    }

    /// All frames mixed.
    pub fn is_done(&self) -> bool {
        todo!()
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
        todo!()
    }

    /// Incoming samples decoded but not mixed; emptied by the call. Pushed
    /// before the incoming decoder takes over, so nothing is lost.
    pub fn take_leftover(&mut self) -> Vec<f32> {
        todo!()
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
        assert_eq!(mix.take_leftover(), (13..=20).map(|v| v as f32).collect::<Vec<_>>());
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
```

- [ ] **Step 2: Run** `cargo test -p znicz-core --lib audio::crossfade` — fails (`not yet implemented`).

- [ ] **Step 3: Implement.**
  - `crossfade_gains(t)`: `let t = t.clamp(0.0, 1.0); ((t * FRAC_PI_2).cos(), (t * FRAC_PI_2).sin())`, but return exactly `(1.0, 0.0)` for `t == 0.0` and `(0.0, 1.0)` for `t >= 1.0` so the ends are exact.
  - `new`: `length.max(1)`, `channels.max(1)`, empty buffer.
  - `is_done`: `done >= length`.
  - `mix`: frames = `outgoing.len() / channels` or `TAIL_CHUNK_FRAMES` when `None`, capped at `length - done`. For each frame: `t = done as f32 / length as f32`; gains; for each channel `out = a * outgoing_sample (0.0 when None) + b * incoming_sample`. Incoming sample: when `incoming_offset == incoming.len()` and not `incoming_ended`, `pull()`: `Some(p)` replaces the buffer (`incoming = p; incoming_offset = 0`), `None` sets `incoming_ended`; when ended or the buffer is empty, use 0.0. Take one sample per channel per frame (`incoming[incoming_offset]`, then `+= 1`). `done += 1` per frame.
  - `take_leftover`: `let rest = self.incoming.split_off(self.incoming_offset); self.incoming.clear(); self.incoming_offset = 0; rest`.

- [ ] **Step 4: Run** the tests — 6 pass. **Commit** `feat(core): crossfade mixer (#90)`.

---

### Task 2: Decoder position and a yield step (`audio/source.rs`, `audio/feeder.rs`)

**Files:** Modify `znicz-core/src/audio/source.rs`, `znicz-core/src/audio/feeder.rs`.

- [ ] **Step 1: Failing tests.**
  - `feeder.rs` tests:

```rust
    #[test]
    fn a_yield_stops_the_pump_and_keeps_nothing_back() {
        let mut sink = FakeSink::new(1024);
        let mut feeder = Feeder::new();
        let mut steps = vec![DecodeStep::Yield, DecodeStep::Samples(vec![1.0; 4])].into_iter();
        let mut decode = || steps.next().unwrap_or(DecodeStep::End);
        assert_eq!(feeder.pump(&mut sink, 1, 8, &mut decode), PumpOutcome::Yielded);
        assert_eq!(feeder.pending_samples(), 0);
        assert_eq!(feeder.pump(&mut sink, 1, 8, &mut decode), PumpOutcome::Finished);
        assert_eq!(sink.queued, vec![1.0; 4]);
    }
```

  - `source.rs` tests (use the existing `silent_wav_bytes(frames)` helper and whatever the module uses to open a decoder from bytes; read the tests module first):

```rust
    #[test]
    fn position_counts_decoded_frames_and_follows_a_seek() {
        // Open a 2 s mono 8 kHz (or whatever silent_wav_bytes makes) WAV.
        // After decoding all packets, position_frames() equals the total
        // frame count; after seek(1 s), the next decode's position is about
        // 1 s worth of frames plus that packet.
    }
```

    Write this test concretely with the existing helpers (if opening from bytes needs a temp file, write it with `tempfile`). Assert exact totals where the WAV makes them exact.

- [ ] **Step 2: Implement.**
  - `DecodeStep::Yield` and `PumpOutcome::Yielded` in `feeder.rs`: in `pump`'s loop, `DecodeStep::Yield => return PumpOutcome::Yielded,` (only reached after the leftover drained, so nothing is pending). Add the matching arms wherever `PumpOutcome` is matched (the engine: treat `Yielded` like `SinkFull` for now; Task 4 gives it meaning).
  - `AudioDecoder` gets `next_frame: u64`; in `decode_next`, after a packet decodes, `self.next_frame = ts_to_frames(packet_ts, self.time_base, self.sample_rate) + frames_in_packet` where `frames_in_packet` is the decoded (untrimmed) frame count. `pub fn position_frames(&self) -> u64 { self.next_frame }`. `seek` sets `next_frame` from the requested position (`position.as_secs_f64() * sample_rate`), corrected on the next decode by the packet timestamp.

- [ ] **Step 3: Run** `cargo test -p znicz-core` (all). **Commit** `feat(core): decoder position and a yield step for the pump (#90)`.

---

### Task 3: Crossfade rules and config (`player/fade.rs`, `player/gapless.rs`, binary config)

**Files:** Modify `znicz-core/src/player/fade.rs`, `znicz-core/src/player/gapless.rs`, `znicz/src/main.rs`, `znicz/src/default-config.toml`.

- [ ] **Step 1: Failing tests.**
  - `fade.rs`:

```rust
    #[test]
    fn crossfade_is_off_by_default() {
        assert_eq!(FadeTimes::default().crossfade, Duration::ZERO);
    }

    #[test]
    fn the_crossfade_starts_its_length_before_the_end() {
        let s = |n| Duration::from_secs(n);
        // 60 s track at 1 kHz, 5 s crossfade: start at frame 55_000.
        assert_eq!(crossfade_from(s(5), Some(s(60)), 1000, false), Some(55_000));
        // Capped at half the track.
        assert_eq!(crossfade_from(s(5), Some(s(6)), 1000, false), Some(3_000));
        // Off, radio, or an unknown length: none.
        assert_eq!(crossfade_from(Duration::ZERO, Some(s(60)), 1000, false), None);
        assert_eq!(crossfade_from(s(5), Some(s(60)), 1000, true), None);
        assert_eq!(crossfade_from(s(5), None, 1000, false), None);
    }

    #[test]
    fn a_short_next_track_is_not_crossfaded_into() {
        let s = |n| Duration::from_secs(n);
        assert!(incoming_long_enough(Some(s(10)), 5_000, 1000));
        assert!(!incoming_long_enough(Some(s(9)), 5_000, 1000));
        assert!(incoming_long_enough(None, 5_000, 1000), "unknown length: try");
    }
```

  - `gapless.rs`: change `repeat_one_and_radio_do_not_chain` to assert `may_chain(RepeatMode::One, false)` is true and `may_chain(_, true)` false; rename it `radio_does_not_chain`.
  - `znicz/src/main.rs`: `crossfade_ms` parses into `fade_times(&config).crossfade`; the default config has `crossfade_ms = 0` (extend the existing fade tests).

- [ ] **Step 2: Implement.**
  - `FadeTimes.crossfade: Duration` (Default zero; `#[serde(default)]` on the field so an older client's `SetFades` still deserializes).
  - In `fade.rs`:

```rust
/// The frame where a crossfade into the next track starts: `length` before
/// the end, at most half the track. `None` when crossfade is off, for radio,
/// or when the track's length is unknown.
pub fn crossfade_from(length: Duration, track: Option<Duration>, rate: u32, stream: bool) -> Option<u64> {
    if length.is_zero() || stream {
        return None;
    }
    let total = frames(track?, rate);
    let fade = frames(length, rate).min(total / 2);
    (fade > 0).then(|| total - fade)
}

/// The next track is long enough to crossfade into: at least twice the
/// crossfade. An unknown length is given the benefit of the doubt.
pub fn incoming_long_enough(track: Option<Duration>, fade_frames: u64, rate: u32) -> bool {
    track.is_none_or(|t| frames(t, rate) >= 2 * fade_frames)
}

fn frames(time: Duration, rate: u32) -> u64 {
    (time.as_secs_f64() * f64::from(rate)).round() as u64
}
```

  - `gapless.rs`: `may_chain(_repeat, current_is_stream) -> bool { !current_is_stream }` — keep the signature for now but drop the repeat rule, or remove the parameter and fix the caller; update the doc comment ("Repeat-one loops seamlessly; radio keeps stop-and-reopen.").
  - Binary: `FadeSection.crossfade_ms: Option<u64>`; `fade_times` maps it; `default-config.toml` adds under `[audio.fade]`:

```toml
crossfade_ms = 0  # automatic track changes overlap by this much; 0 = off
```

- [ ] **Step 3: Run** `cargo test --workspace`. **Commit** `feat: crossfade rules and config; gapless loops repeat-one (#90)`.

---

### Task 4: Crossfade in the engine (`player/engine.rs`)

**Files:** Modify `znicz-core/src/player/engine.rs` (and `audio/crossfade.rs` if the engine-level struct lives there — see below).

Read the spec's "Engine" section and the whole engine first: `pump_decode`, `on_decoder_finished`, `chain_next`, `open_matching`, `play_item`, `stop`, `seek`, `replay_current`, `finish_track`, `advance_pending`, `start_chained`, `playback()`, the `SetDevice` and `SetFades` arms, the idle wait in `run`, `tick_position`.

- [ ] **Step 1: The engine-level crossfade.** In `audio/crossfade.rs` (it is audio plumbing, no player state), add:

```rust
/// A decoder with its own channel mapping and rate converter: one side of a
/// crossfade.
pub struct DecodedTrack {
    pub decoder: AudioDecoder,
    pub converter: Option<RateConverter>,
}

impl DecodedTrack {
    /// The next packet in the output's channel count and rate; `None` at the end.
    pub fn next_packet(&mut self, output_channels: usize) -> Result<Option<Vec<f32>>> { … }
}

/// A running crossfade: the outgoing track (until it ends), the incoming one,
/// and the mix between them.
pub struct Crossfade {
    outgoing: Option<DecodedTrack>,
    incoming: DecodedTrack,
    incoming_duration: Option<Duration>,
    mix: CrossfadeMix,
    leftover_sent: bool,
}

impl Crossfade {
    pub fn new(outgoing: DecodedTrack, incoming: DecodedTrack, incoming_duration: Option<Duration>, length: u64, channels: usize) -> Self;
    /// One pump step: a mixed packet; the incoming leftover once the mix is
    /// done; then `Yield` so the engine hands over. `End` if both ended.
    pub fn next_step(&mut self, output_channels: usize) -> DecodeStep;
    pub fn is_done(&self) -> bool;          // mix done and leftover sent
    pub fn into_incoming(self) -> (DecodedTrack, Option<Duration>);
    /// Format of the tracks (same for both), for a device reopen.
    pub fn format(&self) -> (u32, u16);
}
```

  `next_packet` does what `pump_decode`'s closure does today (`adapt_channels`, then the converter). Use it in the main path too, so the logic is not duplicated: the engine's `decoder` + `converter` fields become one `Option<DecodedTrack>` (`self.track`), or keep the fields and build a `DecodedTrack` only for the crossfade — choose the smaller diff, and say which in the report.

- [ ] **Step 2: Arming.** Field `crossfade_from: Option<u64>`; helper `fn arm_crossfade(&mut self, duration: Option<Duration>)` sets it with `fade::crossfade_from(self.fades.crossfade, duration, decoder_rate, is_stream)`. Call it when a decoder becomes current: end of `play_item` (track_info.duration), `chain_next` success (info.duration), crossfade hand-over (incoming duration), and in `SetFades` (store the current track's duration in a field `track_duration: Option<Duration>` set at those same points, so `SetFades` can re-arm).

- [ ] **Step 3: Yield at the start frame.** In `pump_decode`'s main closure, before `decoder.decode_next()`: `if crossfade_from.is_some_and(|from| decoder.position_frames() >= from) { return DecodeStep::Yield; }` (copy `crossfade_from` into a local before the closure). On `PumpOutcome::Yielded` call `self.start_crossfade(decoder, converter)`.

- [ ] **Step 4: `start_crossfade(outgoing decoder, converter)`.**
  - Pick the next row exactly as `chain_next` does (after the last pending track or `queue_position`; `pick_next_after(after, true)`; repeat-one returns the playing row); it must be a local file; open it with `open_matching` (same format).
  - `length = min(frames(self.fades.crossfade), track_total − position)`; require `fade::incoming_long_enough(info.duration, length, rate)`.
  - Incoming converter: `Some(RateConverter::new(file_rate, output_rate, output_channels))` when the outgoing side has one, else `None`.
  - On success: push `PendingTrack { index, item, info, starts_at: self.feeder.pushed_samples() }`; `self.crossfade = Some(Crossfade::new(...))`; `self.decoder = None`; `self.converter = None`; `self.crossfade_from = None`.
  - On any failure: put the outgoing decoder and converter back, `self.crossfade_from = None` (no retry for this track).
  - Factor the shared "pick and open the next row" part out of `chain_next` into one helper used by both.

- [ ] **Step 5: Pumping a crossfade.** At the top of `pump_decode` (after the paused check): if `self.crossfade` is `Some`, take it, pump with `|| fade.next_step(output_channels)`, then: `Yielded`/`SinkFull` → if `fade.is_done()` hand over (`self.decoder`/`self.converter` from the incoming track, `arm_crossfade(incoming_duration)`), else put it back; `Finished` (both ended) → hand over, then `on_decoder_finished` with the incoming decoder; `Failed` → error and stop, as the main path.

- [ ] **Step 6: "Is a decoder open".** Add `fn has_source(&self) -> bool { self.decoder.is_some() || self.crossfade.is_some() }` and use it in `playback()` (`decoding` and `audible`), the idle wait, `tick_position`'s early return, and any other `self.decoder.is_some()` check where a running crossfade must count. `SetDevice`: reopen with the decoder's format, or `crossfade.format()` during a crossfade.

- [ ] **Step 7: Commands.**
  - `stop()`: `self.crossfade = None; self.crossfade_from = None;`.
  - `seek`: before the existing pending check, `if self.crossfade.is_some() && self.pending.is_empty() { self.finish_crossfade_now(); }` — hand the incoming track over at once (drop the outgoing side and the leftover; arm for the incoming). When pending is not empty the existing `replay_current()` path reopens the playing row and `stop()` drops the crossfade.
  - `SetFades`: store the times, then re-arm for the current track (not a running crossfade).

- [ ] **Step 8: Verify.** `cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`, and the hardware tests `cargo test -p znicz-core --test playback -- --test-threads=1` (this machine has a sound card; all must pass). **Commit** `feat(core): crossfade between tracks (#90)`.

---

### Task 5: Hardware tests for crossfade (`tests/playback.rs`)

**Files:** Modify `znicz-core/tests/playback.rs`.

- [ ] **Step 1: Tests** (reuse `skip_hardware_playback`, `write_silent_wav`, the `play_queue`-style helper if present):
  - `crossfaded_tracks_overlap`: two 3 s 44.1 kHz stereo WAVs, `FadeTimes { crossfade: 1 s, ..Default }`; play the queue; the second `TrackStarted` arrives between 1.7 s and 2.5 s after start; both tracks end (`TrackEnded` twice); total time under 5.5 s (vs. about 6 s without crossfade).
  - `repeat_one_crossfades_into_itself`: one 2 s WAV, repeat-one, crossfade 0.5 s; after 4 s the player is still Playing on the same track and has emitted at least two `TrackStarted`.
- [ ] **Step 2: Run** the playback tests one at a time, three times; all must pass. Do not loosen assertions; report failures verbatim. **Commit** `test(core): crossfade on real output (#90)`.

---

### Task 6: Slider model (`znicz-tui/src/settings.rs`)

**Files:** Modify `znicz-tui/src/settings.rs`.

- [ ] **Step 1: Failing tests:**

```rust
    #[test]
    fn playback_rows_are_sliders_with_crossfade_last() {
        assert_eq!(rows(Tab::Playback, &[]).last(), Some(&Row::Crossfade));
        assert!(matches!(Row::Crossfade.kind(), Kind::Slider(_)));
        assert!(matches!(Row::FadePlay.kind(), Kind::Slider(_)));
    }

    #[test]
    fn sliders_step_on_their_grid_and_stop_at_the_ends() {
        let fade = slider(&Row::FadePause).unwrap();
        let ms = Duration::from_millis;
        assert_eq!(step_slider(fade, ms(150), true), ms(200));
        assert_eq!(step_slider(fade, ms(240), true), ms(250));
        assert_eq!(step_slider(fade, ms(240), false), ms(200));
        assert_eq!(step_slider(fade, ms(2000), true), ms(2000));
        assert_eq!(step_slider(fade, Duration::ZERO, false), Duration::ZERO);
        let cross = slider(&Row::Crossfade).unwrap();
        assert_eq!(step_slider(cross, ms(3000), true), ms(3500));
        assert_eq!(step_slider(cross, ms(12_000), true), ms(12_000));
    }

    #[test]
    fn typed_values_parse_in_the_rows_unit() {
        let fade = slider(&Row::FadeStop).unwrap();
        let cross = slider(&Row::Crossfade).unwrap();
        let ms = Duration::from_millis;
        assert_eq!(parse_slider_value(fade, "150"), Ok(ms(150)));
        assert_eq!(parse_slider_value(fade, "0.2s"), Ok(ms(200)));
        assert_eq!(parse_slider_value(fade, " 300 ms "), Ok(ms(300)));
        assert_eq!(parse_slider_value(cross, "3.5"), Ok(ms(3500)));
        assert_eq!(parse_slider_value(cross, "3500ms"), Ok(ms(3500)));
        assert_eq!(parse_slider_value(cross, "13"), Err("0–12 s".to_string()));
        assert_eq!(parse_slider_value(fade, "-5"), Err("0–2000 ms".to_string()));
        assert_eq!(parse_slider_value(fade, "slow"), Err("0–2000 ms".to_string()));
    }

    #[test]
    fn slider_values_show_their_unit() {
        let tui = TuiConfig::default();
        let mut v = values(&tui);
        v.fades.crossfade = Duration::from_millis(3500);
        assert_eq!(value_text(&Row::Crossfade, &v), "3.5 s");
        v.fades.crossfade = Duration::from_secs(3);
        assert_eq!(value_text(&Row::Crossfade, &v), "3 s");
        v.fades.crossfade = Duration::ZERO;
        assert_eq!(value_text(&Row::Crossfade, &v), "off");
        assert_eq!(value_text(&Row::FadeStop, &v), "300 ms");
    }

    #[test]
    fn crossfade_saves_under_audio_fade() {
        assert_eq!(Row::Crossfade.config_key(), Some(("audio.fade", "crossfade_ms")));
    }
```

  Delete the old ladder tests (`fade_steps_follow_the_ladder_and_snap`) since the ladder goes.

- [ ] **Step 2: Implement.**
  - `#[derive(Debug, Clone, Copy, PartialEq, Eq)] pub enum SliderUnit { Ms, Seconds }` and `pub struct Slider { pub min: Duration, pub max: Duration, pub step: Duration, pub unit: SliderUnit }`.
  - `Kind::Slider(Slider)` replaces `Kind::Choice` for the six Playback rows. `pub fn slider(row: &Row) -> Option<Slider>`: fades `0..=2000 ms, step 50 ms, Ms`; crossfade `0..=12 s, step 500 ms, Seconds`.
  - `Row::Crossfade`: label "Crossfade"; explanation "Overlap the end of a track with the start of the next when one follows the other on its own (repeat-one too). Off joins them as they are."; `config_key` `("audio.fade", "crossfade_ms")`; tab Playback; last on that tab.
  - `step_slider(slider, current, forward) -> Duration`: next grid value (`k * step`) strictly above/below `current`, clamped to `min..=max`.
  - `parse_slider_value(slider, text) -> Result<Duration, String>`: trim; split a trailing `ms` or `s` (case-insensitive, spaces allowed before it); no unit = the slider's unit; parse `f64`; reject NaN, negative, out of range with the range text (`"0–2000 ms"`, `"0–12 s"`); round to whole milliseconds.
  - `value_text`: `off` for zero; `Ms` → `"{n} ms"`; `Seconds` → `"{s} s"` with no trailing `.0` (`3 s`, `3.5 s`).
  - `pub fn slider_range_text(slider) -> String` (used by the prompt label).
  - `fade_slot` covers `Row::Crossfade` (→ `fades.crossfade`). Remove `FADE_LADDER_MS` and `step_fade`.
  - Minimal call-site edits in `app/settings_ui.rs` so the crate builds: `Kind::Slider(s)` in `apply_setting` steps with `step_slider` for Forward/Back, and for Activate does nothing yet (Task 7 opens the prompt).

- [ ] **Step 3: Run** `cargo test -p znicz-tui`. **Commit** `feat(tui): slider rows for fades and crossfade (#90)`.

---

### Task 7: Slider prompt and drawing (`app/settings_ui.rs`, `views/settings.rs`, `keys.rs`)

**Files:** Modify `znicz-tui/src/app/settings_ui.rs`, `znicz-tui/src/views/settings.rs`, `znicz-tui/src/keys.rs`, `znicz-tui/src/app.rs`.

- [ ] **Step 1: Failing tests** (in `settings_ui.rs` tests; helpers `settings_app`, `press`, `saved`, `screen_rows`; add `type_text(app, "3.5")` pressing each char):

```rust
    #[test]
    fn enter_on_a_slider_opens_a_prompt_and_saves_a_typed_value() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = settings_app(&dir);
        press(&mut app, KeyCode::Char('S'));
        press(&mut app, KeyCode::Char('3'));
        press(&mut app, KeyCode::Char('G'));          // Crossfade, last row
        press(&mut app, KeyCode::Enter);
        assert!(app.settings_prompt.is_some());
        for _ in 0..4 { press(&mut app, KeyCode::Backspace); }
        type_text(&mut app, "3.5");
        press(&mut app, KeyCode::Enter);
        assert!(app.settings_prompt.is_none());
        assert_eq!(app.fades.crossfade, Duration::from_millis(3500));
        assert_eq!(saved(&dir)["audio"]["fade"]["crossfade_ms"].as_integer(), Some(3500));
    }

    #[test]
    fn a_bad_value_keeps_the_prompt_open_with_the_range() {
        // open the Crossfade prompt as above, clear it, type "20", Enter:
        // the prompt stays, app.fades.crossfade unchanged, the screen shows "0–12 s".
    }

    #[test]
    fn esc_closes_the_prompt_without_saving_and_keeps_settings_open() {
        // open a fade prompt, type "999", Esc: prompt gone, value unchanged,
        // modal still Settings; a second Esc closes Settings.
    }

    #[test]
    fn keys_type_into_the_prompt_instead_of_switching_tabs() {
        // open a prompt, press '2' and Tab: the tab stays Playback, the
        // prompt text ends with "2".
    }

    #[test]
    fn a_slider_row_draws_a_bar_and_its_value() {
        // Playback tab: the Fade on stop row's line contains "━" and "300 ms".
    }
```

  Write the four sketched tests out in full in the same style.

- [ ] **Step 2: Implement.**
  - `App.settings_prompt: Option<SliderPrompt>` where `pub(crate) struct SliderPrompt { pub row: Row, pub text: LineEdit, pub error: Option<String> }`.
  - In `on_settings_key`, first: if a prompt is open, handle it and return `true` for every key except `Esc` handling inside it: Enter → `parse_slider_value`; Ok → set the slot, `after_setting_changed`/save/`SetFades` as a slider change does, close; Err → `error = Some(range)`; Esc → close the prompt (return `true` so the modal stays); other keys → `text.on_key(key)`, clear the error. Make sure `app.rs`'s Esc handling (`on_esc`) does not close Settings while a prompt is open (check the order: `on_esc` runs before `on_settings_key` — add a prompt check there).
  - Enter or Space on a slider row opens the prompt with the current value written in the row's unit (`150`, `3.5`), cursor at the end.
  - The prompt also closes when Settings closes or another modal opens (`drop_removal_outside_settings` does the same for the removal question; extend it).
  - `views/settings.rs`:
    - A slider row: label, then a 16-cell bar from `format::progress_bar((value − min) / (max − min), 16)` (filled part in the text style, rest dim), then the value. Keep the value flush right as other rows.
    - While a prompt is open, the strip shows `"{label} ({range}): {text}"` with a caret at the cursor (follow how other prompts draw the caret), and the error after it in the warning style when set.
  - `keys.rs`: a `Context::SettingsPrompt` with hints `Enter save · Esc cancel`; `app.rs` maps Settings + open prompt to it; add it to the contexts test list. Help: add `b("Enter on a slider", "type a value")` to `SETTINGS`.

- [ ] **Step 3: Run** `cargo test -p znicz-tui`, clippy. **Commit** `feat(tui): typed values for slider rows (#90)`.

---

### Task 8: Docs, version, release prep

**Files:** `Cargo.toml` (0.19.0), `Cargo.lock`, `THIRD-PARTY-LICENSES.html` (`scripts/third-party-licenses.sh`), `CHANGELOG.md`, `README.md` (config template: `crossfade_ms`), wiki: `wiki/Domain/Playback-Pipeline.md`, `wiki/Architecture/Core-Engine.md`, `wiki/Architecture/TUI.md`, `wiki/Architecture/Audio-Threading.md` if it describes the pump.

- [ ] **Step 1:** Version bump, `cargo build --workspace`, licences script.
- [ ] **Step 2:** CHANGELOG (date 2026-11-20 or the day you run this):

```markdown
## 0.19.0 - 2026-11-20

- Crossfade: when one track follows another on its own, the two overlap for
  the set time with an equal-power curve (`[audio.fade] crossfade_ms`,
  Settings → Playback; off by default, up to 12 s). Repeat-one crossfades
  into itself. Radio, a change of sample rate or channels, an unknown track
  length, or a next track shorter than twice the crossfade join as before.
- Gapless playback also loops a repeat-one track without reopening the
  stream.
- Settings → Playback rows are sliders: `h` / `l` step (50 ms for fades,
  0.5 s for crossfade), Enter types an exact value (`150`, `0.2s`, `3.5`).
```

- [ ] **Step 3:** Wiki and README: read each page first; describe what was built (check names against the code). TUI.md: replace the fade ladder text with the slider rows and the prompt.
- [ ] **Step 4:** `cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`. **Commit** `chore: release 0.19.0 (crossfade)`.
