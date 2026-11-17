# Fades and Settings Tabs Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Play, resume, pause, stop, manual track changes and device switches fade instead of cutting; fade times are configurable and live; the Settings modal gets tabs with a Playback tab for them (#90 part 1, v0.18.0).

**Architecture:** The audio callback multiplies volume gain by a new fade gain, each with its own ramp; the callback reports when a fade lands. The engine holds commands that need a fade-out in an ordered waiting list and runs them once the fade landed. Pure decisions live in `player/fade.rs`. In the TUI, `settings.rs` gains a `Tab` model, `app/settings_ui.rs` a focus (tab bar or rows), `views/settings.rs` draws the tab bar.

**Tech Stack:** Rust 2024, cpal, rtrb, crossbeam-channel, ratatui, toml / toml_edit, serde.

Spec: `docs/superpowers/specs/2026-11-17-fades-and-settings-tabs-design.md` — read it before your task.

Branch: `feat/fades-settings-tabs`. **Every task starts with `git branch --show-current`; it must print `feat/fades-settings-tabs`. Stop if not.**

Every commit message ends with a blank line and:

```
Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
```

Before each commit: `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`, and the tests named in the task (plus `cargo test -p <crate>` for the crate you touched). Do not push.

---

### Task 1: Gain control with a start value and a "landed" marker (`audio/gain.rs`)

**Files:** Modify `znicz-core/src/audio/gain.rs`.

Today `GainControl::set(target, ramp)` returns nothing, and a ramp always starts from the callback's current gain. We need (a) `set` to return the generation it created, (b) `set_from(start, target, ramp)` that makes the ramp start at `start` (a fade-in from silence must start at 0 even when the callback's gain is 1), (c) the callback to report when it reached a generation's target, and (d) a constructor for a new stream's ramp that honours a pending `set_from`.

- [ ] **Step 1: Write the failing tests** (add to the existing `tests` module):

```rust
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
        let (_, _, generation) = control.snapshot();
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
        let (_, _, generation) = control.snapshot();
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
        assert!(!control.has_landed(later), "an older landing does not count");
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
```

- [ ] **Step 2: Run** `cargo test -p znicz-core --lib audio::gain` — expect compile errors (methods missing).

- [ ] **Step 3: Implement.**
  - Add fields `start_bits: AtomicU32` (stores `f32::NAN.to_bits()` for "from current") and `landed: AtomicU64` (initialised to the initial generation, 0) to `GainControl`.
  - `set(&self, target, ramp) -> u64`: store target, ramp, `start = NAN`, then `self.generation.fetch_add(1, Ordering::Release) + 1` and return that.
  - `set_from(&self, start, target, ramp) -> u64`: same, storing `start.clamp(0.0, 1.0)`.
  - `snapshot(&self) -> (Option<f32>, f32, u32, u64)`: `(start, target, ramp_ms, generation)`; `start` is `None` when the stored value is NaN. Update every caller of `snapshot` in `gain.rs` and `output.rs` (`build_stream` destructures it).
  - `has_landed(&self, generation) -> bool`: `self.landed.load(Acquire) >= generation`.
  - `fn mark_landed(&self, generation)`: `self.landed.fetch_max(generation, Release)` (private to the module is fine; `GainRamp` calls it).
  - `GainRamp::follow`: when the generation changed, if `start` is `Some(s)` set `self.current = s` before `retarget`.
  - `GainRamp::report_landed(&self, control)`: if `self.frames_left == 0`, `control.mark_landed(self.generation)`.
  - `GainRamp::following(control, sample_rate) -> Self`: `let (start, target, _, generation) = control.snapshot();` If `start` is `Some(_)`, build `GainRamp::new(target, generation.wrapping_sub(1))` and call `follow` (so it starts from `start` and ramps). Otherwise `GainRamp::new(target, generation)` (no fade for a plain target, as today).
  - Keep `MUTE_FADE` for now (Task 4 removes it).

- [ ] **Step 4: Run** `cargo test -p znicz-core --lib audio` — all pass.
- [ ] **Step 5: Commit** `feat(core): gain ramps with a start value and a landed marker (#90)`.

---

### Task 2: Fade gain in the output (`audio/output.rs`)

**Files:** Modify `znicz-core/src/audio/output.rs`.

- [ ] **Step 1: Failing tests** in the `tests` module (adapt `run_f32` to the new signature described below):

```rust
    #[test]
    fn full_volume_and_full_fade_pass_samples_through_untouched() {
        let gains = Gains::new(Arc::new(GainControl::new(1.0)), Arc::new(GainControl::new(1.0)), 48_000);
        let input = [0.1f32, -0.2, 0.3, -0.4];
        assert_eq!(run_gains(&input, gains), input);
    }

    #[test]
    fn volume_and_fade_multiply() {
        let gains = Gains::new(Arc::new(GainControl::new(0.5)), Arc::new(GainControl::new(0.5)), 48_000);
        assert_eq!(run_gains(&[0.8, 0.8], gains), [0.2, 0.2]);
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
```

Write the helpers `run_gains(input, gains) -> Vec<f32>` / `run_gains_with(input, &mut gains)` in the test module: build an `rtrb::RingBuffer` with the input, call `fill_f32(&mut out, &mut consumer, &AtomicBool::new(false), &mut gains, 2, &AtomicBool::new(false))`.

- [ ] **Step 2: Run** `cargo test -p znicz-core --lib audio::output` — compile errors.

- [ ] **Step 3: Implement.**
  - A struct in `output.rs`:

```rust
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
        Self { volume, fade, volume_ramp, fade_ramp, sample_rate }
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
```

  - `fill_f32/fill_i16/fill_u16(data, consumer, paused, gains: &mut Gains, channels, flush)`: `drain_if_flushing`, read `paused`, `gains.follow()`, per frame `let g = gains.next_frame_gain();`, after the loop `gains.report_landed()`. Remove the `#[allow(clippy::too_many_arguments)]` if no longer needed. 1.0 × 1.0 is exactly 1.0, so pass-through stays exact.
  - `AudioOutput` gets `fade: Arc<GainControl>` (initial 1.0). `build_stream` takes both controls and creates `Gains::new(volume, fade, rate)` inside.
  - Public methods:

