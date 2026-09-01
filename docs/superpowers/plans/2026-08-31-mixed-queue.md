# Mixed Queue Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let files and saved stations share one queue: Radio `a` appends without starting, next/previous leave a live stream, and saving an M3U refuses any station row.

**Architecture:** `QueueItem` and `QueueAdd` already accept streams. Extend `play_station` with `append: bool` (false = clear-and-play, true = `QueueAdd` only). Tighten `m3u_paths` so a single stream row fails the save. TUI `a` calls `play_station(..., true)`. CLI `--append` and MCP `append` (default false) use the same helper. Engine `pick_next` already steps off a stream; keep the TUI toast only when the queue is one station.

**Tech Stack:** Existing Rust workspace (`znicz-core`, `znicz-tui`, `znicz-mcp`, `znicz`). No new crates.

**Spec:** `docs/superpowers/specs/2026-08-31-mixed-queue-design.md`

## Global Constraints

- Workspace version **0.3.0 → 0.3.1** in the same development cycle (Task 5).
- No HLS, no ICY `StreamTitle`, no M3U `http://` playback, no writing `http://` lines.
- Radio **Enter** still clears and plays. Radio **`a`** always appends and never starts, including on an empty queue.
- Next/previous leave a live stream when another row exists. Toast only when the queue is **one** station.
- Save playlist errors if **any** row is a stream: `cannot save a queue that contains a radio station`.
- Tests that open a stream use **loopback only**, never a public station. Skip hardware when `CI` is set.
- Wiki, README, `znicz-tui/src/keys.rs`, and skills stay in sync in the same change as the behaviour.
- Overlay keys stay `n` / `e` / `c` / `d`. Do not resurrect old radio `w` or two-step add copy in the wiki.
- Parked TUI issues `#5`–`#9`, Phase 5, and Phase 6 stay untouched.

---

## File map

| File | Responsibility |
| --- | --- |
| Modify `znicz-core/src/playlist.rs` | `m3u_paths` refuses any stream row |
| Modify `znicz-core/src/station.rs` | `play_station(player, station, append: bool)` |
| Modify `znicz-core/tests/queue.rs` | Append does not play; next from a stream row moves |
| Modify `znicz-core/tests/stream.rs` | Pass `append: false` into `play_station` |
| Modify `znicz-tui/src/app.rs` | Radio `a` appends; skip-track toast unchanged |
| Modify `znicz-tui/src/keys.rs` | RADIO `a` is “add to the queue”, not “later” |
| Modify `znicz-tui/tests/keys.rs` | `a` appends; mixed next; save mixed refused |
| Modify `znicz/src/main.rs` | `station play --append` |
| Modify `znicz-mcp/src/server.rs` | `play_station` `append` default false |
| Modify `znicz-mcp/skills/radio-streaming/SKILL.md` | Append contract |
| Modify wiki + README + `Cargo.toml` version | 0.3.1, mixed queue done |

Shared helper after Task 2:

```rust
pub fn play_station(player: &PlayerHandle, station: &Station, append: bool) -> Result<()> {
    if !append {
        player.send_blocking(Command::QueueClear)?;
    }
    player.send_blocking(Command::QueueAdd(vec![QueueItem::stream(
        station.name.clone(),
        station.url.clone(),
    )]))?;
    if !append {
        player.send_blocking(Command::QueuePlayIndex(0))?;
    }
    Ok(())
}
```

---

### Task 1: Refuse saving a queue that contains a station

**Files:**
- Modify: `znicz-core/src/playlist.rs` (`m3u_paths`, tests at the bottom of the same file)
- Test: `znicz-core/src/playlist.rs` (`#[cfg(test)]`)

**Interfaces:**
- Consumes: `QueueItem`, `ZniczError::Player`
- Produces: `m3u_paths` returns `Err` if **any** item `is_stream()`; files-only queues still return their paths. Error text: `cannot save a queue that contains a radio station`

- [ ] **Step 1: Write the failing tests**

