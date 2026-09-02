# Issues

Open work lives on **[GitHub Issues](https://github.com/eugene-chekan/znicz/issues)**.
This page is an index plus write-ups that never had a GitHub ticket.

## Open

Parked TUI ideas (file, do not start unless asked):

- [#5 Command palette](https://github.com/eugene-chekan/znicz/issues/5)
- [#6 Settings screen overlay](https://github.com/eugene-chekan/znicz/issues/6)
- [#7 Three-column artist / album / tracks library](https://github.com/eugene-chekan/znicz/issues/7)
- [#8 Mouse support](https://github.com/eugene-chekan/znicz/issues/8)
- [#9 Library tree view with expandable nodes](https://github.com/eugene-chekan/znicz/issues/9)
- [#22 Show the app version](https://github.com/eugene-chekan/znicz/issues/22) (place and shape still open)
- [#34 Auto-pan now-playing text](https://github.com/eugene-chekan/znicz/issues/34) (marquee when artist / title does not fit)
- [#36 Reorder list rows with Alt-↑ / Alt-↓](https://github.com/eugene-chekan/znicz/issues/36) (cursor stays on the moved item)

Playlists (file, do not start unless asked):

- [#18 Browse and edit saved playlist contents](https://github.com/eugene-chekan/znicz/issues/18) (view, delete rows, reorder; keys in [#36](https://github.com/eugene-chekan/znicz/issues/36))
- [#19 Add a library item to a saved playlist](https://github.com/eugene-chekan/znicz/issues/19)

Product phases 5–6 (album art, MusicBrainz) and **later radio** (HLS, PLS,
XSPF) are on the
[roadmap](Plans/Roadmap.md), not duplicated as issues yet. Phase 4 (radio
streams, including ICY now playing) is done.

## Closed

### [#32 IPC clients hold one connection forever](https://github.com/eugene-chekan/znicz/issues/32)

- **Fixed:** 2026-09-02
- **Component:** `znicz-core`, `znicz-mcp`, `znicz-tui`, `znicz`
- **Status:** **Fixed** in 0.3.10

TUI and MCP kept one TCP session for their lifetime. After the player process
restarted (idle, crash, rebuild), `state()` swallowed the dead socket and
returned a fake Stopped / empty queue / volume 1.0. That was the remaining
root of [#27](https://github.com/eugene-chekan/znicz/issues/27).

The client now re-reads `ipc.toml` on a transport error, Hellos to the new
host, and retries once. If nothing is advertised, TUI and MCP autostart
`znicz player` the same way they do at first connect. A still-dead socket is
an error, not a default snapshot. `znicz player stop` does not reconnect.

### [#30 session.toml only written on player-daemon exit](https://github.com/eugene-chekan/znicz/issues/30)

- **Fixed:** 2026-09-02
- **Component:** `znicz-core`, `znicz`
- **Status:** **Fixed** in 0.3.9

Mute, volume, queue, repeat, and shuffle were only written to `session.toml`
when the player process exited. A crash or `SIGKILL` dropped the last changes,
and anything reading the file mid-session saw stale values.

The player process now writes after those fields have been stable for about
500 ms, and still flushes on idle exit and `znicz player stop`. Live state
stays on the engine; the file is the restart snapshot.

### [#27 MCP and TUI live player](https://github.com/eugene-chekan/znicz/issues/27)

- **Fixed:** 2026-09-01
- **Component:** `znicz-core`, `znicz-tui`, `znicz-mcp`, `znicz`
- **Status:** **Fixed** in 0.3.8

One `znicz player` process owns decode and the DAC. TUI (`role=ui`) and MCP
(`role=agent`) are clients on localhost JSON TCP. The first `znicz` or
`znicz mcp` autostarts that process. `q` in the TUI does not stop playback.
Stopped with no UI for `idle_secs` (default 900) writes `session.toml` and
exits. Agents do not block that timer. `znicz player stop` shuts the process
down now.

### [#20 Persist the queue across restarts](https://github.com/eugene-chekan/znicz/issues/20)

- **Fixed:** 2026-09-01
- **Component:** `znicz-core`, `znicz-tui`, `znicz-mcp`, `znicz`
- **Status:** **Fixed** in 0.3.6

`session.toml` stores the queue, index, volume, mute, repeat, and shuffle.
Restore is Stopped at 0. Missing files are skipped. A later app-state database
is still later.

### [#15 MCP `queue_remove`](https://github.com/eugene-chekan/znicz/issues/15)

- **Fixed:** 2026-09-01
- **Component:** `znicz-mcp`
- **Status:** **Fixed** in 0.3.5

`queue_remove` takes a 0-based index and uses `Command::QueueRemove`, so a
playing-row delete matches TUI `d`. An index past the end is ignored. The
tool waits for the engine and returns the new state.

### [#14 Deleting the playing queue row](https://github.com/eugene-chekan/znicz/issues/14)

- **Fixed:** 2026-09-01
- **Component:** `znicz-core`, `znicz-tui`
- **Status:** **Fixed** in 0.3.2

`d` on the playing row now stops that file and starts the row that slides
into its index, or stops if it was the last row. Pause/resume match the
decoder.

### Wiki #1 — MCP tools return stale state (read-before-write race)

- **Filed:** 2026-08-19
- **Fixed:** 2026-08-22
- **Component:** `znicz-mcp`, `znicz-core`
- **Severity:** Low (contract violation; no data corruption, playback unaffected)
- **Status:** **Fixed**

**Symptom.** `set_volume` and `play` return a full player-state snapshot that reflects state *before* the operation is applied. The operation itself succeeds, but the return value is one step behind.

Observed through the Hermes MCP client:

| Call | Returned | Actual (verified via `get_player_state`) |
|------|----------|------------------------------------------|
| `set_volume(0.3)` | `volume: 1.0`, `status: Stopped` | `volume: 0.3` |
| `play(<path>)` | `status: Stopped`, `current_track: null` | `status: Playing`, track loaded |

**Impact.** A caller cannot trust the return value to confirm the operation. Any consumer (UI, agent, script) that reads the snapshot and concludes the call failed will misbehave. Workaround: re-poll `get_player_state` after every mutation.

**Likely cause.** The tool handlers send a `Command` into the player's event loop (or a worker thread) and immediately serialize current state without waiting for the mutation to land — a read-before-write race. Same pattern as the TUI issuing a command and redrawing before the engine processes it.

**Confirmed cause.** Every mutating tool called `player.send(command)` (fire and forget) and then immediately serialized `player.state()`. The player thread had not processed the command yet, so the snapshot was the previous one. A test that read state straight after a fire-and-forget `SetVolume(0.3)` reported `1.0`, matching the report exactly.

**Fix (option 2 — wait for the ack).**

1. `CommandEnvelope` in `znicz-core/src/player/commands.rs` carries the command plus an optional reply channel.
2. `PlayerHandle::send_blocking` queues the command and waits for the engine to apply it. `PlayerHandle::send` keeps the old fire-and-forget behaviour for paths that do not need an immediate snapshot.
3. The engine's run loop now waits on the command channel with a timeout instead of sleeping, so an acknowledged command is picked up immediately rather than after a sleep interval.
4. Every mutating MCP tool goes through `ZniczMcpServer::apply`, which waits and then returns the resulting state. TUI keys use `send_blocking` as well, so the next frame and the toast show the real result.

**Bonus fix.** Command failures used to be emitted only as `PlayerEvent::Error`, so `play` on a missing file returned a successful-looking stale snapshot. The acknowledgement carries the engine's `Result`, so the MCP caller now gets a real error.

**Regression tests.**

- `znicz-core/tests/commands.rs` — state is correct with no polling; failures reach the caller
- `znicz-mcp/src/server.rs` tests — returned snapshot shows the new volume and queue; `play` on a missing file errors