```rust
    /// Ramp the fade gain to `target` over `ramp`; returns the generation to
    /// wait for with `fade_landed`.
    pub fn fade_to(&self, target: f32, ramp: Duration) -> u64 {
        self.fade.set(target, ramp)
    }

    /// Fade in from silence over `ramp`.
    pub fn fade_in(&self, ramp: Duration) -> u64 {
        self.fade.set_from(0.0, 1.0, ramp)
    }

    /// Set the fade gain at once (no ramp).
    pub fn set_fade_now(&self, gain: f32) {
        self.fade.set_from(gain, gain, Duration::ZERO);
    }

    /// The stream has reached the fade target set as `generation`.
    pub fn fade_landed(&self, generation: u64) -> bool {
        self.fade.has_landed(generation)
    }
```

  - Existing tests in `output.rs` that build a `GainRamp` and call `fill_f32` are rewritten to use `Gains` (keep what they check: the mute fade test and the pass-through test).

- [ ] **Step 4: Run** `cargo test -p znicz-core` — all pass; clippy clean.
- [ ] **Step 5: Commit** `feat(core): a fade gain beside the volume in the audio callback (#90)`.

---

### Task 3: Fade decisions (`player/fade.rs`)

**Files:** Create `znicz-core/src/player/fade.rs`; modify `player/mod.rs` (`pub mod fade;`), `player/commands.rs` (add `SetFades`), `lib.rs` (re-export `FadeTimes`).

- [ ] **Step 1: Add the command** in `commands.rs`, at the end of `Command`:

```rust
    /// New fade times; used at once.
    SetFades(crate::player::fade::FadeTimes),
```

  The engine's `match command` must stay exhaustive: add a temporary arm `Command::SetFades(_) => {}` in `engine.rs` `handle_command` (Task 4 replaces it).

- [ ] **Step 2: Write the module with tests, bodies `todo!()`:**

```rust
//! Fade-outs before a command: which commands fade, for how long, and the
//! commands waiting for a fade to finish. No audio here; the engine drives it.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::player::commands::{Command, CommandEnvelope};

/// How long each fade takes. Zero turns that fade off.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct FadeTimes {
    /// Fade in on play, resume, after a track change or a device switch.
    pub play: Duration,
    pub pause: Duration,
    pub stop: Duration,
    /// Fade out before a manual track change or a device switch.
    pub track: Duration,
    pub mute: Duration,
}

impl Default for FadeTimes {
    fn default() -> Self {
        Self {
            play: Duration::from_millis(150),
            pause: Duration::from_millis(150),
            stop: Duration::from_millis(300),
            track: Duration::from_millis(150),
            mute: Duration::from_millis(500),
        }
    }
}

/// What the engine is doing, as far as fading is concerned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Playback {
    /// Sound is coming out: status Playing with an open stream.
    pub audible: bool,
    /// The queue row that is playing.
    pub position: usize,
}

/// How long to fade out before `command`, or `None` to run it at once.
pub fn fade_out_time(command: &Command, times: &FadeTimes, playback: Playback) -> Option<Duration> {
    todo!()
}

/// Extra time the engine waits for the callback to report a fade landed.
pub const LANDING_GRACE: Duration = Duration::from_millis(500);

/// A fade-out in progress and the commands waiting for it, in arrival order.
pub struct FadeOut {
    pub generation: u64,
    pub deadline: Instant,
    waiting: VecDeque<CommandEnvelope>,
}

impl FadeOut {
    /// A fade-out of `time` started now for `first`.
    pub fn new(first: CommandEnvelope, generation: u64, time: Duration, now: Instant) -> Self {
        todo!()
    }

    /// Queue a command behind the waiting ones.
    pub fn wait(&mut self, envelope: CommandEnvelope) {
        todo!()
    }

    /// True when the only waiting command is a `Pause`: a `Resume` cancels it.
    pub fn only_a_pause(&self) -> bool {
        todo!()
    }

    /// The fade is done: landed, or past its deadline.
    pub fn is_due(&self, landed: bool, now: Instant) -> bool {
        todo!()
    }

    /// The waiting commands, oldest first.
    pub fn into_waiting(self) -> VecDeque<CommandEnvelope> {
        todo!()
    }
}

/// Commands that never wait for a fade-out.
pub fn runs_during_fade(command: &Command) -> bool {
    todo!()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::player::state::QueueItem;

    const PLAYING: Playback = Playback { audible: true, position: 2 };
    const SILENT: Playback = Playback { audible: false, position: 2 };

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    #[test]
    fn transport_commands_fade_while_playing() {
        let t = FadeTimes::default();
        assert_eq!(fade_out_time(&Command::Pause, &t, PLAYING), Some(t.pause));
        assert_eq!(fade_out_time(&Command::Stop, &t, PLAYING), Some(t.stop));
        for command in [
            Command::NextTrack,
            Command::PreviousTrack,
            Command::Play(QueueItem::file("/a.flac")),
            Command::QueuePlayIndex(0),
            Command::ReplaceQueue { items: vec![], position: 0 },
            Command::QueueRemove(2),
            Command::SetDevice("x".into()),
        ] {
            assert_eq!(fade_out_time(&command, &t, PLAYING), Some(t.track), "{command:?}");
        }
    }

    #[test]
    fn nothing_fades_when_nothing_is_heard() {
        let t = FadeTimes::default();
        assert_eq!(fade_out_time(&Command::Pause, &t, SILENT), None);
        assert_eq!(fade_out_time(&Command::NextTrack, &t, SILENT), None);
    }

    #[test]
    fn other_commands_and_other_rows_do_not_fade() {
        let t = FadeTimes::default();
        assert_eq!(fade_out_time(&Command::QueueRemove(0), &t, PLAYING), None);
        assert_eq!(fade_out_time(&Command::Resume, &t, PLAYING), None);
        assert_eq!(fade_out_time(&Command::Seek(ms(1000)), &t, PLAYING), None);
        assert_eq!(fade_out_time(&Command::SetVolume(0.5), &t, PLAYING), None);
        assert_eq!(fade_out_time(&Command::QueueClear, &t, PLAYING), None);
    }

    #[test]
    fn a_zero_time_means_no_fade() {
        let t = FadeTimes { pause: Duration::ZERO, ..FadeTimes::default() };
        assert_eq!(fade_out_time(&Command::Pause, &t, PLAYING), None);
    }

    #[test]
    fn waiting_commands_keep_their_order() {
        let now = Instant::now();
        let mut fade = FadeOut::new(CommandEnvelope::new(Command::ReplaceQueue { items: vec![], position: 0 }), 7, ms(150), now);
        fade.wait(CommandEnvelope::new(Command::QueuePlayIndex(3)));
        let order: Vec<String> = fade.into_waiting().iter().map(|e| format!("{:?}", e.command)).collect();
        assert!(order[0].starts_with("ReplaceQueue"));
        assert_eq!(order[1], "QueuePlayIndex(3)");
    }

    #[test]
    fn resume_can_cancel_only_a_lone_pause() {
        let now = Instant::now();
        let mut fade = FadeOut::new(CommandEnvelope::new(Command::Pause), 1, ms(150), now);
        assert!(fade.only_a_pause());
        fade.wait(CommandEnvelope::new(Command::NextTrack));
        assert!(!fade.only_a_pause());
        let stop = FadeOut::new(CommandEnvelope::new(Command::Stop), 1, ms(150), now);
        assert!(!stop.only_a_pause());
    }

    #[test]
    fn due_when_landed_or_past_the_deadline() {
        let now = Instant::now();
        let fade = FadeOut::new(CommandEnvelope::new(Command::Stop), 1, ms(300), now);
        assert!(!fade.is_due(false, now));
        assert!(fade.is_due(true, now));
        assert!(fade.is_due(false, now + ms(300) + LANDING_GRACE + ms(1)));
    }

    #[test]
    fn volume_mute_and_fade_times_never_wait() {
        assert!(runs_during_fade(&Command::SetVolume(0.3)));
        assert!(runs_during_fade(&Command::SetMuted(true)));
        assert!(runs_during_fade(&Command::SetFades(FadeTimes::default())));
        assert!(!runs_during_fade(&Command::Seek(ms(10))));
        assert!(!runs_during_fade(&Command::QueueAdd(vec![])));
    }
}
```

  `CommandEnvelope` must be `Debug`-free: the tests format `e.command`, which derives `Debug`. If `Command` lacks `PartialEq`, the tests above avoid it on purpose.