Add at the end of `znicz-core/src/playlist.rs` `#[cfg(test)]` (if `QueueItem` is not in scope, `use crate::player::state::QueueItem;` inside the test module):

```rust
#[test]
fn m3u_paths_writes_files_only_queues() {
    let paths = m3u_paths(&[QueueItem::file("/music/a.flac")]).unwrap();
    assert_eq!(paths, vec![std::path::PathBuf::from("/music/a.flac")]);
}

#[test]
fn m3u_paths_refuses_a_queue_that_contains_a_station() {
    let err = m3u_paths(&[
        QueueItem::file("/music/a.flac"),
        QueueItem::stream("Live", "http://127.0.0.1:1/s"),
    ])
    .unwrap_err();
    assert!(
        err.to_string().contains("radio station"),
        "{err}"
    );
}

#[test]
fn m3u_paths_refuses_a_station_only_queue() {
    let err = m3u_paths(&[QueueItem::stream("Live", "http://127.0.0.1:1/s")]).unwrap_err();
    assert!(err.to_string().contains("radio station"), "{err}");
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p znicz-core m3u_paths_refuses --offline`

Expected: FAIL — station-only queues currently error with `cannot save a radio queue as a playlist` (no `radio station` substring), and mixed queues currently **succeed**, dropping the stream.

- [ ] **Step 3: Implement `m3u_paths`**

Replace `m3u_paths` in `znicz-core/src/playlist.rs` with:

```rust
pub fn m3u_paths(queue: &[crate::player::state::QueueItem]) -> Result<Vec<PathBuf>> {
    if queue.iter().any(|item| item.is_stream()) {
        return Err(ZniczError::Player(
            "cannot save a queue that contains a radio station".into(),
        ));
    }
    let paths: Vec<PathBuf> = queue
        .iter()
        .filter_map(|item| item.as_path().map(Path::to_path_buf))
        .collect();
    if paths.is_empty() {
        return Err(ZniczError::Player("queue is empty".into()));
    }
    Ok(paths)
}
```

Keep the empty-queue error for a queue of zero items. Do not keep the old “radio queue” sentence.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p znicz-core m3u_paths --offline`

Expected: PASS (including existing playlist tests).

Also run: `cargo test -p znicz-tui playlist_save_of_a_stream_queue_is_refused --offline`

Expected: PASS if that test already matches `radio` or `playlist` in the toast. If it fails on the new sentence, update the assertion in Task 3 — do not weaken `m3u_paths` here.

- [ ] **Step 5: Commit**

```bash
git add znicz-core/src/playlist.rs
git commit -m "$(cat <<'EOF'
Refuse playlist save when the queue contains a station.

Dropping stream rows would silently lose radio; files-only save is unchanged.
EOF
)"
```

---

### Task 2: `play_station` append and next off a stream row

**Files:**
- Modify: `znicz-core/src/station.rs` (`play_station`)
- Modify: `znicz-core/tests/stream.rs` (call site)
- Modify: `znicz-tui/src/app.rs` (call site `play_station(..., false)` so the crate compiles)
- Modify: `znicz/src/main.rs` (call site `false`)
- Modify: `znicz-mcp/src/server.rs` (call site `false`)
- Test: `znicz-core/tests/queue.rs`

**Interfaces:**
- Consumes: `PlayerHandle`, `Station`, `Command::QueueClear` / `QueueAdd` / `QueuePlayIndex`
- Produces: `pub fn play_station(player: &PlayerHandle, station: &Station, append: bool) -> Result<()>`
  - `append == false`: clear, add one stream, play index 0 (today’s behaviour)
  - `append == true`: `QueueAdd` only; do not clear; do not play

- [ ] **Step 1: Write the failing tests in `znicz-core/tests/queue.rs`**

Add `QueueItem` to the existing `use znicz_core::{...}` line, then:

```rust
#[test]
fn appending_a_station_does_not_clear_or_start() {
    let (player, _thread) = spawn_player(AudioConfig::default());
    player
        .send_blocking(Command::QueueAdd(vec![znicz_core::QueueItem::file(
            "/music/a.flac",
        )]))
        .expect("seed");
    znicz_core::play_station(
        &player,
        &znicz_core::Station {
            name: "Live".into(),
            url: "http://127.0.0.1:1/s".into(),
        },
        true,
    )
    .expect("append");
    let state = player.state();
    assert_eq!(state.queue.len(), 2);
    assert_eq!(
        state.queue[0],
        znicz_core::QueueItem::file("/music/a.flac")
    );
    assert_eq!(
        state.queue[1],
        znicz_core::QueueItem::stream("Live", "http://127.0.0.1:1/s")
    );
    assert_eq!(state.status, PlaybackStatus::Stopped);
    assert_eq!(state.queue_position, 0);
}

