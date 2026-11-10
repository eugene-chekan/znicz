# Queue Reorder, Finer Volume, and Mute Fades Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Move queue rows (`Alt-↑` / `Alt-↓`, MCP `queue_move`), step volume by 1 % with an eighth-cell volume bar, and fade mute / unmute over 500 ms (20 ms ramps for other gain changes).

**Architecture:** A new `Command::QueueMove { from, to }` in the engine with a pure `moved_position` helper. The audio output replaces its single gain atomic with a `GainControl` (target, ramp time, generation); each stream callback keeps a `GainRamp` that moves the gain per frame and lands exactly on the target.

**Tech Stack:** Rust 2021, cpal, ratatui 0.30, rmcp, cargo test.

**Spec:** [`docs/superpowers/specs/2026-11-09-queue-reorder-and-volume-design.md`](../specs/2026-11-09-queue-reorder-and-volume-design.md)

**Branch:** `feat/queue-reorder-and-volume` — every task starts with `git branch --show-current` and stops if it is anything else.

**Commands:** `cargo test -p <crate> [--lib|--test <name>] <filter>`; lint `cargo clippy --workspace --all-targets -- -D warnings`; format `cargo fmt --all -- --check`.

Every commit message ends with a blank line then `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.

---

### Task 1: Commit the plan

- [x] **Step 1:** `git add docs/superpowers/plans/2026-11-10-queue-reorder-and-volume.md && git commit -m "Add queue reorder and volume implementation plan"`

---

### Task 2: `Command::QueueMove` in the engine (`znicz-core`)

**Files:**
- Modify: `znicz-core/src/player/commands.rs`, `znicz-core/src/player/engine.rs`
- Test: `znicz-core/src/player/engine.rs` (new tests module), `znicz-core/tests/queue.rs`

- [x] **Step 1: Write the failing tests**

At the bottom of `znicz-core/src/player/engine.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::moved_position;

    #[test]
    fn the_playing_row_follows_itself() {
        assert_eq!(moved_position(2, 2, 0), 0);
        assert_eq!(moved_position(2, 2, 4), 4);
    }

    #[test]
    fn rows_crossing_the_playing_row_shift_it() {
        // A row from above moves to or below the playing row: it shifts up.
        assert_eq!(moved_position(2, 0, 2), 1);
        assert_eq!(moved_position(2, 1, 4), 1);
        // A row from below moves to or above it: it shifts down.
        assert_eq!(moved_position(2, 4, 2), 3);
        assert_eq!(moved_position(2, 3, 0), 3);
    }

    #[test]
    fn rows_that_do_not_cross_leave_it_alone() {
        assert_eq!(moved_position(2, 0, 1), 2);
        assert_eq!(moved_position(2, 3, 4), 2);
    }
}
```

In `znicz-core/tests/queue.rs`:

```rust
#[test]
fn moving_a_row_reorders_the_queue_and_keeps_the_playing_index() {
    let (player, _thread) = spawn_player(AudioConfig::default());
    player
        .send_blocking(Command::ReplaceQueue {
            items: paths(4),
            position: 2,
        })
        .expect("replace queue");

    player
        .send_blocking(Command::QueueMove { from: 0, to: 3 })
        .expect("move");

    let state = player.state();
    let names: Vec<_> = state.queue.iter().map(|i| format!("{i:?}")).collect();
    assert!(names[3].contains("track-0"), "{names:?}");
    assert!(names[0].contains("track-1"), "{names:?}");
    assert_eq!(state.queue_position, 1, "track-2 is now at index 1");
}

#[test]
fn moving_out_of_range_is_an_error() {
    let (player, _thread) = spawn_player(AudioConfig::default());
    player
        .send_blocking(Command::QueueAdd(paths(2)))
        .expect("queue add");
    assert!(player
        .send_blocking(Command::QueueMove { from: 0, to: 5 })
        .is_err());
    assert_eq!(player.state().queue.len(), 2);
}

