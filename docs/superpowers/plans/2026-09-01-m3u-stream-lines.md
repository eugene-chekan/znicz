# M3U Stream Lines Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Load `http://` / `https://` M3U lines as stream queue rows, save mixed queues with paths and `#EXTINF` names, and bind previous track to `p` only.

**Architecture:** `playlist::parse` returns `LoadResult { items: Vec<QueueItem>, skipped }`. http(s) lines become `QueueItem::Stream`; `#EXTINF:` sets the next stream’s name. `write_text` / `write_path` take `&[QueueItem]` and replace `m3u_paths`. TUI `N` is unbound.

**Tech Stack:** Existing Rust workspace (`znicz-core`, `znicz-tui`, `znicz-mcp`, `znicz`). No new crates.

**Spec:** `docs/superpowers/specs/2026-09-01-m3u-stream-lines-design.md`

## Global Constraints

- Workspace version **0.3.2 → 0.3.3** in the same development cycle (Task 4).
- No ICY `StreamTitle`. No HLS playback. A `.m3u8` **file** on disk stays an M3U list. An http(s) line ending in `.m3u8` is still a stream row; play may fail later — do not skip it, do not auto-skip a failed open.
- `queue_add` stays paths-only. No new `Command` variant.
- Overlay keys stay `n` / `e` / `c` / `d`. Do not resurrect radio `w`.
- Previous is **`p` only**. **`N` is unbound.** `n` stays next. `P` stays playlists.
- Save never errors because a row is a stream. Empty queue still errors `queue is empty`. Empty parse still errors `playlist had no playable files`.
- Tests that open a stream use **loopback only**. Skip hardware when `CI` is set.
- Wiki, README, `znicz-tui/src/keys.rs`, and `znicz-mcp/skills/playlist-curation/SKILL.md` stay in sync in the same change as the behaviour.
- Parked TUI issues `#5`–`#9`, Phase 5, Phase 6, and MCP `#15` stay untouched.

---

## File map

| File | Responsibility |
| --- | --- |
| Modify `znicz-core/src/playlist.rs` | `LoadResult.items`, parse http(s)+`#EXTINF`, `write_text`/`write_path` on `QueueItem`, drop `m3u_paths` |
| Modify `znicz-core/src/lib.rs` | Stop exporting `m3u_paths` |
| Modify `znicz-tui/src/app.rs` | `items.len()` toasts; save writes `&queue`; drop `m3u_paths`; `N` not previous |
| Modify `znicz-tui/src/keys.rs` | `GLOBAL` previous is `p` |
| Modify `znicz-tui/tests/keys.rs` | Save mixed/stream opens prompt; `N` does not previous |
| Modify `znicz-mcp/src/server.rs` | `loaded` from `items`; `write_path(..., &queue)`; import/save tests |
| Modify `znicz-mcp/skills/playlist-curation/SKILL.md` | URL lines play; save may write `#EXTINF` |
| Modify wiki + README + `Cargo.toml` version | 0.3.3, M3U stream lines done |

Shared helpers after Task 1 (keep them private in `playlist.rs`):

