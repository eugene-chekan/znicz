# Shared Player Process Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans or subagent-driven-development. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** One `znicz player` process owns the DAC; TUI and MCP are remotes with Hello `ui`/`agent`, Stopped idle exit, no private engines.

**Architecture:** Extend `ipc.rs`: persistent TCP, `hello` with role, UI connection count, `Shutdown`, idle when Stopped and zero UI clients. Binary subcommands `player` / `player stop` plus autostart. TUI/`znicz mcp` only attach.

**Tech Stack:** Existing `TcpListener` / JSON lines / serde. No new crates. `config.toml` `[player] idle_secs` (default 900, `0` = never).

**Spec:** `docs/superpowers/specs/2026-09-01-shared-player-design.md`

## Global Constraints

- Version stays **0.3.8** (replace TUI-as-host on the same cycle / PR 29).
- Bind `127.0.0.1` only. Advertise `ipc.toml` (`ZNICZ_IPC_PATH` / `XDG_RUNTIME_DIR` / temp). Unix mode `0600`.
- `player.lock` beside advertise. No second in-process engine in TUI or MCP.
- `q` does not stop playback. Agents do not block Stopped idle.
- Parked TUI `#5`–`#9` / `#22`, playlist `#18` / `#19`, HLS, LAN/HTTP/phone, DAC-unplug, systemd: untouched.
- Wiki matches the code in the same change.

## File map

| File | Role |
| --- | --- |
| Modify `znicz-core/src/player/ipc.rs` | Persistent session, Hello, roles, idle, Shutdown |
| Modify `znicz-core/src/player/engine.rs` | `PlayerOps::drain_events` |
| Modify `znicz-core/src/lib.rs` | Export `ClientRole`, `ensure` helpers as needed |
| Modify `znicz-library/src/lib.rs` | `default_player_lock_path()` beside ipc |
| Modify `znicz/src/main.rs` | `player` / `player stop`, autostart, session only in player |
| Modify `znicz-tui/src/app.rs` | Remote `IpcClient` in production; no `IpcServer`; no session write |
| Modify `znicz-mcp/src/lib.rs` + `server.rs` | Agent client only; no `spawn_player` |
| Wiki + `mcp-control` skill + spec Status Approved | One engine diagram |

---

### Task 1: Persistent IPC, Hello, idle, Shutdown

**Files:**
- Modify: `znicz-core/src/player/ipc.rs`
- Modify: `znicz-core/src/player/engine.rs` (`PlayerOps`)
- Modify: `znicz-core/src/lib.rs`

**Interfaces:**
- Produces: `ClientRole::{Ui, Agent}`, `IpcServer::start(player, advertise, idle: Duration)`, `IpcClient::connect(path, role)`, `IpcClient::shutdown()`, `IpcServer` runs until Shutdown or idle, `ui_connections` counted from Hello `ui` until the socket EOF.

- [ ] **Step 1: Extend protocol and tests in `ipc.rs`**

`IpcRequest` gains `Hello { token, role }` and `Shutdown { token }`. `ClientRole` is `ui` | `agent` (serde snake_case).

`handle_conn` becomes a **session loop**: first line must be Hello; mismatch writes Err; `ui` increments a shared `AtomicUsize` and decrements on disconnect. Further lines are State / Command / Shutdown on the **same** stream.

`IpcClient` holds `Mutex<TcpStream>`, sends Hello on `connect`, then RPC on that stream.

`IpcServer::start(player, path, idle: Duration)` — `Duration::ZERO` means never idle-exit.

Idle thread/loop: every 50ms, if `player.state().status == Stopped` **and** `ui_count == 0` **and** `idle > ZERO`, once that condition has held for `idle`, set stop (same as Shutdown). Playing/Paused or `ui_count > 0` resets the idle clock.

Shutdown: persist is **not** required inside ipc.rs if the binary saves on wait return; tests should still see the process/server stop and advertise removed on Drop/stop.

Tests to add (keep existing volume/token/missing tests, update client to `connect(path, ClientRole::Agent)`):