#[test]
fn next_from_a_stream_row_moves_to_the_following_file() {
    let (player, _thread) = spawn_player(AudioConfig::default());
    player
        .send_blocking(Command::QueueAdd(vec![
            znicz_core::QueueItem::stream("Live", "http://127.0.0.1:1/s"),
            znicz_core::QueueItem::file("/music/a.flac"),
        ]))
        .expect("seed");
    let _ = player.send_blocking(Command::NextTrack);
    assert_eq!(player.state().queue_position, 1);
    assert_eq!(player.state().queue.len(), 2);
}
```

`play_station(..., true)` will not compile until Step 3.

- [ ] **Step 2: Run tests to verify they fail to compile (or fail)**

Run: `cargo test -p znicz-core --test queue appending_a_station --offline`

Expected: compile error `this function takes 2 arguments but 3 arguments were supplied`.

- [ ] **Step 3: Change `play_station` and every call site**

In `znicz-core/src/station.rs` replace `play_station` with:

```rust
pub fn play_station(player: &PlayerHandle, station: &Station, append: bool) -> Result<()> {
    if !append {
        player.send_blocking(Command::QueueClear)?;
    }
    player.send_blocking(Command::QueueAdd(vec![QueueItem::stream(
        station.name.clone(),
        station.url.clone(),
    )]))?;
    if !append {
        player.send_blocking(Command::QueuePlayIndex(0))?;
    }
    Ok(())
}
```

Update call sites to pass `false` (clear-and-play, unchanged product):

- `znicz-core/tests/stream.rs` — `play_station(&player, &Station { ... }, false)`
- `znicz-tui/src/app.rs` `play_selected_station` — `play_station(&self.player, &station, false)`
- `znicz/src/main.rs` `play_station_and_run` — `play_station(&player, &station, false)`
- `znicz-mcp/src/server.rs` `play_station` tool — `play_station(&self.player, &station, false)`

Do not add CLI `--append` or MCP `append` in this task.

- [ ] **Step 4: Run tests to verify they pass**

Run:

```bash
cargo test -p znicz-core --test queue appending_a_station --offline
cargo test -p znicz-core --test queue next_from_a_stream_row --offline
cargo test -p znicz-core --test stream --offline
cargo test -p znicz-tui --test keys enter_on_a_station --offline
```

Expected: all PASS. `NextTrack` may error on the missing file; `queue_position` must still be `1` because `play_queue_index` sets it before `play_item`.

- [ ] **Step 5: Commit**

```bash
git add znicz-core/src/station.rs znicz-core/tests/queue.rs znicz-core/tests/stream.rs \
  znicz-tui/src/app.rs znicz/src/main.rs znicz-mcp/src/server.rs
git commit -m "$(cat <<'EOF'
Let play_station append a stream without clearing the queue.