```rust
fn is_http_url(line: &str) -> bool {
    let lower = line.to_ascii_lowercase();
    lower.starts_with("http://") || lower.starts_with("https://")
}

/// `Some(title)` when this line is `#EXTINF:`. Empty / missing title → `Some(None)`.
fn extinf_title(line: &str) -> Option<Option<String>> {
    let rest = line.strip_prefix("#EXTINF:")?;
    Some(match rest.split_once(',') {
        Some((_, title)) => {
            let title = title.trim();
            if title.is_empty() {
                None
            } else {
                Some(title.to_string())
            }
        }
        None => None,
    })
}
```

---

### Task 1: Parse http(s) lines into `LoadResult.items`

**Files:**
- Modify: `znicz-core/src/playlist.rs` (`LoadResult`, `parse`, `apply_to_player`, `skipped_notice`, module tests)
- Modify: `znicz-tui/src/app.rs` (`play_selected_playlist` uses `result.items.len()`)
- Modify: `znicz-mcp/src/server.rs` (`apply_playlist` `loaded`; `import_playlist_errors_when_nothing_is_playable` fixture)

**Interfaces:**
- Consumes: `QueueItem::file` / `QueueItem::stream`, existing `parse(text, base_dir)`
- Produces: `LoadResult { items: Vec<QueueItem>, skipped: usize }`. `apply_to_player` `QueueAdd`s `result.items`. `skipped_notice` counts `items.len()`. http(s) lines are not skipped.

Do **not** change `write_text`, `write_path`, or `m3u_paths` in this task.

- [ ] **Step 1: Write the failing tests**

At the top of `znicz-core/src/playlist.rs`, add `use crate::player::state::QueueItem;`.

In `#[cfg(test)]`, after `urls_and_missing_files_count_as_skipped` (leave that test as-is for now), add:

```rust
    #[test]
    fn http_and_https_lines_become_stream_items() {
        let dir = tmp();
        let a = touch(&dir, "a.flac");
        let text = format!(
            "http://example.com/x.mp3\nHTTPS://example.com/y\n{}\n",
            a.display()
        );
        let result = parse(&text, &dir);
        assert_eq!(
            result.items,
            vec![
                QueueItem::stream("http://example.com/x.mp3", "http://example.com/x.mp3"),
                QueueItem::stream("HTTPS://example.com/y", "HTTPS://example.com/y"),
                QueueItem::file(a),
            ]
        );
        assert_eq!(result.skipped, 0);
    }

    #[test]
    fn extinf_names_the_next_stream_and_is_ignored_for_files() {
        let dir = tmp();
        let a = touch(&dir, "a.flac");
        let text = format!(
            "#EXTM3U\n#EXTINF:-1,Live\n# a comment\nhttp://127.0.0.1:1/s\n#EXTINF:123,Ignored\n{}\n",
            a.display()
        );
        let result = parse(&text, &dir);
        assert_eq!(
            result.items,
            vec![
                QueueItem::stream("Live", "http://127.0.0.1:1/s"),
                QueueItem::file(a),
            ]
        );
        assert_eq!(result.skipped, 0);
    }

    #[test]
    fn ftp_and_missing_files_are_still_skipped() {
        let dir = tmp();
        let a = touch(&dir, "a.flac");
        let text = format!("ftp://x\nmissing.flac\n{}\n", a.display());
        let result = parse(&text, &dir);
        assert_eq!(result.items, vec![QueueItem::file(a)]);
        assert_eq!(result.skipped, 2);
    }

    #[test]
    fn a_url_only_playlist_is_not_empty() {
        let result = parse("http://127.0.0.1:1/s\n", &tmp());
        assert_eq!(
            result.items,
            vec![QueueItem::stream(
                "http://127.0.0.1:1/s",
                "http://127.0.0.1:1/s"
            )]
        );
        assert_eq!(result.skipped, 0);
    }

    #[test]
    fn apply_append_enqueues_a_stream_without_opening_it() {
        let (player, _thread) = crate::spawn_player(crate::AudioConfig::default());
        apply_to_player(
            &player,
            &LoadResult {
                items: vec![QueueItem::stream("Live", "http://127.0.0.1:1/s")],
                skipped: 0,
            },
            true,
        )
        .unwrap();
        assert_eq!(
            player.state().queue,
            vec![QueueItem::stream("Live", "http://127.0.0.1:1/s")]
        );
        assert_eq!(player.state().status, crate::PlaybackStatus::Stopped);
    }
```

In `znicz-mcp/src/server.rs` tests, after `import_playlist_errors_when_nothing_is_playable`, add:

```rust
    #[test]
    fn import_playlist_appends_an_http_line() {
        let (server, dir) = playlist_server();
        let path = dir.join("radio.m3u");
        std::fs::write(&path, "#EXTINF:-1,Live\nhttp://127.0.0.1:1/s\n").unwrap();

        let result = server
            .import_playlist(Parameters(ImportPlaylistParams {
                path: path.to_string_lossy().into_owned(),
                append: true,
            }))
            .expect("import stream line");
        let payload: serde_json::Value =
            serde_json::from_str(&result_text(&result)).expect("import json");
        assert_eq!(payload["loaded"], 1);
        assert_eq!(payload["skipped"], 0);
        let queue = server.player.state().queue;
        assert_eq!(queue.len(), 1);
        assert_eq!(
            queue[0],
            znicz_core::QueueItem::stream("Live", "http://127.0.0.1:1/s")
        );
        assert_eq!(server.player.state().status, PlaybackStatus::Stopped);

        std::fs::remove_dir_all(&dir).ok();
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run:

```bash
cargo test -p znicz-core http_and_https_lines_become_stream_items --offline
```

Expected: FAIL to compile (`no field items` on `LoadResult`) or FAIL the assertion (http still skipped).

- [ ] **Step 3: Implement parse and `LoadResult.items`**

Replace the module doc, `LoadResult`, `parse`, `apply_to_player`, and `skipped_notice` in `znicz-core/src/playlist.rs`:

```rust
//! M3U / M3U8 playlists: local paths and http(s) stream rows for the queue.
//!
//! Comments and blanks are ignored. Missing files and non-http(s) URLs are
//! skipped and counted. The engine is unchanged: callers send `QueueClear` /
//! `QueueAdd` / `QueuePlayIndex`.

use std::fs;
use std::path::{Path, PathBuf};

use crate::error::{Result, ZniczError};
use crate::player::commands::Command;
use crate::player::engine::PlayerHandle;
use crate::player::state::QueueItem;

/// What a playlist file turned into.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LoadResult {
    pub items: Vec<QueueItem>,
    /// Missing local files and non-http(s) URLs. Comments and blank lines are not counted.
    pub skipped: usize,
}

fn is_http_url(line: &str) -> bool {
    let lower = line.to_ascii_lowercase();
    lower.starts_with("http://") || lower.starts_with("https://")
}

fn extinf_title(line: &str) -> Option<Option<String>> {
    let rest = line.strip_prefix("#EXTINF:")?;
    Some(match rest.split_once(',') {
        Some((_, title)) => {
            let title = title.trim();
            if title.is_empty() {
                None
            } else {
                Some(title.to_string())
            }
        }
        None => None,
    })
}

/// Read an M3U body. `base_dir` resolves relative paths.
pub fn parse(text: &str, base_dir: &Path) -> LoadResult {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut result = LoadResult::default();
    let mut pending: Option<String> = None;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(title) = extinf_title(line) {
            pending = title;
            continue;
        }
        if line.starts_with('#') {
            continue;
        }
        if is_http_url(line) {
            let name = pending.take().unwrap_or_else(|| line.to_string());
            result.items.push(QueueItem::stream(name, line));
            continue;
        }
        if line.contains("://") {
            pending = None;
            result.skipped += 1;
            continue;
        }
        pending = None;
        let path = PathBuf::from(line);
        let path = if path.is_absolute() {
            path
        } else {
            base_dir.join(path)
        };
        if path.is_file() {
            result.items.push(QueueItem::file(path));
        } else {
            result.skipped += 1;
        }
    }
    result
}
```

Keep `load_path` as-is (it already returns `parse`).

Replace `apply_to_player` and `skipped_notice`:

```rust
pub fn apply_to_player(player: &PlayerHandle, result: &LoadResult, append: bool) -> Result<()> {
    if result.items.is_empty() {
        return Err(ZniczError::Player("playlist had no playable files".into()));
    }
    if !append {
        player.send_blocking(Command::QueueClear)?;
    }
    player.send_blocking(Command::QueueAdd(result.items.clone()))?;
    if !append {
        player.send_blocking(Command::QueuePlayIndex(0))?;
    }
    Ok(())
}

