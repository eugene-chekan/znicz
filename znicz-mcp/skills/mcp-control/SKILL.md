---
name: mcp-control
description: Control znicz via MCP tools and resources. Use when driving playback, reading state, or chaining tool calls from an AI agent.
---

# MCP Control

## Tools

| Tool | Use |
|------|-----|
| `play` | Start local file playback |
| `pause` / `resume` / `stop` | Transport |
| `seek` | Seek to seconds |
| `set_volume` | 0.0–1.0 |
| `next_track` / `previous_track` | Queue navigation |
| `queue_add` / `queue_remove` / `queue_clear` / `queue_get` | Queue (paths and http(s) URLs) |
| `get_player_state` | Full snapshot |
| `list_devices` / `set_device` | Output selection |

## Resources

| URI | Content |
|-----|---------|
| `znicz://now-playing` | Current track JSON |
| `znicz://queue` | Queue paths |
| `znicz://player/status` | Status JSON |
| `znicz://devices` | Device list |
| `skill://index.json` | Skills index |

## Patterns

1. `queue_add` with file paths and/or `http(s)` URLs, then `play` first path, or `play` with auto-queue. URLs enqueue as streams and do not start playback.
2. `queue_remove` with a 0-based index to drop one row (same rule as TUI `d`)
3. Poll `get_player_state` or subscribe to resources for progress
4. Load domain skills (`audiophile-playback`, etc.) only when needed
5. `stop` is playback. To exit the player process: `znicz player stop`

The live queue is restored from `session.toml` when `znicz player` starts
(Stopped). The player process updates that file shortly after queue or
transport extras change, and on exit. TUI and MCP attach to that process.
`stop` is playback; it does not exit the player. `znicz player stop` does.
Override `ZNICZ_SESSION_PATH`.
Advertise file override: `ZNICZ_IPC_PATH`.
