# Playlist Files Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Load and save M3U playlists as files, with two play actions (clear and play, add to queue), from the TUI (`P`), CLI, and MCP.

**Architecture:** Parse/write in `znicz-core::playlist` with no new `Command`s. Callers issue `QueueClear` / `QueueAdd` / `QueuePlayIndex(0)`. Saved files live beside `library.db` via `znicz_library::default_playlists_dir()`. TUI gets `Modal::Playlists`.

**Tech Stack:** Rust, existing player commands, clap subcommands, rmcp tools, ratatui overlay.

**Spec:** `docs/superpowers/specs/2026-08-27-playlist-files-design.md`

Do not add PLS/XSPF, stream playback, a settings default, or new engine commands.

---

## File map

| File | Responsibility |
| --- | --- |
| Create `znicz-core/src/playlist.rs` | `LoadResult`, `parse`, `write`, `load_path`, `sanitize_stem` |
| Modify `znicz-core/src/lib.rs` | `pub mod playlist` and re-exports |
| Modify `znicz-library/src/lib.rs` | `default_playlists_dir` |
| Modify `znicz-tui/src/app.rs` | `Modal::Playlists`, `P`, cursor, save prompt, load/save |
| Create `znicz-tui/src/views/playlists.rs` | Centered overlay |
| Modify `znicz-tui/src/views/mod.rs` | Draw the overlay |
| Modify `znicz-tui/src/keys.rs` | Bindings and hints |
| Modify `znicz-tui/tests/keys.rs` | `P`, Esc, Enter, `a`, `s` still stops |
| Modify `znicz-tui/tests/render.rs` | Overlay title at the usual sizes |
| Modify `znicz-tui/examples/preview.rs` | Playlists frame |
| Modify `znicz/src/main.rs` | `playlist` subcommand |
| Modify `znicz-mcp/src/server.rs` | Real tools + tests |
| Modify `znicz-mcp/skills/playlist-curation/SKILL.md` | Not stubbed |
| Modify wiki + README | wiki-sync |

Shared load helper used by TUI, CLI, and MCP (in `znicz-core`):

```rust
pub fn apply_to_player(player: &PlayerHandle, result: &LoadResult, append: bool) -> Result<()> {
    if result.paths.is_empty() {
        return Err(ZniczError::Player(
            "playlist had no playable files".into(),
        ));
    }
    if !append {
        player.send_blocking(Command::QueueClear)?;
    }
    player.send_blocking(Command::QueueAdd(result.paths.clone()))?;
    if !append {
        player.send_blocking(Command::QueuePlayIndex(0))?;
    }
    Ok(())
}
```

---

### Task 1: Parse and write M3U

**Files:**
- Create: `znicz-core/src/playlist.rs`
- Modify: `znicz-core/src/lib.rs`