pub fn skipped_notice(result: &LoadResult) -> Option<String> {
    if result.skipped == 0 {
        None
    } else {
        Some(format!(
            "{} tracks, {} skipped",
            result.items.len(),
            result.skipped
        ))
    }
}
```

Leave `m3u_paths` using `crate::player::state::QueueItem` **or** switch that signature to the imported `QueueItem` — same type.

Update existing tests in the same file:

- Every `result.paths` → `result.items`. File-only expected values become `vec![QueueItem::file(a)]` (or `file(a), file(b)`).
- `urls_and_missing_files_count_as_skipped`: replace with the behaviour in `http_and_https_lines` / `ftp_and_missing_files` — **delete** this old test so it does not fight the new ones (http is no longer skipped).
- `empty_result_when_nothing_playable`: use `"# only a comment\nftp://x\n"` (http-only is no longer empty).
- `write_then_parse_round_trips_absolute_paths`: `result.items` compared to `vec![QueueItem::file(a.canonicalize()...), QueueItem::file(b.canonicalize()...)]`.
- `skipped_notice_is_none_when_every_row_loaded`: `paths: vec![...]` → `items: vec![QueueItem::file("a.flac")]`.
- `empty_apply_leaves_the_queue_alone`: `paths: Vec::new()` → `items: Vec::new()`.

In `znicz-tui/src/app.rs` `play_selected_playlist`, `result.paths.len()` → `result.items.len()`.

In `znicz-mcp/src/server.rs` `apply_playlist`, `"loaded": result.paths.len()` → `result.items.len()`.

Change `import_playlist_errors_when_nothing_is_playable` so the file is not a valid http playlist:

```rust
std::fs::write(&path, "# only a comment\nftp://x\n").unwrap();
```

- [ ] **Step 4: Run tests to verify they pass**

Run:

```bash
cargo test -p znicz-core --lib playlist --offline
cargo test -p znicz-mcp import_playlist --offline
cargo test -p znicz-tui --offline
```

Expected: PASS. If `znicz-tui` still compiles, its playlist play tests only use file fixtures.

- [ ] **Step 5: Commit**

```bash
git add znicz-core/src/playlist.rs znicz-tui/src/app.rs znicz-mcp/src/server.rs
git commit -m "$(cat <<'EOF'
Load M3U http(s) lines as stream queue rows.

EOF
)"
```

---

### Task 2: Save mixed queues; drop `m3u_paths`

**Files:**
- Modify: `znicz-core/src/playlist.rs` (`write_text`, `write_path`, remove `m3u_paths` and its tests)
- Modify: `znicz-core/src/lib.rs` (drop `m3u_paths` from `pub use playlist::{...}`)
- Modify: `znicz-tui/src/app.rs` (`begin_playlist_save`, `confirm_playlist_save`)
- Modify: `znicz-tui/tests/keys.rs` (replace the two save-refused tests)
- Modify: `znicz-mcp/src/server.rs` (`save_playlist`; replace `save_playlist_errors_when_the_queue_has_a_station`)

**Interfaces:**
- Consumes: `QueueItem` from Task 1
- Produces: `write_text(&[QueueItem]) -> String` and `write_path(path, &[QueueItem])`. Named streams write `#EXTINF:-1,{name}` then the URL. Name equal to URL writes the URL only. Empty-queue checks stay in callers (`queue is empty`). No `cannot save a queue that contains a radio station`.

- [ ] **Step 1: Write the failing tests**

In `znicz-core/src/playlist.rs` tests, **replace** `m3u_paths_writes_files_only_queues`, `m3u_paths_refuses_a_queue_that_contains_a_station`, and `m3u_paths_refuses_a_station_only_queue` with:

```rust
    #[test]
    fn write_text_emits_extinf_for_named_streams_and_bare_urls() {
        let dir = tmp();
        let a = touch(&dir, "a.flac");
        let text = write_text(&[
            QueueItem::file(a.clone()),
            QueueItem::stream("Live", "http://127.0.0.1:1/s"),
            QueueItem::stream(
                "http://example.com/bare",
                "http://example.com/bare",
            ),
        ]);
        assert!(!text.contains('\u{feff}'));
        assert!(text.contains("#EXTINF:-1,Live\nhttp://127.0.0.1:1/s\n"), "{text}");
        assert!(!text.contains("#EXTINF:-1,http://example.com/bare"), "{text}");
        assert!(text.contains("http://example.com/bare\n"), "{text}");
        let parsed = parse(&text, &dir);
        assert_eq!(
            parsed.items,
            vec![
                QueueItem::file(a.canonicalize().unwrap()),
                QueueItem::stream("Live", "http://127.0.0.1:1/s"),
                QueueItem::stream(
                    "http://example.com/bare",
                    "http://example.com/bare"
                ),
            ]
        );
        assert_eq!(parsed.skipped, 0);
    }
```

Change `write_then_parse_round_trips_absolute_paths` to pass queue items:

```rust
        let text = write_text(&[QueueItem::file(a.clone()), QueueItem::file(b.clone())]);
```

(Keep the canonicalize comparison on `parsed.items`.)

In `znicz-tui/tests/keys.rs`, **replace** `playlist_save_of_a_stream_queue_is_refused` and `playlist_save_of_a_mixed_queue_is_refused` with:

```rust
#[test]
fn playlist_save_of_a_stream_queue_opens_the_prompt() {
    let mut app = new_app();
    app.player
        .send_blocking(Command::QueueAdd(vec![QueueItem::stream(
            "Live",
            "https://example.com/s",
        )]))
        .unwrap();
    press_char(&mut app, 'P');
    press_char(&mut app, 'n');
    assert!(
        matches!(app.playlist_prompt, Some(PlaylistPrompt::Save(_))),
        "n should save a station-only queue, got {:?}",
        app.playlist_prompt
    );
}

#[test]
fn playlist_save_of_a_mixed_queue_opens_the_prompt() {
    let mut app = new_app();
    app.player
        .send_blocking(Command::QueueAdd(vec![
            QueueItem::file("/music/a.flac"),
            QueueItem::stream("Live", "http://127.0.0.1:1/s"),
        ]))
        .unwrap();
    press_char(&mut app, 'P');
    press_char(&mut app, 'n');
    assert!(
        matches!(app.playlist_prompt, Some(PlaylistPrompt::Save(_))),
        "n should save a mixed queue, got {:?}",
        app.playlist_prompt
    );
}
```

In `znicz-mcp/src/server.rs`, **replace** `save_playlist_errors_when_the_queue_has_a_station` with:

```rust
    #[test]
    fn save_playlist_writes_a_station_row() {
        let (server, dir) = playlist_server();
        server
            .queue_add(Parameters(QueueAddParams {
                paths: vec!["/music/a.flac".into()],
            }))
            .unwrap();
        server
            .player
            .send_blocking(Command::QueueAdd(vec![znicz_core::QueueItem::stream(
                "Live",
                "http://127.0.0.1:1/s",
            )]))
            .unwrap();
        server
            .save_playlist(Parameters(SavePlaylistParams {
                name: "evening".into(),
            }))
            .expect("save mixed queue");
        let body = std::fs::read_to_string(dir.join("evening.m3u")).unwrap();
        assert!(body.contains("#EXTINF:-1,Live"), "{body}");
        assert!(body.contains("http://127.0.0.1:1/s"), "{body}");
        assert!(body.contains("/music/a.flac") || body.contains("a.flac"), "{body}");
        std::fs::remove_dir_all(&dir).ok();
    }
```

`playlist_server` already sets `playlists_dir`. The test module has `use super::*;`, so `Command` is in scope.

- [ ] **Step 2: Run tests to verify they fail**

Run:

```bash
cargo test -p znicz-core write_text_emits_extinf --offline
```

