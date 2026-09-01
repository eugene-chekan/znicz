# Persist the queue across restarts ([#20](https://github.com/eugene-chekan/znicz/issues/20))

**Date:** 2026-09-01
**Status:** Approved
**Crates:** `znicz-core`, `znicz-library` (data-dir helper), `znicz-tui`, `znicz-mcp`, `znicz`

## Problem

Closing znicz drops the live queue, the playing-row index, and transport extras
(repeat, shuffle, volume, mute). Stations and playlists are files; the session
is not. Opening the player again is an empty queue.

## Goals

1. Write the queue (file and stream rows), `queue_position`, volume, mute,
   repeat, and shuffle to **`session.toml`**.
2. Restore on start when the start did not already set a queue. Do **not**
   auto-play. Seek stays 0. Status is **Stopped**.
3. Skip missing local files (like M3U). Keep stream rows. Clamp the index.
4. Write on quit and shortly after queue/transport changes so a crash is not a
   total loss.
5. Same file for TUI and MCP.

## This slice does not include

- File seek position or radio byte offset
- Auto-play / resume playing
- Device (stays `config.toml`)
- ICY / HLS / PLS / XSPF
- Settings UI ([#6](https://github.com/eugene-chekan/znicz/issues/6))
- A separate app-state / config **database** — **later** (see roadmap). This
  slice is TOML. `library.db` stays the music index only.
- Using a playlist file as the session (`P` then `n` stays a named save)

## File

Default, beside `stations.toml` and `library.db`:

- Linux: `~/.local/share/znicz/session.toml`
- Windows: `%APPDATA%\znicz\session.toml`

Override: **`ZNICZ_SESSION_PATH`**. Create the parent folder on first write.

This is not `~/.cache/znicz/znicz-session.log` (stderr while the TUI runs).

Example:

```toml
queue_position = 1
volume = 0.8
muted = false
repeat = "Off"
shuffle = false

[[queue]]
kind = "file"
path = "/music/a.flac"

[[queue]]
kind = "stream"
name = "Live"
url = "https://example.com/s"
```

Missing file → empty session (default volume 1.0, empty queue). Corrupt TOML →
warn and start empty; do not refuse to open the player.

## Restore

| Start | Queue from session? | Transport extras? |
|-------|---------------------|-------------------|
| Bare `znicz` | Yes | Yes |
| `znicz mcp` | Yes | Yes |
| `znicz file.flac` (one or more files) | No | Yes |
| `znicz playlist play NAME` / `import` without `--append` | No | Yes |
| Same with `--append` | Yes, then append | Yes |
| `znicz station play NAME` without `--append` | No | Yes |
| Same with `--append` | Yes, then append | Yes |

Transport extras: volume, mute, repeat, shuffle.

After prune, status is Stopped, position 0, no decoder. Space still starts the
row at `queue_position`.

TUI: if any files were skipped, a warn toast (same idea as playlist skipped
notice). MCP: log only.

## Write

Snapshot from `PlayerState` (queue, index, volume, mute, repeat, shuffle). Not
seek, not playing/paused, not device.

- **TUI:** write on quit; while running, write when that snapshot changed and
  has been stable ~500 ms (not on seek-bar ticks).
- **MCP:** write after a mutating tool that changes player state; write on
  process exit if possible.
- Clearing the queue writes an empty `queue = []` so the next start is empty.

Tests must not write into the user’s real data dir: set `ZNICZ_SESSION_PATH` to
a temp file (same pattern as stations).

## Engine

`Command::ReplaceQueue { items, position }` replaces the list, clamps
`queue_position`, **stops** playback. Used only for session restore of the
queue. Volume/mute/repeat/shuffle use the existing commands.

## Tests

- Round-trip TOML (file + stream row)
- Missing path dropped; stream kept; index clamped
- Missing session file loads defaults
- `ReplaceQueue` does not start playback
- MCP `set_volume` then a new server with restore sees 0.3 (temp path)

## Wiki

Session paragraph on [Formats and metadata](../../../wiki/Domain/Formats-and-Metadata.md)
or TUI/MCP. Roadmap: #20 done this slice; later “app state / config database”
(not `library.db`). Close #20 in Issues.
