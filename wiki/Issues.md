# Issues

Open bugs and loose ends, newest first. Fixed items move to "Closed" at the bottom.

## Open

_(none)_

## Closed

### #1 — MCP tools return stale state (read-before-write race)

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
2. `PlayerHandle::send_blocking` queues the command and waits for the engine to apply it. `PlayerHandle::send` keeps the old fire-and-forget behaviour for the TUI, which redraws on its own tick.
3. The engine's run loop now waits on the command channel with a timeout instead of sleeping, so an acknowledged command is picked up immediately rather than after a sleep interval.
4. Every mutating MCP tool goes through `ZniczMcpServer::apply`, which waits and then returns the resulting state.

**Bonus fix.** Command failures used to be emitted only as `PlayerEvent::Error`, so `play` on a missing file returned a successful-looking stale snapshot. The acknowledgement carries the engine's `Result`, so the MCP caller now gets a real error.

**Regression tests.**

- `znicz-core/tests/commands.rs` — state is correct with no polling; failures reach the caller
- `znicz-mcp/src/server.rs` tests — returned snapshot shows the new volume and queue; `play` on a missing file errors