- [ ] **Step 3: Run** `cargo test -p znicz-core --lib player::fade` — fails with `not yet implemented`.

- [ ] **Step 4: Implement.**

```rust
pub fn fade_out_time(command: &Command, times: &FadeTimes, playback: Playback) -> Option<Duration> {
    if !playback.audible {
        return None;
    }
    let time = match command {
        Command::Pause => times.pause,
        Command::Stop => times.stop,
        Command::NextTrack
        | Command::PreviousTrack
        | Command::Play(_)
        | Command::QueuePlayIndex(_)
        | Command::ReplaceQueue { .. }
        | Command::SetDevice(_) => times.track,
        Command::QueueRemove(index) if *index == playback.position => times.track,
        _ => return None,
    };
    (!time.is_zero()).then_some(time)
}

impl FadeOut {
    pub fn new(first: CommandEnvelope, generation: u64, time: Duration, now: Instant) -> Self {
        Self {
            generation,
            deadline: now + time + LANDING_GRACE,
            waiting: VecDeque::from([first]),
        }
    }

    pub fn wait(&mut self, envelope: CommandEnvelope) {
        self.waiting.push_back(envelope);
    }

    pub fn only_a_pause(&self) -> bool {
        self.waiting.len() == 1 && matches!(self.waiting[0].command, Command::Pause)
    }

    pub fn is_due(&self, landed: bool, now: Instant) -> bool {
        landed || now > self.deadline
    }

    pub fn into_waiting(self) -> VecDeque<CommandEnvelope> {
        self.waiting
    }
}

pub fn runs_during_fade(command: &Command) -> bool {
    matches!(
        command,
        Command::SetVolume(_) | Command::SetMuted(_) | Command::SetFades(_)
    )
}
```

- [ ] **Step 5: Run** the tests — 8 pass. `cargo build --workspace` (the IPC and MCP crates must still compile with the new variant; if a `match` on `Command` elsewhere is exhaustive, add the arm there doing nothing special).
- [ ] **Step 6: Commit** `feat(core): fade-out decisions and the SetFades command (#90)`.

---

### Task 4: Engine: fade out, wait, fade in (`player/engine.rs`)

**Files:** Modify `znicz-core/src/player/engine.rs`, `znicz-core/src/audio/gain.rs` (remove `MUTE_FADE` once unused; keep the gain tests compiling by using a local `Duration::from_millis(500)`).

Read the spec's "Engine" section first. No new unit tests here (the engine needs a sound card for playback); keep `cargo test -p znicz-core` green. Task 5 adds hardware tests.

- [ ] **Step 1: Config and fields.**
  - `AudioConfig` gets `pub fades: FadeTimes` (Default: `FadeTimes::default()`).
  - `PlayerEngine` gets `fades: FadeTimes` (from config) and `fade_out: Option<FadeOut>`.

- [ ] **Step 2: Route commands through the fade logic.** Replace `apply` with:

```rust
    /// Run one command, or hold it while a fade-out finishes.
    fn apply(&mut self, envelope: CommandEnvelope) {
        if let Some(fade) = self.fade_out.as_mut() {
            if runs_during_fade(&envelope.command) {
                self.run(envelope);
            } else if matches!(envelope.command, Command::Resume) && fade.only_a_pause() {
                self.cancel_pause(envelope);
            } else {
                fade.wait(envelope);
            }
            return;
        }
        match fade_out_time(&envelope.command, &self.fades, self.playback()) {
            Some(time) => {
                let generation = self.output.fade_to(0.0, time);
                self.fade_out = Some(FadeOut::new(envelope, generation, time, Instant::now()));
            }
            None => self.run(envelope),
        }
    }

    /// Run a command now and answer its caller.
    fn run(&mut self, envelope: CommandEnvelope) {
        let CommandEnvelope { command, ack } = envelope;
        let result = self.handle_command(command);
        if let Err(e) = &result {
            self.emit_error(e.to_string());
        }
        if let Some(ack) = ack {
            // A caller that gave up waiting is fine; ignore the send error.
            ack.send(result).ok();
        }
    }

    /// What fading needs to know about the engine now.
    fn playback(&self) -> Playback {
        let state = self.state.read().unwrap();
        Playback {
            audible: state.status == PlaybackStatus::Playing
                && (self.decoder.is_some() || self.draining_since.is_some()),
            position: state.queue_position,
        }
    }

    /// Resume during a pause's fade-out: the pause never happens; the sound
    /// comes back up from where the fade got to.
    fn cancel_pause(&mut self, resume: CommandEnvelope) {
        let Some(fade) = self.fade_out.take() else {
            return;
        };
        self.output.fade_to(1.0, self.fades.play);
        for envelope in fade.into_waiting().into_iter().chain([resume]) {
            if let Some(ack) = envelope.ack {
                ack.send(Ok(())).ok();
            }
        }
    }

    /// Once a fade-out landed, run the commands that waited for it, in order.
    /// A command that starts another fade-out takes the rest along.
    fn finish_fade_out(&mut self) {
        let due = self.fade_out.as_ref().is_some_and(|fade| {
            fade.is_due(self.output.fade_landed(fade.generation), Instant::now())
        });
        if !due {
            return;
        }
        let mut waiting = self.fade_out.take().map(FadeOut::into_waiting).unwrap_or_default();
        // The fade for the first command already happened.
        if let Some(first) = waiting.pop_front() {
            self.run(first);
        }
        while let Some(envelope) = waiting.pop_front() {
            self.apply(envelope);
            if let Some(fade) = self.fade_out.as_mut() {
                for rest in waiting.drain(..) {
                    fade.wait(rest);
                }
            }
        }
    }
```

  In `run()`'s loop, call `self.finish_fade_out();` right after the command-receiving block (before `pump_decode`). Make the idle wait 5 ms while `self.fade_out.is_some()` too.

  Note: a paused engine's fade-out never happens (it is not audible), so `finish_fade_out` only runs while sound plays.

- [ ] **Step 3: Fade in where a track starts from a command.**
  - `play_item(&mut self, item, fade_in: bool)`: before `self.output.open_stream(...)`, call `self.output.set_fade_now(if fade_in { 0.0 } else { 1.0 })`. After `set_paused(false)`, `if fade_in { self.output.fade_in(self.fades.play); }`.
  - `play_queue_index(&mut self, index, fade_in: bool)` passes it through.
  - Callers: `Command::Play` → `true`; `Command::QueuePlayIndex` → `true`; `next_track` → `true`; `previous_track` → `true`; `remove_from_queue` (playing row) → `true`; `finish_track` → `false`; `replay_current` → `false`.
  - `Command::Resume`: `self.output.set_paused(false); self.output.fade_in(self.fades.play);` then status and event as before.
  - `Command::Pause` (runs after the fade-out landed, or at once when the fade is off): unchanged.
  - `Command::SetDevice`: after the stream reopens (inside `if let Some(decoder)`), call `self.output.fade_in(self.fades.play)` when the status is Playing; when paused, `self.output.set_fade_now(1.0)` is not needed (the resume fades in).
  - `Command::SetMuted`: `self.output.set_gain(effective, self.fades.mute)`; remove the `MUTE_FADE` import, and the constant from `gain.rs` if nothing else uses it (tests: replace with `Duration::from_millis(500)`).
  - `Command::SetFades(times)`: `self.fades = times;` (replace the Task 3 placeholder arm).
  - `stop()`: also reset the fade gain for the next stream is not needed (each `play_item` sets it), but clear `self.fade_out` is NOT done here: `stop()` runs from inside waiting commands.

- [ ] **Step 4: Engine unit test** (no device needed) in `engine.rs` tests:

```rust
    #[test]
    fn fade_times_are_accepted() {
        let player = player();
        player
            .send_blocking(Command::SetFades(crate::player::fade::FadeTimes::default()))
            .expect("set fades");
    }
```

- [ ] **Step 5: Verify:** `cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`. Commit `feat(core): fade out before pause, stop and track changes; fade in on play (#90)`.

---

### Task 5: Hardware tests for fades (`tests/playback.rs`)

**Files:** Modify `znicz-core/tests/playback.rs` (reuse `skip_hardware_playback`, `write_silent_wav`). This machine has a sound card, so these run for real.

- [ ] **Step 1: Add tests:**

```rust
fn playing(seconds: u32, fades: znicz_core::FadeTimes) -> (znicz_core::PlayerHandle, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let wav = dir.path().join("fade.wav");
    write_silent_wav(&wav, 44_100, 2, seconds);
    let (player, _thread) = spawn_player(AudioConfig { fades, ..AudioConfig::default() });
    player
        .send_blocking(Command::Play(znicz_core::QueueItem::file(wav)))
        .expect("play");
    (player, dir)
}

#[test]
fn pause_waits_for_its_fade() {
    if skip_hardware_playback() {
        return;
    }
    let fades = znicz_core::FadeTimes { pause: Duration::from_millis(300), ..Default::default() };
    let (player, _dir) = playing(5, fades);
    std::thread::sleep(Duration::from_millis(300));
    let started = Instant::now();
    player.send_blocking(Command::Pause).expect("pause");
    let took = started.elapsed();
    assert_eq!(player.state().status, znicz_core::PlaybackStatus::Paused);
    assert!(took >= Duration::from_millis(250), "pause returned after {took:?}");
    assert!(took < Duration::from_millis(1500), "pause took {took:?}");
}

#[test]
fn resume_during_a_pause_fade_keeps_playing() {
    if skip_hardware_playback() {
        return;
    }
    let fades = znicz_core::FadeTimes { pause: Duration::from_millis(500), ..Default::default() };
    let (player, _dir) = playing(5, fades);
    std::thread::sleep(Duration::from_millis(300));
    player.send(Command::Pause).expect("pause");
    player.send_blocking(Command::Resume).expect("resume");
    std::thread::sleep(Duration::from_millis(700));
    assert_eq!(player.state().status, znicz_core::PlaybackStatus::Playing);
}

#[test]
fn next_returns_with_the_new_track_playing() {
    if skip_hardware_playback() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let a = dir.path().join("a.wav");
    let b = dir.path().join("b.wav");
    write_silent_wav(&a, 44_100, 2, 5);
    write_silent_wav(&b, 44_100, 2, 5);
    let (player, _thread) = spawn_player(AudioConfig::default());
    player
        .send_blocking(Command::ReplaceQueue {
            items: vec![znicz_core::QueueItem::file(a), znicz_core::QueueItem::file(b.clone())],
            position: 0,
        })
        .expect("queue");
    player.send_blocking(Command::QueuePlayIndex(0)).expect("play");
    std::thread::sleep(Duration::from_millis(300));
    player.send_blocking(Command::NextTrack).expect("next");
    let state = player.state();
    assert_eq!(state.queue_position, 1);
    assert_eq!(state.current_track.and_then(|t| t.path), Some(b));
}
```

  Re-export `FadeTimes` and `PlaybackStatus` from `znicz_core` if they are not already (`lib.rs`; check with `grep -n "PlaybackStatus\|FadeTimes" znicz-core/src/lib.rs`).

