# MCP Attach TUI Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans or subagent-driven-development.

**Superseded** by [Shared player process](../specs/2026-09-01-shared-player-design.md). Do not implement TUI-as-host.

**Goal:** While the TUI runs, MCP `get_player_state` and mutating tools use that engine ([#27](https://github.com/eugene-chekan/znicz/issues/27)).

**Architecture:** TUI binds `127.0.0.1:0`, writes `ipc.toml` (`port`, `token`). MCP tries that file each call; otherwise the local headless player. MCP exit must not wipe `session.toml` with an unused empty local player.

**Tech Stack:** `std::net::TcpListener` / `TcpStream`, serde JSON. No new crates.

**Spec:** `docs/superpowers/specs/2026-09-01-mcp-attach-tui-design.md`

## Global Constraints

- Version **0.3.7 → 0.3.8**.
- TCP localhost only. `ZNICZ_IPC_PATH` override. Unix file mode `0600`.
- No second TUI sharing. No MCP-as-host. No playing/paused in `session.toml`.
- Parked TUI `#5`–`#9` / `#22`, playlist `#18` / `#19` untouched.
- Wiki matches the code in the same change.

## File map

| File | Role |
| --- | --- |
| Modify `znicz-core/src/player/commands.rs` | `Serialize`/`Deserialize` on `Command` |
| Create `znicz-core/src/player/ipc.rs` | Advertise, server, client |
| Modify `znicz-core/src/player/engine.rs` | `PlayerOps` trait on `PlayerHandle` |
| Modify `znicz-core/src/player/mod.rs` | `pub mod ipc` |
| Modify playlist/station `apply`/`play_station` | `&dyn PlayerOps` |
| Modify `znicz-library/src/lib.rs` | `default_ipc_path()` |
| Modify TUI `App` | Start/stop `IpcServer` |
| Modify MCP server | Attach per call; exit persist rules |
| Wiki + skills + `Cargo.toml` | 0.3.8, #27 fixed |

---

### Task 1: IPC loopback

TDD in `ipc.rs`: serve + `try_command(SetVolume(0.4))` + `try_state` sees 0.4. Wrong token errors. Missing file is a connect error.

`PlayerOps`: `send_blocking`, `state`. `PlayerHandle` implements it.

### Task 2: TUI host + MCP attach

The `znicz` binary calls `App::host_player_ipc` (warn and continue if bind fails). Tests and the preview example do not, so they never steal a live `ipc.toml`. Drop on quit.

MCP: `live_state()` / `send_live()`. `apply` and resources use those. `local_used` when falling back. `persist_on_exit` as spec. Remove unconditional session save in `run_mcp` after stdio.

### Task 3: Wiki 0.3.8

Roadmap, MCP, Overview, Issues (#27 closed), mcp-control skill, Cargo-Workspace version.