Expected: FAIL — `write_text` still takes `&[PathBuf]`, so the test does not compile, or `m3u_paths` still exists and the new test is absent from a passing run.

- [ ] **Step 3: Implement writers and callers**

Replace `write_text` and `write_path` in `znicz-core/src/playlist.rs`:

```rust
/// UTF-8, no BOM. Files as absolute paths; streams as URL lines, with
/// `#EXTINF:-1,Name` when the name is not the URL.
pub fn write_text(queue: &[QueueItem]) -> String {
    let mut out = String::new();
    for item in queue {
        match item {
            QueueItem::File { path } => {
                let absolute = path.canonicalize().unwrap_or_else(|_| path.clone());
                out.push_str(&absolute.to_string_lossy());
                out.push('\n');
            }
            QueueItem::Stream { name, url } => {
                if name != url {
                    out.push_str("#EXTINF:-1,");
                    out.push_str(name);
                    out.push('\n');
                }
                out.push_str(url);
                out.push('\n');
            }
        }
    }
    out
}

pub fn write_path(path: &Path, queue: &[QueueItem]) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, write_text(queue))?;
    Ok(())
}
```

**Delete** `m3u_paths` entirely.

In `znicz-core/src/lib.rs`, remove `m3u_paths` from the `pub use playlist::{...}` list.

In `znicz-tui/src/app.rs` `begin_playlist_save`, keep the empty-queue toast and **remove** the `m3u_paths` check:

```rust
    fn begin_playlist_save(&mut self) {
        let queue = self.player.state().queue;
        if queue.is_empty() {
            self.toasts.warn("queue is empty");
            return;
        }
        self.playlist_prompt = Some(PlaylistPrompt::Save(LineEdit::new()));
    }
```

In `confirm_playlist_save`, write the queue directly:

```rust
        let queue = self.player.state().queue;
        match write_path(&path, &queue) {
```

Delete the `let paths = match znicz_core::m3u_paths(...)` block.

In `znicz-mcp/src/server.rs` `save_playlist`:

```rust
        let queue = self.player.state().queue;
        if queue.is_empty() {
            return Err(McpError::invalid_params("queue is empty", None));
        }
        let name = sanitize_stem(&params.name).map_err(Self::map_io)?;
        write_path(&self.playlists_dir.join(&name), &queue).map_err(Self::map_io)?;
        Self::json_result(&serde_json::json!({ "saved": name }))
```

Grep the repo for `m3u_paths` and delete remaining references (tests, comments). Do not leave a wrapper.

- [ ] **Step 4: Run tests to verify they pass**

Run:

```bash
cargo test -p znicz-core --lib playlist --offline
cargo test -p znicz-tui playlist_save --offline
cargo test -p znicz-mcp save_playlist --offline
```

Expected: PASS. Workspace must compile: `cargo test --workspace --offline` if the crate tests pass.

- [ ] **Step 5: Commit**

```bash
git add znicz-core/src/playlist.rs znicz-core/src/lib.rs znicz-tui/src/app.rs znicz-tui/tests/keys.rs znicz-mcp/src/server.rs
git commit -m "$(cat <<'EOF'
Save mixed queues as paths, URLs, and EXTINF names.

EOF
)"
```

---

### Task 3: Previous track is `p` only

**Files:**
- Modify: `znicz-tui/src/app.rs` (global key dispatch)
- Modify: `znicz-tui/src/keys.rs` (`GLOBAL` plus the unit test that documents previous)
- Modify: `znicz-tui/tests/keys.rs` (new `N` test)

**Interfaces:**
- Consumes: existing `skip_track(true)`
- Produces: `KeyCode::Char('p')` still previous. `KeyCode::Char('N')` does nothing. Help string for previous is `p`, not `N / p`.

- [ ] **Step 1: Write the failing tests**

In `znicz-tui/src/keys.rs` tests, replace the assertion that only checks `keys.contains("p")` with:

```rust
        assert!(
            GLOBAL
                .iter()
                .any(|b| b.keys == "p" && b.action.contains("previous")),
            "global help should document p as previous"
        );
        assert!(
            GLOBAL.iter().all(|b| !b.keys.contains('N')),
            "N must not appear in the keymap"
        );