- [ ] **Step 2: Run** `cargo test -p znicz-core --test playback -- --test-threads=1`. All must pass for real. Do not loosen an assertion to make it pass; report a failure instead.
- [ ] **Step 3: Commit** `test(core): fades on real output (#90)`.

---

### Task 6: Nested sections in `ConfigFile` (`znicz-tui/src/config_file.rs`)

**Files:** Modify `znicz-tui/src/config_file.rs`.

- [ ] **Step 1: Failing tests** (use the module's `file_in` / `read` helpers):

```rust
    #[test]
    fn a_dotted_section_is_a_nested_table() {
        let dir = TempDir::new().unwrap();
        let config = file_in(&dir, "config.toml", "[audio]\n# keep me\nvolume = 1.0\n");
        config.set_int("audio.fade", "play_ms", 200).unwrap();
        let text = read(&config);
        assert!(text.contains("# keep me"));
        assert!(text.contains("[audio.fade]"), "{text}");
        assert!(text.contains("play_ms = 200"), "{text}");
        let parsed: toml::Table = text.parse().unwrap();
        assert_eq!(parsed["audio"]["fade"]["play_ms"].as_integer(), Some(200));
    }

    #[test]
    fn remove_finds_a_nested_key() {
        let dir = TempDir::new().unwrap();
        let config = file_in(&dir, "config.toml", "[audio.fade]\nplay_ms = 200\nstop_ms = 300\n");
        config.remove("audio.fade", "play_ms").unwrap();
        let text = read(&config);
        assert!(!text.contains("play_ms"));
        assert!(text.contains("stop_ms = 300"));
    }
```

- [ ] **Step 2: Run** `cargo test -p znicz-tui --lib config_file` — the first test fails (a key `"audio.fade"` is written instead).
- [ ] **Step 3: Implement:** a helper that walks `section.split('.')`, creating missing tables with `Table::new()`; for a created parent table call `set_implicit(true)` so `[audio]` is not emitted empty when only `[audio.fade]` is needed. `edit` uses it; `remove` walks without creating (a missing table is a no-op, as today). Error text keeps `[{section}] is not a table`.
- [ ] **Step 4: Run** the config_file tests — all pass. **Commit** `feat(tui): nested config sections such as [audio.fade] (#90)`.

---

### Task 7: `[audio.fade]` in the binary config (`znicz/src/main.rs`, `default-config.toml`)

**Files:** Modify `znicz/src/main.rs`, `znicz/src/default-config.toml`, `znicz-tui/src/settings.rs` (`SettingsValues.fades`).

- [ ] **Step 1: Failing tests** in `main.rs` tests (next to the existing `parse_config_text` tests):

```rust
    #[test]
    fn fade_times_parse_and_default() {
        let config = parse_config_text("[audio.fade]\nplay_ms = 0\nstop_ms = 800\n").unwrap();
        let fades = fade_times(&config);
        assert_eq!(fades.play, Duration::ZERO);
        assert_eq!(fades.stop, Duration::from_millis(800));
        assert_eq!(fades.pause, FadeTimes::default().pause);
    }

    #[test]
    fn a_bad_fade_value_is_a_config_error() {
        let err = parse_config_text("[audio.fade]\npause_ms = \"slow\"\n").unwrap_err();
        assert!(matches!(err, ConfigProblem::Parse { line: 2, .. }), "{err:?}");
    }

    #[test]
    fn the_default_config_file_has_the_default_fades() {
        let config = parse_config_text(DEFAULT_CONFIG).unwrap();
        assert_eq!(fade_times(&config), FadeTimes::default());
    }
```

  (`DEFAULT_CONFIG` is whatever the existing `include_str!("default-config.toml")` constant is called; use its real name.)

- [ ] **Step 2: Implement.**
  - `AudioSection` gets `#[serde(default)] fade: FadeSection` with `FadeSection { play_ms, pause_ms, stop_ms, track_ms, mute_ms: Option<u64> }` (`Deserialize, Default`).
  - `fn fade_times(config: &Config) -> FadeTimes`: each `Option<u64>` maps to `Duration::from_millis`, missing → `FadeTimes::default()`'s value.
  - `AudioConfig { fades: fade_times(&config), .. }` wherever `AudioConfig` is built (both places in `main.rs`).
  - `SettingsValues` (in `znicz-tui/src/settings.rs`) gets `pub fades: FadeTimes` (Default: `FadeTimes::default()`); `main.rs` fills it from `fade_times(&config)`.
  - `default-config.toml`, after `bit_perfect = true` in `[audio]`:

```toml

[audio.fade]
# Fade times in milliseconds; 0 turns a fade off.
play_ms = 150    # play and resume (fade in)
pause_ms = 150
stop_ms = 300
track_ms = 150   # manual track change and device switch (fade out)
mute_ms = 500
```

- [ ] **Step 3: Run** `cargo test -p znicz` and `cargo build --workspace`. **Commit** `feat: [audio.fade] config (#90)`.

---

### Task 8: Settings model: tabs and fade rows (`znicz-tui/src/settings.rs`)

**Files:** Modify `znicz-tui/src/settings.rs`. (The UI in `app/settings_ui.rs` and `views/settings.rs` will not compile after this task's API change until Task 9/10; to keep each commit building, this task also makes the minimal call-site changes listed in Step 4.)

- [ ] **Step 1: Failing tests** (in `settings.rs` tests; update the `values()` helper to include `fades: FadeTimes::default()`):

```rust
    #[test]
    fn each_tab_has_its_rows() {
        let folders = [folder("jazz")];
        assert_eq!(rows(Tab::Audio, &folders), vec![Row::DefaultDevice, Row::BitPerfect]);
        assert_eq!(
            rows(Tab::Playback, &folders),
            vec![Row::FadePlay, Row::FadePause, Row::FadeStop, Row::FadeTrack, Row::FadeMute]
        );
        assert_eq!(
            rows(Tab::Library, &folders),
            vec![Row::Folder(folder("jazz")), Row::AddFolder, Row::RescanAll]
        );
        assert_eq!(rows(Tab::General, &folders), vec![Row::UpdateCheck]);
        assert_eq!(rows(Tab::Display, &folders)[0], Row::ShowCover);
    }

    #[test]
    fn tabs_have_labels_and_number_keys() {
        assert_eq!(Tab::ALL.map(Tab::label), ["Display", "Audio", "Playback", "Library", "General"]);
        assert_eq!(Tab::from_key('3'), Some(Tab::Playback));
        assert_eq!(Tab::from_key('6'), None);
        assert_eq!(Tab::General.next(), Tab::Display);
        assert_eq!(Tab::Display.previous(), Tab::General);
    }

    #[test]
    fn fade_rows_show_ms_or_off() {
        let tui = TuiConfig::default();
        let mut v = values(&tui);
        v.fades.pause = Duration::ZERO;
        v.fades.play = Duration::from_millis(240);
        assert_eq!(value_text(&Row::FadePause, &v), "off");
        assert_eq!(value_text(&Row::FadePlay, &v), "240 ms");
        assert_eq!(value_text(&Row::FadeStop, &v), "300 ms");
    }

    #[test]
    fn fade_steps_follow_the_ladder_and_snap() {
        let ms = Duration::from_millis;
        assert_eq!(step_fade(ms(150), true, false), ms(200));
        assert_eq!(step_fade(ms(150), false, false), ms(100));
        assert_eq!(step_fade(Duration::ZERO, false, false), Duration::ZERO);
        assert_eq!(step_fade(ms(2000), true, false), ms(2000));
        assert_eq!(step_fade(ms(2000), true, true), Duration::ZERO);
        // Off the ladder: to the next value in that direction.
        assert_eq!(step_fade(ms(240), true, false), ms(300));
        assert_eq!(step_fade(ms(240), false, false), ms(200));
    }

    #[test]
    fn fade_rows_save_under_audio_fade() {
        assert_eq!(Row::FadePlay.config_key(), Some(("audio.fade", "play_ms")));
        assert_eq!(Row::FadeMute.config_key(), Some(("audio.fade", "mute_ms")));
    }
```

  Delete tests that only exist for headings (`first_selectable`/`next_selectable` skipping headings) or rewrite them for a heading-free list.

- [ ] **Step 2: Implement.**
  - `#[derive(Debug, Clone, Copy, PartialEq, Eq)] pub enum Tab { Display, Audio, Playback, Library, General }` with `pub const ALL: [Tab; 5]`, `label(self) -> &'static str`, `from_key(c: char) -> Option<Tab>` (`'1'..='5'`), `index(self) -> usize`, `next(self)` / `previous(self)` (wrapping).
  - `pub fn rows(tab: Tab, folders: &[PathBuf]) -> Vec<Row>` per the spec's table. Remove `Row::Heading`, `Kind::Heading`, `Row::selectable`, `first_selectable`, `last_selectable`, `nearest_selectable`, `next_selectable` (Task 9 replaces their callers with plain index arithmetic). If removing them breaks callers you are not changing in Step 4, keep a simplified version for now and note it.
  - `Row::tab(&self) -> Tab` (which tab a row lives on).
  - New rows `FadePlay, FadePause, FadeStop, FadeTrack, FadeMute`: `Kind::Choice`; labels "Fade in (play, resume)", "Fade on pause", "Fade on stop", "Fade on track change", "Mute fade"; explanations:
    - FadePlay: "Fade in over this time on play, resume, and after a track change or device switch. Off starts at full volume."
    - FadePause: "Fade out over this time before pausing. Off pauses at once."
    - FadeStop: "Fade out over this time before stopping. Off stops at once."
    - FadeTrack: "Fade out over this time before Next, Previous, a new track or a device switch. Off cuts at once."
    - FadeMute: "How long mute and unmute take."
  - `config_key`: `("audio.fade", "play_ms" | "pause_ms" | "stop_ms" | "track_ms" | "mute_ms")`.
  - `Values` gets `pub fades: FadeTimes`. `value_text` for a fade row: `"off"` for zero, else `"{n} ms"`.
  - `pub const FADE_LADDER_MS: [u64; 11] = [0, 50, 100, 150, 200, 300, 500, 750, 1000, 1500, 2000];` and

```rust
/// One step along the fade ladder. A value between steps moves to the next
/// step in that direction; at an end it wraps when `wrap` is set.
pub fn step_fade(current: Duration, forward: bool, wrap: bool) -> Duration {
    let now = current.as_millis() as u64;
    let next = if forward {
        FADE_LADDER_MS.iter().copied().find(|&ms| ms > now)
    } else {
        FADE_LADDER_MS.iter().rev().copied().find(|&ms| ms < now)
    };
    let ms = match (next, wrap, forward) {
        (Some(ms), _, _) => ms,
        (None, true, true) => FADE_LADDER_MS[0],
        (None, true, false) => FADE_LADDER_MS[FADE_LADDER_MS.len() - 1],
        (None, false, _) => now,
    };
    Duration::from_millis(ms)
}
```

  - `pub fn fade_slot<'a>(row: &Row, fades: &'a mut FadeTimes) -> Option<&'a mut Duration>` mapping the five rows to the fields.

- [ ] **Step 3: Run** `cargo test -p znicz-tui --lib settings::` until the model tests pass.
- [ ] **Step 4: Keep the crate building** with minimal call-site edits (Task 9 does the real UI work):
  - `app/settings_ui.rs` and `app.rs`: call `settings::rows(Tab::Display, ...)` where `rows(...)` was called; replace `first_selectable(..)` with `0`, `last_selectable(rows)` with `rows.len().saturating_sub(1)`, `next_selectable` with a ±1 clamp, `nearest_selectable(rows, i)` with `i.min(rows.len().saturating_sub(1))`; remove `Kind::Heading` arms; the click handler no longer checks `selectable`.
  - `views/settings.rs`: remove the `Row::Heading` branch of `row_line`; add `fades: app.fades` to `values()` (add a `pub fades: FadeTimes` field to `App`, initialised from `SettingsValues.fades` in `with_settings_values`, default `FadeTimes::default()`).
  - Add `pub(crate) settings_tab: Tab` to `App` (default `Tab::Display`); the
    modal shows `settings::rows(self.settings_tab, …)`.
  - In the `settings_ui.rs` tests, the `select(app, row)` helper first sets
    `app.settings_tab = row.tab()` and rebuilds `app.settings_rows`, so tests
    of rows outside Display keep working.
  - Delete tests that only checked headings (`cursor_skips_headings`, the
    heading part of the click test).
  - `cargo test -p znicz-tui` passes.
- [ ] **Step 5: Commit** `feat(tui): Settings tabs model and fade rows (#90)`.

---

### Task 9: Settings keys, focus and fade changes (`app/settings_ui.rs`, `app.rs`, `keys.rs`)

**Files:** Modify `znicz-tui/src/app/settings_ui.rs`, `znicz-tui/src/app.rs`, `znicz-tui/src/keys.rs`.

State on `App` (replace `settings_rows`/`settings_cursor` semantics where needed):
- `settings_tab: Tab` (kept across openings in this run),
- `settings_on_tab_bar: bool`,
- `settings_rows: Vec<Row>` = `settings::rows(self.settings_tab, &self.library_status.folders)`,
- `settings_cursor: usize` (row index on the active tab).

- [ ] **Step 1: Failing tests** in `settings_ui.rs` tests (use the existing `settings_app`, `press`, `saved`, `screen_rows` helpers; add a `press_mod(app, code, modifiers)` helper if needed):

```rust
    #[test]
    fn tab_and_shift_tab_switch_tabs_and_wrap() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = settings_app(&dir);
        press(&mut app, KeyCode::Char('S'));
        assert_eq!(app.settings_tab, Tab::Display);
        press(&mut app, KeyCode::Tab);
        assert_eq!(app.settings_tab, Tab::Audio);
        assert_eq!(app.settings_rows[app.settings_cursor], Row::DefaultDevice);
        press(&mut app, KeyCode::BackTab);
        press(&mut app, KeyCode::BackTab);
        assert_eq!(app.settings_tab, Tab::General);
    }

    #[test]
    fn number_keys_jump_to_a_tab() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = settings_app(&dir);
        press(&mut app, KeyCode::Char('S'));
        press(&mut app, KeyCode::Char('3'));
        assert_eq!(app.settings_tab, Tab::Playback);
        assert_eq!(app.settings_rows[0], Row::FadePlay);
    }

    #[test]
    fn up_on_the_first_row_focuses_the_tab_bar_where_arrows_switch_tabs() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = settings_app(&dir);
        press(&mut app, KeyCode::Char('S'));
        press(&mut app, KeyCode::Up);
        assert!(app.settings_on_tab_bar);
        press(&mut app, KeyCode::Right);
        assert_eq!(app.settings_tab, Tab::Audio);
        press(&mut app, KeyCode::Char('l'));
        assert_eq!(app.settings_tab, Tab::Playback);
        press(&mut app, KeyCode::Down);
        assert!(!app.settings_on_tab_bar);
        assert_eq!(app.settings_cursor, 0);
    }

    #[test]
    fn arrows_on_the_tab_bar_stop_at_the_ends() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = settings_app(&dir);
        press(&mut app, KeyCode::Char('S'));
        press(&mut app, KeyCode::Up);
        press(&mut app, KeyCode::Left);
        assert_eq!(app.settings_tab, Tab::Display);
    }

    #[test]
    fn row_keys_do_nothing_on_the_tab_bar() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = settings_app(&dir);
        press(&mut app, KeyCode::Char('S'));
        let before = app.tui.show_cover;
        press(&mut app, KeyCode::Up);
        press(&mut app, KeyCode::Char(' '));
        assert_eq!(app.tui.show_cover, before);
        assert_eq!(app.modal, Modal::Settings);
    }

    #[test]
    fn a_fade_change_saves_and_reaches_the_player() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = settings_app(&dir);
        press(&mut app, KeyCode::Char('S'));
        press(&mut app, KeyCode::Char('3'));
        press(&mut app, KeyCode::Char('l')); // play 150 -> 200
        assert_eq!(app.fades.play, Duration::from_millis(200));
        let saved = saved(&dir);
        assert_eq!(saved["audio"]["fade"]["play_ms"].as_integer(), Some(200));
    }

    #[test]
    fn the_last_tab_is_kept_for_the_next_opening() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = settings_app(&dir);
        press(&mut app, KeyCode::Char('S'));
        press(&mut app, KeyCode::Char('4'));
        press(&mut app, KeyCode::Esc);
        press(&mut app, KeyCode::Char('S'));
        assert_eq!(app.settings_tab, Tab::Library);
        assert!(!app.settings_on_tab_bar);
    }
```

  Also update `choosing_a_default_device_switches_and_saves` (and similar) to expect the Audio tab after returning from Devices: `assert_eq!(app.settings_tab, Tab::Audio)`.

  Check how the existing tests' `settings_app` builds a player: `SetFades` is sent with `self.player.send(...)` (fire and forget), so a test player is fine.

- [ ] **Step 2: Implement.**
  - `toggle_settings_modal`: on open, `settings_on_tab_bar = false`, rebuild rows for `settings_tab`, cursor 0, scroll 0.
  - `fn set_settings_tab(&mut self, tab: Tab)`: set tab, rebuild rows, cursor 0, scroll 0, `settings_confirm = None`.
  - `on_settings_key` order:
    1. `Tab` → `set_settings_tab(tab.next())`; `BackTab` → `previous()`; `Char(c)` with `Tab::from_key(c)` → that tab. These work from the bar and the rows; focus stays where it is.
    2. If `settings_on_tab_bar`: `Left`/`h` → previous tab unless Display; `Right`/`l` → next tab unless General; `Down`/`j`/`Enter` → focus rows (`settings_on_tab_bar = false`, cursor 0). Every other key: return `true` for `Char(' ')` and `Char('d')` (swallow row keys), else `false` (so `Esc`, `?`, global keys still work as today).
    3. In the rows: `Up`/`k` when `settings_cursor == 0` → focus the bar (`return true`). The rest as today.
  - `settings_move(delta)`: plain clamp within `0..rows.len()`; `Ctrl-u`/`g` paths in `app.rs` set cursor 0; `G` sets last.
  - `apply_setting` for a fade row: `let slot = settings::fade_slot(row, &mut self.fades)`; `*slot = settings::step_fade(*slot, forward, wrap)`; changed when different.
  - `after_setting_changed` for a fade row: `self.player.send(Command::SetFades(self.fades)).ok();` (an error is shown as other player errors are: use the same pattern the file uses for `send`, e.g. toast on `Err`).
  - `stored_value` for a fade row: `Stored::Int(slot.as_millis() as i64)`.
  - `SettingsSnapshot` keeps `fades` and `restore_settings` puts it back.
  - `select_setting(row)`: `set_settings_tab(row.tab())` first, then find the row.
  - `refresh_settings_rows`: rows for the current tab.
  - Mouse: a click on the overlay list selects that row (no heading check now). Clicks on the tab bar are not required (no hit area yet).
  - `keys.rs`: new `Context::SettingsTabs` (add to `ALL_CONTEXTS` in tests and to `context_title` → `Some("Settings")`); hints: `SettingsTabs => &[TAB_SWITCH, ROWS, CLOSE]` with `const TAB_SWITCH: Hint = hint("←/→ tab", KeyCode::Right); const ROWS: Hint = hint("↓ rows", KeyCode::Down);` and `Settings => &[ENTER_CHANGE, CHOOSE, NEXT_TAB, CLOSE]` with `const NEXT_TAB: Hint = hint("Tab next tab", KeyCode::Tab);`. Check the footer-width test still passes (shorten a label if a test complains).
  - `SETTINGS` bindings table: add `b("Tab / Shift-Tab", "next / previous tab")`, `b("1–5", "go to a tab")`, `b("↑ on the first row", "tab bar")`.
  - `app.rs` context mapping: `Modal::Settings => if self.settings_on_tab_bar { Context::SettingsTabs } else { Context::Settings }`.
  - A footer click on the `Tab next tab` hint must act like the key: check how footer hints dispatch their `KeyCode` (`footer_hits`) — `KeyCode::Tab` must reach `on_settings_key` while Settings is open (it does if the hit dispatch goes through the normal key path; verify with a quick test or reading the code).

- [ ] **Step 3: Run** `cargo test -p znicz-tui` until all pass; clippy clean.
- [ ] **Step 4: Commit** `feat(tui): Settings tabs keys and live fade settings (#90)`.

---

### Task 10: Draw the tab bar (`views/settings.rs`)

**Files:** Modify `znicz-tui/src/views/settings.rs`; test in `settings_ui.rs` tests (they already render with `screen_rows`).

- [ ] **Step 1: Failing tests:**

```rust
    #[test]
    fn the_tab_bar_shows_every_tab() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = settings_app(&dir);
        press(&mut app, KeyCode::Char('S'));
        let screen = screen_rows(&mut app).join("\n");
        for label in ["Display", "Audio", "Playback", "Library", "General"] {
            assert!(screen.contains(label), "{label} missing:\n{screen}");
        }
    }

    #[test]
    fn the_modal_keeps_its_height_across_tabs() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = settings_app(&dir);
        press(&mut app, KeyCode::Char('S'));
        let _ = screen_rows(&mut app);
        let display = app.hits.overlay;
        press(&mut app, KeyCode::Char('5'));
        let _ = screen_rows(&mut app);
        assert_eq!(app.hits.overlay, display);
    }
```

- [ ] **Step 2: Implement.**
  - Inside the block: line 0 = tab bar, line 1 = a dim `─` rule, then the rows list, then the existing rule + strip.
  - Tab bar spans: ` Display │ Audio │ Playback │ Library │ General ` — `│` dim; the active tab `theme::strong()` when the rows have focus, and the list cursor style (whatever `row_line` uses for the selected row) when `app.settings_on_tab_bar`; others `theme::text()`. If the bar is wider than the modal, cut it (no wrap).
  - The popup rect: today `list_modal_rect(list)`; keep it (it already has a fixed size independent of the row count — verify; if it depends on the row count, size it by the tallest tab: `Tab::ALL.map(|t| rows(t, folders).len()).max()`).
  - Row lines no longer have headings; the selected row is only highlighted when the rows have focus.
  - `hits.overlay_list` must point at the rows area (below the tab bar) so clicks still map to rows.

- [ ] **Step 3: Run** `cargo test -p znicz-tui`. **Commit** `feat(tui): Settings tab bar (#90)`.

---

### Task 11: Docs, version, release prep

**Files:** `Cargo.toml` (0.18.0), `Cargo.lock`, `CHANGELOG.md`, `README.md` (config template: `[audio.fade]` block from Task 7), `THIRD-PARTY-LICENSES.html` (run `scripts/third-party-licenses.sh`), wiki: `wiki/Architecture/TUI.md` (Settings section: tabs, keys table, Playback rows; remove "grouped under headings the cursor skips"), `wiki/Architecture/Audio-Threading.md` (two gains, the landed marker), `wiki/Architecture/Core-Engine.md` (fade-outs and waiting commands), `wiki/Domain/Playback-Pipeline.md` (fades and bit-perfect).

- [ ] **Step 1:** Bump the version, `cargo build --workspace`, run the licences script.
- [ ] **Step 2:** CHANGELOG:

```markdown
## 0.18.0 - 2026-11-18

- Fades: play and resume fade in; pause, stop, Next/Previous, choosing a new
  track and switching the output device fade out first. Times are set in
  `[audio.fade]` (milliseconds, 0 = off) and in Settings → Playback, where a
  change applies at once. Defaults: in 150 ms, pause 150 ms, stop 300 ms,
  track change 150 ms, mute 500 ms. Automatic track changes and gapless joins
  do not fade. Steady playback stays bit-perfect.
- Settings has tabs: Display, Audio, Playback, Library, General. `Tab` /
  `Shift-Tab` or `1`–`5` switch tabs; `↑` on the first row moves to the tab
  bar, where `←` / `→` switch.
```

- [ ] **Step 3:** Wiki and README edits: read each page first, match its style, describe exactly what was built (check names against the code).
- [ ] **Step 4:** `cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`. Commit `chore: release 0.18.0 (fades, Settings tabs)`.
