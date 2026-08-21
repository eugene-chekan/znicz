# Issues

Open bugs and loose ends, newest first. Fixed items move to "Closed" at the bottom.

## Open

### #1 — MCP tools return stale state (read-before-write race)

- **Filed:** 2026-08-19
- **Component:** `znicz-mcp`
- **Severity:** Low (contract violation; no data corruption, playback unaffected)
- **Status:** Open

**Symptom.** `set_volume` and `play` return a full player-state snapshot that reflects state *before* the operation is applied. The operation itself succeeds, but the return value is one step behind.

Observed through the Hermes MCP client:

| Call | Returned | Actual (verified via `get_player_state`) |
|------|----------|------------------------------------------|
| `set_volume(0.3)` | `volume: 1.0`, `status: Stopped` | `volume: 0.3` |
| `play(<path>)` | `status: Stopped`, `current_track: null` | `status: Playing`, track loaded |

**Impact.** A caller cannot trust the return value to confirm the operation. Any consumer (UI, agent, script) that reads the snapshot and concludes the call failed will misbehave. Workaround: re-poll `get_player_state` after every mutation.

**Likely cause.** The tool handlers send a `Command` into the player's event loop (or a worker thread) and immediately serialize current state without waiting for the mutation to land — a read-before-write race. Same pattern as the TUI issuing a command and redrawing before the engine processes it.

**Fix options.**
1. Return a minimal acknowledgement instead of a full snapshot, e.g. `{"ok": true, "volume": 0.3}`.
2. Wait for the mutation to be applied before reading state back (await the command ack).

## Closed

_(none yet)_
