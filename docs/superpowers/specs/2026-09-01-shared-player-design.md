# Shared player process

**Date:** 2026-09-01
**Status:** Draft (awaiting review)
**Crates:** `znicz-core` (IPC + shutdown + idle), `znicz-library` (paths), `znicz-tui` (client), `znicz-mcp` (client), `znicz` (`player` subcommand)
**Version:** 0.3.7 → 0.3.8 (replaces TUI-as-host in [#27](https://github.com/eugene-chekan/znicz/issues/27) / PR 29)
**Supersedes:** [MCP attaches to the running TUI](2026-09-01-mcp-attach-tui-design.md)

## Problem

TUI and MCP each `spawn_player`. Agents talk to a different engine than the
speakers. Hosting the engine inside the TUI makes MCP depend on the TUI. A
later phone remote cannot attach to a UI process. `session.toml` is a restart
snapshot (Stopped), not a live bus.

## Goals

1. **One player process** owns decode and the DAC. TUI, MCP, and a later phone
   app are remotes. MCP does not go through the TUI.
2. First local client (`znicz` or `znicz mcp`) **autostarts** that process if
   it is not running. `znicz player` / `znicz player stop` exist as well.
3. **Playing / Paused** keep the process up after the TUI quits.
4. **Stopped** with **no TUI/phone** connected: after `idle_secs` the process
   writes `session.toml`, exits, and removes the advertise file. Agents do
   **not** block this.
5. No second in-process engine in TUI or MCP. No “close the connection” user
   step: `stop` is playback; tearing down an MCP host drops its socket.

## This slice does not include

- LAN bind, HTTP/WebSocket, TLS, pairing, or a phone app (same JSON later)
- systemd unit file
- Stop when the DAC disappears (later safety net)
- Persisting playing/paused or seek in `session.toml`
- App-state database, HLS, parked TUI `#5`–`#9` / `#22`, playlist `#18` / `#19`

## Processes

```
znicz player  →  znicz-core engine  →  DAC
     ▲
     │  JSON TCP 127.0.0.1 + token
     │
TUI, MCP, (later phone)
```

Only `znicz player` calls `spawn_player` and opens cpal. Library scan/search
and playlist/station **file** CLI stay as today (no engine). Commands that
open the UI (`znicz file.flac`, playlist/station play) autostart the player,
send the play command, then open the TUI as a client.

## Advertise and single instance

Same path as today’s `ipc.toml`: `ZNICZ_IPC_PATH`, else
`$XDG_RUNTIME_DIR/znicz/ipc.toml`, else `{temp}/znicz/ipc.toml`. Fields:
`port`, `token`. Unix mode `0600`. Not `session.toml`.

`player.lock` beside that file: two autostarts cannot spawn two engines. If
advertise exists and TCP works, `znicz player` exits 0 (already running).
Stale file (dead port): replace.

Bind `127.0.0.1:0`. Autostart: spawn this binary as `znicz player` via
`current_exe()`, detached, then wait until advertise + TCP work. Failure:
surface the error; do **not** fall back to a private engine.

`znicz player stop`: send `Shutdown` (no-op if nothing is running). Explicit
foreground `znicz player` is for logs or a user service.

## Protocol

JSON lines, token on every request. Connection stays open for the client
process (many requests, not one TCP connect per command).

First message after connect:

```json
{ "kind": "hello", "token": "…", "role": "ui" }
```

`role` is `ui` (TUI, later phone) or `agent` (MCP). Wrong token: error, no
engine mutation. Then `State` and `Command` as today (`Command` includes
`Shutdown`).

The player counts **UI** connections (`role=ui`). Agent connections do not
change that count.

## Idle (`Stopped` timeout)

Config `config.toml`:

```toml
[player]
idle_secs = 900
```

Default **900**. **0** means never exit on this timer.

| Condition | Player process |
| --- | --- |
| Playing or Paused | Stays up (zero UIs is fine) |
| Stopped and at least one `ui` connection | Stays up |
| Stopped and zero `ui` connections | After `idle_secs`, persist session, exit, remove advertise |

Agents never block the Stopped timer. Always-on MCP hosts can stay connected;
a Stopped engine still exits; the next `play` autostarts.

`q` in the TUI only drops that UI connection. **Quit does not stop playback.**
Music continues until `Stop`, `znicz player stop`, or the Stopped timer after
status is already Stopped and no UI is attached.

More than one `ui` connection is allowed (two TUI windows, later a phone).

## Who writes `session.toml`

Only the player process (debounce on queue/transport extras, and on idle
exit / `Shutdown`). Restore **Stopped** when the player process starts.
TUI and MCP do not write session from a local engine.

## User-facing stop

- Agent or TUI **stop**: status Stopped. Connection stays until that remote
  exits. If no UI is connected, the Stopped timer starts.
- `znicz player stop`: process exits now.
- Users do not close sockets. MCP stdio ending drops the agent connection.

## Errors

- Autostart/connect fail: error in TUI toast / MCP error. No private engine.
- Wrong token: error, host unchanged.
- `player stop` with no process: exit 0.
- Idle exit: normal, not an error.

## Tests

- Two clients: `SetVolume` / play on the host is visible to both.
- Missing advertise: autostart then connect (or test double: start host, then
  client).
- Stale advertise: replaced, one live engine.
- Stopped, zero UI connections, short `idle_secs`: process exits.
- Stopped, one UI hello held: no exit.
- Agent connection only, Stopped: still exits after timeout.
- Playing, zero UIs: no exit.
- `Shutdown` tears down and removes advertise.
- Session file written by the player process on shutdown.

## Wiki

Overview diagram: one player process, TUI and MCP as clients. MCP.md: attach
to the player, not the TUI. TUI.md: `q` leaves playback running. Roadmap:
this is the live bus, not the later app-state database. Index #27 as this
shape in 0.3.8. `mcp-control` skill: stop playback vs player process.

## Later

Phone: another `role=ui` client on a later LAN/HTTP transport, same
`Command` / `PlayerState`. DAC disappeared → stop playback (safety net).
systemd user unit optional; autostart + `znicz player` are enough now.