- [ ] **Step 1: Write failing tests** in `znicz-core/src/playlist.rs`

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::Write;

    fn tmp() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "znicz-playlist-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn touch(dir: &Path, name: &str) -> PathBuf {
        let path = dir.join(name);
        fs::write(&path, b"x").unwrap();
        path
    }

    #[test]
    fn comments_and_blank_lines_are_not_skipped_counts() {
        let dir = tmp();
        let a = touch(&dir, "a.flac");
        let text = format!("#EXTM3U\n\n#EXTINF:123,Title\n{}\n", a.display());
        let result = parse(&text, &dir);
        assert_eq!(result.paths, vec![a]);
        assert_eq!(result.skipped, 0);
    }

    #[test]
    fn relative_paths_resolve_against_the_playlist_directory() {
        let dir = tmp();
        let a = touch(&dir, "a.flac");
        let result = parse("a.flac\n", &dir);
        assert_eq!(result.paths, vec![a]);
    }

    #[test]
    fn a_bom_is_stripped() {
        let dir = tmp();
        let a = touch(&dir, "a.flac");
        let mut text = String::from("\u{feff}");
        text.push_str(&format!("{}\n", a.display()));
        let result = parse(&text, &dir);
        assert_eq!(result.paths, vec![a]);
    }

    #[test]
    fn urls_and_missing_files_count_as_skipped() {
        let dir = tmp();
        let a = touch(&dir, "a.flac");
        let text = format!(
            "http://example.com/x.mp3\nmissing.flac\n{}\n",
            a.display()
        );
        let result = parse(&text, &dir);
        assert_eq!(result.paths, vec![a]);
        assert_eq!(result.skipped, 2);
    }

    #[test]
    fn empty_result_when_nothing_playable() {
        let result = parse("# only a comment\nhttp://x\n", &tmp());
        assert!(result.paths.is_empty());
        assert_eq!(result.skipped, 1);
    }

    #[test]
    fn write_then_parse_round_trips_absolute_paths() {
        let dir = tmp();
        let a = touch(&dir, "a.flac");
        let b = touch(&dir, "b.flac");
        let text = write_text(&[a.clone(), b.clone()]);
        assert!(!text.contains('\u{feff}'));
        let result = parse(&text, &dir);
        assert_eq!(result.paths, vec![a, b]);
        assert_eq!(result.skipped, 0);
    }

    #[test]
    fn sanitize_stem_rejects_path_bits() {
        assert!(sanitize_stem("evening").is_ok());
        assert_eq!(sanitize_stem("evening.m3u").unwrap(), "evening.m3u");
        assert_eq!(sanitize_stem("  weekend  ").unwrap(), "weekend.m3u");
        assert!(sanitize_stem("").is_err());
        assert!(sanitize_stem("a/b").is_err());
        assert!(sanitize_stem("..").is_err());
        assert!(sanitize_stem("a\\b").is_err());
    }
}
```

- [ ] **Step 2:** `cargo test -p znicz-core playlist --offline` — FAIL (module missing)

- [ ] **Step 3:** Implement `parse`, `write_text`, `load_path`, `sanitize_stem`, `apply_to_player`. A line is skipped (and counted) only if it is a URL (`://`) or a resolved path that is not a file. Comments and blanks are ignored and not counted.

- [ ] **Step 4:** Tests PASS

- [ ] **Step 5:** Commit `Parse and write M3U playlists.`

---

### Task 2: Playlists directory

**Files:**
- Modify: `znicz-library/src/lib.rs`

- [ ] **Step 1:** Test next to `default_database_path`:

```rust
#[test]
fn playlists_dir_sits_beside_the_library_database() {
    let db = default_database_path().expect("data dir");
    let playlists = default_playlists_dir().expect("data dir");
    assert_eq!(playlists, db.parent().unwrap().join("playlists"));
}
```

Put the test in `znicz-library/src/lib.rs` under `#[cfg(test)]`.

- [ ] **Step 2:** Run it — FAIL (`default_playlists_dir` missing)

- [ ] **Step 3:**

```rust
pub fn default_playlists_dir() -> Option<std::path::PathBuf> {
    dirs_data_dir().map(|dir| dir.join("znicz").join("playlists"))
}
```

- [ ] **Step 4:** PASS

- [ ] **Step 5:** Commit `Point saved playlists at the user data directory.`

Also add `list_saved(dir) -> Vec<String>` (stems, sorted) in `znicz-core/src/playlist.rs` so the TUI does not reimplement directory listing. Test with a temp dir containing `b.m3u`, `a.m3u8`, `ignore.txt`.

---

### Task 3: TUI overlay

**Files:**
- Create: `znicz-tui/src/views/playlists.rs`
- Modify: `znicz-tui/src/app.rs`, `keys.rs`, `views/mod.rs`, `views/library.rs` (unfocus when Playlists), `views/queue.rs`, `views/status.rs`, `views/help.rs`
- Test: `znicz-tui/tests/keys.rs`, `tests/render.rs`
- Modify: `examples/preview.rs`

`App` fields: `playlists_dir: PathBuf`, `playlists: Vec<String>`, `playlist_cursor: Cursor`, `playlist_input: Option<String>`.

Load the directory when opening the modal (`P`). `sanitize_stem` + `playlists_dir.join(name)` for save.

Clear and play / add call `playlist::load_path` then `playlist::apply_to_player`.

- [ ] **Step 1:** Keys tests: `P` toggles `Modal::Playlists`; Esc closes; `s` while open still sets `should_quit` false but stop (status becomes Stopped if something was queued — or just `modal` stays open and global `s` is consumed). Spec: `s` still **stop**. Assert modal stays `Playlists` after `s`.

- [ ] **Step 2:** FAIL

