# Architecture overview

Znicz is one workspace with four crates. Each crate has one job.

```
AI host (Cursor) --stdio--> znicz-mcp --commands--> znicz-core --> DAC
Human keys ---------------> znicz-tui --commands--> znicz-core --> DAC
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

Commands travel on a [crossbeam channel](https://docs.rs/crossbeam-channel/). State lives in an `Arc<RwLock<PlayerState>>` so many readers can clone a snapshot.

## Config

`~/.config/znicz/config.toml`:

```toml
[audio]
device = "default"
volume = 1.0
bit_perfect = true

[mcp]
skills_dirs = []
```

`bit_perfect` is a flag for later policy (skip software volume, refuse resampling). Phase 1 still has a software volume control.

## Pages

- [Audio engine](Core-Engine.md)
- [Threads](Audio-Threading.md)
- [TUI](TUI.md)
- [MCP](MCP.md)