False keeps clear-and-play; true is QueueAdd only so files and stations can share a list.
EOF
)"
```

---

### Task 3: Radio `a` appends; mixed next in the TUI

**Files:**
- Modify: `znicz-tui/src/app.rs` (`on_radio_key` `a`, new helper)
- Modify: `znicz-tui/src/keys.rs` (RADIO `a` action text)
- Modify: `znicz-tui/tests/keys.rs`

**Interfaces:**
- Consumes: `play_station(&player, &station, true)` from Task 2
- Produces: Radio `a` enqueues the highlighted station, toast `added {name}`, does not start playback. Skip-track toast remains only when `queue.len() == 1 && queue[0].is_stream()`.

- [ ] **Step 1: Write the failing tests in `znicz-tui/tests/keys.rs`**

Replace `a_on_radio_toasts_that_queue_add_is_later` with:

```rust
#[test]
fn a_on_radio_appends_the_station_without_starting() {
    let (mut app, path) = station_fixture();
    znicz_core::save_stations(
        &path,
        &[znicz_core::Station {
            name: "Example".into(),
            url: "http://127.0.0.1:1/stream".into(),
        }],
    )
    .unwrap();
    let other = znicz_core::QueueItem::file("/music/a.flac");
    app.player
        .send_blocking(Command::QueueAdd(vec![other.clone()]))
        .unwrap();
    press_char(&mut app, 'R');
    press_char(&mut app, 'a');
    let queue = app.state().queue;
    assert_eq!(queue.len(), 2);
    assert_eq!(queue[0], other);
    assert_eq!(
        queue[1],
        QueueItem::stream("Example", "http://127.0.0.1:1/stream")
    );
    assert_eq!(app.state().status, znicz_core::PlaybackStatus::Stopped);
    let toast = app.toasts.current().expect("toast");
    assert!(toast.text.contains("Example"), "{}", toast.text);
}
```

Add:

```rust
#[test]
fn n_on_a_stream_with_another_row_moves_on() {
    let mut app = new_app();
    app.player
        .send_blocking(Command::QueueAdd(vec![
            QueueItem::stream("Live", "http://127.0.0.1:1/s"),
            QueueItem::file("/music/a.flac"),
        ]))
        .unwrap();
    press_char(&mut app, 'n');
    assert_eq!(app.state().queue_position, 1);
    assert!(
        app.toasts.current().is_none()
            || !app
                .toasts
                .current()
                .unwrap()
                .text
                .contains("no next track"),
        "{:?}",
        app.toasts.current()
    );
}

#[test]
fn playlist_save_of_a_mixed_queue_is_refused() {
    let mut app = new_app();
    app.player
        .send_blocking(Command::QueueAdd(vec![
            QueueItem::file("/music/a.flac"),
            QueueItem::stream("Live", "http://127.0.0.1:1/s"),
        ]))
        .unwrap();
    press_char(&mut app, 'P');
    press_char(&mut app, 'n');
    assert!(app.playlist_prompt.is_none());
    let toast = app.toasts.current().unwrap();
    assert!(
        toast.text.contains("radio station"),
        "{}",
        toast.text
    );
}
```

Keep `n_on_a_single_stream_toasts_instead_of_sending_next` as it is.

Update `playlist_save_of_a_stream_queue_is_refused` so the toast must contain `radio station` (not only `radio` / `playlist`).

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p znicz-tui --test keys a_on_radio_appends --offline`

Expected: FAIL — queue still length 1 or toast still says “later”.

- [ ] **Step 3: Implement TUI + keymap**

In `znicz-tui/src/app.rs` `on_radio_key`, change `a` to:

```rust
KeyCode::Char('a') => self.queue_selected_station(),
```

Add next to `play_selected_station`:

```rust
fn queue_selected_station(&mut self) {
    let Some(station) = self.selected_station().cloned() else {
        self.toasts.info("no stations");
        return;
    };
    match znicz_core::play_station(&self.player, &station, true) {
        Ok(()) => self.toasts.success(format!("added {}", station.name)),
        Err(e) => self.toasts.error(e.to_string()),
    }
}
```

Leave `skip_track` as:

```rust
if queue.len() == 1 && queue[0].is_stream() {
    // existing toasts
    return;
}
```

In `znicz-tui/src/keys.rs` RADIO table:

```rust
b("a", "add to the queue"),
```

