# Architecture overview

Znicz is one workspace with five crates. Each crate has one job.

```
AI host (Cursor) --stdio--> znicz-mcp --commands--> znicz-core --> DAC
Human keys ---------------> znicz-tui --commands--> znicz-core --> DAC
                                  │
                                  └──queries──> znicz-library (SQLite index)
```

`znicz` (the binary) starts either the TUI or the MCP server. Both talk to the same player type: `PlayerHandle`.

## Why split crates?

- **Compile faster** in the long run (you change UI without always rebuilding decoders)
- **Clear borders** (TUI must not call cpal)
- **Tests** can use `znicz-core` without a terminal

## Data that moves

| From | To | What |
|------|----|------|
| TUI / MCP | Core | `Command` enum (play, pause, seek, …) |
| Core | TUI / MCP | `PlayerState` + `PlayerEvent` |
| Decoder thread | Audio callback | `f32` samples in a lock-free ring |
| MCP / CLI | Library | search and album queries |

Commands travel on a [crossbeam channel](https://docs.rs/crossbeam-channel/) inside a `CommandEnvelope`, which can carry a reply channel. State lives in an `Arc<RwLock<PlayerState>>` so many readers can clone a snapshot.

Two ways to send a command:

| Method | Waits? | Used by |
|--------|--------|---------|
| `send` | No | TUI (redraws on its own tick) |
| `send_blocking` | Yes, and returns the engine's result | MCP tools, scripts |

`send_blocking` exists because reading state right after a fire-and-forget send returns the old snapshot. See [MCP](MCP.md#tools-wait-for-the-player).

## Config

`~/.config/znicz/config.toml`:

```toml
[audio]
device = "default"
volume = 1.0
bit_perfect = true

[mcp]
skills_dirs = []

[library]
# Defaults to ~/.local/share/znicz/library.db on Linux
path = "~/.local/share/znicz/library.db"
```

`bit_perfect` is a flag for later policy (skip software volume, refuse resampling). Phase 1 still has a software volume control.

## Pages

- [Audio engine](Core-Engine.md)
- [Threads](Audio-Threading.md)
- [TUI](TUI.md)
- [MCP](MCP.md)
- [Music library](Library.md)