```rust
#[test]
fn ui_hello_blocks_stopped_idle() {
    let path = advertise_path();
    let (player, _thread) = spawn_player(AudioConfig::default());
    let server = IpcServer::start(player, &path, Duration::from_millis(80)).unwrap();
    let _ui = IpcClient::connect(&path, ClientRole::Ui).unwrap();
    thread::sleep(Duration::from_millis(200));
    assert!(IpcClient::connect(&path, ClientRole::Agent).is_ok());
    drop(server);
}

#[test]
fn agent_only_does_not_block_stopped_idle() {
    let path = advertise_path();
    let (player, _thread) = spawn_player(AudioConfig::default());
    let _server = IpcServer::start(player, &path, Duration::from_millis(80)).unwrap();
    let _agent = IpcClient::connect(&path, ClientRole::Agent).unwrap();
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(2) {
        if IpcClient::connect(&path, ClientRole::Agent).is_err() {
            return;
        }
        thread::sleep(Duration::from_millis(20));
    }
    panic!("player should idle-exit with only an agent connected");
}

#[test]
fn playing_does_not_idle_without_ui() {
    // SetVolume does not play; use status Playing via a tiny command if needed.
    // After start, send Pause/Resume won't play. Use QueueAdd+play only if
    // hardware-safe: instead set a test hook OR document that idle checks
    // PlaybackStatus::Playing — spawn and Command::SetMuted is still Stopped.
    // Implement idle only on Stopped: this test sends nothing, waits 200ms
    // with idle 80ms, server must still be up (Stopped + zero UI WOULD exit).
    // So this test must put status Playing. Add `IpcServer` test helper
    // `force_status`? Better: `player.send_blocking` cannot set Playing
    // without a file. Check PlaybackStatus in idle: for this test, use idle
    // ZERO (never) vs agent_only test. Spec: playing + zero UI no exit.
    // Use `AudioConfig` and skip if we cannot play; or inject via
    // `PlayerHandle::state_arc` in the test like stream.rs bookkeeping.
}

#[test]
fn shutdown_stops_the_server() {
    let path = advertise_path();
    let (player, _thread) = spawn_player(AudioConfig::default());
    let _server = IpcServer::start(player, &path, Duration::ZERO).unwrap();
    let client = IpcClient::connect(&path, ClientRole::Agent).unwrap();
    client.shutdown().unwrap();
    thread::sleep(Duration::from_millis(100));
    assert!(IpcClient::connect(&path, ClientRole::Agent).is_err());
}
```

For `playing_does_not_idle_without_ui`: in the test, write `player.state_arc()` status to `Playing` (same pattern as `znicz-core/tests/stream.rs` bookkeeping) so idle must not fire.

- [ ] **Step 2: Run** `cargo test -p znicz-core ipc::` — new tests fail (no Hello/idle).
- [ ] **Step 3: Implement** session loop, Hello, idle, Shutdown, persistent client.
- [ ] **Step 4: Run** `cargo test -p znicz-core ipc::` — pass. `cargo clippy -p znicz-core --all-targets -- -D warnings`.
- [ ] **Step 5: Commit** `Serve one player socket with UI roles and Stopped idle exit.`

---

### Task 2: `znicz player` process, lock, autostart, session

**Files:**
- Modify: `znicz-library/src/lib.rs` — `default_player_lock_path()` next to `ipc.toml` (`player.lock`).
- Modify: `znicz/src/main.rs` — `Commands::Player { command: Option<PlayerCmd> }` with `Stop` subcommand; default `znicz player` runs the daemon.
- Session restore/save **only** here. Remove TUI/MCP session writes in later tasks.

**Interfaces:**
- Produces: `run_player_daemon(audio, idle_secs, session_path)` blocks until idle/Shutdown. `ensure_player(exe, ipc_path)` connect or spawn `current_exe() player` detached then poll connect ≤ 3s. `znicz player stop` = `IpcClient::shutdown` or exit 0 if missing.

```rust
#[derive(Subcommand)]
enum Commands {
    /// Shared playback engine (autostarted by TUI and MCP)
    Player {
        #[command(subcommand)]
        command: Option<PlayerCmd>,
    },
    // ...
}
enum PlayerCmd { Stop }
```

`Config` gains `player: PlayerSection { idle_secs: Option<u64> }` default 900.

Autostart spawn:

```rust
std::process::Command::new(std::env::current_exe()?)
    .arg("player")
    .stdin(std::process::Stdio::null())
    .stdout(std::process::Stdio::null())
    .stderr(std::process::Stdio::null())
    .spawn()?;
```

Lock: `OpenOptions::create_new` on `player.lock`; if exists and advertise TCP is live → already running (exit 0). If advertise dead, remove lock + advertise and retry.

- [ ] **Step 1:** Test lock path in `znicz-library` (same style as `ipc_path_honours_override_or_runtime_dir`).
- [ ] **Step 2:** `cargo test -p znicz-library ipc_path` / new lock test.
- [ ] **Step 3:** Implement daemon + stop + ensure. Daemon: `spawn_player`, `restore_session` Stopped, `IpcServer::start(..., Duration::from_secs(idle))`, `server.wait()`, `save_session_from_player`.
- [ ] **Step 4:** `cargo test --workspace` (may still have TUI hosting — next task). Clippy on `znicz`.
- [ ] **Step 5: Commit** `Add znicz player as the only process that opens the DAC.`

