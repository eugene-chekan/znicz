# Remove Playing Queue Row Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Deleting the playing queue row stops that file and either starts the row that slid into its index, or stops if nothing remains there; the playing marker and pause/resume always match the decoder.

**Architecture:** Keep `Command::QueueRemove(usize)`. Fix `remove_from_queue` so a playing-row delete **stops first**, then `play_queue_index` on the same index if it is still in range, else clamp `queue_position`. Make `play_queue_index` keep that index after `play_item` (do not snap to the first matching path). TUI `d` clamps the cursor using the queue length **after** the command.

**Tech Stack:** Existing Rust workspace (`znicz-core`, `znicz-tui`). No new crates. No MCP tool (that is [#15](https://github.com/eugene-chekan/znicz/issues/15)).

**Spec:** `docs/superpowers/specs/2026-08-31-remove-playing-queue-row-design.md`

## Global Constraints

- Workspace version **0.3.1 → 0.3.2** in the same development cycle (Task 3).
- No MCP `queue_remove`. Parked `#5`–`#9`, Phase 5, Phase 6, Next/shuffle/repeat, and `QueueClear` stay untouched.
- Delete the playing row: play **by index** the row that slid in, or **stop**. Shuffle and repeat do not pick a different row.
- Failed open of the replacement: error, stay stopped, do not skip, do not revive the deleted file.
- Same rule for a file and for a station.
- Tests that open a stream use **loopback only**. Tests that open the sound card skip when `CI` is set (same helper as `znicz-core/tests/playback.rs`).
- Wiki, README, and `znicz-tui/src/keys.rs` stay in sync in the same change as the behaviour. Queue `d` copy in keys stays “remove from the queue”; the wiki queue paragraph gets the playing-row sentence.

---

## File map

| File | Responsibility |
| --- | --- |
| Modify `znicz-core/src/player/engine.rs` | `remove_from_queue` stop-then-play-or-clamp; `play_queue_index` keeps its index |
| Modify `znicz-core/tests/queue.rs` | Playing-row remove tests (WAV + skip hardware on CI) |
| Modify `znicz-tui/src/app.rs` | Clamp queue cursor from post-remove length |
| Modify `znicz-tui/tests/keys.rs` | Cursor after `d` on last / middle row |
| Modify wiki + `Cargo.toml` version | 0.3.2, TUI queue sentence |

---

### Task 1: Engine stop-then-play-or-clamp

**Files:**
- Modify: `znicz-core/src/player/engine.rs` (`remove_from_queue`, `play_queue_index`)
- Test: `znicz-core/tests/queue.rs`

**Interfaces:**
- Consumes: `Command::QueueRemove(usize)`, `PlaybackStatus`, `QueueItem`
- Produces: After remove of the playing row, decoder and `queue_position` follow the spec table. `play_queue_index(index)` ends with `queue_position == index` when play succeeds.

- [ ] **Step 1: Write the failing tests**

At the top of `znicz-core/tests/queue.rs`, add `use std::path::Path;` and `AudioOutput`, `PlaybackStatus` (PlaybackStatus is already imported). Copy these helpers from `znicz-core/tests/playback.rs` (keep them private to this file):

```rust
fn skip_hardware_playback() -> bool {
    if std::env::var_os("CI").is_some() {
        eprintln!("CI: skipping hardware playback");
        return true;
    }
    let no_device = AudioOutput::list_devices()
        .map(|devices| devices.is_empty())
        .unwrap_or(true);
    if no_device {
        eprintln!("no audio output device, skipping");
    }
    no_device
}

fn write_silent_wav(path: &Path, sample_rate: u32, channels: u16, seconds: u32) {
    use std::io::Write;

    let frames = sample_rate * seconds;
    let bytes_per_frame = channels as u32 * 2;
    let data_size = frames * bytes_per_frame;
    let file_size = 36 + data_size;

    let mut file = std::fs::File::create(path).expect("create wav");
    file.write_all(b"RIFF").unwrap();
    file.write_all(&file_size.to_le_bytes()).unwrap();
    file.write_all(b"WAVE").unwrap();
    file.write_all(b"fmt ").unwrap();
    file.write_all(&16u32.to_le_bytes()).unwrap();
    file.write_all(&1u16.to_le_bytes()).unwrap();
    file.write_all(&channels.to_le_bytes()).unwrap();
    file.write_all(&sample_rate.to_le_bytes()).unwrap();
    file.write_all(&(sample_rate * bytes_per_frame).to_le_bytes())
        .unwrap();
    file.write_all(&(bytes_per_frame as u16).to_le_bytes())
        .unwrap();
    file.write_all(&16u16.to_le_bytes()).unwrap();
    file.write_all(b"data").unwrap();
    file.write_all(&data_size.to_le_bytes()).unwrap();

    let silence = vec![0u8; (bytes_per_frame * sample_rate) as usize];
    for _ in 0..seconds {
        file.write_all(&silence).unwrap();
    }
}
```

Add tests (skip the ones that must open the device when `skip_hardware_playback()` is true):

```rust
#[test]
fn removing_the_playing_row_starts_the_row_that_slid_in() {
    if skip_hardware_playback() {
        return;
    }
    let dir = std::env::temp_dir().join("znicz-remove-playing-next");
    std::fs::create_dir_all(&dir).unwrap();
    let a = dir.join("a.wav");
    let b = dir.join("b.wav");
    write_silent_wav(&a, 44_100, 2, 2);
    write_silent_wav(&b, 44_100, 2, 2);

    let (player, _thread) = spawn_player(AudioConfig::default());
    player
        .send_blocking(Command::QueueAdd(vec![
            QueueItem::file(&a),
            QueueItem::file(&b),
        ]))
        .unwrap();
    player
        .send_blocking(Command::QueuePlayIndex(0))
        .unwrap();
    assert_eq!(player.state().status, PlaybackStatus::Playing);

    player
        .send_blocking(Command::QueueRemove(0))
        .unwrap();

    let state = player.state();
    assert_eq!(state.queue.len(), 1);
    assert_eq!(state.queue_position, 0);
    assert_eq!(state.status, PlaybackStatus::Playing);
    assert_eq!(
        state.current_track.as_ref().and_then(|t| t.path.as_deref()),
        Some(b.as_path()),
        "pause/resume must control the replacement, not the deleted file"
    );

    player.send_blocking(Command::Stop).ok();
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn removing_the_last_playing_row_stops() {
    if skip_hardware_playback() {
        return;
    }
    let dir = std::env::temp_dir().join("znicz-remove-playing-last");
    std::fs::create_dir_all(&dir).unwrap();
    let a = dir.join("a.wav");
    let b = dir.join("b.wav");
    write_silent_wav(&a, 44_100, 2, 2);
    write_silent_wav(&b, 44_100, 2, 2);

    let (player, _thread) = spawn_player(AudioConfig::default());
    player
        .send_blocking(Command::QueueAdd(vec![
            QueueItem::file(&a),
            QueueItem::file(&b),
        ]))
        .unwrap();
    player
        .send_blocking(Command::QueuePlayIndex(1))
        .unwrap();

    player
        .send_blocking(Command::QueueRemove(1))
        .unwrap();

    let state = player.state();
    assert_eq!(state.queue.len(), 1);
    assert_eq!(state.queue[0], QueueItem::file(&a));
    assert_eq!(state.queue_position, 0, "playing index must stay in range");
    assert_eq!(state.status, PlaybackStatus::Stopped);
    assert!(
        state.current_track.is_none(),
        "the deleted file must not keep making sound"
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn removing_the_only_playing_row_leaves_an_empty_stopped_queue() {
    if skip_hardware_playback() {
        return;
    }
    let wav = std::env::temp_dir().join("znicz-remove-playing-only.wav");
    write_silent_wav(&wav, 44_100, 2, 2);

    let (player, _thread) = spawn_player(AudioConfig::default());
    player
        .send_blocking(Command::QueueAdd(vec![QueueItem::file(&wav)]))
        .unwrap();
    player
        .send_blocking(Command::QueuePlayIndex(0))
        .unwrap();
    player
        .send_blocking(Command::QueueRemove(0))
        .unwrap();

    let state = player.state();
    assert!(state.queue.is_empty());
    assert_eq!(state.queue_position, 0);
    assert_eq!(state.status, PlaybackStatus::Stopped);
    assert!(state.current_track.is_none());

    std::fs::remove_file(&wav).ok();
}

#[test]
fn removing_the_playing_row_while_paused_starts_the_replacement() {
    if skip_hardware_playback() {
        return;
    }
    let dir = std::env::temp_dir().join("znicz-remove-playing-paused");
    std::fs::create_dir_all(&dir).unwrap();
    let a = dir.join("a.wav");
    let b = dir.join("b.wav");
    write_silent_wav(&a, 44_100, 2, 2);
    write_silent_wav(&b, 44_100, 2, 2);

    let (player, _thread) = spawn_player(AudioConfig::default());
    player
        .send_blocking(Command::QueueAdd(vec![
            QueueItem::file(&a),
            QueueItem::file(&b),
        ]))
        .unwrap();
    player
        .send_blocking(Command::QueuePlayIndex(0))
        .unwrap();
    player.send_blocking(Command::Pause).unwrap();
    assert_eq!(player.state().status, PlaybackStatus::Paused);

    player
        .send_blocking(Command::QueueRemove(0))
        .unwrap();

    let state = player.state();
    assert_eq!(state.status, PlaybackStatus::Playing);
    assert_eq!(
        state.current_track.as_ref().and_then(|t| t.path.as_deref()),
        Some(b.as_path())
    );

    player.send_blocking(Command::Stop).ok();
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_dead_replacement_errors_and_stays_stopped() {
    if skip_hardware_playback() {
        return;
    }
    let a = std::env::temp_dir().join("znicz-remove-dead-a.wav");
    write_silent_wav(&a, 44_100, 2, 2);

    let (player, _thread) = spawn_player(AudioConfig::default());
    player
        .send_blocking(Command::QueueAdd(vec![
            QueueItem::file(&a),
            QueueItem::file("/music/missing-replacement.flac"),
        ]))
        .unwrap();
    player
        .send_blocking(Command::QueuePlayIndex(0))
        .unwrap();

    let err = player.send_blocking(Command::QueueRemove(0));
    assert!(err.is_err(), "a missing replacement must be reported");

    let state = player.state();
    assert_eq!(state.queue.len(), 1);
    assert_eq!(state.status, PlaybackStatus::Stopped);
    assert!(
        state.current_track.is_none(),
        "the deleted file must not keep playing"
    );

    std::fs::remove_file(&a).ok();
}
```

Keep the existing `removing_an_entry_closes_the_gap` tests (stopped queue, no decoder).

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p znicz-core --test queue removing_the_playing --offline`

Expected: FAIL (or skip entirely when `CI` is set). On a machine with a device, `removing_the_last_playing_row_stops` should fail while `queue_position` is still `1` with `len == 1`, and/or `current_track` still names the deleted file.

If the whole filter skips because `CI` is set, run the same command locally with `CI` unset. Do not weaken the tests to avoid a device.

- [ ] **Step 3: Implement `remove_from_queue` and keep `play_queue_index` on its index**

Replace `remove_from_queue` in `znicz-core/src/player/engine.rs` with:

```rust
fn remove_from_queue(&mut self, index: usize) -> Result<()> {
    let playing_removed = {
        let mut state = self.state.write().unwrap();
        if index >= state.queue.len() {
            return Ok(());
        }
        let playing = state.status != PlaybackStatus::Stopped;
        let was_playing_row = index == state.queue_position && playing;
        state.queue.remove(index);
        if index < state.queue_position {
            state.queue_position -= 1;
        }
        was_playing_row
    };

    self.event_tx.send(PlayerEvent::QueueChanged).ok();

    if playing_removed {
        self.stop()?;
        let (pos, len) = {
            let state = self.state.read().unwrap();
            (state.queue_position, state.queue.len())
        };
        if pos < len {
            self.play_queue_index(pos)?;
        } else {
            self.state.write().unwrap().queue_position = len.saturating_sub(1);
        }
    }

    self.emit_state_changed();
    Ok(())
}
```

Replace `play_queue_index` with:

```rust
fn play_queue_index(&mut self, index: usize) -> Result<()> {
    let item = {
        let state = self.state.read().unwrap();
        match state.queue.get(index) {
            Some(item) => item.clone(),
            None => return Ok(()),
        }
    };
    self.play_item(item)?;
    let mut state = self.state.write().unwrap();
    if index < state.queue.len() {
        state.queue_position = index;
    }
    Ok(())
}
```

Do not call `pick_next`. Do not add an MCP tool.

- [ ] **Step 4: Run tests to verify they pass**

Run:

```bash
cargo test -p znicz-core --test queue --offline
```

Expected: PASS. Hardware tests skip on CI; they must PASS when a device exists.

- [ ] **Step 5: Commit**

```bash
git add znicz-core/src/player/engine.rs znicz-core/tests/queue.rs
git commit -m "$(cat <<'EOF'
Stop the decoder when the playing queue row is removed.

The list was closing the gap while the old file kept making sound.
EOF
)"
```

---

### Task 2: TUI cursor after `d`

**Files:**
- Modify: `znicz-tui/src/app.rs` (`on_queue_key` `d` / Delete)
- Test: `znicz-tui/tests/keys.rs`

**Interfaces:**
- Consumes: `Command::QueueRemove` from Task 1; `Cursor::clamp(len)` takes the **list length**
- Produces: After `d`, cursor is clamped with `self.player.state().queue.len()` (length after the command)

- [ ] **Step 1: Write the failing tests**

Add next to `d_removes_the_selected_queue_entry` in `znicz-tui/tests/keys.rs`:

```rust
#[test]
fn d_on_the_last_queue_row_leaves_the_cursor_on_the_new_last() {
    let mut app = new_app();
    queue(&mut app, 3);
    open_queue(&mut app);
    press_char(&mut app, 'G');
    assert_eq!(app.queue_cursor.index(), 2);

    press_char(&mut app, 'd');

    assert_eq!(app.state().queue.len(), 2);
    assert_eq!(
        app.queue_cursor.selected(app.state().queue.len()),
        Some(1),
        "cursor should sit on the last remaining row"
    );
}

#[test]
fn d_on_a_middle_queue_row_keeps_the_cursor_on_that_index() {
    let mut app = new_app();
    queue(&mut app, 3);
    open_queue(&mut app);
    press_char(&mut app, 'j');
    assert_eq!(app.queue_cursor.index(), 1);

    press_char(&mut app, 'd');

    assert_eq!(app.state().queue.len(), 2);
    assert_eq!(
        app.queue_cursor.selected(app.state().queue.len()),
        Some(1),
        "the row that slid in should stay under the cursor"
    );
}
```

These tests do not start playback. They pin cursor vs list length only.

- [ ] **Step 2: Run tests to verify they fail (or already pass)**

Run: `cargo test -p znicz-tui --test keys d_on_the_last_queue --offline`

Expected: PASS or FAIL depending on the current `clamp(old_len - 1)` accident. Either way, Step 3 still changes the clamp to the post-remove snapshot so the next person can read it.

- [ ] **Step 3: Clamp from the length after `apply`**

In `znicz-tui/src/app.rs` `on_queue_key`, replace the `d` / Delete arm with:

```rust
KeyCode::Char('d') | KeyCode::Delete => {
    if let Some(index) = self.queue_cursor.selected(state.queue.len()) {
        self.apply(
            Command::QueueRemove(index),
            Some("removed from queue".into()),
        );
        let len = self.player.state().queue.len();
        self.queue_cursor.clamp(len);
    }
}
```

Keep using `state` only to compute the index **before** the command. Do not reuse that snapshot for `clamp`.

- [ ] **Step 4: Run tests to verify they pass**

Run:

```bash
cargo test -p znicz-tui --test keys d_on_the_last_queue --offline
cargo test -p znicz-tui --test keys d_on_a_middle_queue --offline
cargo test -p znicz-tui --test keys d_removes_the_selected --offline
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add znicz-tui/src/app.rs znicz-tui/tests/keys.rs
git commit -m "$(cat <<'EOF'
Clamp the queue cursor after a remove using the new length.

A snapshot from before d can leave the highlight on a row that is gone.
EOF
)"
```

---

### Task 3: Version 0.3.2 and wiki

**Files:**
- Modify: `Cargo.toml` (`[workspace.package] version`)
- Modify: `wiki/Rust/Cargo-Workspace.md` (the **0.3.1** mention)
- Modify: `wiki/Architecture/TUI.md` (queue `d` sentence)
- `wiki/Issues.md` already lists [#14](https://github.com/eugene-chekan/znicz/issues/14) and [#15](https://github.com/eugene-chekan/znicz/issues/15) — do not invent extra issue copy

**Interfaces:**
- Consumes: behaviour from Tasks 1–2
- Produces: docs that match the running player; version **0.3.2**

- [ ] **Step 1: Bump version**

In root `Cargo.toml`:

```toml
version = "0.3.2"
```

In `wiki/Rust/Cargo-Workspace.md` replace `currently **0.3.1**` with `currently **0.3.2**`. Include `Cargo.lock` if the bump rewrites crate versions.

- [ ] **Step 2: Wiki queue sentence**

In `wiki/Architecture/TUI.md`, replace:

```markdown
`Enter` plays a row, `d` removes one, `C` clears, `o` jumps back to whatever is
playing.
```

with:

```markdown
`Enter` plays a row, `d` removes one, `C` clears, `o` jumps back to whatever is
playing. `d` on the playing row starts the next remaining one, or stops if
that was the last row.
```

Do not mention old keys `w`. Do not add MCP `queue_remove`. Leave [#14](https://github.com/eugene-chekan/znicz/issues/14) open until this ships (the merge closes it). `znicz-tui/src/keys.rs` QUEUE `d` stays `remove from the queue`.

- [ ] **Step 3: Run the full gate**

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --offline -- -D warnings
cargo test -p znicz-core -p znicz-tui --offline
```

Expected: fmt clean, clippy 0 warnings, tests PASS (hardware queue tests skip on CI).

- [ ] **Step 4: Commit**

```bash
git add Cargo.toml Cargo.lock wiki/Rust/Cargo-Workspace.md wiki/Architecture/TUI.md
git commit -m "$(cat <<'EOF'
Document queue d on the playing row and ship 0.3.2.

The wiki still read as if d only edited the list.
EOF
)"
```

---

## Spec coverage

| Spec requirement | Task |
| --- | --- |
| Playing marker / now-playing / pause-resume match the decoder | 1 |
| Delete playing row, replacement exists → play by index | 1 |
| Delete last / only playing row → stop, index in range | 1 |
| Delete before / after playing → keep going | 1 (existing gap tests + unchanged decoder path) |
| Paused on playing row, replacement starts | 1 |
| Dead replacement: error, stopped, no ghost | 1 |
| Same rule for files and stations | 1 (same `remove_from_queue`; no extra MCP) |
| TUI cursor uses length after remove | 2 |
| Version 0.3.2 + TUI wiki sentence | 3 |
| No MCP `queue_remove` | Global; #15 stays later |