```

In `znicz-tui/tests/keys.rs`, after `lowercase_p_is_still_previous_track`, add:

```rust
#[test]
fn capital_n_is_not_previous_track() {
    let mut app = new_app();
    queue(&mut app, 2);
    press_char(&mut app, 'n');
    assert_eq!(app.state().queue_position, 1);
    press_char(&mut app, 'N');
    assert_eq!(
        app.state().queue_position,
        1,
        "N must not go to the previous row"
    );
    press_char(&mut app, 'p');
    assert_eq!(app.state().queue_position, 0);
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run:

```bash
cargo test -p znicz-tui capital_n_is_not_previous_track --offline
cargo test -p znicz-tui keys --offline --lib
```

Expected: FAIL — `N` still moves position to 0, and/or `GLOBAL` still has `N / p`.

- [ ] **Step 3: Unbind `N`**

In `znicz-tui/src/app.rs`, change:

```rust
            KeyCode::Char('n') => self.skip_track(false),
            KeyCode::Char('N') | KeyCode::Char('p') => self.skip_track(true),
```

to:

```rust
            KeyCode::Char('n') => self.skip_track(false),
            KeyCode::Char('p') => self.skip_track(true),
```

In `znicz-tui/src/keys.rs` `GLOBAL`:

```rust
    b("p", "previous track"),
```

(replacing `b("N / p", "previous track")`).

Do not bind `N` to anything else. Overlay keys stay as they are.

- [ ] **Step 4: Run tests to verify they pass**

Run:

```bash
cargo test -p znicz-tui capital_n_is_not_previous_track lowercase_p_is_still_previous_track --offline
cargo test -p znicz-tui --lib keys --offline
cargo test -p znicz-tui the_help_overlay_draws_at_every_size --offline
```

Expected: PASS (`the_help_overlay` still contains `previous track`).

- [ ] **Step 5: Commit**

```bash
git add znicz-tui/src/app.rs znicz-tui/src/keys.rs znicz-tui/tests/keys.rs
git commit -m "$(cat <<'EOF'
Bind previous track to p only.

EOF
)"
```

---

### Task 4: Version 0.3.3 and docs

**Files:**
- Modify: `Cargo.toml` (`[workspace.package] version`)
- Modify: `README.md` (essentials table)
- Modify: `wiki/Domain/Formats-and-Metadata.md`
- Modify: `wiki/Architecture/TUI.md`
- Modify: `wiki/Architecture/MCP.md`
- Modify: `wiki/Plans/Roadmap.md`
- Modify: `wiki/Issues.md`
- Modify: `wiki/Rust/Cargo-Workspace.md`
- Modify: `znicz-mcp/skills/playlist-curation/SKILL.md`

**Interfaces:**
- Consumes: behaviour from Tasks 1–3
- Produces: docs that match the code. Version **0.3.3**. No invented ICY/HLS.

- [ ] **Step 1: Bump the version and edit docs**

In root `Cargo.toml`: `version = "0.3.3"`.

In `wiki/Rust/Cargo-Workspace.md`: **0.3.2** → **0.3.3**.

In `README.md` essentials table, replace `| N / p | Previous track |` with `| p | Previous track |`.

In `wiki/Domain/Formats-and-Metadata.md`, replace the playlists section so it is no longer “local paths only / URLs skipped”. Use this table and writing rules:

```markdown
## Playlists (Phase 3)

A playlist is an **M3U / M3U8 file** of local paths and `http://` / `https://`
stream rows. PLS and XSPF are still later.

| Line | Meaning |
|------|---------|
| Empty, or a `#` comment other than `#EXTINF:` | Ignored (`#EXTM3U`, `#EXT-X-…`) |
| `#EXTINF:…,Title` | Title for the **next** http(s) line only |
| Starts with `http://` or `https://` | Stream row. Name is the `#EXTINF` title, or the URL |
| Other `://` (`ftp://`, `file://`, …) | Skipped and counted |
| Anything else | A path. Relative paths resolve against the playlist file’s directory |

A UTF-8 BOM is stripped. Missing files are skipped and counted. If nothing
playable remains (no files and no http(s) lines), the queue is left alone.

**Writing** is UTF-8, no BOM. File rows are one absolute path per line. Stream
rows are the URL; if the queue name is not the URL, write `#EXTINF:-1,Name`
on the line before. Saved files live beside the library database:
```

Keep the Linux/Windows paths, `ZNICZ_PLAYLISTS_DIR`, play actions, rename/copy/delete, and `znicz-core::playlist` sentences that follow.

In the Radio section, **delete** “M3U playlists still **skip** `http://` / `https://` lines.” Replace the later-radio sentence with: M3U URL lines play as streams. **Later:** ICY song titles on the transport, HLS. Point at the [roadmap](../Plans/Roadmap.md#later-radio-after-phase-4).

In `wiki/Architecture/TUI.md` Playlists intro, keep “not `p`, which is previous track”. Add one sentence: a playlist file may list local paths and http(s) URLs; save writes both. Do not mention `N`.

In `wiki/Architecture/MCP.md`, after the playlist tools sentence, say playlists may contain http(s) stream rows; `save_playlist` writes those URLs (and `#EXTINF` when named). `queue_add` stays paths-only.

In `wiki/Plans/Roadmap.md`:

- Phase 3 list: add `http(s)` lines in an M3U enqueue as streams; save writes URLs and `#EXTINF` names.
- Later radio: **delete** the M3U stream lines bullet. Leave ICY and HLS only. Do not invent a new phase.

In `wiki/Issues.md`, change “later radio (ICY, HLS, M3U stream lines)” to “later radio (ICY, HLS)”.

In `znicz-mcp/skills/playlist-curation/SKILL.md`:

- Opening: playlists are M3U files of **local paths and http(s) URLs**, not “local paths” only. Stream URLs are in scope for load/save. PLS/XSPF still out.
- Notes: `#EXTINF` before a URL is the queue name. Bare URL uses the URL as the name. `save_playlist` writes `#EXTINF` when the row has a name that is not the URL.
- Delete “Do not invent stream playback from `http://` lines; they are skipped.”
- `append: true` still “adds and does not start”. `loaded` counts files and stream rows.

Do not edit overlay key tables. Do not add ICY or HLS features to the wiki.

- [ ] **Step 2: Run the full workspace tests**

Run:

```bash
cargo test --workspace --offline
cargo fmt --all -- --check
```

Expected: PASS. If rustfmt fails, run `cargo fmt --all` and include the formatting in this commit.

- [ ] **Step 3: Commit**

```bash
git add Cargo.toml README.md wiki znicz-mcp/skills/playlist-curation/SKILL.md
git commit -m "$(cat <<'EOF'
Document M3U stream lines and ship 0.3.3.

EOF
)"
```

If `Cargo.lock` lists the workspace version, add it too.

---

## Spec coverage

| Spec requirement | Task |
| --- | --- |
| `LoadResult.items`; http(s) as streams; `#EXTINF`; ftp/missing skipped | 1 |
| URL-only playlist valid; empty still `playlist had no playable files` | 1 |
| `apply_to_player` / `skipped_notice` / MCP `loaded` / TUI added-count | 1 |
| MCP import of an http line with `append: true` | 1 |
| `write_text`/`write_path` on `QueueItem`; drop `m3u_paths` | 2 |
| TUI/MCP save mixed and station-only queues | 2 |
| Previous is `p`; `N` unbound | 3 |
| Version 0.3.3, README, wiki, skill | 4 |
| No ICY, HLS play, `#15`, overlay keymap change, `queue_add` URLs | constraints (no task) |