#[test]
fn moving_a_row_onto_itself_does_nothing() {
    let (player, _thread) = spawn_player(AudioConfig::default());
    player
        .send_blocking(Command::QueueAdd(paths(3)))
        .expect("queue add");
    player
        .send_blocking(Command::QueueMove { from: 1, to: 1 })
        .expect("no-op move");
    assert_eq!(player.state().queue, paths(3));
}
```

(If `QueueItem` has no `Debug` that shows the path, compare with `QueueItem::file("/music/track-0.flac")` directly instead of the `format!`.)

- [x] **Step 2: Run** — `cargo test -p znicz-core moved_position` and `--test queue moving` — compile errors (`QueueMove`, `moved_position`).

- [x] **Step 3: Implement**

`commands.rs`, in `enum Command` after `QueueRemove(usize),`:

```rust
    /// Move one queue row to another index (both 0-based, as the queue is
    /// before the move). The playing row stays the playing row.
    QueueMove { from: usize, to: usize },
```

`engine.rs`:
- In the command match: `Command::QueueMove { from, to } => self.move_in_queue(from, to)?,`
- After `remove_from_queue`:

```rust
    fn move_in_queue(&mut self, from: usize, to: usize) -> Result<()> {
        {
            let mut state = self.state.write().unwrap();
            let len = state.queue.len();
            if from >= len || to >= len {
                return Err(ZniczError::Player("queue index out of range".into()));
            }
            if from == to {
                return Ok(());
            }
            let item = state.queue.remove(from);
            state.queue.insert(to, item);
            state.queue_position = moved_position(state.queue_position, from, to);
        }
        self.event_tx.send(PlayerEvent::QueueChanged).ok();
        self.emit_state_changed();
        Ok(())
    }
```

- A free function above the tests module:

```rust
/// Where the playing row sits after the row at `from` moves to `to`.
pub(crate) fn moved_position(position: usize, from: usize, to: usize) -> usize {
    if from == position {
        to
    } else if from < position && to >= position {
        position - 1
    } else if from > position && to <= position {
        position + 1
    } else {
        position
    }
}
```

(Use whatever error type/constructor the engine already uses for command errors; `ZniczError::Player` is the existing pattern.)

- [x] **Step 4:** `cargo test -p znicz-core`, clippy, fmt; commit `Move queue rows in the engine`.

---

### Task 3: `Alt-↑` / `Alt-↓` in the queue drawer (`znicz-tui`)

**Files:**
- Modify: `znicz-tui/src/app.rs`, `znicz-tui/src/keys.rs`
- Test: `znicz-tui/tests/keys.rs`

- [x] **Step 1: Write the failing tests** (in `tests/keys.rs`; helpers `new_app`, `queue`, `open_queue`, `press_char` exist)

```rust
fn press_alt(app: &mut App, code: KeyCode) {
    app.on_key(KeyEvent::new(code, KeyModifiers::ALT));
}

#[test]
fn alt_down_moves_the_highlighted_queue_row_and_the_cursor() {
    let mut app = new_app();
    queue(&mut app, 3);
    open_queue(&mut app);
    press_alt(&mut app, KeyCode::Down);
    let state = app.state();
    assert_eq!(state.queue[1], znicz_core::QueueItem::file("/music/track-0.flac"));
    assert_eq!(app.queue_cursor.index(), 1, "the cursor follows the row");
    press_alt(&mut app, KeyCode::Up);
    assert_eq!(app.state().queue[0], znicz_core::QueueItem::file("/music/track-0.flac"));
    assert_eq!(app.queue_cursor.index(), 0);
}

#[test]
fn alt_arrows_stop_at_the_ends() {
    let mut app = new_app();
    queue(&mut app, 2);
    open_queue(&mut app);
    press_alt(&mut app, KeyCode::Up);
    assert_eq!(app.state().queue[0], znicz_core::QueueItem::file("/music/track-0.flac"));
    press_char(&mut app, 'G');
    press_alt(&mut app, KeyCode::Down);
    assert_eq!(app.state().queue[1], znicz_core::QueueItem::file("/music/track-1.flac"));
    assert!(app.toasts.is_empty(), "no error at the ends");
}