Footer `"Radio"` already contains `a add`. The unit test `hints("Radio").contains("add")` must still pass. Change any test that requires `(later)`.

- [ ] **Step 4: Run tests to verify they pass**

Run:

```bash
cargo test -p znicz-tui --test keys a_on_radio_appends --offline
cargo test -p znicz-tui --test keys n_on_a_stream_with_another_row --offline
cargo test -p znicz-tui --test keys n_on_a_single_stream --offline
cargo test -p znicz-tui --test keys playlist_save_of_a_mixed_queue --offline
cargo test -p znicz-tui --test keys playlist_save_of_a_stream_queue --offline
cargo test -p znicz-tui --lib keys::tests --offline
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add znicz-tui/src/app.rs znicz-tui/src/keys.rs znicz-tui/tests/keys.rs
git commit -m "$(cat <<'EOF'
Let radio a append a station to the queue.

Enter still replaces; a matches playlists so a live file is not stopped.
EOF
)"
```

---

### Task 4: CLI `--append` and MCP `append`

**Files:**
- Modify: `znicz/src/main.rs`
- Modify: `znicz-mcp/src/server.rs`
- Test: `znicz-mcp/src/server.rs` tests module

**Interfaces:**
- Consumes: `play_station(..., append: bool)` from Task 2
- Produces:
  - CLI: `StationCmd::Play { name: String, append: bool }` with `#[arg(long)] append`
  - MCP: `StationPlayParams { name: String, #[serde(default)] append: bool }`; `play_station` tool uses it. `StationNameParams` stays name-only for remove.

- [ ] **Step 1: Write the failing MCP tests**

Do **not** reuse `StationNameParams` for play once `append` exists. Add:

```rust
#[derive(Debug, Deserialize, JsonSchema)]
struct StationPlayParams {
    name: String,
    #[serde(default)]
    append: bool,
}
```

Replace the `play_station_clears_the_queue_then_errors_on_a_dead_url` params type with `StationPlayParams { name: "Dead".into(), append: false }` (or default).

Add:

```rust
#[test]
fn play_station_append_keeps_the_existing_queue() {
    let (server, path) = station_server();
    server
        .queue_add(Parameters(QueueAddParams {
            paths: vec!["/music/a.flac".into()],
        }))
        .unwrap();
    server
        .add_radio_station(Parameters(StationAddParams {
            name: "Example".into(),
            url: "http://127.0.0.1:1/stream".into(),
        }))
        .unwrap();
    server
        .play_station(Parameters(StationPlayParams {
            name: "Example".into(),
            append: true,
        }))
        .expect("append");
    let queue = server.player.state().queue;
    assert_eq!(queue.len(), 2);
    assert_eq!(queue[0], znicz_core::QueueItem::file("/music/a.flac"));
    assert!(queue[1].is_stream());
    assert_eq!(server.player.state().status, PlaybackStatus::Stopped);
    let _ = std::fs::remove_file(path);
}

#[test]
fn save_playlist_errors_when_the_queue_has_a_station() {
    let (server, dir) = playlist_server();
    server
        .queue_add(Parameters(QueueAddParams {
            paths: vec!["/music/a.flac".into()],
        }))
        .unwrap();
    server
        .add_radio_station(Parameters(StationAddParams {
            name: "Live".into(),
            url: "http://127.0.0.1:1/s".into(),
        }))
        .unwrap();
    server
        .play_station(Parameters(StationPlayParams {
            name: "Live".into(),
            append: true,
        }))
        .expect("append station");
    let err = server
        .save_playlist(Parameters(SavePlaylistParams {
            name: "evening".into(),
        }))
        .unwrap_err();
    assert!(err.to_string().contains("radio station"), "{err}");
    std::fs::remove_dir_all(&dir).ok();
}
```

`playlist_server` and `station_server` each set different env vars. `save_playlist_errors_when_the_queue_has_a_station` uses `playlist_server` then `add_radio_station`, which writes `server.stations_path` (temp from `station_server` only if that constructor ran). `playlist_server` still has a `stations_path` (default or env). Use **one** server: `playlist_server` already constructs `ZniczMcpServer` with a stations path. `add_radio_station` on that server is enough — do not mix `station_server` into that test.

