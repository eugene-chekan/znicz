# Architecture overview

Znicz is one workspace with five crates. Each crate has one job.

```
AI host (Cursor) --stdio--> znicz-mcp ──┐
Human keys ---------------> znicz-tui ──┼── JSON TCP 127.0.0.1 ──> znicz player ──> znicz-core ──> DAC
                                        │
                          (later phone) ┘
                              │
                              └──queries──> znicz-library (SQLite index)
```

Both front ends can browse the library: the MCP server for agents, and the TUI
for its library pane.

`znicz` (the binary) starts the TUI, MCP, library CLI, or **`znicz player`**.
The first TUI or MCP start **autostarts** the player process if it is not
running. Only that process calls `spawn_player` and opens the DAC.
[#27](https://github.com/eugene-chekan/znicz/issues/27).

## Why split crates?

- **Compile faster** in the long run (you change UI without always rebuilding decoders)
- **Clear borders** (TUI must not call cpal)
- **Tests** can use `znicz-core` without a terminal

## Data that moves

| From | To | What |
|------|----|------|
| TUI / MCP | Core | `Command` enum (`Play(QueueItem)`, pause, seek, …) |
| Core | TUI / MCP | `PlayerState` + `PlayerEvent` |
| Decoder thread | Audio callback | `f32` samples in a lock-free ring |
| TUI / MCP / CLI | Library | search and album queries |

`PlayerState` is the whole of what a front end can see: transport status, current
track with its tags, position, volume and mute, queue and position within it,
repeat and shuffle, the chosen device, and `output` — the stream the device
actually opened, which is what tells the TUI whether playback is bit perfect.
A queue row is a `QueueItem`: a local file or a named HTTP stream
(`kind` is `file` or `stream` when the state is serialised). `Play` takes that
same `QueueItem`.

Commands travel on a [crossbeam channel](https://docs.rs/crossbeam-channel/) inside a `CommandEnvelope`, which can carry a reply channel. State lives in an `Arc<RwLock<PlayerState>>` so many readers can clone a snapshot.

Two ways to send a command:

| Method | Waits? | Used by |
|--------|--------|---------|
| `send` | No | startup, where the next redraw picks the change up |
| `send_blocking` | Yes, and returns the engine's result | MCP tools, TUI keys, scripts |

`send_blocking` exists because reading state right after a fire-and-forget send returns the old snapshot. See [MCP](MCP.md#tools-wait-for-the-player). The TUI uses it on key handlers for the same reason.

The TUI uses it for two reasons: the frame drawn right after a keypress shows the
new volume rather than the old one, and a failure (missing file, unusable device)
comes back as a value it can display instead of disappearing into the log.

## Config

`~/.config/znicz/config.toml`:

```toml
[audio]
device = "default"
volume = 1.0
bit_perfect = true

[mcp]
skills_dirs = []

[player]
idle_secs = 900

[library]
# Defaults to ~/.local/share/znicz/library.db on Linux
path = "~/.local/share/znicz/library.db"
```

`bit_perfect` is a flag for later policy (skip software volume, refuse resampling). Phase 1 still has a software volume control.

The last queue lives in `session.toml` in the data dir (see [Formats and metadata](../Domain/Formats-and-Metadata.md#session)). The **player process** writes it after queue or transport extras settle, and on idle exit / `znicz player stop`. Device pick stays in `config.toml`. A later app-state database may hold both.

`idle_secs` is how long a **Stopped** player stays up with no TUI (or later phone) connected. Default **900**. **0** means never exit on that timer. Playing or paused keeps the process up after you quit the TUI. Agents do not block the timer. If the player process then exits, the next TUI key or MCP tool re-reads `ipc.toml` and autostarts if needed.

## Pages

- [Audio engine](Core-Engine.md)
- [Threads](Audio-Threading.md)
- [TUI](TUI.md)
- [MCP](MCP.md)
- [Music library](Library.md)