---

### Task 3: TUI is a UI client

**Files:**
- Modify: `znicz-tui/src/app.rs` — `Live` enum `Local(PlayerHandle) | Remote(IpcClient)`; tests/preview stay Local; no `IpcServer`; **do not** `save_session` from the TUI; `q` only `should_quit` (no Stop).
- Modify: `znicz/src/main.rs` — `run_tui*`: `ensure_player`, `IpcClient::connect(..., Ui)`, `App` remote; send Play/queue **on the client** after attach; no `spawn_player` for TUI.

`PlayerOps` gains `fn drain_events(&self) -> Vec<PlayerEvent>` — `PlayerHandle` real drain; `IpcClient` returns `vec![]`.

- [ ] **Step 1:** TUI unit tests still construct `App::with_library(test_player(), None)` (Local). Add nothing that starts IPC on default path.
- [ ] **Step 2:** `cargo test -p znicz-tui`.
- [ ] **Step 3:** Remove `host_player_ipc` / `_ipc`. Wire binary to remote client.
- [ ] **Step 4:** `cargo test -p znicz-tui` and `cargo test -p znicz`.
- [ ] **Step 5: Commit** `Make the TUI a UI client of znicz player.`

---

### Task 4: MCP is an agent client

**Files:**
- Modify: `znicz-mcp/src/lib.rs` — `run_stdio` takes `IpcClient` (or `Box<dyn PlayerOps>`), no `PlayerHandle`.
- Modify: `znicz-mcp/src/server.rs` — store `IpcClient` (Clone via Arc/Mutex stream already). Remove local `spawn_player`, `local_used`, `persist_on_exit` session writes. Tests: start `IpcServer` + `connect(Agent)` instead of missing ipc path + local player.
- Modify: `znicz/src/main.rs` `run_mcp`: ensure_player, connect Agent, no restore/spawn.

- [ ] **Step 1:** Rewrite MCP tests that used `server.player` local state to use a test `IpcServer` + agent client (volume/queue assertions on the **host** handle).
- [ ] **Step 2:** `cargo test -p znicz-mcp` — fail until server is client-only.
- [ ] **Step 3:** Implement. Errors if ensure/connect fails (no fallback engine).
- [ ] **Step 4:** `cargo test -p znicz-mcp`.
- [ ] **Step 5: Commit** `Make MCP an agent client with no private player.`

---

### Task 5: Wiki, skill, spec status

**Files:**
- `wiki/Architecture/Overview.md` — diagram: TUI and MCP → `znicz player` → DAC
- `wiki/Architecture/MCP.md`, `TUI.md`, `Home.md`, `Domain/Formats-and-Metadata.md`, `Plans/Roadmap.md`, `Issues.md` (#27 closed as shared player 0.3.8)
- `wiki/Rust/Cargo-Workspace.md` if version text
- `znicz-mcp/skills/mcp-control/SKILL.md` — stop vs `znicz player stop`; `ZNICZ_IPC_PATH`
- `docs/superpowers/specs/2026-09-01-shared-player-design.md` Status: **Approved**
- `README.md` one line: shared player process

- [ ] **Step 1:** Edit wiki/skill/README/spec to match code (simple English, no TUI-host, no “MCP fallback engine”).
- [ ] **Step 2:** `cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings`
- [ ] **Step 3: Commit** `Document the shared player process and close #27 as this shape.`

---

## Spec coverage

| Spec | Task |
| --- | --- |
| One player process, autostart, `player` / `player stop` | 2 |
| Hello ui/agent, persistent TCP, UI count | 1 |
| Stopped + zero UI → idle_secs exit; playing stays; agents don’t block | 1 |
| Session only in player | 2–4 |
| TUI client, `q` leaves playback | 3 |
| MCP no private engine | 4 |
| Tests listed in spec | 1 (+ 4 host assertions) |
| Wiki | 5 |
| No LAN/systemd/DAC-gone/HLS/parked | — |

## Type names

- `ClientRole::{Ui, Agent}`
- `IpcClient::connect(path, role) -> Result<IpcClient>`
- `IpcClient::shutdown(&self) -> Result<()>`
- `IpcServer::start(player, advertise, idle: Duration) -> Result<IpcServer>`
- `IpcServer::wait(&mut self)`
- `ensure_player()` lives in `znicz/src/main.rs` (spawns this binary)