If `playlist_server`’s stations file is a shared default, prefer `station_server()` and call `save_playlist` on it (it has a playlists dir too). `station_server` sets playlists to temp via default_playlists_dir. Simplest: use `station_server` for both new tests; `save_playlist` writes under that server’s `playlists_dir`.

Rewrite the save test to use `station_server()` only:

```rust
#[test]
fn save_playlist_errors_when_the_queue_has_a_station() {
    let (server, path) = station_server();
    server
        .queue_add(Parameters(QueueAddParams {
            paths: vec!["/music/a.flac".into()],
        }))
        .unwrap();
    server
        .add_radio_station(Parameters(StationAddParams {
            name: "Live".into(),
            url: "http://127.0.0.1:1/s".into(),
        }))
        .unwrap();
    server
        .play_station(Parameters(StationPlayParams {
            name: "Live".into(),
            append: true,
        }))
        .expect("append station");
    let err = server
        .save_playlist(Parameters(SavePlaylistParams {
            name: "evening".into(),
        }))
        .unwrap_err();
    assert!(err.to_string().contains("radio station"), "{err}");
    let _ = std::fs::remove_file(path);
}
```

- [ ] **Step 2: Run tests to verify they fail to compile**

Run: `cargo test -p znicz-mcp play_station_append_keeps --offline`

Expected: `StationPlayParams` not found / `play_station` still takes `StationNameParams`.

- [ ] **Step 3: Implement CLI and MCP**

`znicz/src/main.rs` `StationCmd::Play`:

```rust
    /// Play a saved station and open the player
    Play {
        name: String,
        /// Add to the queue instead of replacing it and starting playback
        #[arg(long)]
        append: bool,
    },
```

Match arm:

```rust
            StationCmd::Play { name, append } => {
                play_station_and_run(&name, append, audio_config, library_path)?;
            }
```

`play_station_and_run`:

```rust
fn play_station_and_run(
    name: &str,
    append: bool,
    audio_config: AudioConfig,
    library_path: Option<PathBuf>,
) -> color_eyre::Result<()> {
    let path = stations_path()?;
    let stations = znicz_core::load_stations(&path)?;
    let station = znicz_core::find_station(&stations, name)
        .ok_or_else(|| color_eyre::eyre::eyre!("no station named {name}"))?
        .clone();
    let (player, _thread) = spawn_player(audio_config);
    znicz_core::play_station(&player, &station, append)?;
    run_tui_with_player(player, library_path, None)
}
```

MCP: add `StationPlayParams`. Change the tool:

```rust
    #[tool(description = "Play a saved radio station by name. append true adds without starting; default clears the queue and plays")]
    fn play_station(
        &self,
        Parameters(params): Parameters<StationPlayParams>,
    ) -> Result<rmcp::model::CallToolResult, McpError> {
        let stations = znicz_core::load_stations(&self.stations_path).map_err(Self::map_io)?;
        let station = znicz_core::find_station(&stations, &params.name)
            .cloned()
            .ok_or_else(|| {
                McpError::invalid_params(format!("no station named {:?}", params.name), None)
            })?;
        map_player_err(znicz_core::play_station(
            &self.player,
            &station,
            params.append,
        ))?;
        self.ok_state()
    }
```

Update every test `play_station(Parameters(StationNameParams {` used for **play** to `StationPlayParams`. Leave `remove_radio_station` on `StationNameParams`.

- [ ] **Step 4: Run tests to verify they pass**

Run:

```bash
cargo test -p znicz-mcp play_station --offline
cargo test -p znicz-mcp save_playlist_errors_when_the_queue_has_a_station --offline
cargo test -p znicz --offline
```

Expected: PASS. `znicz` has no unit tests; it must compile.

- [ ] **Step 5: Commit**