- [ ] **Step 3:** Wire `Modal::Playlists` like Devices (cursor, Esc, global `P` toggle). Overlay view copies devices: `Clear` + list + bottom hint. Save prompt reuses the library search look (`save: █`).

- [ ] **Step 4:** PASS + render test contains `Playlists`

- [ ] **Step 5:** Commit `Open playlists from P with clear-and-play or add.`

---

### Task 4: CLI

**Files:**
- Modify: `znicz/src/main.rs`

```rust
Playlist {
    #[command(subcommand)]
    command: PlaylistCmd,
}

enum PlaylistCmd {
    List,
    Import { file: PathBuf, #[arg(long)] append: bool },
    Save { name: String },
    Play { name: String, #[arg(long)] append: bool },
}
```

`List` prints stems. `Save` always errors with the spec message. `Play` / `Import` call `run_tui` after `load_path` + `apply_to_player` (need the player before TUI: spawn, apply, then `App::with_library`).

- [ ] **Step 1:** `cargo build -p znicz` with the new subcommand; a tiny unit test is not required if `znicz playlist list` is exercised by `cargo run -p znicz -- playlist list`.

- [ ] **Step 2–4:** Implement and `cargo build -p znicz`

- [ ] **Step 5:** Commit `Add znicz playlist list, import, play, and save.`

---

### Task 5: MCP tools

**Files:**
- Modify: `znicz-mcp/src/server.rs`
- Modify: `znicz-mcp/skills/playlist-curation/SKILL.md`

Replace stubs. `append` defaults false via `#[serde(default)]`. Tests with temp playlists dir: inject dir via env `ZNICZ_PLAYLISTS_DIR` so tests do not touch `$HOME`.

Add in `playlist.rs`:

```rust
pub fn playlists_dir_override() -> Option<PathBuf> {
    std::env::var_os("ZNICZ_PLAYLISTS_DIR").map(PathBuf::from)
}
```

`default_playlists_dir` in library should prefer that env if set — **do this in Task 2** so MCP tests can isolate.

- [ ] **Step 1:** MCP test: existing queue of 1, `play_playlist(append: true)` → queue length 2, still not playing if stopped; `append: false` → `QueuePlayIndex` so status Playing (needs a real file). Use the same temp flac trick as library tests, or skip playback assert and only check queue paths with a touched empty file — `QueuePlayIndex` will error on invalid audio. For MCP, use a tiny valid fixture if one exists; otherwise assert `apply_to_player` errors on empty and queue-add works with a dummy path that fails decode... Spec wants playback started. Library browse tests create flacs with ffmpeg.

Prefer: unit-test `apply_to_player` in znicz-core with QueueAdd of temp files; `QueuePlayIndex` may fail decode. Spec: "play_playlist with append: false starts playback". If decode fails, MCP returns error. Use ffmpeg fixture when available, otherwise assert queue contents after QueueAdd without PlayIndex in core tests, and MCP test checks JSON `loaded`/`skipped` with missing-only playlist error.

Pragmatic MCP test: `save_playlist` errors on empty queue; `list_playlists` sees a written file; `import_playlist` with append true extends queue using paths that exist as empty files — `QueuePlayIndex` will fail. So **clear and play** MCP test should use `append: true` for queue length, and a separate core test that `apply_to_player(..., false)` sends the three commands... we can't intercept. Integration: if ffmpeg exists, create a tiny wav/flac.

Follow `znicz-tui/tests/library_browse.rs` ffmpeg helper if present.

- [ ] **Step 5:** Commit `Implement MCP playlist tools.`

---

### Task 6: Docs

**Files:** README, `wiki/Domain/Formats-and-Metadata.md`, `wiki/Architecture/TUI.md`, `wiki/Architecture/MCP.md`, `wiki/Plans/Roadmap.md` (Phase 3 **Done**), `wiki/Domain/TUI-Players.md` if keys mentioned.

- [ ] Update in the same change as the last code commit, or this commit if code already landed.
- [ ] Commit `Document M3U playlists in the wiki and README.`

---

## Spec coverage

| Spec | Task |
| --- | --- |
| parse/write/skip rules | 1 |
| data dir | 2 |
| Enter vs `a`, `P`, `w` | 3 |
| CLI TUI launch, save refused | 4 |
| MCP + list + skill | 5 |
| wiki-sync | 6 |