#[test]
fn alt_arrows_do_nothing_with_library_focus() {
    let mut app = new_app();
    queue(&mut app, 2);
    press_alt(&mut app, KeyCode::Down);
    assert_eq!(app.state().queue[0], znicz_core::QueueItem::file("/music/track-0.flac"));
}
```

- [x] **Step 2: Run** — FAIL (Alt-↓ currently falls into the plain `Down` arm and just moves the cursor).

- [x] **Step 3: Implement**

`app.rs` `on_global_key`: **before** the `KeyCode::Down | KeyCode::Char('j')` and `KeyCode::Up | KeyCode::Char('k')` arms (and next to the existing Alt-Left/Right pan arms), add

```rust
            KeyCode::Up if key.modifiers.contains(KeyModifiers::ALT) => self.move_queue_row(-1),
            KeyCode::Down if key.modifiers.contains(KeyModifiers::ALT) => self.move_queue_row(1),
```

and the method:

```rust
    /// `Alt-↑` / `Alt-↓`: move the highlighted queue row; the cursor follows it.
    fn move_queue_row(&mut self, delta: isize) {
        if self.modal != Modal::None || !self.queue_open || self.focus != Focus::Queue {
            return;
        }
        let len = self.player.state().queue.len();
        let Some(from) = self.queue_cursor.selected(len) else {
            return;
        };
        let to = from as isize + delta;
        if to < 0 || to as usize >= len {
            return;
        }
        let to = to as usize;
        self.apply(Command::QueueMove { from, to }, None);
        self.queue_cursor.set(to, len);
        self.queue_h_offset = 0;
    }
