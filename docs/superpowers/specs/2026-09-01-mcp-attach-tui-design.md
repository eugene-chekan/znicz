# MCP attaches to the running TUI ([#27](https://github.com/eugene-chekan/znicz/issues/27))

**Date:** 2026-09-01
**Status:** Superseded by [Shared player process](2026-09-01-shared-player-design.md)
**Crates:** `znicz-core` (IPC), `znicz-library` (path), `znicz-tui` (host), `znicz-mcp` (client), `znicz`
**Version:** 0.3.7 → 0.3.8

## Problem

`znicz` and `znicz mcp` each call `spawn_player`. Cursor keeps a long-lived MCP
process. `get_player_state` reads that process, so it stays Stopped / empty
while the TUI is playing. `session.toml` is a snapshot on start, not a live bus,
and it does not store playing/paused, seek, or ICY titles.

## Goals

1. While the TUI is running, MCP tools and resources use **that** engine
   (queue, volume, status, current track, ICY title).
2. With no TUI, MCP keeps today’s headless player (restore session Stopped).
3. MCP must not overwrite `session.toml` with an unused empty local player on
   exit.

## This slice does not include

- Two TUIs sharing one engine
- MCP-first audio, then TUI attaching to MCP
- Persisting playing/paused in `session.toml`
- A separate app-state database
- HLS / parked TUI issues

## Approach

The TUI **hosts**. MCP **attaches** when the host is up.

- Bind `127.0.0.1:0` (TCP, Linux and Windows).
- Write `ipc.toml` with `port` and a random `token` (file mode `0600` on Unix).
- Default path: `ZNICZ_IPC_PATH`, else `$XDG_RUNTIME_DIR/znicz/ipc.toml`, else a
  temp folder. Not `session.toml`.
- One JSON request and one JSON response per connection. Token must match.
- `State` returns `PlayerState`. `Command` runs `send_blocking` on the TUI
  handle and returns the new state (or an error).

Each MCP tool call: try the advertise file and connect. Success → TUI.
Failure (missing file, dead port, bad token) → local player.

## Session

- After a **local** mutation, write `session.toml` from the local player (today).
- After a **remote** mutation, write `session.toml` from the returned TUI state
  (same queue/volume; TUI still debounces too).
- MCP process exit: if the TUI is reachable, save TUI state (or skip — TUI
  already writes). If the TUI is gone and this MCP process **never** used the
  local player, **do not write**. If it did use the local player, save local.

## Tests

- Loopback IPC: SetVolume on the server handle is visible to the client.
- Dead advertise file → client falls back (no panic).
- Wrong token → error, local player unchanged.

## Wiki

MCP and Overview: TUI hosts; MCP attaches when present. Index #27 as fixed in
0.3.8. Roadmap: this slice, not the later app-state database.
