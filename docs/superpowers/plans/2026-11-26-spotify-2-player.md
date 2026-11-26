# Spotify control, part 2: the player, IPC and CLI — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** The `znicz player` process hosts a `SpotifyService` built from a new `[spotify]` config section, exposes it over IPC, enforces the pause rule in both directions, and `znicz spotify login | logout | status` drive it from the command line, with the browser helpers the TUI will reuse later.

**Architecture:** `znicz-core` gets a `Spotify { op, id }` and a `SpotifyStatus` request, mirroring `Library`. Change requests get their own dedupe record, so a slow Spotify call cannot hold up `Command` or `Library` requests. `IpcClient` implements `SpotifyHost`. `znicz-spotify` gets a `browser` module (`OpenBrowser`, `should_open`, `open`). The binary parses `[spotify]`, builds the service in a new `spotify_pause` module that also holds the pause rule and the watcher, and adds a `spotify_cli` module with the three commands. Pure decision and formatting functions are unit-tested; the IO wrappers stay thin.

**Tech Stack:** Rust 2024 (MSRV 1.85: no `let` chains), `clap` derive, `serde`, `toml`, `serde_json` (tests), `color-eyre`, `tracing`; `tempfile` for tests. No new third-party crates.

**Spec:** `docs/superpowers/specs/2026-11-26-spotify-control-design.md`. **Part 1:** `docs/superpowers/plans/2026-11-26-spotify-1-crate.md` (done).

**Part map** (each part is its own plan, written when the previous one is done):

1. Core types plus the `znicz-spotify` crate (done).
2. This plan: player, IPC, the pause rule, `znicz spotify login|logout|status`, `[spotify]` config, browser helpers.
3. MCP tools, resource and skill.
4. TUI Settings → Spotify tab and the login panel.
5. TUI `StageModel` / `TransportModel` split and Spotify mode (`O`).