```bash
git add znicz/src/main.rs znicz-mcp/src/server.rs
git commit -m "$(cat <<'EOF'
Add station play --append for CLI and MCP.

Same contract as playlists: default clears and plays; append only enqueues.
EOF
)"
```

---

### Task 5: Version 0.3.1 and docs

**Files:**
- Modify: `Cargo.toml` (`[workspace.package] version`)
- Modify: `wiki/Rust/Cargo-Workspace.md` (the **0.3.0** mention)
- Modify: `README.md` (station play `--append`)
- Modify: `wiki/Architecture/TUI.md` (Radio `a`)
- Modify: `wiki/Domain/Formats-and-Metadata.md`
- Modify: `wiki/Architecture/MCP.md`
- Modify: `wiki/Plans/Roadmap.md`
- Modify: `wiki/Issues.md` (later radio list without mixed queue)
- Modify: `znicz-mcp/skills/radio-streaming/SKILL.md`

**Interfaces:**
- Consumes: behaviour from Tasks 1–4
- Produces: docs that match the running player; version **0.3.1**

- [ ] **Step 1: Bump version**

In root `Cargo.toml`:

```toml
version = "0.3.1"
```

In `wiki/Rust/Cargo-Workspace.md` replace `currently **0.3.0**` with `currently **0.3.1**`.

- [ ] **Step 2: Wiki, README, skill — exact copy**

`README.md` under station examples, add:

```bash
znicz station play Example --append
```

`wiki/Architecture/TUI.md` Radio table row for `a`:

```markdown
| `a` | Add the highlighted station to the queue (does not start or stop playback) |
```

Delete “later / toast for now / mixed queue is not in this version” from that row.

`wiki/Domain/Formats-and-Metadata.md`:

- CLI blurb: mention `play Example --append`.
- Replace “`a` will add … later; for now it only toasts” with: Radio `a` appends the station. Enter / `play_station` still replace the queue.
- Later-radio sentence: drop “and a mixed queue of files and stations”. Keep ICY, HLS, M3U URL lines.

`wiki/Architecture/MCP.md` radio tools sentence: `play_station` clears by default; `append: true` adds without starting.

`wiki/Plans/Roadmap.md` Phase 4 bullets: change “`a` add-to-queue is later” to “`a` appends a station; Enter still replaces”.

Later radio list: **delete** the Mixed queue bullet (it is done). Keep ICY, HLS, M3U stream lines.

`wiki/Issues.md`: “later radio (ICY, HLS, M3U stream lines)” — remove mixed queue.

`znicz-mcp/skills/radio-streaming/SKILL.md`:

```markdown
3. `play_station` with the exact name — default **clears the queue** and starts the stream. `append: true` adds the station and does not start or stop playback.
```

Remove “Mixing a station into an existing file queue is later; do not invent that.”

Do not mention old keys `w` or two-step add.

- [ ] **Step 3: Run the full gate**

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --offline -- -D warnings
cargo test -p znicz-core -p znicz-tui -p znicz-mcp --offline
```

Expected: fmt clean, clippy 0 warnings, all tests PASS.

- [ ] **Step 4: Commit**

```bash
git add Cargo.toml README.md wiki znicz-mcp/skills/radio-streaming/SKILL.md
git commit -m "$(cat <<'EOF'
Document mixed queues and ship 0.3.1.

Radio a is a real append; the wiki must not still say that is later.
EOF
)"
```

---

## Spec coverage

| Spec requirement | Task |
| --- | --- |
| Save refuses any station row | 1 |
| `play_station` append / clear-and-play | 2 |
| Next leaves a stream when another row exists | 2 (engine), 3 (TUI) |
| Single-station next/previous toast | 3 (existing test kept) |
| Radio `a` append never starts | 3 |
| Enter still clear-and-play | 2 call sites `false`, 3 unchanged Enter |
| CLI `--append` | 4 |
| MCP `append` default false | 4 |
| Version 0.3.1 + wiki/README/skills | 5 |
| No ICY / HLS / M3U URL play | Global constraints; no tasks add them |