```

`keys.rs` `QUEUE`: add `b("Alt-↑ / Alt-↓", "move the row up / down"),` after the `o` row.

- [x] **Step 4:** `cargo test -p znicz-tui`, clippy, fmt; commit `Move queue rows with Alt-arrows`.

---

### Task 4: MCP `queue_move` (`znicz-mcp`)

**Files:**
- Modify: `znicz-mcp/src/server.rs`, `znicz-mcp/skills/*/SKILL.md` (whichever lists queue tools)

- [x] **Step 1: Write the failing tests** (next to `queue_remove_drops_one_row_and_returns_the_new_queue`)

```rust
    #[test]
    fn queue_move_reorders_and_returns_the_new_queue() {
        let server = server();
        server
            .queue_add(Parameters(QueueAddParams {
                paths: vec!["/music/a.flac".into(), "/music/b.flac".into(), "/music/c.flac".into()],
            }))
            .expect("queue_add");
        let result = server
            .queue_move(Parameters(QueueMoveParams { from: 2, to: 0 }))
            .expect("queue_move");
        let state: serde_json::Value = serde_json::from_str(&result_text(&result)).unwrap();
        let queue = state["queue"].as_array().expect("queue");
        assert!(queue[0]["path"].as_str().unwrap().ends_with("c.flac"), "{queue:?}");
        assert!(queue[2]["path"].as_str().unwrap().ends_with("b.flac"), "{queue:?}");
    }

    #[test]
    fn queue_move_out_of_range_is_an_error() {
        let server = server();
        server
            .queue_add(Parameters(QueueAddParams {
                paths: vec!["/music/a.flac".into()],
            }))
            .expect("queue_add");
        assert!(server
            .queue_move(Parameters(QueueMoveParams { from: 0, to: 3 }))
            .is_err());
    }
```

- [x] **Step 2: Run** — compile error.

- [x] **Step 3: Implement**

After `QueueRemoveParams`:

```rust
#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct QueueMoveParams {
    /// 0-based index of the row to move.
    from: usize,
    /// 0-based index it should end up at (as the queue is before the move).
    to: usize,
}
```

(match the derive list `QueueRemoveParams` uses). After `queue_remove`:

```rust
    #[tool(description = "Move one queue row to another 0-based index; the playing track keeps playing")]
    fn queue_move(
        &self,
        Parameters(params): Parameters<QueueMoveParams>,
    ) -> Result<rmcp::model::CallToolResult, McpError> {
        self.apply(Command::QueueMove {
            from: params.from,
            to: params.to,
        })
    }
```

Add `queue_move` to the MCP skill that lists the queue tools (grep `queue_remove` in `znicz-mcp/skills`).

- [x] **Step 4:** `cargo test -p znicz-mcp`, clippy, fmt; commit `Add the queue_move MCP tool`.

---

### Task 5: 1 % volume steps and an eighth-cell volume bar (`znicz-tui`)

**Files:**
- Modify: `znicz-tui/src/app.rs`, `znicz-tui/src/format.rs`, `znicz-tui/src/views/now_playing.rs`, `znicz-tui/src/views/stage.rs`
- Test: `format.rs` tests, `znicz-tui/tests/keys.rs`

- [x] **Step 1: Write the failing tests**

`format.rs` tests (keep the existing 4-cell assertions):

```rust
    #[test]
    fn the_volume_bar_has_eighth_cell_resolution() {
        assert_eq!(volume_bar(0.0, 6), "▁▁▁▁▁▁");
        assert_eq!(volume_bar(1.0, 6), "██████");
        assert_eq!(volume_bar(0.5, 6), "███▁▁▁");
        assert_eq!(volume_bar(1.0 / 8.0, 1), "▏");
        assert_eq!(volume_bar(3.0 / 8.0, 1), "▍");
        assert_eq!(volume_bar(0.52, 6), "███▏▁▁", "25 of 48 eighths");
        for v in [0.0, 0.01, 0.33, 0.77, 0.99, 1.0] {
            assert_eq!(volume_bar(v, 10).chars().count(), 10, "{v}");
        }
    }
```

`tests/keys.rs`: replace the body of `volume_keys_move_in_steps_and_stop_at_the_ends` so one press is `0.99` (within 0.0001), and the loops are 120 presses each way (still ending exactly at `0.0` and `1.0`).

- [x] **Step 2: Run** — FAIL (`0.52` gives `███▁▁▁`; one press gives 0.95).

- [x] **Step 3: Implement**

`format.rs`:

```rust
/// A volume bar `cells` wide with eighth-cell resolution: full blocks, at
/// most one partial block, then low blocks for the rest.
pub fn volume_bar(volume: f32, cells: usize) -> String {
    const PARTIAL: [char; 7] = ['▏', '▎', '▍', '▌', '▋', '▊', '▉'];
    let eighths = (volume.clamp(0.0, 1.0) * (cells * 8) as f32).round() as usize;
    let full = eighths / 8;
    let rest = eighths % 8;
    let mut bar = String::with_capacity(cells * 3);
    for _ in 0..full {
        bar.push('█');
    }
    if full < cells {
        bar.push(if rest > 0 { PARTIAL[rest - 1] } else { '▁' });
        for _ in full + 1..cells {
            bar.push('▁');
        }
    }
    bar
}
```

`app.rs`: `const VOLUME_STEP: f32 = 0.01;` and in `adjust_volume`, snap to whole percent so repeated presses don't drift:

```rust
        let volume = (((state.volume + delta) * 100.0).round() / 100.0).clamp(0.0, 1.0);
```

`now_playing.rs`: the transport's `format::volume_bar(state.volume, 4)` → `6`. `stage.rs`: `volume_bar(state.volume, 4)` → `10`.

- [x] **Step 4:** `cargo test -p znicz-tui`, clippy, fmt; commit `Step volume by 1% and draw a finer bar`.

---

### Task 6: Gain ramps and the mute fade (`znicz-core`)

**Files:**
- Create: `znicz-core/src/audio/gain.rs`
- Modify: `znicz-core/src/audio/mod.rs` (or wherever `output` is declared), `znicz-core/src/audio/output.rs`, `znicz-core/src/player/engine.rs`

- [x] **Step 1: Write `gain.rs` with failing tests**

```rust
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
            assert!(g <= last + f32::EPSILON, "never rises on the way down at {i}");
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
        assert!(next > halfway && next < halfway + 0.02, "{halfway} -> {next}");
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
```

Declare it next to `output` (`pub mod gain;` in the audio module) and re-export nothing new from the crate root unless the engine needs it (`crate::audio::gain::{MUTE_FADE, VOLUME_RAMP}`).

- [x] **Step 2: Run** — `cargo test -p znicz-core --lib gain` (write tests first with `todo!()` bodies for `retarget`/`next_frame_gain` to see them fail), then PASS.

- [x] **Step 3: Wire it into the output**

`output.rs`:
- `AudioOutput.volume_bits: Arc<AtomicU32>` → `gain: Arc<GainControl>` (init `Arc::new(GainControl::new(1.0))`).
- Replace `set_volume` with:

```rust
    /// Move the output gain to `target` over `ramp`.
    pub fn set_gain(&self, target: f32, ramp: Duration) {
        self.gain.set(target, ramp);
    }

    /// A volume change: a short ramp so it does not click.
    pub fn set_volume(&self, volume: f32) {
        self.set_gain(volume, VOLUME_RAMP);
    }
```

- In `open_stream`, pass `self.gain.clone()`, `config.sample_rate.0`, and `config.channels` to `build_stream`; in `build_stream` create, per stream,

```rust
    let (target, _, generation) = gain.snapshot();
    let mut ramp = GainRamp::new(target, generation);
    let channels = usize::from(config.channels.max(1));
    let rate = config.sample_rate.0;
```

and give each closure `&mut ramp`, `&gain`, `channels`, `rate` (move `ramp` into the closure for each match arm — build one `ramp` inside each arm, or make the snapshot before the match and copy the `GainRamp` value, which is `Copy`).
- `fill_f32` / `fill_i16` / `fill_u16` take `gain: &GainControl, ramp: &mut GainRamp, channels: usize, sample_rate: u32` instead of `volume_bits`, call `ramp.follow(gain, sample_rate)` after `drain_if_flushing`, then loop per frame:

```rust
    for frame in data.chunks_mut(channels) {
        let g = ramp.next_frame_gain();
        for out in frame.iter_mut() {
            *out = next_sample(consumer, g, paused);   // f32
        }
    }
```

(i16 / u16 keep their existing conversion of the sample into the slot.)

`engine.rs`: `Command::SetMuted` calls `self.output.set_gain(effective, MUTE_FADE)` instead of `set_volume`. `SetVolume` keeps `set_volume` (now a 20 ms ramp).

- [x] **Step 4: Verify**

`cargo test -p znicz-core` (hardware playback tests skip under `CI` / without a device), `cargo test --workspace`, clippy, fmt. If a sound device is available locally, play a file in the TUI and press `m` twice to hear the fade.

- [x] **Step 5:** Commit `Fade mute and ramp volume changes in the audio callback`.

---

### Task 7: Version and docs

- [x] **Step 1:** Root `Cargo.toml` `0.9.0` → `0.10.0`; `cargo build -p znicz`; update the "currently" version in `wiki/Rust/Cargo-Workspace.md`.
- [x] **Step 2: README:** key table — `+` / `-` step 1 %, `m` mute fades in / out over half a second, `Alt-↑` / `Alt-↓` move a queue row (note: some terminals, e.g. macOS Terminal without "Use Option as Meta", do not send Alt-arrows); MCP tool list gains `queue_move` if the README lists tools.
- [x] **Step 3: Wiki:** `Architecture/TUI.md` (queue keys, volume bar 6 / 10 cells with eighths, 1 % steps); `Architecture/MCP.md` (`queue_move`); `Architecture/Core-Engine.md` (`QueueMove` and how the playing index follows) and `Architecture/Audio-Threading.md` (gain control and per-frame ramps, 500 ms mute fade, 20 ms volume ramp, exact landing for bit-perfect); `Plans/Roadmap.md` part 5 done in 0.10.0 — the rework is complete; `Issues.md` #36 and #47 to Closed (fixed in 0.10.0).
- [x] **Step 4:** `cargo test --workspace`, fmt; commit `Bump to 0.10.0 and document reordering, volume steps, and mute fades`.

---

### Task 8: Verify and open the PR

- [x] **Step 1:** fmt check, clippy `-D warnings`, `cargo test --workspace` — all clean.
- [x] **Step 2:** Tick every checkbox here; commit `Mark queue reorder and volume plan complete`.
- [x] **Step 3:** Push `feat/queue-reorder-and-volume`; open the PR "Queue reordering, finer volume, and mute fades (TUI rework part 5)" with `Closes #36` and `Closes #47`, ending with `🤖 Generated with [Claude Code](https://claude.com/claude-code)`.