**Spec refinements made in this part:**
- `open_browser` is parsed by the binary and read only by the CLI (and later the TUI). It is not part of `SpotifyConfig`, so the player never sees it.
- The pause rule's glue lives in a new binary module, `znicz/src/spotify_pause.rs`, not in `daemon.rs`. `daemon.rs` is about lock files and autostart; keeping the watcher beside the rule keeps it testable on its own.
- The state request is `SpotifyStatus` (the spec's "SpotifyStatus query").
- A Spotify change request gets its own dedupe record (`recent_spotify`), not the `Command`/`Library` one. Reason: `apply_once` holds its lock for the whole `apply`, and a Spotify call can take about 40 s.
- The dedupe record holds its lock for the whole Spotify call (up to about 40 s). That is accepted: the service already serialises its playback and login calls on one client lock, so a second change request would wait there anyway, and a resend of the same id must wait for the first attempt rather than run twice. Reads and `SpotifyStatus` carry no id and never take the lock. The IPC client's read timeout is 30 s, so a call over 30 s makes the client reconnect and resend the same id; the resend waits on the lock and gets the stored answer.
- The watcher allows one Spotify pause call at a time (an `AtomicBool` slot), so a flapping engine state cannot pile up threads.
- `browser::open` reaps the opener on a small thread (`child.wait()`), so the long-lived TUI (Part 4) does not collect zombie processes.
- Pausing Znicz for Spotify sends `Command::Pause` only while the engine is `Playing`. On a Stopped engine, `Pause` sets the status to `Paused`, which would show a stopped player as paused.

**Conventions for every task:**
- Run commands from the repo root `/home/eugene/projects/znicz`.
- Commit messages end with a blank line and the line `Co-Authored-By: Claude Haiku 5.5 <noreply@anthropic.com>`.
- Never chain `git commit … && git push`. This plan does not push.
- Intermediate commits may warn about unused code until the task that wires it in lands. Task 12 runs clippy with `-D warnings`, and it must pass there.
- In `ipc.rs` and the binary, `IpcClient` implements both `LibraryHost` and `SpotifyHost`, and both have an `apply` method. Where both traits are in scope, call `apply` path-qualified (`SpotifyHost::apply(&client, op)`), or the call is ambiguous. Keep `SpotifyHost` out of scope in modules that call library `apply`.

---

## File structure

| File | Responsibility |
| --- | --- |
| `znicz-core/src/player/ipc.rs` | `Spotify` and `SpotifyStatus` requests, their responses, the server hooks, `IpcClient: SpotifyHost`, IPC tests; `RecentOutcomes` made generic and `run_once` shared by both dedupe records |
| `znicz-spotify/src/browser.rs` (new) | `OpenBrowser`, `should_open` (pure), `open` (spawns the platform opener) |
| `znicz-spotify/src/lib.rs` | `pub mod browser;` |
| `znicz-spotify/src/service.rs` | Doc comments on `ServiceOptions::on_start_playback` and `pause_if_playing` made accurate |
| `znicz/Cargo.toml` | Depends on `znicz-spotify` |
| `znicz/src/spotify_pause.rs` (new) | The pause rule: `started_playing` (pure), `pause_engine_if_playing`, `start_service`, `spawn_pause_watcher` |
| `znicz/src/main.rs` | `[spotify]` config section and mapping, `Spotify` subcommand, daemon wiring, config tests |
| `znicz/src/spotify_cli.rs` (new) | `SpotifyCmd`; pure login / status / message functions with tests; thin IO wrappers |
| `znicz/src/default-config.toml` | Commented `[spotify]` block |
| `wiki/Architecture/Spotify.md` | Player hosts the service; CLI and login (SSH, `ssh -L`); config; pause rule both ways |
| `wiki/Architecture/Overview.md` | IPC request in "Data that moves"; Spotify host beside the library host |
| `wiki/Architecture/MCP.md` | Checked; no change expected (it lists tools, not request kinds) |
| `THIRD-PARTY-LICENSES.html` | Regenerated, since the binary now links `znicz-spotify` |

---

### Task 1: Generic `RecentOutcomes` and a shared `run_once`

Refactor only. The Library and Command paths keep their behaviour and tests; the record becomes generic so Spotify can store a `SpotifyReply` in a second record.

**Files:**
- Modify: `znicz-core/src/player/ipc.rs` (the `Shared` struct, `impl Shared`, `RecentOutcomes`)

- [x] **Step 1: Confirm the current baseline passes**

Run: `cargo test -p znicz-core ipc -q`
Expected: PASS (the `recent_outcomes_keep_the_last_request_per_client_and_cap_the_clients` test and the dedupe tests are green).

- [x] **Step 2: Replace `RecentOutcomes` with a generic version and add `run_once`**

In `znicz-core/src/player/ipc.rs`, replace the whole block from `/// How many clients' last outcomes the player remembers.` through the end of `impl RecentOutcomes { … }` with:

```rust
/// How many clients' last outcomes the player remembers.
const RECENT_CLIENTS: usize = 64;

/// The last change request's outcome per client, oldest client first.
struct RecentOutcomes<T> {
    entries: std::collections::VecDeque<(RequestId, std::result::Result<T, String>)>,
}

impl<T> Default for RecentOutcomes<T> {
    fn default() -> Self {
        Self {
            entries: std::collections::VecDeque::new(),
        }
    }
}

impl<T: Clone> RecentOutcomes<T> {
    fn get(&self, id: RequestId) -> Option<std::result::Result<T, String>> {
        self.entries
            .iter()
            .find(|(seen, _)| *seen == id)
            .map(|(_, outcome)| outcome.clone())
    }

    fn record(&mut self, id: RequestId, outcome: std::result::Result<T, String>) {
        self.entries.retain(|(seen, _)| seen.client != id.client);
        self.entries.push_back((id, outcome));
        if self.entries.len() > RECENT_CLIENTS {
            self.entries.pop_front();
        }
    }
}

/// Run one change request once per id. A resend of the last id from the same
/// client gets the stored outcome instead of running `apply` again. A request
/// without an id always runs.
///
/// The lock is held while `apply` runs, so a resend that arrives while the
/// first attempt is still running waits for it rather than racing it.
fn run_once<T: Clone>(
    recent: &Mutex<RecentOutcomes<T>>,
    id: Option<RequestId>,
    apply: impl FnOnce() -> Result<T>,
) -> std::result::Result<T, String> {
    let Some(id) = id else {
        return apply().map_err(|e| e.to_string());
    };
    let mut recent = recent
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(outcome) = recent.get(id) {
        return outcome;
    }
    let outcome = apply().map_err(|e| e.to_string());
    recent.record(id, outcome.clone());
    outcome
}
```

- [x] **Step 3: Make `Shared` use the generic record and `run_once`**

In the `Shared` struct, replace the `recent` field line:

```rust
    recent: Mutex<RecentOutcomes>,
```

with:

```rust
    /// Command and Library change requests.
    recent: Mutex<RecentOutcomes<()>>,
```

Then replace the whole `impl Shared { … }` block's `apply_once` method (keep `ok`) so that `impl Shared` reads:

```rust
impl Shared {
    fn ok(&self) -> IpcResponse {
        IpcResponse::Ok {
            state: Box::new(self.player.state()),
            library: self.library.as_ref().map(|host| Box::new(host.status())),
            queue_revision: None,
            queue_unchanged: false,
        }
    }

    /// Run a Command or Library change request once per id (see [`run_once`]).
    fn apply_once(&self, id: Option<RequestId>, apply: impl FnOnce() -> Result<()>) -> IpcResponse {
        match run_once(&self.recent, id, apply) {
            Ok(()) => self.ok(),
            Err(message) => IpcResponse::Err { message },
        }
    }
}
```

- [x] **Step 4: Run the existing tests**

Run: `cargo test -p znicz-core ipc -q`
Expected: PASS. `recent_outcomes_keep_the_last_request_per_client_and_cap_the_clients` still compiles unchanged: `RecentOutcomes::default()` takes `T = ()` from `record(…, Ok(()))`.

- [x] **Step 5: Commit**

```bash
git add znicz-core/src/player/ipc.rs
git commit -m "refactor(core): make the IPC change-id record generic

Co-Authored-By: Claude Haiku 5.5 <noreply@anthropic.com>"
```

---

### Task 2: Spotify requests, replies and the server side

Tests first, then the wire types, the server dispatch and the constructor changes. The binary's one caller of `start_with_session_and_library` passes `None` until Task 6 wires the service in. The tests here talk to the server with raw requests (`rpc_raw`); Task 3 adds the `IpcClient` side and its tests.

**Files:**
- Modify: `znicz-core/src/player/ipc.rs`
- Modify: `znicz/src/main.rs` (one call site, `None` for the new argument)

- [x] **Step 1: Write the failing tests**

At the end of `mod tests` in `znicz-core/src/player/ipc.rs` (after `a_running_scan_holds_off_idle_exit`), add:

```rust
    use crate::spotify_host::SpotifyLogin;

    /// Records the ops it is given; refuses `Logout`.
    #[derive(Default)]
    struct FakeSpotify {
        ops: Mutex<Vec<SpotifyOp>>,
    }

    impl crate::spotify_host::SpotifyHost for FakeSpotify {
        fn apply(&self, op: SpotifyOp) -> Result<SpotifyReply> {
            if op == SpotifyOp::Logout {
                return Err(ZniczError::Spotify("refused".into()));
            }
            self.ops.lock().unwrap().push(op);
            Ok(SpotifyReply::Done)
        }

        fn state(&self) -> SpotifyState {
            SpotifyState {
                login: SpotifyLogin::LoggedIn,
                ..SpotifyState::default()
            }
        }
    }

    /// Takes 300 ms per call and counts the calls.
    #[derive(Default)]
    struct SlowSpotify {
        calls: AtomicUsize,
    }

    impl crate::spotify_host::SpotifyHost for SlowSpotify {
        fn apply(&self, _op: SpotifyOp) -> Result<SpotifyReply> {
            thread::sleep(Duration::from_millis(300));
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(SpotifyReply::Done)
        }

        fn state(&self) -> SpotifyState {
            SpotifyState::default()
        }
    }

    fn spotify_pause(token: &str, id: Option<RequestId>) -> IpcRequest {
        IpcRequest::Spotify {
            token: token.to_string(),
            op: SpotifyOp::Pause,
            id,
        }
    }

    fn is_done(response: &IpcResponse) -> bool {
        matches!(
            response,
            IpcResponse::Spotify {
                reply: SpotifyReply::Done
            }
        )
    }

    #[test]
    fn a_resent_spotify_change_is_applied_once() {
        let (_dir, path) = advertise_path();
        let (player, _thread) = spawn_player(AudioConfig::default());
        let host = Arc::new(FakeSpotify::default());
        let _server =
            IpcServer::start_with_spotify(player, &path, Duration::ZERO, host.clone())
                .expect("start ipc");
        let client = IpcClient::connect(&path, ClientRole::Agent).expect("connect");
        let token = load_advertise(&path).expect("advertise").token;
        let id = RequestId { client: 9, seq: 1 };

        for _ in 0..2 {
            let reply = client
                .rpc_raw(spotify_pause(&token, Some(id)))
                .expect("rpc");
            assert!(is_done(&reply), "{reply:?}");
        }

        assert_eq!(host.ops.lock().unwrap().len(), 1);
    }

    #[test]
    fn a_spotify_request_without_an_id_always_runs() {
        let (_dir, path) = advertise_path();
        let (player, _thread) = spawn_player(AudioConfig::default());
        let host = Arc::new(FakeSpotify::default());
        let _server =
            IpcServer::start_with_spotify(player, &path, Duration::ZERO, host.clone())
                .expect("start ipc");
        let client = IpcClient::connect(&path, ClientRole::Agent).expect("connect");
        let token = load_advertise(&path).expect("advertise").token;

        for _ in 0..2 {
            let reply = client.rpc_raw(spotify_pause(&token, None)).expect("rpc");
            assert!(is_done(&reply), "{reply:?}");
        }

        assert_eq!(host.ops.lock().unwrap().len(), 2);
    }

    #[test]
    fn a_resend_that_arrives_while_the_first_attempt_runs_waits_and_does_not_run_twice() {
        let (_dir, path) = advertise_path();
        let (player, _thread) = spawn_player(AudioConfig::default());
        let host = Arc::new(SlowSpotify::default());
        let _server =
            IpcServer::start_with_spotify(player, &path, Duration::ZERO, host.clone())
                .expect("start ipc");
        let first = IpcClient::connect(&path, ClientRole::Agent).expect("connect");
        let second = IpcClient::connect(&path, ClientRole::Agent).expect("connect");
        let token = load_advertise(&path).expect("advertise").token;
        let id = RequestId { client: 7, seq: 1 };

        let first_token = token.clone();
        let running = thread::spawn(move || first.rpc_raw(spotify_pause(&first_token, Some(id))));
        // The first attempt is inside the 300 ms call by now.
        thread::sleep(Duration::from_millis(100));
        let reply = second
            .rpc_raw(spotify_pause(&token, Some(id)))
            .expect("rpc");

        assert!(is_done(&reply), "{reply:?}");
        let first_reply = running.join().expect("thread").expect("rpc");
        assert!(is_done(&first_reply), "{first_reply:?}");
        assert_eq!(host.calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn the_spotify_snapshot_comes_back_and_a_missing_host_is_an_error() {
        let (_dir, path) = advertise_path();
        let (player, _thread) = spawn_player(AudioConfig::default());
        let host = Arc::new(FakeSpotify::default());
        let _server =
            IpcServer::start_with_spotify(player, &path, Duration::ZERO, host).expect("start ipc");
        let client = IpcClient::connect(&path, ClientRole::Agent).expect("connect");
        let token = load_advertise(&path).expect("advertise").token;
        match client.rpc_raw(IpcRequest::SpotifyStatus { token }) {
            Ok(IpcResponse::SpotifyStatus { state }) => {
                assert_eq!(state.login, SpotifyLogin::LoggedIn);
            }
            other => panic!("{other:?}"),
        }

        let (_dir2, path2) = advertise_path();
        let (player2, _thread2) = spawn_player(AudioConfig::default());
        let _bare = IpcServer::start(player2, &path2).expect("start ipc");
        let bare = IpcClient::connect(&path2, ClientRole::Agent).expect("connect");
        let token2 = load_advertise(&path2).expect("advertise").token;
        for request in [
            spotify_pause(&token2, None),
            IpcRequest::SpotifyStatus {
                token: token2.clone(),
            },
        ] {
            match bare.rpc_raw(request) {
                Ok(IpcResponse::Err { message }) => assert_eq!(message, NO_SPOTIFY),
                other => panic!("{other:?}"),
            }
        }
    }
```

- [x] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p znicz-core ipc -q`
Expected: FAIL to compile: `start_with_spotify`, `IpcRequest::Spotify`, `IpcRequest::SpotifyStatus`, `IpcResponse::Spotify`, `IpcResponse::SpotifyStatus` and `NO_SPOTIFY` do not exist yet.

- [x] **Step 3: Add the imports, the wire types and the server field**

At the top of `znicz-core/src/player/ipc.rs`, after `use crate::library_host::{LibraryHost, LibraryOp, LibraryStatus};`, add:

```rust
use crate::spotify_host::{SpotifyOp, SpotifyReply, SpotifyState};
```

Note: `SpotifyHost` is deliberately not imported here (see the conventions). The server holds `dyn crate::spotify_host::SpotifyHost`, and calling its methods on the trait object needs no import.

In `enum IpcRequest`, after the `Library { … },` variant, add:

```rust
    /// A Spotify op. Change ops (`!op.is_read()`) carry an id, as `Library` does.
    Spotify {
        token: String,
        op: SpotifyOp,
        #[serde(default)]
        id: Option<RequestId>,
    },
    /// The Spotify snapshot, for `SpotifyHost::try_state`.
    SpotifyStatus {
        token: String,
    },
```

In `enum IpcResponse`, before the `Err { message: String, },` variant, add:

```rust
    /// The answer to a `Spotify` request.
    Spotify {
        reply: SpotifyReply,
    },
    /// The answer to a `SpotifyStatus` request.
    SpotifyStatus {
        state: Box<SpotifyState>,
    },
```

Replace `struct Shared` (as Task 1 left it) with:

```rust
struct Shared {
    player: PlayerHandle,
    token: String,
    ui_count: AtomicUsize,
    stop: Arc<AtomicBool>,
    idle: Duration,
    advertise: PathBuf,
    persist: Mutex<Option<SessionPersister>>,
    library: Option<Arc<dyn LibraryHost>>,
    spotify: Option<Arc<dyn crate::spotify_host::SpotifyHost>>,
    /// Command and Library change requests.
    recent: Mutex<RecentOutcomes<()>>,
    /// Spotify change requests. Kept apart from `recent`: a Spotify call can
    /// take about 40 s, and it must not hold up Command or Library requests.
    recent_spotify: Mutex<RecentOutcomes<SpotifyReply>>,
}
```

- [x] **Step 4: Thread the host through the constructors**

Replace the constructor block from `pub fn start_with_idle` through `fn start_inner`'s body with the following. The public signatures are unchanged except `start_with_session_and_library`, which gains the `spotify` argument.

```rust
    pub fn start_with_idle(
        player: PlayerHandle,
        advertise: impl Into<PathBuf>,
        idle: Duration,
    ) -> Result<Self> {
        Self::start_inner(player, advertise, idle, None, None, None)
    }

    pub fn start_with_session(
        player: PlayerHandle,
        advertise: impl Into<PathBuf>,
        idle: Duration,
        session_path: impl Into<PathBuf>,
        debounce: Duration,
    ) -> Result<Self> {
        Self::start_with_session_and_library(
            player,
            advertise,
            idle,
            session_path,
            debounce,
            None,
            None,
        )
    }

    /// A server whose clients can also write the library through `host`.
    pub fn start_with_library(
        player: PlayerHandle,
        advertise: impl Into<PathBuf>,
        idle: Duration,
        host: Arc<dyn LibraryHost>,
    ) -> Result<Self> {
        Self::start_inner(player, advertise, idle, None, Some(host), None)
    }

    /// A server whose clients can control Spotify through `host` (tests).
    #[cfg(test)]
    pub fn start_with_spotify(
        player: PlayerHandle,
        advertise: impl Into<PathBuf>,
        idle: Duration,
        host: Arc<dyn crate::spotify_host::SpotifyHost>,
    ) -> Result<Self> {
        Self::start_inner(player, advertise, idle, None, None, Some(host))
    }

    /// The `znicz player` setup: session persistence, an optional library
    /// writer and an optional Spotify host.
    pub fn start_with_session_and_library(
        player: PlayerHandle,
        advertise: impl Into<PathBuf>,
        idle: Duration,
        session_path: impl Into<PathBuf>,
        debounce: Duration,
        host: Option<Arc<dyn LibraryHost>>,
        spotify: Option<Arc<dyn crate::spotify_host::SpotifyHost>>,
    ) -> Result<Self> {
        let mut persister = SessionPersister::new(session_path, debounce);
        persister.sync_from_player(&player);
        Self::start_inner(player, advertise, idle, Some(persister), host, spotify)
    }

    fn start_inner(
        player: PlayerHandle,
        advertise: impl Into<PathBuf>,
        idle: Duration,
        persist: Option<SessionPersister>,
        library: Option<Arc<dyn LibraryHost>>,
        spotify: Option<Arc<dyn crate::spotify_host::SpotifyHost>>,
    ) -> Result<Self> {
        let advertise = advertise.into();
        let listener = TcpListener::bind("127.0.0.1:0")
            .map_err(|e| ZniczError::Player(format!("ipc bind: {e}")))?;
        let addr = listener
            .local_addr()
            .map_err(|e| ZniczError::Player(format!("ipc bind: {e}")))?;
        let token = random_token();
        save_advertise(
            &advertise,
            &Advertise {
                port: addr.port(),
                token: token.clone(),
                version: Some(env!("CARGO_PKG_VERSION").to_string()),
            },
        )?;

        let stop = Arc::new(AtomicBool::new(false));
        let shared = Arc::new(Shared {
            player,
            token,
            ui_count: AtomicUsize::new(0),
            stop: stop.clone(),
            idle,
            advertise: advertise.clone(),
            persist: Mutex::new(persist),
            library,
            spotify,
            recent: Mutex::default(),
            recent_spotify: Mutex::default(),
        });
        let join = match thread::Builder::new()
            .name("znicz-ipc".into())
            .spawn(move || serve_loop(listener, shared))
        {
            Ok(join) => join,
            Err(e) => {
                let _ = fs::remove_file(&advertise);
                return Err(ZniczError::Player(format!("ipc thread: {e}")));
            }
        };

        Ok(Self {
            advertise,
            addr,
            stop,
            join: Some(join),
        })
    }
```

Then in `fn handle_session`, replace the `req_token` match:

```rust
        let req_token = match &req {
            IpcRequest::Hello { token, .. }
            | IpcRequest::State { token, .. }
            | IpcRequest::Command { token, .. }
            | IpcRequest::Shutdown { token }
            | IpcRequest::Library { token, .. } => token.clone(),
        };
```

with:

```rust
        let req_token = match &req {
            IpcRequest::Hello { token, .. }
            | IpcRequest::State { token, .. }
            | IpcRequest::Command { token, .. }
            | IpcRequest::Shutdown { token }
            | IpcRequest::Library { token, .. }
            | IpcRequest::Spotify { token, .. }
            | IpcRequest::SpotifyStatus { token } => token.clone(),
        };
```

And in the same function's `match req { … }`, after the `IpcRequest::Library { … } => { … }` arm and before `IpcRequest::Shutdown { .. } => {`, add:

```rust
            IpcRequest::Spotify { op, id, .. } => {
                let response = match &shared.spotify {
                    None => IpcResponse::Err {
                        message: NO_SPOTIFY.into(),
                    },
                    Some(host) => match run_once(&shared.recent_spotify, id, || host.apply(op)) {
                        Ok(reply) => IpcResponse::Spotify { reply },
                        Err(message) => IpcResponse::Err { message },
                    },
                };
                let _ = write_json_line(&mut writer, &response);
            }
            IpcRequest::SpotifyStatus { .. } => {
                let response = match &shared.spotify {
                    None => IpcResponse::Err {
                        message: NO_SPOTIFY.into(),
                    },
                    Some(host) => match host.try_state() {
                        Ok(state) => IpcResponse::SpotifyStatus {
                            state: Box::new(state),
                        },
                        Err(e) => IpcResponse::Err {
                            message: e.to_string(),
                        },
                    },
                };
                let _ = write_json_line(&mut writer, &response);
            }
```

Add this constant next to `RECENT_CLIENTS`:

```rust
/// What a player without a Spotify service answers Spotify requests with.
const NO_SPOTIFY: &str = "no Spotify service in this player";
```

- [x] **Step 5: Update the one non-test caller in the binary**

In `znicz/src/main.rs`, in `run_player_daemon`, change the `IpcServer::start_with_session_and_library(` call so its last argument list ends with `library,` followed by a new `None,`:

```rust
    let mut server = IpcServer::start_with_session_and_library(
        player.clone(),
        ipc,
        idle,
        session_path()?,
        SESSION_SAVE_DEBOUNCE,
        library,
        None,
    )
```

- [x] **Step 6: Run the tests to verify they pass**

Run: `cargo test -p znicz-core ipc -q`
Expected: PASS for the four new tests (`a_resent_spotify_change_is_applied_once`, `a_spotify_request_without_an_id_always_runs`, `a_resend_that_arrives_while_the_first_attempt_runs_waits_and_does_not_run_twice`, `the_spotify_snapshot_comes_back_and_a_missing_host_is_an_error`) and the existing IPC tests. `start_with_spotify` is `#[cfg(test)]`, so a non-test build does not warn about it.

- [x] **Step 7: Commit**

```bash
git add znicz-core/src/player/ipc.rs znicz/src/main.rs
git commit -m "feat(core): Spotify requests and replies over IPC

Co-Authored-By: Claude Haiku 5.5 <noreply@anthropic.com>"
```

---

### Task 3: `IpcClient` implements `SpotifyHost`

**Files:**
- Modify: `znicz-core/src/player/ipc.rs`

- [x] **Step 1: Write the failing tests**

At the end of `mod tests` in `znicz-core/src/player/ipc.rs` (after the Task 2 tests), add:

```rust
    #[test]
    fn spotify_ops_reach_the_host_and_errors_come_back_as_text() {
        let (_dir, path) = advertise_path();
        let (player, _thread) = spawn_player(AudioConfig::default());
        let host = Arc::new(FakeSpotify::default());
        let _server =
            IpcServer::start_with_spotify(player, &path, Duration::ZERO, host.clone())
                .expect("start ipc");
        let client = IpcClient::connect(&path, ClientRole::Agent).expect("connect");

        let reply =
            crate::spotify_host::SpotifyHost::apply(&client, SpotifyOp::Pause).expect("apply");
        assert_eq!(reply, SpotifyReply::Done);
        assert_eq!(host.ops.lock().unwrap().len(), 1);

        let err = crate::spotify_host::SpotifyHost::apply(&client, SpotifyOp::Logout)
            .expect_err("host refused");
        assert!(matches!(err, ZniczError::Spotify(_)), "{err:?}");
        assert_eq!(err.to_string(), "refused");
    }

    #[test]
    fn the_client_reads_the_snapshot_and_reports_a_player_without_spotify() {
        let (_dir, path) = advertise_path();
        let (player, _thread) = spawn_player(AudioConfig::default());
        let host = Arc::new(FakeSpotify::default());
        let _server =
            IpcServer::start_with_spotify(player, &path, Duration::ZERO, host).expect("start ipc");
        let client = IpcClient::connect(&path, ClientRole::Agent).expect("connect");
        let state = crate::spotify_host::SpotifyHost::try_state(&client).expect("state");
        assert_eq!(state.login, SpotifyLogin::LoggedIn);

        let (_dir2, path2) = advertise_path();
        let (player2, _thread2) = spawn_player(AudioConfig::default());
        let _bare = IpcServer::start(player2, &path2).expect("start ipc");
        let bare = IpcClient::connect(&path2, ClientRole::Agent).expect("connect");
        let err = crate::spotify_host::SpotifyHost::apply(&bare, SpotifyOp::Devices)
            .expect_err("no host");
        assert_eq!(err.to_string(), NO_SPOTIFY);
        assert!(crate::spotify_host::SpotifyHost::try_state(&bare).is_err());
        // `state` never fails: it falls back to the default.
        assert_eq!(
            crate::spotify_host::SpotifyHost::state(&bare),
            SpotifyState::default()
        );
    }
```

- [x] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p znicz-core ipc -q`
Expected: FAIL to compile: `SpotifyHost` is not implemented for `IpcClient`.

- [x] **Step 3: Add the client impl**

In `znicz-core/src/player/ipc.rs`, directly after the `impl LibraryHost for IpcClient { … }` block, add:

```rust
impl crate::spotify_host::SpotifyHost for IpcClient {
    fn apply(&self, op: SpotifyOp) -> Result<SpotifyReply> {
        // Reads carry no id; change ops do, so a resend is answered from memory.
        let id = (!op.is_read()).then(|| self.next_request_id());
        match self
            .rpc_detailed(|live| IpcRequest::Spotify {
                token: live.token.clone(),
                op: op.clone(),
                id,
            })
            .map_err(RpcFailure::into_error)?
        {
            IpcResponse::Spotify { reply } => Ok(reply),
            IpcResponse::Err { message } => Err(ZniczError::Spotify(message)),
            other => Err(ZniczError::Spotify(format!("unexpected reply: {other:?}"))),
        }
    }

    /// The player's snapshot; the default when it is unreachable or has no
    /// Spotify service. `try_state` says which.
    fn state(&self) -> SpotifyState {
        crate::spotify_host::SpotifyHost::try_state(self).unwrap_or_default()
    }

    fn try_state(&self) -> Result<SpotifyState> {
        match self
            .rpc_detailed(|live| IpcRequest::SpotifyStatus {
                token: live.token.clone(),
            })
            .map_err(RpcFailure::into_error)?
        {
            IpcResponse::SpotifyStatus { state } => Ok(*state),
            IpcResponse::Err { message } => Err(ZniczError::Spotify(message)),
            other => Err(ZniczError::Spotify(format!("unexpected reply: {other:?}"))),
        }
    }
}
```

The client's read timeout is 30 s and a Spotify call can take about 40 s. A change op that times out is reconnected and resent with the same id; the server answers the resend from its record once the first attempt ends (tested in Task 2).

- [x] **Step 4: Run the core tests**

Run: `cargo test -p znicz-core -q`
Expected: PASS, including the two new tests and the existing `library_ops_reach_the_host_and_status_comes_back` (the Library calls are unqualified and `SpotifyHost` is not in scope in this module, so they stay unambiguous).

- [x] **Step 5: Commit**

```bash
git add znicz-core/src/player/ipc.rs
git commit -m "feat(core): IpcClient implements SpotifyHost

Co-Authored-By: Claude Haiku 5.5 <noreply@anthropic.com>"
```

---

### Task 4: Browser helpers in `znicz-spotify`

**Files:**
- Create: `znicz-spotify/src/browser.rs`
- Modify: `znicz-spotify/src/lib.rs`

- [x] **Step 1: Write the failing tests**

Create `znicz-spotify/src/browser.rs` with only the tests for now:

```rust
//! Opening the login page in a browser, and whether to try. Front ends (the
//! CLI now, the TUI later) decide; the player never opens a browser.

#[cfg(test)]
mod tests {
    use super::*;

    /// A fake environment: the listed variables are set, every other one is unset.
    fn env_of(pairs: Vec<(&'static str, &'static str)>) -> impl Fn(&str) -> Option<String> {
        move |key: &str| {
            pairs
                .iter()
                .find(|(name, _)| *name == key)
                .map(|(_, value)| value.to_string())
        }
    }

    #[test]
    fn always_and_never_ignore_the_environment() {
        let remote = env_of(vec![("SSH_TTY", "/dev/pts/1"), ("DISPLAY", ":0")]);
        assert!(should_open(OpenBrowser::Always, &remote));
        assert!(!should_open(OpenBrowser::Never, &env_of(vec![("DISPLAY", ":0")])));
    }

    #[test]
    fn auto_never_opens_over_ssh() {
        for key in ["SSH_CONNECTION", "SSH_CLIENT", "SSH_TTY"] {
            let set = env_of(vec![(key, "1"), ("DISPLAY", ":0")]);
            assert!(!should_open(OpenBrowser::Auto, &set), "{key} set");
        }
    }

    #[test]
    fn an_empty_ssh_variable_counts_as_unset() {
        let empty = env_of(vec![("SSH_TTY", ""), ("DISPLAY", ":0")]);
        assert!(should_open(OpenBrowser::Auto, &empty));
    }

    #[test]
    fn auto_on_linux_needs_a_display() {
        if cfg!(target_os = "linux") {
            assert!(!should_open(OpenBrowser::Auto, &env_of(vec![])));
            assert!(should_open(OpenBrowser::Auto, &env_of(vec![("DISPLAY", ":0")])));
            assert!(should_open(
                OpenBrowser::Auto,
                &env_of(vec![("WAYLAND_DISPLAY", "wayland-0")])
            ));
        } else {
            assert!(should_open(OpenBrowser::Auto, &env_of(vec![])));
        }
    }

    #[test]
    fn the_setting_reads_its_config_spelling() {
        assert_eq!(OpenBrowser::parse("auto"), Some(OpenBrowser::Auto));
        assert_eq!(OpenBrowser::parse(" always "), Some(OpenBrowser::Always));
        assert_eq!(OpenBrowser::parse("never"), Some(OpenBrowser::Never));
        assert_eq!(OpenBrowser::parse("sometimes"), None);
        let from_serde: OpenBrowser = serde_json::from_str("\"never\"").expect("serde");
        assert_eq!(from_serde, OpenBrowser::Never);
        assert_eq!(OpenBrowser::default(), OpenBrowser::Auto);
    }

    #[cfg(unix)]
    #[test]
    fn the_opener_is_waited_for_so_it_leaves_no_zombie() {
        // The reaper thread ends once the child has exited and been waited on.
        let reaper = spawn_reaped(Command::new("true")).expect("spawn");
        reaper.join().expect("reaper thread");
    }

    #[cfg(unix)]
    #[test]
    fn a_missing_opener_is_an_error() {
        assert!(spawn_reaped(Command::new("/nonexistent/znicz-opener")).is_err());
    }
}
```

- [x] **Step 2: Run the tests to verify they fail**

Register the module first: in `znicz-spotify/src/lib.rs`, add `pub mod browser;` before `mod api;`:

```rust
pub mod browser;
mod api;
```

Run: `cargo test -p znicz-spotify browser -q`
Expected: FAIL to compile: `should_open`, `OpenBrowser`, `spawn_reaped` not found.

- [x] **Step 3: Write the implementation**

Insert above `#[cfg(test)]` in `znicz-spotify/src/browser.rs`:

```rust
use std::process::{Command, Stdio};

use serde::{Deserialize, Serialize};

/// The `open_browser` setting under `[spotify]`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OpenBrowser {
    #[default]
    Auto,
    Always,
    Never,
}

impl OpenBrowser {
    /// The config spelling (`auto`, `always`, `never`); `None` for anything else.
    pub fn parse(text: &str) -> Option<Self> {
        match text.trim() {
            "auto" => Some(Self::Auto),
            "always" => Some(Self::Always),
            "never" => Some(Self::Never),
            _ => None,
        }
    }
}

const REMOTE_VARS: [&str; 3] = ["SSH_CONNECTION", "SSH_CLIENT", "SSH_TTY"];

/// Whether to open the login page. `env` reads one environment variable; an
/// empty value counts as unset.
pub fn should_open(setting: OpenBrowser, env: &dyn Fn(&str) -> Option<String>) -> bool {
    let set = |key: &str| env(key).is_some_and(|value| !value.is_empty());
    match setting {
        OpenBrowser::Always => true,
        OpenBrowser::Never => false,
        OpenBrowser::Auto => {
            if REMOTE_VARS.iter().any(|key| set(key)) {
                return false;
            }
            if cfg!(target_os = "linux") {
                return set("DISPLAY") || set("WAYLAND_DISPLAY");
            }
            true
        }
    }
}

/// Start the platform's opener on `url` and return without waiting for it.
/// Its output goes to null. A small thread waits for the opener to exit, so a
/// long-lived caller (the TUI) does not collect zombie processes.
pub fn open(url: &str) -> std::io::Result<()> {
    spawn_reaped(opener(url)).map(|_| ())
}

/// Spawn `command` with its stdio on null, and reap it on a thread. The handle
/// finishes once the child has exited.
fn spawn_reaped(mut command: Command) -> std::io::Result<std::thread::JoinHandle<()>> {
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    std::thread::Builder::new()
        .name("znicz-browser-reap".into())
        .spawn(move || {
            let _ = child.wait();
        })
}

#[cfg(target_os = "windows")]
fn opener(url: &str) -> Command {
    use std::os::windows::process::CommandExt;
    let mut command = Command::new("cmd");
    // `&` in the URL would end the cmd line, and `args` would leave it unquoted,
    // so the line is passed raw with the URL in quotes.
    command.raw_arg(format!("/C start \"\" \"{url}\""));
    command
}

#[cfg(target_os = "macos")]
fn opener(url: &str) -> Command {
    let mut command = Command::new("open");
    command.arg(url);
    command
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
fn opener(url: &str) -> Command {
    let mut command = Command::new("xdg-open");
    command.arg(url);
    command
}
```

- [x] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p znicz-spotify browser -q`
Expected: PASS (7 tests on Unix, 5 elsewhere).

- [x] **Step 5: Commit**

```bash
git add znicz-spotify/src/browser.rs znicz-spotify/src/lib.rs
git commit -m "feat(spotify): browser helpers for the login page

Co-Authored-By: Claude Haiku 5.5 <noreply@anthropic.com>"
```

---

### Task 5: The pause rule, and accurate service docs

The rule's decision is a pure function, tested. The engine-side actions are small wrappers. The binary gains the `znicz-spotify` dependency here.

**Files:**
- Modify: `znicz/Cargo.toml`
- Create: `znicz/src/spotify_pause.rs`
- Modify: `znicz/src/main.rs` (`mod` line only)
- Modify: `znicz-spotify/src/service.rs` (two doc comments)

- [x] **Step 1: Add the dependency**

In `znicz/Cargo.toml`, after `znicz-mcp = { path = "../znicz-mcp" }`, add:

```toml
znicz-spotify = { path = "../znicz-spotify" }
```

- [x] **Step 2: Write the failing tests and the module skeleton**

Create `znicz/src/spotify_pause.rs`:

```rust
//! The pause rule between Znicz and Spotify, run by the player process.
//!
//! - Spotify starts (an op through Znicz that starts playback): the service's
//!   `on_start_playback` pauses the engine, if it plays.
//! - Znicz starts: a watcher sees the engine move into `Playing` and asks
//!   Spotify to pause, on a short-lived thread (the call can block for ~40 s).
//!
//! The two cannot loop. The first only moves the engine away from `Playing`,
//! and the watcher acts only on a move into `Playing`. Spotify's own pause
//! never goes through the first path, because `Pause` does not start playback.
//!
//! The watcher allows one pause call at a time, so an engine that flaps in and
//! out of `Playing` while a call is slow cannot pile up threads.

#[cfg(test)]
mod tests {
    use super::*;
    use znicz_core::PlaybackStatus::{Paused, Playing, Stopped};

    #[test]
    fn only_a_move_into_playing_counts_as_starting() {
        assert!(started_playing(Stopped, Playing));
        assert!(started_playing(Paused, Playing));
        assert!(!started_playing(Playing, Playing));
        assert!(!started_playing(Playing, Paused));
        assert!(!started_playing(Stopped, Paused));
        assert!(!started_playing(Paused, Stopped));
    }

    #[test]
    fn only_one_pause_call_runs_at_a_time() {
        let flag = Arc::new(AtomicBool::new(false));
        let first = PauseSlot::take(&flag).expect("free");
        assert!(PauseSlot::take(&flag).is_none(), "busy while the first is held");
        drop(first);
        assert!(PauseSlot::take(&flag).is_some(), "free again once it is dropped");
    }

    #[test]
    fn pausing_for_spotify_leaves_a_stopped_engine_stopped() {
        let (player, _thread) = znicz_core::spawn_player(znicz_core::AudioConfig::default());
        pause_engine_if_playing(&player);
        // A state-neutral command, sent after the pause would have been: its
        // answer means the engine has processed everything sent before it.
        let _ = player.send_blocking(znicz_core::Command::SetMuted(false));
        assert_eq!(player.state().status, Stopped);
    }
}
```

Register it: in `znicz/src/main.rs`, after `mod daemon;`, add `mod spotify_pause;` (a line each; keep the existing `mod update;` after them).

- [x] **Step 3: Run the tests to verify they fail**

Run: `cargo test -p znicz spotify_pause -q`
Expected: FAIL to compile: `started_playing`, `pause_engine_if_playing`, `PauseSlot` not found.

- [x] **Step 4: Write the implementation**

Insert above `#[cfg(test)]` in `znicz/src/spotify_pause.rs`:

```rust
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use znicz_core::{Command, PlaybackStatus, PlayerHandle, SpotifyConfig};
use znicz_spotify::{ServiceOptions, SpotifyService, default_token_path};

/// How often the watcher samples the engine.
const SAMPLE: Duration = Duration::from_millis(250);

/// Whether the engine has just started playing.
pub fn started_playing(prev: PlaybackStatus, now: PlaybackStatus) -> bool {
    prev != PlaybackStatus::Playing && now == PlaybackStatus::Playing
}

/// Pause the engine, but only while it plays. `Pause` on a Stopped engine
/// sets its status to `Paused`, which would misreport a stopped player.
///
/// Called from inside `SpotifyHost::apply` on the IPC thread. It takes no lock
/// the IPC server holds, and it only queues the command: it does not wait for
/// the engine. The Spotify Web call that follows is what takes the time.
pub fn pause_engine_if_playing(player: &PlayerHandle) {
    if player.state().status != PlaybackStatus::Playing {
        return;
    }
    if let Err(e) = player.send(Command::Pause) {
        tracing::warn!(error = %e, "could not pause Znicz for Spotify");
    }
}

/// Build the player's Spotify service. `None` only when there is no data
/// folder for the token; with no client ID the service still runs
/// (`NotConfigured`).
pub fn start_service(config: SpotifyConfig, player: &PlayerHandle) -> Option<Arc<SpotifyService>> {
    let Some(token_path) = default_token_path() else {
        tracing::warn!("no data folder for the Spotify token: Spotify is unavailable");
        return None;
    };
    // A clone of the player handle is cheap and owns nothing of the service,
    // so the service and the player do not keep each other alive.
    let engine = player.clone();
    let options = ServiceOptions {
        token_path,
        on_start_playback: Box::new(move || pause_engine_if_playing(&engine)),
    };
    Some(Arc::new(SpotifyService::start(config, options)))
}

/// The right to run one Spotify pause call. Held by the call's thread and
/// released when it ends, however it ends (including a failed thread spawn,
/// which drops the closure and with it the slot).
struct PauseSlot(Arc<AtomicBool>);

impl PauseSlot {
    /// `None` while another pause call holds the slot.
    fn take(flag: &Arc<AtomicBool>) -> Option<Self> {
        flag.compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .ok()
            .map(|_| Self(flag.clone()))
    }
}

impl Drop for PauseSlot {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

/// Watch the engine until `stop` is set. Each move into `Playing` asks
/// Spotify to pause, on its own thread, so this loop never blocks on Spotify.
/// A start that comes while an earlier call still runs is skipped: that call
/// reads Spotify's state itself, and flapping must not spawn a thread per flap.
pub fn spawn_pause_watcher(
    player: PlayerHandle,
    spotify: Arc<SpotifyService>,
    stop: Arc<AtomicBool>,
) -> std::io::Result<JoinHandle<()>> {
    thread::Builder::new()
        .name("znicz-spotify-pause".into())
        .spawn(move || {
            let in_flight = Arc::new(AtomicBool::new(false));
            let mut prev = player.state().status;
            while !stop.load(Ordering::SeqCst) {
                thread::sleep(SAMPLE);
                let now = player.state().status;
                if started_playing(prev, now) {
                    if let Some(slot) = PauseSlot::take(&in_flight) {
                        let spotify = spotify.clone();
                        let _ = thread::Builder::new()
                            .name("znicz-spotify-pause-call".into())
                            .spawn(move || {
                                let _slot = slot;
                                spotify.pause_if_playing();
                            });
                    }
                }
                prev = now;
            }
        })
}
```

Also add this test to the `tests` module (it documents that the watcher's check is a move, and the loop-safety claim):

```rust
    #[test]
    fn the_engine_pausing_for_spotify_is_not_a_start() {
        // (a) moves the engine from Playing to Paused; the watcher only reacts
        // to a move into Playing, so (a) cannot trigger (b).
        assert!(!started_playing(Playing, Paused));
    }
```

- [x] **Step 5: Run the tests to verify they pass**

Run: `cargo test -p znicz spotify_pause -q`
Expected: PASS (4 tests). The binary builds with the new dependency.

- [x] **Step 6: Tidy the two service doc comments**

In `znicz-spotify/src/service.rs`, replace:

```rust
    /// Called before an op that starts Spotify playback, so the player can
    /// pause its own engine.
    /// Runs on the caller's thread, inside `apply`. It must not take a lock the caller of `apply` holds (for example the player's), and should hold a `Weak` to the player rather than an `Arc`.
    pub on_start_playback: Box<dyn Fn() + Send + Sync>,
```

with:

```rust
    /// Called inside `SpotifyHost::apply`, on the caller's thread, before a
    /// playback op that will run (the service is logged in), so the player can
    /// pause its own engine.
    ///
    /// It must not wait for a lock the caller of `apply` holds: the IPC server
    /// holds one per change request. Capture a cheap handle to the player (a
    /// clone of its `PlayerHandle`), not an `Arc` that keeps the service and
    /// the player alive together.
    pub on_start_playback: Box<dyn Fn() + Send + Sync>,
```

And replace:

```rust
    /// Pause Spotify if it is playing: Znicz is about to play.
    /// Blocks for a whole Web API request (up to about 40 s: a refresh, a retry and a rate-limit wait), so never call it while holding the player's lock.
```

with:

```rust
    /// Pause Spotify if it is playing: Znicz is about to play.
    ///
    /// Blocks for a whole Web API request (up to about 40 s: a refresh, a retry
    /// and a rate-limit wait). Never call it while holding the player's lock;
    /// call it on its own thread.
```

- [x] **Step 7: Run the crate tests**

Run: `cargo test -p znicz-spotify -q`
Expected: PASS.

- [x] **Step 8: Commit**

```bash
git add znicz/Cargo.toml Cargo.lock znicz/src/spotify_pause.rs znicz/src/main.rs znicz-spotify/src/service.rs
git commit -m "feat(player): the pause rule between Znicz and Spotify

Co-Authored-By: Claude Haiku 5.5 <noreply@anthropic.com>"
```

---

### Task 6: The `[spotify]` config section and the player hosts the service

**Files:**
- Modify: `znicz/src/main.rs`

- [x] **Step 1: Write the failing config tests**

In `znicz/src/main.rs`, inside the existing `mod tests` at the end of the file, add:

```rust
    #[test]
    fn spotify_section_maps_to_the_core_config() {
        let config = parse_config_text(
            "[spotify]\nclient_id = \"0123456789abcdef0123456789abcdef\"\nredirect_port = 9001\ndevice = \"Kitchen\"\nopen_browser = \"never\"\n",
        )
        .expect("parses");
        assert_eq!(
            spotify_config(&config.spotify),
            znicz_core::SpotifyConfig {
                client_id: "0123456789abcdef0123456789abcdef".into(),
                redirect_port: 9001,
                device: "Kitchen".into(),
            }
        );
        assert_eq!(open_browser_setting(&config.spotify), OpenBrowser::Never);
    }

    #[test]
    fn a_bad_open_browser_falls_back_to_auto_without_losing_the_config() {
        let config = parse_config_text("[spotify]\nopen_browser = \"sometimes\"\n")
            .expect("parses");
        assert_eq!(open_browser_setting(&config.spotify), OpenBrowser::Auto);
        assert_eq!(spotify_config(&config.spotify), SpotifyConfig::default());
    }

    #[test]
    fn the_default_config_maps_to_an_unconfigured_service() {
        let config = parse_config_text(DEFAULT_CONFIG).expect("default config parses");
        assert_eq!(spotify_config(&config.spotify), SpotifyConfig::default());
        assert!(!spotify_config(&config.spotify).is_configured());
    }
```

(`OpenBrowser`, `SpotifyConfig`, `spotify_config`, `open_browser_setting` come in Step 2. `DEFAULT_CONFIG` already exists in `main.rs`.)

- [x] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p znicz spotify -q`
Expected: FAIL to compile: `spotify_config`, `open_browser_setting` not found.

- [x] **Step 3: Add the config section and the mapping**

In `znicz/src/main.rs`, add these imports next to the other `use` lines:

```rust
use znicz_core::SpotifyConfig;
use znicz_spotify::browser::OpenBrowser;
```

In `struct Config`, add a field after `covers: CoversSection,`:

```rust
    spotify: SpotifySection,
```

Add this section and these functions right after the `struct CoversSection { … }` definition:

```rust
/// `[spotify]`: the player's Spotify service, and how front ends open a browser.
#[derive(Debug, Deserialize, Default)]
struct SpotifySection {
    /// The Spotify app's client ID: 32 hex characters. Unset: not configured.
    client_id: Option<String>,
    /// The login listener's port on 127.0.0.1. Default 8898.
    redirect_port: Option<u16>,
    /// The device used when nothing is active. Unset: none.
    device: Option<String>,
    /// `auto`, `always` or `never`. Read by front ends only.
    open_browser: Option<String>,
}

/// The core config the player is given. `open_browser` is not part of it.
fn spotify_config(section: &SpotifySection) -> SpotifyConfig {
    SpotifyConfig {
        client_id: section
            .client_id
            .as_deref()
            .unwrap_or_default()
            .trim()
            .to_string(),
        redirect_port: section.redirect_port.unwrap_or(SpotifyConfig::DEFAULT_PORT),
        device: section.device.clone().unwrap_or_default(),
    }
}

/// The browser setting. An unknown value logs a warning and means `auto`; it
/// does not discard the rest of the config.
fn open_browser_setting(section: &SpotifySection) -> OpenBrowser {
    match section.open_browser.as_deref() {
        None => OpenBrowser::default(),
        Some(text) => OpenBrowser::parse(text).unwrap_or_else(|| {
            tracing::warn!(value = text, "[spotify] open_browser: use auto, always or never");
            OpenBrowser::default()
        }),
    }
}
```

- [x] **Step 4: Run the config tests to verify they pass**

Run: `cargo test -p znicz spotify -q`
Expected: PASS for the three config tests. (The default-config test already passes because the block is not yet in the default file; Task 9 adds it and re-runs this test.)

- [x] **Step 5: Host the service in the player**

The daemon uses the trait by path (`znicz_core::SpotifyHost`), not by import: `main.rs` also calls `LibraryHost::apply` on `IpcClient`, and an imported `SpotifyHost` would make those calls ambiguous.

Change the `Player` arm in `main` from:

```rust
            None => run_player_daemon(
                audio_config,
                config.player.idle_secs.unwrap_or(900),
                library_path.clone(),
            )?,
```

to:

```rust
            None => run_player_daemon(
                audio_config,
                config.player.idle_secs.unwrap_or(900),
                library_path.clone(),
                spotify_config(&config.spotify),
            )?,
```

Replace the whole `fn run_player_daemon` with:

```rust
fn run_player_daemon(
    audio_config: AudioConfig,
    idle_secs: u64,
    library_path: Option<PathBuf>,
    spotify: SpotifyConfig,
) -> color_eyre::Result<()> {
    let ipc = ipc_path()?;
    let lock_path = player_lock_path()?;
    clear_stale_player_files(&lock_path, &ipc);
    let Some(_lock) = acquire_player_lock(&lock_path, &ipc)? else {
        return Ok(());
    };
    if player_is_up(&ipc) {
        return Ok(());
    }

    let (player, _thread) = spawn_player(audio_config);
    if let Err(e) = restore_session(&player, &session_path()?, true) {
        tracing::warn!("session restore: {e}");
    }
    let idle = if idle_secs == 0 {
        std::time::Duration::ZERO
    } else {
        std::time::Duration::from_secs(idle_secs)
    };
    // A broken library must not stop playback; library writes then report it.
    let library: Option<Arc<dyn LibraryHost>> =
        library_path.and_then(|path| match znicz_library::LibraryService::open(&path) {
            Ok(service) => Some(Arc::new(service) as _),
            Err(e) => {
                tracing::warn!(path = %path.display(), error = %e, "library service unavailable");
                None
            }
        });
    // Always built: with no client ID it answers as `NotConfigured`.
    let spotify_service = spotify_pause::start_service(spotify, &player);
    let spotify_host: Option<Arc<dyn znicz_core::SpotifyHost>> = spotify_service
        .clone()
        .map(|service| -> Arc<dyn znicz_core::SpotifyHost> { service });
    let mut server = IpcServer::start_with_session_and_library(
        player.clone(),
        ipc,
        idle,
        session_path()?,
        SESSION_SAVE_DEBOUNCE,
        library,
        spotify_host,
    )
    .map_err(|e| color_eyre::eyre::eyre!("{e}"))?;
    if let Some(service) = spotify_service {
        if let Err(e) = spotify_pause::spawn_pause_watcher(
            player.clone(),
            service,
            server.stop_flag(),
        ) {
            tracing::warn!("Spotify pause watcher: {e}");
        }
    }
    install_player_signals(server.stop_flag());
    server.wait();
    if let Err(e) = save_session_from_player(&player, &session_path()?) {
        tracing::warn!("session.toml: {e}");
    }
    Ok(())
}
```

- [x] **Step 6: Run the binary's tests and a build**

Run: `cargo test -p znicz -q && cargo build -p znicz -q`
Expected: PASS and build. The `spotify_pause` functions are now used, so the dead-code warnings from Task 5 are gone. (`spotify_config` and `open_browser_setting` stay unused in non-test code until Task 8 uses `open_browser_setting`; `spotify_config` is used here.)

- [x] **Step 7: Commit**

```bash
git add znicz/src/main.rs
git commit -m "feat(player): the player hosts the Spotify service from [spotify]

Co-Authored-By: Claude Haiku 5.5 <noreply@anthropic.com>"
```

---

### Task 7: Login, logout and status: pure decisions and formatting

The CLI's decisions and text are pure functions and are tested here. Task 8 wires the IO.

**Files:**
- Create: `znicz/src/spotify_cli.rs`
- Modify: `znicz/src/main.rs` (`mod` line only)

- [x] **Step 1: Write the failing tests and the skeleton**

Create `znicz/src/spotify_cli.rs`:

```rust
//! `znicz spotify login | logout | status`. Each command autostarts the
//! player like the other CLI commands, sends the current `[spotify]` settings
//! first, then talks to the player over IPC. The decisions and the wording
//! are pure functions below; the IO wrappers are in the later task.

#[cfg(test)]
mod tests {
    use super::*;
    use znicz_core::{SpotifyAccount, SpotifyDevice, SpotifyLogin, SpotifyPlayback, SpotifyState, SpotifyTrack};

    fn track() -> SpotifyTrack {
        SpotifyTrack {
            title: "So What".into(),
            artists: vec!["Miles Davis".into()],
            duration_ms: 562_000,
            ..SpotifyTrack::default()
        }
    }

    fn kitchen() -> SpotifyDevice {
        SpotifyDevice {
            id: "d1".into(),
            name: "Kitchen speaker".into(),
            kind: "Speaker".into(),
            is_active: true,
            volume_percent: None,
        }
    }

    #[test]
    fn the_login_intro_depends_on_the_listener() {
        assert_eq!(
            login_intro(true),
            "Waiting for the browser; or paste the address you were sent to."
        );
        assert_eq!(login_intro(false), "Paste the address you were sent to.");
    }

    #[test]
    fn the_login_waits_while_logging_in() {
        let state = SpotifyState {
            login: SpotifyLogin::LoggingIn,
            ..SpotifyState::default()
        };
        assert_eq!(login_wait(&state, Duration::ZERO), LoginWait::Pending);
    }

    #[test]
    fn a_finished_login_names_the_user_once_known() {
        let known = SpotifyState {
            login: SpotifyLogin::LoggedIn,
            account: Some(SpotifyAccount {
                user: "Eugene".into(),
                premium: None,
            }),
            ..SpotifyState::default()
        };
        assert_eq!(
            login_wait(&known, Duration::ZERO),
            LoginWait::LoggedIn {
                user: Some("Eugene".into())
            }
        );

        // The account read follows the login; wait a little for it.
        let unknown_yet = SpotifyState {
            login: SpotifyLogin::LoggedIn,
            ..SpotifyState::default()
        };
        assert_eq!(login_wait(&unknown_yet, Duration::from_secs(1)), LoginWait::Pending);
        assert_eq!(
            login_wait(&unknown_yet, Duration::from_secs(5)),
            LoginWait::LoggedIn { user: None }
        );
        assert_eq!(logged_in_message(None), "Logged in to Spotify");
        assert_eq!(logged_in_message(Some("Eugene")), "Logged in as Eugene");
    }

    #[test]
    fn a_failed_or_cancelled_login_ends_with_a_reason() {
        let failed = SpotifyState {
            login: SpotifyLogin::Failed("login expired, press Enter to start again".into()),
            ..SpotifyState::default()
        };
        // The service's wording is for the TUI (Enter); the CLI says what to run.
        assert_eq!(
            login_wait(&failed, Duration::ZERO),
            LoginWait::Failed("login expired, run `znicz spotify login` again".into())
        );
        let cancelled = SpotifyState {
            login: SpotifyLogin::NotLoggedIn,
            ..SpotifyState::default()
        };
        assert_eq!(
            login_wait(&cancelled, Duration::ZERO),
            LoginWait::Failed("the login was cancelled".into())
        );
    }

    #[test]
    fn a_client_id_is_checked_before_anything_is_sent() {
        let empty = SpotifyConfig::default();
        let message = check_client_id(&empty).expect_err("not configured");
        assert!(message.contains("developer.spotify.com"), "{message}");
        assert!(message.contains("http://127.0.0.1:8898/callback"), "{message}");
        assert!(message.contains("config.toml"), "{message}");

        let typo = SpotifyConfig {
            client_id: "not-hex".into(),
            ..SpotifyConfig::default()
        };
        assert!(check_client_id(&typo).expect_err("bad id").contains("32 hex"));

        let good = SpotifyConfig {
            client_id: "0123456789abcdef0123456789abcdef".into(),
            ..SpotifyConfig::default()
        };
        assert!(check_client_id(&good).is_ok());
    }

    #[test]
    fn the_clock_reads_minutes_and_seconds() {
        assert_eq!(clock(0), "0:00");
        assert_eq!(clock(62_000), "1:02");
        assert_eq!(clock(562_000), "9:22");
        assert_eq!(clock(3_600_000), "60:00");
    }

    #[test]
    fn status_of_an_unconfigured_player_says_so() {
        let lines = status_lines(&SpotifyState::default(), 0);
        assert_eq!(
            lines,
            vec![
                "Login: not configured (set client_id under [spotify] in config.toml)".to_string(),
                "User: -".to_string(),
                "Premium: unknown (learned at the first play)".to_string(),
                "Device: none".to_string(),
                "Playing: nothing".to_string(),
            ]
        );
    }

    #[test]
    fn status_shows_the_account_device_and_track() {
        let state = SpotifyState {
            login: SpotifyLogin::LoggedIn,
            account: Some(SpotifyAccount {
                user: "Eugene".into(),
                premium: Some(true),
            }),
            playback: Some(SpotifyPlayback {
                item: Some(track()),
                is_playing: true,
                progress_ms: 62_000,
                read_at_ms: 1_000,
                device: Some(kitchen()),
                ..SpotifyPlayback::default()
            }),
            last_error: Some("needs Spotify Premium".into()),
            ..SpotifyState::default()
        };
        let lines = status_lines(&state, 1_000);
        assert_eq!(lines[0], "Login: logged in");
        assert_eq!(lines[1], "User: Eugene");
        assert_eq!(lines[2], "Premium: yes");
        assert_eq!(lines[3], "Device: Kitchen speaker (Speaker)");
        assert_eq!(lines[4], "Playing: So What — Miles Davis, playing 1:02 / 9:22");
        assert_eq!(lines[5], "Last error: needs Spotify Premium");
    }

    #[test]
    fn status_says_paused_and_not_premium() {
        let state = SpotifyState {
            login: SpotifyLogin::LoggedIn,
            account: Some(SpotifyAccount {
                user: "Eugene".into(),
                premium: Some(false),
            }),
            playback: Some(SpotifyPlayback {
                item: Some(track()),
                is_playing: false,
                progress_ms: 5_000,
                ..SpotifyPlayback::default()
            }),
            devices: vec![kitchen()],
            ..SpotifyState::default()
        };
        let lines = status_lines(&state, 99_999);
        assert_eq!(lines[2], "Premium: no");
        assert_eq!(lines[3], "Device: Kitchen speaker (Speaker)");
        assert_eq!(lines[4], "Playing: So What — Miles Davis, paused 0:05 / 9:22");
    }

    #[test]
    fn an_unsupported_item_is_named_as_such() {
        let state = SpotifyState {
            playback: Some(SpotifyPlayback {
                unsupported: true,
                ..SpotifyPlayback::default()
            }),
            ..SpotifyState::default()
        };
        assert_eq!(
            status_lines(&state, 0)[4],
            "Playing: an item Znicz does not show (podcast or ad)"
        );
    }
}
```

Register the module: in `znicz/src/main.rs`, after `mod spotify_pause;`, add `mod spotify_cli;`.

- [x] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p znicz spotify_cli -q`
Expected: FAIL to compile: `login_intro`, `login_wait`, `LoginWait`, `logged_in_message`, `check_client_id`, `clock`, `status_lines`, `SpotifyConfig`, `Duration` not found.

- [x] **Step 3: Write the pure functions**

Replace the top of `znicz/src/spotify_cli.rs` (everything above `#[cfg(test)]`) with:

```rust
//! `znicz spotify login | logout | status`. Each command autostarts the
//! player like the other CLI commands, sends the current `[spotify]` settings
//! first, then talks to the player over IPC. The decisions and the wording
//! are pure functions here; the IO wrappers are in the next task.

use std::time::Duration;

use znicz_core::{SpotifyConfig, SpotifyLogin, SpotifyState, valid_client_id};

/// How long the login waits for the account name after `LoggedIn`. The
/// account read runs after the login is recorded, so it can lag.
pub const ACCOUNT_WAIT: Duration = Duration::from_secs(5);

/// The line under the login URL.
pub fn login_intro(listening: bool) -> &'static str {
    if listening {
        "Waiting for the browser; or paste the address you were sent to."
    } else {
        "Paste the address you were sent to."
    }
}

/// What a login poll concludes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoginWait {
    Pending,
    LoggedIn { user: Option<String> },
    Failed(String),
}

/// The login's outcome from one state read. `account_wait` is how long the
/// login has been `LoggedIn` without an account name.
pub fn login_wait(state: &SpotifyState, account_wait: Duration) -> LoginWait {
    match &state.login {
        SpotifyLogin::LoggingIn => LoginWait::Pending,
        SpotifyLogin::LoggedIn => match &state.account {
            Some(account) => LoginWait::LoggedIn {
                user: Some(account.user.clone()),
            },
            None if account_wait < ACCOUNT_WAIT => LoginWait::Pending,
            None => LoginWait::LoggedIn { user: None },
        },
        SpotifyLogin::Failed(reason) => LoginWait::Failed(
            reason.replace("press Enter to start again", "run `znicz spotify login` again"),
        ),
        SpotifyLogin::NotLoggedIn | SpotifyLogin::NotConfigured => {
            LoginWait::Failed("the login was cancelled".into())
        }
    }
}

pub fn logged_in_message(user: Option<&str>) -> String {
    match user {
        Some(user) => format!("Logged in as {user}"),
        None => "Logged in to Spotify".into(),
    }
}

/// Why login cannot start, or `Ok` when the client ID is usable.
pub fn check_client_id(config: &SpotifyConfig) -> Result<(), String> {
    if !config.is_configured() {
        return Err(format!(
            "Spotify is not configured. Register a free app at https://developer.spotify.com/dashboard, \
             add the redirect URI {}, then set client_id under [spotify] in config.toml \
             (find the file with `znicz config path`).",
            config.redirect_uri()
        ));
    }
    if !valid_client_id(&config.client_id) {
        return Err(
            "the client_id under [spotify] must be 32 hex characters; copy it from the app's page in the Spotify dashboard"
                .into(),
        );
    }
    Ok(())
}

/// `m:ss`, with minutes past an hour left to grow.
pub fn clock(ms: u64) -> String {
    let secs = ms / 1000;
    format!("{}:{:02}", secs / 60, secs % 60)
}

/// The lines `znicz spotify status` prints. `now_ms` is Unix time, for the
/// playback position.
pub fn status_lines(state: &SpotifyState, now_ms: u64) -> Vec<String> {
    let user = state
        .account
        .as_ref()
        .map_or("-", |account| account.user.as_str());
    let premium = match state.account.as_ref().and_then(|account| account.premium) {
        Some(true) => "yes",
        Some(false) => "no",
        None => "unknown (learned at the first play)",
    };
    let mut lines = vec![
        format!("Login: {}", login_text(&state.login)),
        format!("User: {user}"),
        format!("Premium: {premium}"),
        format!("Device: {}", device_text(state)),
        format!("Playing: {}", playing_text(state, now_ms)),
    ];
    if let Some(error) = &state.last_error {
        lines.push(format!("Last error: {error}"));
    }
    lines
}

fn login_text(login: &SpotifyLogin) -> String {
    match login {
        SpotifyLogin::NotConfigured => {
            "not configured (set client_id under [spotify] in config.toml)".into()
        }
        SpotifyLogin::NotLoggedIn => "not logged in".into(),
        SpotifyLogin::LoggingIn => "logging in".into(),
        SpotifyLogin::LoggedIn => "logged in".into(),
        SpotifyLogin::Failed(reason) => format!("failed: {reason}"),
    }
}

fn device_text(state: &SpotifyState) -> String {
    let device = state
        .playback
        .as_ref()
        .and_then(|playback| playback.device.as_ref())
        .or_else(|| state.devices.iter().find(|device| device.is_active));
    match device {
        Some(device) => format!("{} ({})", device.name, device.kind),
        None => "none".into(),
    }
}

fn playing_text(state: &SpotifyState, now_ms: u64) -> String {
    let Some(playback) = &state.playback else {
        return "nothing".into();
    };
    let Some(track) = &playback.item else {
        return if playback.unsupported {
            "an item Znicz does not show (podcast or ad)".into()
        } else {
            "nothing".into()
        };
    };
    let status = if playback.is_playing { "playing" } else { "paused" };
    format!(
        "{} — {}, {} {} / {}",
        track.title,
        track.artist_line(),
        status,
        clock(playback.progress_at(now_ms)),
        clock(track.duration_ms)
    )
}
```

- [x] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p znicz spotify_cli -q`
Expected: PASS (10 tests). `SpotifyConfig` in the test module comes from `super::*`; `Duration` is imported at the top.

- [x] **Step 5: Commit**

```bash
git add znicz/src/spotify_cli.rs znicz/src/main.rs
git commit -m "feat(cli): pure login and status decisions for znicz spotify

Co-Authored-By: Claude Haiku 5.5 <noreply@anthropic.com>"
```

---

### Task 8: The `znicz spotify` command

The IO wrappers and the clap subcommand. Their decisions were tested in Task 7.

**Files:**
- Modify: `znicz/src/spotify_cli.rs`
- Modify: `znicz/src/main.rs`

- [x] **Step 1: Add the IO wrappers and the subcommand type**

In `znicz/src/spotify_cli.rs`, add these imports at the top, after the existing `use std::time::Duration;` line. Keep the Task 7 `use znicz_core::{SpotifyConfig, SpotifyLogin, SpotifyState, valid_client_id};` line as it is.

```rust
use std::io::BufRead;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use clap::Subcommand;
use color_eyre::eyre::{bail, eyre};
use znicz_core::{IpcClient, SpotifyHost, SpotifyOp, SpotifyReply};
use znicz_spotify::browser::{self, OpenBrowser};
```

`SpotifyHost` is imported here and nowhere else in this module, so `host.apply` and `host.try_state` are unambiguous.

Add the constant, the subcommand enum and the wrappers after `pub const ACCOUNT_WAIT: Duration = Duration::from_secs(5);`:

```rust
/// How often the login polls the player.
const LOGIN_POLL: Duration = Duration::from_millis(500);

#[derive(Debug, Subcommand)]
pub enum SpotifyCmd {
    /// Log in to Spotify: opens a browser, or paste the address you were sent to
    Login,
    /// Log out of Spotify and delete the saved token
    Logout,
    /// Show the login, account, device and what is playing
    Status,
}

/// Run one `znicz spotify` command against the player. `connect` autostarts
/// and connects to the player; `login` with no usable client ID fails before
/// it, so nothing is started or sent for a setting that cannot work.
pub fn run(
    command: SpotifyCmd,
    config: SpotifyConfig,
    open: OpenBrowser,
    connect: impl FnOnce() -> color_eyre::Result<IpcClient>,
) -> color_eyre::Result<()> {
    if matches!(command, SpotifyCmd::Login) {
        if let Err(message) = check_client_id(&config) {
            bail!("{message}");
        }
    }
    let host = connect()?;
    host.apply(SpotifyOp::Configure {
        config,
    })
    .map_err(|e| eyre!("{e}"))?;
    match command {
        SpotifyCmd::Login => login(&host, open),
        SpotifyCmd::Logout => {
            host.apply(SpotifyOp::Logout).map_err(|e| eyre!("{e}"))?;
            println!("Logged out of Spotify");
            Ok(())
        }
        SpotifyCmd::Status => {
            let state = host.try_state().map_err(|e| eyre!("{e}"))?;
            for line in status_lines(&state, now_ms()) {
                println!("{line}");
            }
            Ok(())
        }
    }
}

fn login(host: &IpcClient, open: OpenBrowser) -> color_eyre::Result<()> {
    let SpotifyReply::AuthorizeUrl { url, listening, .. } = host
        .apply(SpotifyOp::BeginLogin)
        .map_err(|e| eyre!("{e}"))?
    else {
        bail!("the player sent an unexpected reply to the login");
    };
    println!("Open this URL in any browser and approve:\n{url}");
    println!("{}", login_intro(listening));
    let env = |key: &str| std::env::var(key).ok();
    if browser::should_open(open, &env) {
        if let Err(e) = browser::open(&url) {
            println!("Could not open a browser: {e}");
        }
    }

    let paste = host.clone();
    std::thread::Builder::new()
        .name("znicz-spotify-paste".into())
        .spawn(move || paste_lines(&paste))?;

    let mut logged_in_since: Option<Instant> = None;
    loop {
        let state = host.try_state().map_err(|e| eyre!("{e}"))?;
        if state.login == SpotifyLogin::LoggedIn {
            logged_in_since.get_or_insert_with(Instant::now);
        }
        let waited = logged_in_since.map_or(Duration::ZERO, |since| since.elapsed());
        match login_wait(&state, waited) {
            LoginWait::Pending => std::thread::sleep(LOGIN_POLL),
            LoginWait::LoggedIn { user } => {
                println!("{}", logged_in_message(user.as_deref()));
                return Ok(());
            }
            LoginWait::Failed(reason) => bail!("{reason}"),
        }
    }
}

/// Each non-empty line on stdin is a pasted address. Errors are printed and
/// the wait goes on; a wrong address is the usual one.
fn paste_lines(host: &IpcClient) {
    for line in std::io::stdin().lock().lines() {
        let Ok(line) = line else { break };
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Err(e) = host.apply(SpotifyOp::CompleteLogin {
            redirect_url: line.to_string(),
        }) {
            println!("{e}");
        }
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}
```

`login` ends with exit 0 on `Logged in…`, and with exit 1 (an `Err` out of `main`) on a missing or bad client ID, a failed or cancelled login, or a player that stops answering. A wrong pasted address only prints the service's message and the wait goes on.

- [x] **Step 2: Wire the subcommand into the binary**

In `znicz/src/main.rs`:

1. Add the import `use spotify_cli::SpotifyCmd;` next to the others. The `SpotifyConfig` and `OpenBrowser` imports from Task 6 stay.
2. In `enum Commands`, after the `Station { command: StationCmd, },` variant, add:

```rust
    /// Log in to Spotify, log out, or show its status
    Spotify {
        #[command(subcommand)]
        command: SpotifyCmd,
    },
```

3. In `main`, add this arm to `match cli.command`, after the `Some(Commands::Playlist { … }) => match … { … },` arm and before `None => {`:

```rust
        Some(Commands::Spotify { command }) => {
            spotify_cli::run(
                command,
                spotify_config(&config.spotify),
                open_browser_setting(&config.spotify),
                || connect_agent(audio_config.device_id.as_deref(), config_file.as_deref()),
            )?;
        }
```

- [x] **Step 3: Build and run the tests**

Run: `cargo build -p znicz -q && cargo test -p znicz -q`
Expected: PASS. `znicz spotify --help` lists login, logout and status.

- [x] **Step 4: Check the command against the real binary (no client ID)**

Run: `ZNICZ_SPOTIFY_TOKEN_PATH=/tmp/znicz-check-token.json cargo run -q -p znicz -- --config /dev/null spotify status; echo "exit $?"`

Expected: prints `Login: not configured (set client_id under [spotify] in config.toml)` and the other four lines, then `exit 0`. (This uses a throwaway token path, and only if no player is already running: a running player keeps its own environment and config, so run `cargo run -q -p znicz -- player stop` first. Stop the autostarted one the same way afterwards.)

Run: `cargo run -q -p znicz -- --config /dev/null spotify login; echo "exit $?"`

Expected: an error that says Spotify is not configured and names `http://127.0.0.1:8898/callback`, then `exit 1`. The check runs before the player is contacted.

- [x] **Step 5: Commit**

```bash
git add znicz/src/spotify_cli.rs znicz/src/main.rs
git commit -m "feat(cli): znicz spotify login, logout and status

Co-Authored-By: Claude Haiku 5.5 <noreply@anthropic.com>"
```

---

### Task 9: Document the `[spotify]` block in the default config

**Files:**
- Modify: `znicz/src/default-config.toml`

- [x] **Step 1: Confirm the test passes and the block is absent**

Run: `cargo test -p znicz the_default_config_maps_to_an_unconfigured_service -q`
Expected: PASS (the default file has no `[spotify]` yet, so it parses to defaults).

- [x] **Step 2: Add the commented block**

In `znicz/src/default-config.toml`, insert this block directly before `[updates]`:

```toml
[spotify]
# Control the Spotify app from Znicz. Needs your own Spotify developer app: register
# a free one at https://developer.spotify.com/dashboard, add the redirect URI below,
# then set client_id. Playback control needs a Premium account. Log in: znicz spotify login.
# client_id = "0123456789abcdef0123456789abcdef"  # the app's client ID (32 hex characters)
# redirect_port = 8898       # login listener on 127.0.0.1; redirect URI http://127.0.0.1:8898/callback
# device = "Kitchen speaker" # preferred Spotify device, used when none is active; unset = none
# open_browser = "auto"      # auto | always | never: open the login page in a browser

```

- [x] **Step 3: Run the config tests**

Run: `cargo test -p znicz spotify -q`
Expected: PASS, and `the_default_config_maps_to_an_unconfigured_service` still passes with the block present (every key is commented out).

- [x] **Step 4: Commit**

```bash
git add znicz/src/default-config.toml
git commit -m "docs(config): the commented [spotify] block

Co-Authored-By: Claude Haiku 5.5 <noreply@anthropic.com>"
```

---

### Task 10: Wiki (wiki-sync rule)

Behaviour, config and the IPC request changed, so the wiki changes in this same PR.

**Files:**
- Modify: `wiki/Architecture/Spotify.md`
- Modify: `wiki/Architecture/Overview.md`
- Check: `wiki/Architecture/MCP.md`

- [x] **Step 1: Update the crate table in `Spotify.md`**

In `wiki/Architecture/Spotify.md`, in the table under `## The crate`, after the `| \`listener\` | …` row and before `| \`service\` | …`, the table stays as is; add one row after the `service` row:

```markdown
| `browser` | `OpenBrowser` (`auto` / `always` / `never`), the rule for when a front end opens a browser, and the platform opener |
```

- [x] **Step 2: Replace the opening section of `Spotify.md` and add the new sections**

In `wiki/Architecture/Spotify.md`, directly after the paragraph that starts `The shared types (\`SpotifyOp\`, …` and its closing sentence, add this new section before `## Login`:

```markdown
## Where it runs

The **player process** (`znicz player`) hosts the `SpotifyService`, the same way
it hosts the library service. It is always built from `[spotify]` and the token
file. With no client ID it still runs and reports `not configured`.

The TUI, MCP and `znicz spotify` reach it over IPC. Playback and browse ops are
`Spotify { op }`; the snapshot is `SpotifyStatus`. A change op (anything that is
not a read) carries a change id, so a resend after a lost reply is not applied
twice. A player without a Spotify service answers `no Spotify service in this
player`.

## Commands

```
znicz spotify login    # log in; prints the URL, opens a browser when it can
znicz spotify logout   # forget the token
znicz spotify status   # login, user, Premium, device, what is playing, last error
```

Each command first sends the current `[spotify]` settings to the player, so a
client ID added after the player started takes effect without a restart. The
command starts the player if it is not running.

`login` with no client ID prints how to get one, names the redirect URI to
register, and exits 1. `status` prints `Premium: unknown (learned at the first
play)` until a playback call has been accepted or refused.

### Logging in over SSH

The login finishes by whichever comes first:

1. **The redirect reaches the listener.** On the same machine, the browser
   redirects to `127.0.0.1:<port>` and the listener finishes the login. Over
   SSH, forward the port first, from your own machine:

   ```
   ssh -L 8898:127.0.0.1:8898 user@server
   ```

   Then run `znicz spotify login` on the server. Approve in the browser on your
   machine; the redirect goes through the tunnel.

2. **You paste the address.** Without a tunnel, approve the URL in any browser.
   The browser lands on `http://127.0.0.1:8898/callback?code=…` and shows "can't
   connect". That is expected. Copy the whole address from the address bar and
   paste it into the waiting `znicz spotify login` (on its stdin).

Over SSH (`SSH_CONNECTION`, `SSH_CLIENT` or `SSH_TTY` set), `login` does not
open a browser. It always prints the URL and the paste prompt, so a wrong guess
never blocks login. `open_browser` overrides the guess.

## Configuration

```toml
[spotify]
client_id = "0123456789abcdef0123456789abcdef"  # 32 hex characters
redirect_port = 8898       # http://127.0.0.1:8898/callback
device = "Kitchen speaker" # used when nothing is active; unset = none
open_browser = "auto"      # auto | always | never
```

The default config ships this block commented out. `open_browser` is read only
by front ends (the CLI, and the TUI later); the player never opens a browser. An
unknown `open_browser` value logs a warning and means `auto`.

Browser rule for `auto`: no browser when `SSH_CONNECTION`, `SSH_CLIENT` or
`SSH_TTY` is set; on Linux, a browser only if `DISPLAY` or `WAYLAND_DISPLAY` is
set; elsewhere, a browser.

## The pause rule

Znicz and Spotify never play together. The player enforces this, so the TUI,
CLI and MCP behave the same.

- **Spotify starts** (a `Play` or a `Transfer` with play, sent through Znicz):
  the player pauses its own engine first, if the engine is playing.
- **Znicz starts** (the engine moves into `Playing`, from any source): the
  player asks Spotify to pause, if Spotify is playing. The player samples the
  engine about every 250 ms and makes that call on its own thread, because it
  can take about 40 s.

The two directions cannot loop. Spotify starting only moves the engine out of
`Playing`, and the watcher acts only on a move *into* `Playing`. Spotify's own
pause does not go through the "Spotify starts" path.
```

- [x] **Step 3: Update `Overview.md`**

In `wiki/Architecture/Overview.md`, replace:

```markdown
`znicz-spotify` controls the Spotify app through the Web API. The player
process will host its `SpotifyService` the way it hosts the library service;
see [Spotify](Spotify.md).
```

with:

```markdown
`znicz-spotify` controls the Spotify app through the Web API. The player
process hosts its `SpotifyService` the way it hosts the library service: the
TUI, MCP, and `znicz spotify` reach it through `SpotifyHost` over IPC
(`Spotify { op }`, and `SpotifyStatus` for the snapshot). See
[Spotify](Spotify.md).
```

In the same file, replace `Change requests (\`Command\`, \`Library\`) carry an id` with `Change requests (\`Command\`, \`Library\`, \`Spotify\`) carry an id`. Spotify change requests have their own record, so a slow Spotify call does not hold up the others.

In the `## Data that moves` table, after the row `| TUI / MCP / CLI | Player process | scan, rescan, and prune jobs (\`LibraryOp\`) |`, add:

```markdown
| TUI / MCP / CLI | Player process | Spotify ops (`SpotifyOp`) and the Spotify snapshot (`SpotifyStatus`) |
```

- [x] **Step 4: Check `MCP.md`**

Run: `grep -n "Command\|Library {\|request" wiki/Architecture/MCP.md`

Expected: the IPC section lists no request kinds (only the `ipc.toml` advert and the autostart behaviour), so `MCP.md` needs no change in this part. If the grep shows a request-kind list, add `Spotify { op }` and `SpotifyStatus` to it in the same style.

- [x] **Step 5: Commit**

```bash
git add wiki/Architecture/Spotify.md wiki/Architecture/Overview.md
git commit -m "docs(wiki): the player hosts Spotify; CLI, login and the pause rule

Co-Authored-By: Claude Haiku 5.5 <noreply@anthropic.com>"
```

---

### Task 11: Regenerate the third-party licences

The binary now links `znicz-spotify`, so `THIRD-PARTY-LICENSES.html` changes.

**Files:**
- Modify: `THIRD-PARTY-LICENSES.html` (generated)

- [x] **Step 1: Check the tool**

Run: `cargo about --version`
Expected: a version line. If the command is missing, install it with `cargo install --locked cargo-about`, then run the next step.

- [x] **Step 2: Regenerate and check it changed only as expected**

Run: `sh scripts/third-party-licenses.sh && git diff --stat THIRD-PARTY-LICENSES.html`

Expected: the script exits 0 and the diff is limited to the licence entries of the crates `znicz-spotify` pulls in (the existing `ureq` stack is already there; new entries are the Spotify crate's own dependencies, if any are new). `Cargo.lock` is not changed by the script.

- [x] **Step 3: Commit**

```bash
git add THIRD-PARTY-LICENSES.html
git commit -m "chore(licences): regenerate THIRD-PARTY-LICENSES.html for znicz-spotify

Co-Authored-By: Claude Haiku 5.5 <noreply@anthropic.com>"
```

---

### Task 12: Full check

**Files:** none (verification; fix any finding in the file where it is).

- [x] **Step 1: Run the whole workspace**

Run: `cargo test --workspace -q`
Expected: PASS.

- [x] **Step 2: Clippy and formatting**

Run: `cargo clippy --workspace --all-targets -- -D warnings && cargo fmt --all --check`
Expected: both pass. If `fmt` reports differences, run `cargo fmt --all`, re-run the check, and commit the formatting as `style: cargo fmt` (its own commit).

- [x] **Step 3: Confirm the working tree is clean**

Run: `git status --short`
Expected: no output (everything is committed). If `Cargo.lock` or a generated file still shows, commit it with the task that changed it.

- [x] **Step 4: Commit any fixes**

If steps 1–3 needed changes:

```bash
git add <the changed files>
git commit -m "fix(spotify): <what the check found>

Co-Authored-By: Claude Haiku 5.5 <noreply@anthropic.com>"
```

If nothing needed changing, there is no commit for this task.

---

### Task 13: Manual check

Run these on a real machine with a real Spotify Premium account and a Development Mode app (client ID set under `[spotify]`, redirect URI `http://127.0.0.1:8898/callback` registered, the account added under User Management). No code is written in this task. Note any surprise and fix it in its own commit.

- [x] **Step 1: No client ID**

With `client_id` unset (or the `[spotify]` block commented out), run: `znicz spotify status`

Expected: the five status lines, login `not configured`, exit 0. (Stop any running player first, so it is restarted with the current config.)

Run: `znicz spotify login; echo "exit $?"`

Expected: the "Spotify is not configured" message naming `http://127.0.0.1:8898/callback`, then `exit 1`.

- [x] **Step 2: Login by paste over SSH**

On the server, over SSH, with a client ID set: `znicz spotify login`

Expected: the URL is printed and no browser opens (`SSH_CONNECTION` is set). The line under the URL is "Waiting for the browser; or paste the address you were sent to." when the callback listener is up, and "Paste the address you were sent to." when it is not. Open the URL in a browser on your own machine, approve, and copy the address the browser lands on (it shows "can't connect"). Paste it into the SSH session and press Enter.

Expected: `Logged in as <your name>`, exit 0. Then `znicz spotify status` shows `Login: logged in`, your name, and `Premium: yes` once a playback call has been accepted (`unknown (learned at the first play)` before that).

- [x] **Step 3: Login with `ssh -L`**

Log out first: `znicz spotify logout`. From your machine: `ssh -L 8898:127.0.0.1:8898 user@server`. In that session: `znicz spotify login`. Approve in the browser on your machine.

Expected: the redirect goes through the tunnel and the login finishes without any paste: `Logged in as <your name>`, exit 0.

- [x] **Step 4: Spotify pauses when Znicz starts**

Start Spotify playing on a device (phone or desktop app). Run `znicz` (the TUI, with any local track in the queue) and press play.

Expected: Spotify pauses within a second or two. Check with `znicz spotify status`: `Playing: … paused`.

- [ ] **Step 5: Znicz pauses when Spotify starts (not testable from the CLI yet)**

Part 2 has no CLI or TUI command that starts Spotify, so this direction cannot be checked by hand here. The engine side (`pause_engine_if_playing` on a Playing engine only) is covered by the Task 5 tests, and the service side by the Part 1 `play_pauses_znicz_first_and_calls_the_api` test. Re-check this step in Part 3, once MCP `spotify_play` exists: start a local track in Znicz, call `spotify_play` through MCP, and the local track must pause while Spotify plays.

- [x] **Step 6: Logout**

Run: `znicz spotify logout`

Expected: `Logged out of Spotify`. `znicz spotify status` then shows `Login: not logged in`, and the token file at the data folder path is gone.

- [x] **Step 7: Commit**

Nothing to commit unless a check found a bug. If one did, fix it, run Task 12, and commit with `fix(spotify): <what the manual check found>` and the Haiku trailer.

---

## Self-review against the spec and the brief

| Brief item | Where |
| --- | --- |
| 1. `[spotify]` keys, parsed with the other sections, mapped to `SpotifyConfig`; `open_browser` never to the player; commented block | Task 6 (parse and map), Task 9 (default block) |
| 2. Player always hosts `SpotifyService`, `NotConfigured` without a client ID | Task 6 (`start_service`, daemon wiring) |
| 3a. Spotify starts → pause the engine; no lock held; a cheap handle, not an `Arc` cycle; non-blocking send | Task 5 (`pause_engine_if_playing`, `start_service`), Task 5 service doc tidy |
| 3b. Znicz starts → Spotify pauses; watcher ~250 ms; short-lived thread; pure `started_playing` | Task 5 (`started_playing`, `spawn_pause_watcher`) |
| 3 loop-safety argument | Task 5 module doc, Task 10 wiki "The pause rule"; the watcher's one-call-at-a-time guard is `PauseSlot` |
| 4. `Spotify { op, id }` with token; state request; dedupe for non-reads; reads carry no id | Task 2 (wire types, server), Task 3 (client sends ids only for change ops) |
| 4. `IpcClient: SpotifyHost`: apply sends; state default when unreachable; try_state errors | Task 3 |
| 4. Player without a host answers with an error | Task 2 (`NO_SPOTIFY`), tested |
| 4. Errors cross IPC as text (`ZniczError::Spotify`); no typed codes | Task 2 and 3 (`ZniczError::Spotify(message)`) |
| 5. `OpenBrowser` (serde lowercase), `should_open` with injected env, `open` | Task 4 |
| 6. `znicz spotify login | logout | status`; Configure before each command | Task 8 (`run`), Task 7 (pure) |
| 6. login: no client ID → message and non-zero, before the player is contacted; BeginLogin; listening text; open browser; print failure; paste thread; poll 500 ms; LoggedIn/Failed exit codes | Task 7 (`check_client_id`, `login_intro`, `login_wait`, `logged_in_message`), Task 8 (`login`, `paste_lines`) |
| 6. logout message | Task 8 |
| 6. status fields and formatting as a pure function | Task 7 (`status_lines`) |
| 7. Wiki: Spotify.md (host, CLI, login incl. SSH and `ssh -L`, config, pause rule); Overview (table, host); MCP.md checked | Task 10 |
| 7. Regenerate `THIRD-PARTY-LICENSES.html` | Task 11 |
| 8. Out of scope: MCP, TUI, version bump, CHANGELOG | Not planned here (Parts 3–5 and the final PR) |
| Manual check: no client ID; paste over SSH; `ssh -L`; pausing both ways | Task 13 (the Spotify-starts direction is deferred to Part 3, see Step 5) |
| Commit message `docs(plan): Spotify part 2, player, IPC and CLI` | Written when this plan is committed |

**Review fixes (concurrency and cleanup):** the Spotify dedupe lock is held across the call on purpose and a test covers a resend arriving mid-call (Task 2); the watcher has a one-call-at-a-time guard, `PauseSlot` (Task 5); `browser::open` reaps its child (Task 4); the CLI checks the client ID before autostarting the player and rewords the TUI-only expiry text (Tasks 7, 8).

**Placeholder scan:** no `TBD`, no "handle edge cases" left open. Every code step has its code. Task 13 Step 5 is an explicit deferral with a reason, not a gap.

**Type consistency:**
- `IpcRequest::Spotify { token, op, id }`, `IpcRequest::SpotifyStatus { token }`, `IpcResponse::Spotify { reply }`, `IpcResponse::SpotifyStatus { state }` are defined in Task 2 and used in Tasks 2 (server arms) and 3 (client).
- `IpcServer::start_with_session_and_library(…, library, spotify)` is changed in Task 2, called with `None` in Task 2 and with the real host in Task 6.
- `spotify_pause::{started_playing, pause_engine_if_playing, start_service, spawn_pause_watcher}` are defined in Task 5 and used in Task 6.
- `spotify_cli::{SpotifyCmd, run, login_intro, login_wait, LoginWait, logged_in_message, check_client_id, clock, status_lines}` are defined in Tasks 7–8 and used in Task 8.
- `OpenBrowser::{default, parse}`, `browser::should_open`, `browser::open` are defined in Task 4 and used in Tasks 6 and 8.
- `spotify_config`, `open_browser_setting`, `SpotifySection` are defined in Task 6 and used in Tasks 6 (daemon/Player arm) and 8 (Spotify arm).

