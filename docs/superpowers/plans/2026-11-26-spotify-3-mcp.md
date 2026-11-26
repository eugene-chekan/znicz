# Spotify control, part 3: MCP tools, resource and skill — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** The MCP server exposes Spotify through `spotify_*` tools, a `znicz://spotify/now-playing` resource and a bundled `spotify` skill. Spotify errors carry a typed kind across IPC, and Spotify calls use their own player connection so they never block the player tools.

**Architecture:** `znicz-core` gets a `SpotifyErrorKind` and `ZniczError::Spotify { kind, message }`. The kind travels in an optional `spotify_kind` field on `IpcResponse::Err`, and the Spotify dedupe record keeps it, so a resent change still reports its kind. A timed-out or dropped Spotify request becomes a `network` error instead of the Library-flavoured outdated-player hint. `znicz mcp` opens a second `IpcClient` for Spotify. The tools live in `znicz-mcp/src/server.rs`, next to the other tools, and their arguments, pure helpers and JSON shaping live in a new `znicz-mcp/src/spotify_json.rs`. Blocking Spotify calls run on `spawn_blocking`, as library scans do. Pure functions are unit-tested; the IO wrappers stay thin.

**Tech Stack:** Rust 2024 (MSRV 1.85: no `let` chains), `rmcp` 0.3.2 (`#[tool_router]`, `#[tool]`, `Parameters<T>`, `McpError`), `schemars` 1.x, `serde`, `serde_json`, `tokio` (`spawn_blocking`), `thiserror`; `tempfile` for tests. No new third-party crates.

**Spec:** `docs/superpowers/specs/2026-11-26-spotify-control-design.md` (MCP section and "Notes for parts 3–5"). **Part 2:** `docs/superpowers/plans/2026-11-26-spotify-2-player.md` (done).

**Part map** (each part is its own plan, written when the previous one is done):

1. Core types plus the `znicz-spotify` crate (done).
2. Player, IPC, the pause rule, `znicz spotify login|logout|status`, `[spotify]` config, browser helpers (done).
3. This plan: typed Spotify errors over IPC, the second connection, MCP tools, resource, skill, wiki, manual check.
4. TUI Settings → Spotify tab and the login panel.
5. TUI `StageModel` / `TransportModel` split and Spotify mode (`O`).

**Spec refinements and findings made in this part:**
- **The brief's error mapping was incomplete.** It listed eight `SpotifyError` variants. The enum has eleven more (`NotConfigured`, `NotLoggedIn`, `LoginExpired`, `WrongAddress`, `PremiumRequired`, `Unavailable`, `NoActiveDevice`, `RateLimited`, `Network`). Each maps to the kind with the same meaning (Task 1).
- **Tool errors are `McpError`, not JSON bodies.** The existing tools return `McpError::internal_error(message, None)`, and no tool sets `data`. The Spotify tools keep that convention and put the kind in the error's `data` field: `data = {"kind": "premium_required"}`. Agents see the message and the kind. The brief's literal `{"error": ..., "kind": ...}` body would be the only tool error shaped that way, so it was not used.
- **The dedupe record cannot store a plain `String`.** `RecentOutcomes` stored `Result<T, String>`, so a resent change would lose its kind. The record is made generic over the error type, and Spotify stores a `SpotifyFailure` (Task 2).
- **`IpcServer::start_with_spotify` is `#[cfg(test)]` inside `znicz-core`.** The MCP crate's tests cannot call it. They use `start_with_session_and_library`, which is public and takes a `Spotify` host (Task 7).
- **The timeout is not in the error text.** `read_json_line` turns the read error into a string, so `RpcFailure` cannot see that it was a timeout. `rpc_on` now reports timeouts as their own variant (Task 3).
- **Tool names clash with the library tools.** `server.rs` already has `PlayParams`, `SeekParams`, `VolumeParams` and `QueueAddParams`. The Spotify argument types are prefixed `Spotify…Params`.
- **`spotify_status` sends `SpotifyOp::Touch` first.** Only an `apply` call marks Spotify use for fast polling (Spec: polling). A plain `try_state` does not, so the status tool touches before it reads (Task 7).
- **`read_resource` cannot be unit-tested here.** It needs an rmcp `RequestContext`, which tests cannot build. The resource is covered by the manual check (Task 15).
- **Timeouts for Command and Library requests keep the outdated-player hint.** Only Spotify requests change (the narrowest change). The hint is still wrong for a Command timeout; that is a follow-up, not part of this plan.
- **No artist top-tracks tool.** The brief's artist rule is documented in the skill and the wiki only.
- **Not in this part:** version bump, CHANGELOG, README (brief item 8; the final PR handles them).

**Conventions for every task:**
- Run commands from the repo root `/home/eugene/projects/znicz`.
- Commit messages end with a blank line and the line `Co-Authored-By: Claude Haiku 5.5 <noreply@anthropic.com>`.
- Never chain `git commit … && git push`. This plan does not push.
- Intermediate commits may warn about unused code until the task that wires it in lands. The last task runs clippy with `-D warnings`, and it must pass there.
- In `ipc.rs`, `IpcClient` implements both `LibraryHost` and `SpotifyHost`, and both have an `apply` method. Call `SpotifyHost::apply(&client, op)` path-qualified. In `server.rs`, `SpotifyHost` is imported, so every Spotify call there is path-qualified too.
- `rmcp` tool methods follow the existing pattern in `server.rs`: `#[tool(description = "…")]` on a method of the `#[tool_router] impl ZniczMcpServer` block, returning `Result<rmcp::model::CallToolResult, McpError>`, with arguments taken as `Parameters(params): Parameters<T>`.

---

## File structure

| File | Responsibility |
| --- | --- |
| `znicz-core/src/spotify_host.rs` | `SpotifyErrorKind` (serde snake_case enum) |
| `znicz-core/src/error.rs` | `ZniczError::Spotify { kind, message }`, Display is the message |
| `znicz-core/src/lib.rs` | Exports `SpotifyErrorKind` |
| `znicz-core/src/player/ipc.rs` | `spotify_kind` on `IpcResponse::Err`; generic dedupe record with `SpotifyFailure`; `spotify_failure` on the client; `RpcError` and timeouts; `RpcFailure::into_spotify_error`; IPC tests |
| `znicz-spotify/src/error.rs` | `SpotifyError::kind()`, `From<SpotifyError> for ZniczError` with the kind |
| `znicz/src/spotify_cli.rs` | `unavailable_message` and `host_error` match on the kind, not the text |
| `znicz/src/main.rs` | `run_mcp` opens a second agent connection and passes it to `run_stdio` |
| `znicz-mcp/src/lib.rs` | `mod spotify_json;`; `run_stdio` takes the Spotify client |
| `znicz-mcp/src/spotify_json.rs` (new) | Spotify tool argument types, pure op builders, status JSON, error mapping, tests |
| `znicz-mcp/src/server.rs` | `spotify` field and constructors; `spotify_apply` and status helpers; all `spotify_*` tools; the resource; the Spotify fake and tests |
| `znicz-mcp/src/skills.rs` | Bundled-skills test names `spotify` |
| `znicz-mcp/skills/spotify/SKILL.md` (new) | The bundled skill |
| `wiki/Architecture/MCP.md` | Spotify tools, second connection, error kinds, resource, skill |
| `wiki/Architecture/Spotify.md` | Typed errors |
| `docs/superpowers/plans/2026-11-26-spotify-2-player.md` | Task 13 Step 5 note points at this part |

---

### Task 1: The typed error kind in core and in `znicz-spotify`

Adds the kind and the struct variant. The IPC layer still sends only text, so it uses `spotify_failure(message, None)` for now. Task 2 sends the kind.

**Files:**
- Modify: `znicz-core/src/spotify_host.rs`
- Modify: `znicz-core/src/error.rs`
- Modify: `znicz-core/src/lib.rs`
- Modify: `znicz-spotify/src/error.rs`
- Modify: `znicz-core/src/player/ipc.rs` (the client's `SpotifyHost` impl, one helper, two tests)

- [x] **Step 1: Write the failing tests**

In `znicz-spotify/src/error.rs`, add to the `tests` module (keep the two existing tests):

```rust
    #[test]
    fn every_error_has_a_kind() {
        use znicz_core::SpotifyErrorKind as K;
        let cases = [
            (SpotifyError::NotConfigured, K::NotConfigured),
            (SpotifyError::NotLoggedIn, K::NotLoggedIn),
            (SpotifyError::NoLogin, K::WrongAddress),
            (SpotifyError::LoginExpired, K::LoginExpired),
            (SpotifyError::WrongAddress, K::WrongAddress),
            (SpotifyError::Declined("no".into()), K::Other),
            (
                SpotifyError::InvalidClient {
                    redirect_uri: "http://127.0.0.1:8898/callback".into(),
                },
                K::NotConfigured,
            ),
            (SpotifyError::Revoked, K::NotLoggedIn),
            (SpotifyError::Unauthorized, K::NotLoggedIn),
            (SpotifyError::PremiumRequired, K::PremiumRequired),
            (SpotifyError::Unavailable("x".into()), K::Unavailable),
            (SpotifyError::NoActiveDevice, K::NoActiveDevice),
            (
                SpotifyError::RateLimited {
                    retry_after_secs: 5,
                },
                K::RateLimited,
            ),
            (
                SpotifyError::Http {
                    status: 500,
                    message: "x".into(),
                },
                K::Other,
            ),
            (SpotifyError::Network("x".into()), K::Network),
            (SpotifyError::Parse("x".into()), K::Other),
            (SpotifyError::Io("x".into()), K::Other),
        ];
        for (error, kind) in cases {
            assert_eq!(error.kind(), kind, "{error}");
        }
    }

    #[test]
    fn conversion_keeps_the_kind_and_the_message() {
        let core: ZniczError = SpotifyError::NoActiveDevice.into();
        match core {
            ZniczError::Spotify { kind, message } => {
                assert_eq!(kind, znicz_core::SpotifyErrorKind::NoActiveDevice);
                assert_eq!(message, SpotifyError::NoActiveDevice.to_string());
            }
            other => panic!("{other:?}"),
        }
    }
```

In `znicz-core/src/spotify_host.rs`, add to its `tests` module:

```rust
    #[test]
    fn error_kinds_are_snake_case_on_the_wire() {
        let text = serde_json::to_string(&SpotifyErrorKind::PremiumRequired).unwrap();
        assert_eq!(text, r#""premium_required""#);
        assert_eq!(
            serde_json::from_str::<SpotifyErrorKind>(r#""no_active_device""#).unwrap(),
            SpotifyErrorKind::NoActiveDevice
        );
    }
```

- [x] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p znicz-spotify every_error_has_a_kind -q`
Expected: FAIL to compile with "no method named `kind`" and "cannot find type `SpotifyErrorKind`".

- [x] **Step 3: Add the kind enum**

In `znicz-core/src/spotify_host.rs`, add this block directly above `/// A request to the Spotify service.` (it sits after `valid_client_id`):

```rust
/// What went wrong with a Spotify request, so that agents and front ends can
/// act on it. The names are the `kind` in the MCP tool errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpotifyErrorKind {
    /// No client ID is set.
    NotConfigured,
    /// No login, or the login was removed.
    NotLoggedIn,
    /// The login was started too long ago, or it was refused.
    LoginExpired,
    /// The pasted address is not from the login in progress.
    WrongAddress,
    /// Playback needs Spotify Premium.
    PremiumRequired,
    /// Spotify has no active device to play on.
    NoActiveDevice,
    /// Spotify asked us to wait.
    RateLimited,
    /// Spotify does not offer this for the item (for example a playlist the user does not own).
    Unavailable,
    /// Spotify could not be reached, or the player did not answer.
    Network,
    /// The running player has no Spotify service.
    NoService,
    /// Anything else.
    Other,
}
```

- [x] **Step 4: Change the error variant**

Replace the whole of `znicz-core/src/error.rs` with:

```rust
use thiserror::Error;

use crate::spotify_host::SpotifyErrorKind;

#[derive(Debug, Error)]
pub enum ZniczError {
    #[error("audio error: {0}")]
    Audio(String),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("decode error: {0}")]
    Decode(String),

    #[error("player error: {0}")]
    Player(String),

    /// A library write was refused or failed; the message is shown as is.
    #[error("{0}")]
    Library(String),

    /// A Spotify request failed. `message` is shown as is; `kind` says what to do about it.
    #[error("{message}")]
    Spotify {
        kind: SpotifyErrorKind,
        message: String,
    },

    #[error("not implemented: {0}")]
    NotImplemented(String),
}

pub type Result<T> = std::result::Result<T, ZniczError>;
```

In `znicz-core/src/lib.rs`, change the `pub use spotify_host::{…}` list so it includes `SpotifyErrorKind` (after `SpotifyDevice,`):

```rust
pub use spotify_host::{
    SpotifyAccount, SpotifyConfig, SpotifyDevice, SpotifyErrorKind, SpotifyHost, SpotifyItem,
    SpotifyKind, SpotifyLogin, SpotifyOp, SpotifyPage, SpotifyPlayback, SpotifyRepeat,
    SpotifyReply, SpotifyState, SpotifyTrack, valid_client_id,
};
```

- [x] **Step 5: Map every `SpotifyError` to a kind**

In `znicz-spotify/src/error.rs`, change the `use` line at the top to:

```rust
use znicz_core::{SpotifyErrorKind, ZniczError};
```

Add this `impl` block directly under the `pub type Result<T> = …;` line, and replace the existing `impl From<SpotifyError> for ZniczError` with the version below it:

```rust
impl SpotifyError {
    /// The kind that crosses IPC and reaches the MCP tools.
    pub fn kind(&self) -> SpotifyErrorKind {
        use SpotifyErrorKind as K;
        match self {
            Self::NotConfigured | Self::InvalidClient { .. } => K::NotConfigured,
            Self::NotLoggedIn | Self::Revoked | Self::Unauthorized => K::NotLoggedIn,
            Self::NoLogin | Self::WrongAddress => K::WrongAddress,
            Self::LoginExpired => K::LoginExpired,
            Self::PremiumRequired => K::PremiumRequired,
            Self::NoActiveDevice => K::NoActiveDevice,
            Self::RateLimited { .. } => K::RateLimited,
            Self::Unavailable(_) => K::Unavailable,
            Self::Network(_) => K::Network,
            Self::Declined(_) | Self::Http { .. } | Self::Parse(_) | Self::Io(_) => K::Other,
        }
    }
}

impl From<SpotifyError> for ZniczError {
    fn from(error: SpotifyError) -> Self {
        ZniczError::Spotify {
            kind: error.kind(),
            message: error.to_string(),
        }
    }
}
```

- [x] **Step 6: Keep `ipc.rs` compiling with the new variant**

In `znicz-core/src/player/ipc.rs`, make these changes.

(a) Add `SpotifyErrorKind` to the import on the line `use crate::spotify_host::{SpotifyLogin, SpotifyOp, SpotifyReply, SpotifyState};`:

```rust
use crate::spotify_host::{SpotifyErrorKind, SpotifyLogin, SpotifyOp, SpotifyReply, SpotifyState};
```

(b) Directly below the `const UNEXPECTED_REPLY: &str = "unexpected reply from the player";` line, add:

```rust
/// A Spotify failure from the player. Without a kind (an older player sends
/// none), the kind is worked out from the text.
fn spotify_failure(message: String, kind: Option<SpotifyErrorKind>) -> ZniczError {
    let kind = kind.unwrap_or(if message == NO_SPOTIFY {
        SpotifyErrorKind::NoService
    } else {
        SpotifyErrorKind::Other
    });
    ZniczError::Spotify { kind, message }
}

fn unexpected_spotify() -> ZniczError {
    spotify_failure(UNEXPECTED_REPLY.into(), None)
}
```

(c) In `impl crate::spotify_host::SpotifyHost for IpcClient`, in `fn apply` and in `fn try_state`, replace the constructions:
- `IpcResponse::Err { message } => Err(ZniczError::Spotify(message)),` becomes `IpcResponse::Err { message, .. } => Err(spotify_failure(message, None)),`
- `_ => Err(ZniczError::Spotify(UNEXPECTED_REPLY.into())),` becomes `_ => Err(unexpected_spotify()),`

(d) In the test `spotify_ops_reach_the_host_and_errors_come_back_as_text`'s host, `FakeSpotify::apply` (inside the test module), replace `return Err(ZniczError::Spotify("refused".into()));` with:

```rust
                return Err(ZniczError::Spotify {
                    kind: SpotifyErrorKind::Other,
                    message: "refused".into(),
                });
```

(e) In the same test, replace `assert!(matches!(err, ZniczError::Spotify(_)), "{err:?}");` with `assert!(matches!(err, ZniczError::Spotify { .. }), "{err:?}");`.

Also the `IpcResponse::Err { message }` patterns in the Spotify impl are the only ones this task touches; the other `IpcResponse::Err` sites are untouched until Task 2.

- [x] **Step 7: Run the tests to verify they pass**

Run: `cargo test -p znicz-spotify -q && cargo test -p znicz-core -q`
Expected: PASS. The compiler may still report unused-import or dead-code warnings for `SpotifyErrorKind` in `ipc.rs` until Task 2 lands; that is fine for this commit.

- [x] **Step 8: Commit**

```bash
git add znicz-core/src/spotify_host.rs znicz-core/src/error.rs znicz-core/src/lib.rs znicz-core/src/player/ipc.rs znicz-spotify/src/error.rs
git commit -m "feat(spotify): typed Spotify errors in core and the Spotify crate

Co-Authored-By: Claude Haiku 5.5 <noreply@anthropic.com>"
```

---

### Task 2: The kind crosses IPC, and a resend keeps it

The server sends `spotify_kind` on error replies. The Spotify dedupe record stores the failure with its kind, so a resent change gets the same kind back.

**Files:**
- Modify: `znicz-core/src/player/ipc.rs`

- [x] **Step 1: Write the failing test**

In `znicz-core/src/player/ipc.rs`, add this to the test module (put it after `spotify_ops_reach_the_host_and_errors_come_back_as_text`):

```rust
    /// Refuses every op as a free account would, and counts the calls.
    #[derive(Default)]
    struct KindSpotify {
        calls: Mutex<usize>,
    }

    impl crate::spotify_host::SpotifyHost for KindSpotify {
        fn apply(&self, _op: SpotifyOp) -> Result<SpotifyReply> {
            *self.calls.lock().unwrap() += 1;
            Err(ZniczError::Spotify {
                kind: SpotifyErrorKind::PremiumRequired,
                message: "needs Spotify Premium".into(),
            })
        }

        fn state(&self) -> SpotifyState {
            SpotifyState::default()
        }
    }

    #[test]
    fn the_kind_survives_ipc_and_a_resend() {
        let (_dir, path) = advertise_path();
        let (player, _thread) = spawn_player(AudioConfig::default());
        let host = Arc::new(KindSpotify::default());
        let _server = IpcServer::start_with_spotify(player, &path, Duration::ZERO, host.clone())
            .expect("start ipc");
        let client = IpcClient::connect(&path, ClientRole::Agent).expect("connect");
        let token = load_advertise(&path).expect("advertise").token;
        let id = RequestId { client: 9, seq: 1 };
        let request = || IpcRequest::Spotify {
            token: token.clone(),
            op: SpotifyOp::Pause,
            id: Some(id),
        };

        for _ in 0..2 {
            match client.rpc_raw(request()).expect("reply") {
                IpcResponse::Err {
                    message,
                    spotify_kind,
                } => {
                    assert_eq!(message, "needs Spotify Premium");
                    assert_eq!(spotify_kind, Some(SpotifyErrorKind::PremiumRequired));
                }
                other => panic!("{other:?}"),
            }
        }
        assert_eq!(
            *host.calls.lock().unwrap(),
            1,
            "the resend is answered from memory"
        );
    }
```

Add a wire-compatibility test next to it. It pins the JSON, so an older client keeps parsing a newer player's `Err`, and a newer client keeps parsing an older player's:

```rust
    /// What an older client's `IpcResponse::Err` looked like: no `spotify_kind`.
    #[derive(Deserialize)]
    #[serde(tag = "kind", rename_all = "snake_case")]
    enum OldResponse {
        Err { message: String },
    }

    #[test]
    fn err_replies_stay_compatible_with_older_players_and_clients() {
        // A newer client reads an older player's reply: no kind.
        let old: IpcResponse = serde_json::from_str(r#"{"kind":"err","message":"boom"}"#).unwrap();
        assert!(matches!(
            old,
            IpcResponse::Err {
                spotify_kind: None,
                ..
            }
        ));

        // Without a kind nothing extra is written, so Command and Library
        // errors are byte-for-byte what they were.
        let plain = serde_json::to_string(&IpcResponse::Err {
            message: "boom".into(),
            spotify_kind: None,
        })
        .unwrap();
        assert_eq!(plain, r#"{"kind":"err","message":"boom"}"#);

        // An older client reads a newer player's Spotify error: the extra
        // field is ignored.
        let with_kind = serde_json::to_string(&IpcResponse::Err {
            message: "needs Spotify Premium".into(),
            spotify_kind: Some(SpotifyErrorKind::PremiumRequired),
        })
        .unwrap();
        assert!(with_kind.contains(r#""spotify_kind":"premium_required""#), "{with_kind}");
        let OldResponse::Err { message } = serde_json::from_str(&with_kind).unwrap();
        assert_eq!(message, "needs Spotify Premium");
    }
```

- [x] **Step 2: Run the test to verify it fails**

Run: `cargo test -p znicz-core the_kind_survives_ipc_and_a_resend -q`
Expected: FAIL to compile ("no field `spotify_kind`" on `IpcResponse::Err`).

- [x] **Step 3: Add the field to the reply**

In `ipc.rs`, replace the `Err` variant of `enum IpcResponse` with:

```rust
    Err {
        message: String,
        /// The Spotify error kind, set only for a failed Spotify request. Older
        /// players do not send it.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        spotify_kind: Option<SpotifyErrorKind>,
    },
```

Every constructor `IpcResponse::Err { message: … }` now needs `spotify_kind: None` (or the kind, where a Spotify failure is built), and every pattern `IpcResponse::Err { message }` needs `..` or `spotify_kind`. The compiler lists them: run `cargo build -p znicz-core` and fix each site. There are about 21 `IpcResponse::Err {` occurrences in this file. Do not change which error text is sent, only the struct shape.

- [x] **Step 4: Make the dedupe record generic over the stored error**

Replace the block from `/// The last change request's outcome per client, oldest client first.` through the end of `fn run_once` (the existing `run_once` ends with `    outcome\n}`) with:

```rust
/// The last change request's outcome per client, oldest client first. `E` is
/// what a failed request stores: the text for Command and Library, a
/// [`SpotifyFailure`] for Spotify, which keeps the kind.
struct RecentOutcomes<T, E = String> {
    entries: std::collections::VecDeque<(RequestId, std::result::Result<T, E>)>,
}

impl<T, E> Default for RecentOutcomes<T, E> {
    fn default() -> Self {
        Self {
            entries: std::collections::VecDeque::new(),
        }
    }
}

impl<T: Clone, E: Clone> RecentOutcomes<T, E> {
    fn get(&self, id: RequestId) -> Option<std::result::Result<T, E>> {
        self.entries
            .iter()
            .find(|(seen, _)| *seen == id)
            .map(|(_, outcome)| outcome.clone())
    }

    fn record(&mut self, id: RequestId, outcome: std::result::Result<T, E>) {
        self.entries.retain(|(seen, _)| seen.client != id.client);
        self.entries.push_back((id, outcome));
        if self.entries.len() > RECENT_CLIENTS {
            self.entries.pop_front();
        }
    }
}

/// Run one change request once per id. A resend of the last id from the same
/// client gets the stored outcome instead of running `apply` again. A request
/// without an id always runs. `stored` turns a failure into what is kept.
///
/// The lock is held while `apply` runs, so a resend that arrives while the
/// first attempt is still running waits for it rather than racing it.
fn run_once<T: Clone, E: Clone>(
    recent: &Mutex<RecentOutcomes<T, E>>,
    id: Option<RequestId>,
    apply: impl FnOnce() -> Result<T>,
    stored: impl Fn(&ZniczError) -> E,
) -> std::result::Result<T, E> {
    let Some(id) = id else {
        return apply().map_err(|e| stored(&e));
    };
    let mut recent = recent
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(outcome) = recent.get(id) {
        return outcome;
    }
    let outcome = apply().map_err(|e| stored(&e));
    recent.record(id, outcome.clone());
    outcome
}

/// A failed Spotify request as the player keeps it: its kind and its text.
#[derive(Debug, Clone)]
struct SpotifyFailure {
    kind: Option<SpotifyErrorKind>,
    message: String,
}

impl SpotifyFailure {
    fn of(error: &ZniczError) -> Self {
        match error {
            ZniczError::Spotify { kind, message } => Self {
                kind: Some(*kind),
                message: message.clone(),
            },
            other => Self {
                kind: None,
                message: other.to_string(),
            },
        }
    }

    fn response(self) -> IpcResponse {
        IpcResponse::Err {
            message: self.message,
            spotify_kind: self.kind,
        }
    }
}

/// The reply for a player that has no Spotify service.
fn no_spotify() -> IpcResponse {
    IpcResponse::Err {
        message: NO_SPOTIFY.into(),
        spotify_kind: Some(SpotifyErrorKind::NoService),
    }
}
```

Change the two fields in `struct Shared`:

```rust
    /// Command and Library change requests.
    recent: Mutex<RecentOutcomes<()>>,
    /// Spotify change requests. Kept apart from `recent`: a Spotify call can
    /// take about 40 s, and it must not hold up Command or Library requests.
    recent_spotify: Mutex<RecentOutcomes<SpotifyReply, SpotifyFailure>>,
```

Change `Shared::apply_once` to:

```rust
    /// Run a Command or Library change request once per id (see [`run_once`]).
    fn apply_once(&self, id: Option<RequestId>, apply: impl FnOnce() -> Result<()>) -> IpcResponse {
        match run_once(&self.recent, id, apply, |e| e.to_string()) {
            Ok(()) => self.ok(),
            Err(message) => IpcResponse::Err {
                message,
                spotify_kind: None,
            },
        }
    }
```

In the existing test `recent_outcomes_keep_the_last_request_per_client_and_cap_the_clients`, change the first line to:

```rust
        let mut recent: RecentOutcomes<()> = RecentOutcomes::default();
```

- [x] **Step 5: Send the kind from the server**

In `handle_client`'s request loop, replace the `IpcRequest::Spotify { op, id, .. }` arm and the `IpcRequest::SpotifyStatus { .. }` arm with:

```rust
            IpcRequest::Spotify { op, id, .. } => {
                let response = match &shared.spotify {
                    None => no_spotify(),
                    Some(host) => {
                        match run_once(&shared.recent_spotify, id, || host.apply(op), SpotifyFailure::of) {
                            Ok(reply) => IpcResponse::Spotify { reply },
                            Err(failure) => failure.response(),
                        }
                    }
                };
                let _ = write_json_line(&mut writer, &response);
            }
            IpcRequest::SpotifyStatus { .. } => {
                let response = match &shared.spotify {
                    None => no_spotify(),
                    Some(host) => match host.try_state() {
                        Ok(state) => IpcResponse::SpotifyStatus {
                            state: Box::new(state),
                        },
                        Err(e) => SpotifyFailure::of(&e).response(),
                    },
                };
                let _ = write_json_line(&mut writer, &response);
            }
```

Also in the `IpcRequest::Library` arm, the `None` case stays as it is, with `spotify_kind: None`.

- [x] **Step 6: Read the kind on the client**

In `impl crate::spotify_host::SpotifyHost for IpcClient`, replace the whole `fn apply` and `fn try_state` bodies' error arms so they read the kind. The final `fn apply` is:

```rust
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
            IpcResponse::Err {
                message,
                spotify_kind,
            } => Err(spotify_failure(message, spotify_kind)),
            _ => Err(unexpected_spotify()),
        }
    }
```

and `fn try_state` becomes:

```rust
    fn try_state(&self) -> Result<SpotifyState> {
        match self
            .rpc_detailed(|live| IpcRequest::SpotifyStatus {
                token: live.token.clone(),
            })
            .map_err(RpcFailure::into_error)?
        {
            IpcResponse::SpotifyStatus { state } => Ok(*state),
            IpcResponse::Err {
                message,
                spotify_kind,
            } => Err(spotify_failure(message, spotify_kind)),
            _ => Err(unexpected_spotify()),
        }
    }
```

(Task 3 changes the `.map_err(RpcFailure::into_error)` in these two functions.)

- [x] **Step 7: Run the tests to verify they pass**

Run: `cargo test -p znicz-core -q`
Expected: PASS, including `the_kind_survives_ipc_and_a_resend`, `err_replies_stay_compatible_with_older_players_and_clients` and the existing dedupe tests (Command and Library requests still use the text-only record, and their replies are unchanged on the wire).

- [x] **Step 8: Commit**

```bash
git add znicz-core/src/player/ipc.rs
git commit -m "feat(spotify): the error kind crosses IPC, and a resend keeps it

Co-Authored-By: Claude Haiku 5.5 <noreply@anthropic.com>"
```

---

### Task 3: A slow or dropped player is a network error, not the outdated-player hint

A Spotify call that runs past the 30 s read timeout is retried once after a reconnect. The second timeout currently comes back as the Library-flavoured hint. This task reports timeouts as their own case and gives Spotify requests a `network` error.

**Files:**
- Modify: `znicz-core/src/player/ipc.rs`

- [x] **Step 1: Write the failing tests**

Add to the `tests` module of `ipc.rs`:

```rust
    #[test]
    fn a_timeout_is_a_network_error_not_the_outdated_player_hint() {
        let failure = RpcFailure::sent(RpcError::TimedOut);
        match failure.into_spotify_error() {
            ZniczError::Spotify { kind, message } => {
                assert_eq!(kind, SpotifyErrorKind::Network);
                assert_eq!(message, "the player did not answer in time");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_dropped_connection_mentions_an_older_player() {
        let failure = RpcFailure::sent(RpcError::Other(ZniczError::Player("ipc empty".into())));
        let err = failure.into_spotify_error();
        assert!(
            matches!(
                err,
                ZniczError::Spotify {
                    kind: SpotifyErrorKind::Network,
                    ..
                }
            ),
            "{err:?}"
        );
        assert!(err.to_string().contains("older player"), "{err}");
    }

    #[test]
    fn a_command_timeout_still_gets_the_outdated_player_hint() {
        // Only Spotify requests change; Command and Library keep the hint.
        let text = command_result(Err(RpcFailure::sent(RpcError::TimedOut)))
            .unwrap_err()
            .to_string();
        assert!(text.contains("did not answer in time"), "{text}");
        assert!(text.contains("znicz player stop"), "{text}");
    }

    #[test]
    fn a_silent_player_makes_rpc_on_time_out() {
        // A real socket that never answers, with a short read timeout, so the
        // test does not wait 30 s and no network is involved.
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let stream = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (_silent, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_millis(50)))
            .unwrap();
        let mut live = Live {
            writer: stream.try_clone().unwrap(),
            reader: BufReader::new(stream),
            token: "t".into(),
            version: None,
            queue_cache: None,
        };
        let request = IpcRequest::Hello {
            token: "t".into(),
            role: ClientRole::Agent,
        };
        assert!(matches!(
            rpc_on(&mut live, &request),
            Err(RpcError::TimedOut)
        ));
    }

    #[test]
    fn timeouts_are_recognised_by_their_io_kind() {
        assert!(is_timeout(std::io::ErrorKind::WouldBlock));
        assert!(is_timeout(std::io::ErrorKind::TimedOut));
        assert!(!is_timeout(std::io::ErrorKind::ConnectionReset));
    }
```

- [x] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p znicz-core timeout -q`
Expected: FAIL to compile ("cannot find type `RpcError`", "no function `sent`").

The timeout wording of Command, Library and shutdown requests changes from `ipc read: <os error>` to `the player did not answer in time` (the outdated-player hint is still added to Command and Library). Nothing else about them changes.

- [x] **Step 3: Report timeouts from `rpc_on`**

In `ipc.rs`, replace the whole `fn rpc_on` (the one-line body that calls `write_json_line` and then `read_json_line`) with the following, and add the types directly above it:

```rust
/// Why a request on an open connection failed.
enum RpcError {
    /// No reply within the read timeout.
    TimedOut,
    Other(ZniczError),
}

impl From<ZniczError> for RpcError {
    fn from(error: ZniczError) -> Self {
        Self::Other(error)
    }
}

impl RpcError {
    /// As a plain error, for callers that do not retry or label timeouts.
    fn into_player_error(self) -> ZniczError {
        match self {
            Self::TimedOut => ZniczError::Player(TIMED_OUT.into()),
            Self::Other(error) => error,
        }
    }
}

/// The text of a request that got no reply in time.
const TIMED_OUT: &str = "the player did not answer in time";

/// A read timeout shows as `WouldBlock` on Unix and `TimedOut` on Windows.
fn is_timeout(kind: std::io::ErrorKind) -> bool {
    matches!(
        kind,
        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
    )
}

fn rpc_on(live: &mut Live, request: &IpcRequest) -> std::result::Result<IpcResponse, RpcError> {
    write_json_line(&mut live.writer, request)?;
    let mut line = String::new();
    match live.reader.read_line(&mut line) {
        Ok(_) => {}
        Err(e) if is_timeout(e.kind()) => return Err(RpcError::TimedOut),
        Err(e) => return Err(ZniczError::Player(format!("ipc read: {e}")).into()),
    }
    if line.is_empty() {
        return Err(ZniczError::Player("ipc empty".into()).into());
    }
    serde_json::from_str(line.trim())
        .map_err(|e| ZniczError::Player(format!("ipc json: {e}")).into())
}
```

`read_json_line` stays as it is for `open_stream`, which does not need the distinction.

- [x] **Step 4: Update the callers of `rpc_on`**

In `rpc_raw` (the `#[cfg(test)]` helper), change the final line to:

```rust
        rpc_on(&mut live, &request).map_err(RpcError::into_player_error)
```

In `shutdown`, change the match scrutinee to:

```rust
        match rpc_on(&mut live, &IpcRequest::Shutdown { token }).map_err(RpcError::into_player_error)? {
```

- [x] **Step 5: Carry the timeout through `RpcFailure`**

Replace `struct RpcFailure` and its `impl` (the block that has `fn connect`, `fn into_error`) with:

```rust
/// A failed rpc, and whether a connection was open when it failed.
struct RpcFailure {
    error: ZniczError,
    /// The request was sent (or tried) on an open connection. Errors from
    /// (re)opening the connection, such as "connection refused", are not.
    mid_request: bool,
    /// The reply did not come within the read timeout.
    timed_out: bool,
}

impl RpcFailure {
    fn connect(error: ZniczError) -> Self {
        Self {
            error,
            mid_request: false,
            timed_out: false,
        }
    }

    /// A request that was sent on an open connection and failed there.
    fn sent(error: RpcError) -> Self {
        match error {
            RpcError::TimedOut => Self {
                error: ZniczError::Player(TIMED_OUT.into()),
                mid_request: true,
                timed_out: true,
            },
            RpcError::Other(error) => Self {
                error,
                mid_request: true,
                timed_out: false,
            },
        }
    }

    /// The outdated-player hint only fits a request that died on an open
    /// connection; a player that is simply not running stays a plain error.
    /// Command and Library requests still use this.
    fn into_error(self) -> ZniczError {
        if self.mid_request {
            outdated_player_hint(self.error)
        } else {
            self.error
        }
    }

    /// A Spotify request that failed. A timeout or a dropped connection is a
    /// network error; the outdated-player hint is Library wording and is not used.
    fn into_spotify_error(self) -> ZniczError {
        let message = if self.timed_out {
            TIMED_OUT.to_string()
        } else if self.mid_request {
            "lost the connection to the player; an older player may not know Spotify requests (run `znicz player stop` and try again)".to_string()
        } else {
            self.error.to_string()
        };
        ZniczError::Spotify {
            kind: SpotifyErrorKind::Network,
            message,
        }
    }
}
```

The two struct literals in the existing test `command_transport_errors_get_the_hint_and_player_errors_stay_verbatim` (`RpcFailure { error: …, mid_request: … }`) need the new field: add `timed_out: false,` to both.

In `rpc_live`, change the retry's error mapping. Replace:

```rust
                rpc_on(&mut live, &request).map_err(|error| RpcFailure {
                    error,
                    mid_request: true,
                })?
```

with:

```rust
                rpc_on(&mut live, &request).map_err(RpcFailure::sent)?
```

- [x] **Step 6: Use the Spotify mapping in the Spotify impl**

In `impl crate::spotify_host::SpotifyHost for IpcClient`, in both `fn apply` and `fn try_state`, change `.map_err(RpcFailure::into_error)?` to `.map_err(RpcFailure::into_spotify_error)?`.

- [x] **Step 7: Run the tests to verify they pass**

Run: `cargo test -p znicz-core -q`
Expected: PASS. The existing `outdated_player_hint` tests still pass because `into_error` is unchanged.

Note: no test sends a whole request to a player that never answers through `IpcClient`, because that takes the 30 s read timeout twice. `a_silent_player_makes_rpc_on_time_out` covers the real socket timeout with a 50 ms read timeout, and the `RpcFailure` tests cover the mapping.

- [x] **Step 8: Commit**

```bash
git add znicz-core/src/player/ipc.rs
git commit -m "fix(spotify): a player that does not answer is a network error

Spotify requests no longer get the Library-flavoured outdated-player hint
when the reply times out or the connection drops.

Co-Authored-By: Claude Haiku 5.5 <noreply@anthropic.com>"
```

---

### Task 4: The CLI reads the kind

**Files:**
- Modify: `znicz/src/spotify_cli.rs`

- [x] **Step 1: Write the failing test**

In `znicz/src/spotify_cli.rs`, replace the test `a_player_without_a_spotify_service_gets_a_clear_line` with:

```rust
    #[test]
    fn a_player_without_a_spotify_service_gets_a_clear_line() {
        let no_service = ZniczError::Spotify {
            kind: SpotifyErrorKind::NoService,
            message: NO_SPOTIFY.into(),
        };
        assert_eq!(
            unavailable_message(&no_service),
            Some(
                "Spotify is unavailable: this player has no Spotify service (no data folder for the login token?)"
            )
        );
        let other = ZniczError::Spotify {
            kind: SpotifyErrorKind::Other,
            message: NO_SPOTIFY.into(),
        };
        assert_eq!(unavailable_message(&other), None);
        assert_eq!(
            unavailable_message(&ZniczError::Player("the player went away".into())),
            None
        );
    }
```

- [x] **Step 2: Run the test to verify it fails**

Run: `cargo test -p znicz spotify_cli -q`
Expected: FAIL to compile ("expected `&str`, found `&ZniczError`", and `SpotifyErrorKind` not found).

- [x] **Step 3: Switch the CLI to the kind**

In `znicz/src/spotify_cli.rs`, change the `use znicz_core::{…}` block (lines 13–16) to:

```rust
use znicz_core::{
    IpcClient, SpotifyConfig, SpotifyErrorKind, SpotifyHost, SpotifyLogin, SpotifyOp,
    SpotifyReply, SpotifyState, ZniczError, valid_client_id,
};
```

(`NO_SPOTIFY` leaves the non-test code, so the test module imports it itself; see Step 4.)

Replace `unavailable_message` and `host_error` with:

```rust
/// The line for a player that has no Spotify service, or `None` for any
/// other error.
pub fn unavailable_message(error: &ZniczError) -> Option<&'static str> {
    matches!(
        error,
        ZniczError::Spotify {
            kind: SpotifyErrorKind::NoService,
            ..
        }
    )
    .then_some(
        "Spotify is unavailable: this player has no Spotify service (no data folder for the login token?)",
    )
}

/// An IPC error as the command's error: the no-service case gets its own line.
fn host_error(error: ZniczError) -> color_eyre::Report {
    match unavailable_message(&error) {
        Some(line) => eyre!(line),
        None => eyre!("{error}"),
    }
}
```

In the `#[cfg(test)] mod tests` of this file, add `use znicz_core::NO_SPOTIFY;` at the top of the test module (below `use super::*;`).

- [x] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p znicz spotify_cli -q`
Expected: PASS. Then `cargo build -p znicz` must print no unused-import warning for `NO_SPOTIFY` in non-test code.

- [x] **Step 5: Commit**

```bash
git add znicz/src/spotify_cli.rs
git commit -m "refactor(spotify): the CLI reads the error kind, not the text

Co-Authored-By: Claude Haiku 5.5 <noreply@anthropic.com>"
```

---

### Task 5: A second player connection for Spotify in the MCP server

`IpcClient` serialises requests on one connection and clones share it. The MCP server gets its own `IpcClient` for Spotify, opened the same way as the player client.

**Files:**
- Modify: `znicz-mcp/src/server.rs` (struct, constructors, `build`, test helpers)
- Modify: `znicz-mcp/src/lib.rs` (`run_stdio`)
- Modify: `znicz/src/main.rs` (`run_mcp`)

- [x] **Step 1: Write the failing test**

In `znicz-mcp/src/server.rs`, add to the `tests` module (near `hosted_player`):

```rust
    /// A second, independent connection to the player at `dir`, as `znicz mcp`
    /// opens for Spotify calls.
    fn second_client(dir: &Path) -> IpcClient {
        IpcClient::connect(dir.join("ipc.toml"), ClientRole::Agent)
            .expect("connect the spotify agent")
    }

    #[test]
    fn the_server_takes_a_spotify_connection_of_its_own() {
        let dir = tempfile::tempdir().unwrap();
        let (_host, _ipc, client) = hosted_player(dir.path());
        let spotify = second_client(dir.path());
        let server = ZniczMcpServer::new(client, spotify, Vec::new());
        assert!(server.live_state().is_ok());
    }
```

- [x] **Step 2: Run the test to verify it fails**

Run: `cargo test -p znicz-mcp the_server_takes_a_spotify_connection_of_its_own -q`
Expected: FAIL to compile (`ZniczMcpServer::new` takes 2 arguments, not 3).

- [x] **Step 3: Add the field and the constructors**

In `znicz-mcp/src/server.rs`, replace the struct and the constructor block (from `#[derive(Clone)]` through the end of `fn build`) with:

```rust
#[derive(Clone)]
pub struct ZniczMcpServer {
    player: IpcClient,
    /// A second, independent connection for Spotify calls. A call can take
    /// about 40 s, and it must not hold up the player tools on `player`.
    spotify: IpcClient,
    skills: SkillRegistry,
    library: SharedLibrary,
    playlists_dir: PathBuf,
    stations_path: PathBuf,
    tool_router: ToolRouter<Self>,
}

impl ZniczMcpServer {
    /// Server without a music library. Library tools report that it is off.
    pub fn new(player: IpcClient, spotify: IpcClient, skills_dirs: Vec<PathBuf>) -> Self {
        Self::build(player, spotify, skills_dirs, None)
    }

    pub fn with_library(
        player: IpcClient,
        spotify: IpcClient,
        skills_dirs: Vec<PathBuf>,
        library: Library,
    ) -> Self {
        Self::build(
            player,
            spotify,
            skills_dirs,
            Some(Arc::new(Mutex::new(library))),
        )
    }

    fn build(
        player: IpcClient,
        spotify: IpcClient,
        skills_dirs: Vec<PathBuf>,
        library: SharedLibrary,
    ) -> Self {
        let bundled = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("skills");
        let mut dirs = vec![bundled];
        dirs.extend(skills_dirs);
        let skills = SkillRegistry::load(&dirs);

        Self {
            player,
            spotify,
            skills,
            library,
            playlists_dir: znicz_library::default_playlists_dir()
                .unwrap_or_else(|| temp_fallback("znicz-playlists")),
            stations_path: znicz_library::default_stations_path()
                .unwrap_or_else(|| temp_fallback("znicz-stations.toml")),
            tool_router: Self::tool_router(),
        }
    }
```

- [x] **Step 4: Update `run_stdio` and `run_mcp`**

In `znicz-mcp/src/lib.rs`, replace `run_stdio` with:

```rust
/// Serve MCP over stdio.
///
/// `spotify` is a second connection to the same player, used only by the
/// Spotify tools, so a slow Spotify call does not hold up the player tools.
/// `library` is optional: without it the player tools still work and the
/// library tools explain that no library is configured.
pub async fn run_stdio(
    player: IpcClient,
    spotify: IpcClient,
    skills_dirs: Vec<PathBuf>,
    library: Option<Library>,
) -> Result<(), Box<dyn std::error::Error>> {
    let (stdin, stdout) = rmcp::transport::stdio();
    let server = match library {
        Some(library) => ZniczMcpServer::with_library(player, spotify, skills_dirs, library),
        None => ZniczMcpServer::new(player, spotify, skills_dirs),
    };
    let service = server.serve((stdin, stdout)).await?;
    service.waiting().await?;
    Ok(())
}
```

In `znicz/src/main.rs`, in `fn run_mcp`, replace `let player = connect_agent(device, config)?;` with:

```rust
    let player = connect_agent(device, config)?;
    // A second connection: a Spotify call can take about 40 s, and it must not
    // hold up the player tools that use the first one.
    let spotify = connect_agent(device, config)?;
```

and change the `block_on` line to `.block_on(run_stdio(player, spotify, skills_dirs, library))`.

- [x] **Step 5: Update the existing test constructors**

In `znicz-mcp/src/server.rs`'s tests:

- `hosted_with_library`: change the server line to `let server = ZniczMcpServer::with_library(client, second_client(root.path()), Vec::new(), Library::open(&db).unwrap());`
- `server()`: replace the body with:

```rust
    fn server() -> Harness {
        let _lock = env_lock();
        let dir = tempfile::tempdir().unwrap();
        let (host, ipc, client) = hosted_player(dir.path());
        let spotify = second_client(dir.path());
        Harness {
            server: ZniczMcpServer::new(client, spotify, Vec::new()),
            host,
            _ipc: ipc,
            _dir: dir,
        }
    }
```

- `server_with_library()`: replace the body with:

```rust
    fn server_with_library() -> Harness {
        let library = Library::open_in_memory().expect("in-memory library");
        let _lock = env_lock();
        let dir = tempfile::tempdir().unwrap();
        let (host, ipc, client) = hosted_player(dir.path());
        let spotify = second_client(dir.path());
        Harness {
            server: ZniczMcpServer::with_library(client, spotify, Vec::new(), library),
            host,
            _ipc: ipc,
            _dir: dir,
        }
    }
```

- `station_server()`: change `let server = ZniczMcpServer::new(client, Vec::new());` to `let server = ZniczMcpServer::new(client, second_client(dir.path()), Vec::new());`

- [x] **Step 6: Run the tests to verify they pass**

Run: `cargo test -p znicz-mcp -q && cargo build -p znicz -q`
Expected: PASS. A dead-code warning for `spotify` may appear until Task 7 reads it; that is fine.

- [x] **Step 7: Commit**

```bash
git add znicz-mcp/src/server.rs znicz-mcp/src/lib.rs znicz/src/main.rs
git commit -m "feat(mcp): a second player connection for Spotify calls

Co-Authored-By: Claude Haiku 5.5 <noreply@anthropic.com>"
```

---

### Task 6: Spotify tool arguments and pure helpers

Everything here is pure: argument types, the op builders, the status JSON and the error mapping. The tools in Task 7 onward call these.

**Files:**
- Create: `znicz-mcp/src/spotify_json.rs`
- Modify: `znicz-mcp/src/lib.rs` (`mod spotify_json;`)

- [x] **Step 1: Write the module with its tests**

Create `znicz-mcp/src/spotify_json.rs` with exactly this content:

```rust
//! Spotify tool arguments, and the JSON the Spotify tools return. The tools in
//! `server.rs` send the ops built here. Everything in this file is pure.

use std::time::{SystemTime, UNIX_EPOCH};

use rmcp::ErrorData as McpError;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::json;
use znicz_core::{
    SpotifyKind, SpotifyLogin, SpotifyOp, SpotifyRepeat, SpotifyState, ZniczError,
};

/// Rows per page when `limit` is not given.
const PAGE_LIMIT: u32 = 20;
/// The largest page for playlists, albums and tracks.
const PAGE_MAX: u32 = 50;
/// Spotify returns at most this many search results per kind.
const SEARCH_MAX: u32 = 10;

/// A kind to search for.
#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SpotifyTypeArg {
    Track,
    Album,
    Artist,
    Playlist,
}

impl From<SpotifyTypeArg> for SpotifyKind {
    fn from(arg: SpotifyTypeArg) -> Self {
        match arg {
            SpotifyTypeArg::Track => Self::Track,
            SpotifyTypeArg::Album => Self::Album,
            SpotifyTypeArg::Artist => Self::Artist,
            SpotifyTypeArg::Playlist => Self::Playlist,
        }
    }
}

/// Repeat mode for `spotify_set_repeat`.
#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SpotifyRepeatArg {
    Off,
    Track,
    Context,
}

impl From<SpotifyRepeatArg> for SpotifyRepeat {
    fn from(arg: SpotifyRepeatArg) -> Self {
        match arg {
            SpotifyRepeatArg::Off => Self::Off,
            SpotifyRepeatArg::Track => Self::Track,
            SpotifyRepeatArg::Context => Self::Context,
        }
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SpotifySearchParams {
    /// Text to search for.
    pub query: String,
    /// Kinds to search: track, album, artist, playlist. Default: all four.
    pub types: Option<Vec<SpotifyTypeArg>>,
    /// Results per kind, 1 to 10 (default 10).
    pub limit: Option<u32>,
    /// Skip this many results per kind (default 0).
    pub offset: Option<u32>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SpotifyPagedParams {
    /// Skip this many rows (default 0).
    pub offset: Option<u32>,
    /// Rows per page, 1 to 50 (default 20).
    pub limit: Option<u32>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SpotifyUriPagedParams {
    /// Spotify URI, such as spotify:album:… or spotify:playlist:….
    pub uri: String,
    /// Skip this many rows (default 0).
    pub offset: Option<u32>,
    /// Rows per page, 1 to 50 (default 20).
    pub limit: Option<u32>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SpotifyLoginCompleteParams {
    /// The full address the browser landed on after the user approved the login,
    /// such as http://127.0.0.1:8898/callback?code=…&state=….
    pub redirect_url: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SpotifyPlayParams {
    /// A track URI to play, such as spotify:track:….
    pub uri: Option<String>,
    /// An album, playlist or artist URI to play from its start or from `offset`.
    pub context_uri: Option<String>,
    /// Position to start from, 0-based.
    pub offset: Option<u32>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SpotifySeekParams {
    /// Position in the track, in milliseconds.
    pub position_ms: u64,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SpotifyVolumeParams {
    /// Volume from 0.0 to 1.0.
    pub volume: f32,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SpotifyRepeatParams {
    /// off, track (repeat this track) or context (repeat the album or playlist).
    pub mode: SpotifyRepeatArg,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SpotifyShuffleParams {
    /// true to shuffle, false to play in order.
    pub on: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SpotifyQueueAddParams {
    /// Track URI to add to the Spotify queue, such as spotify:track:….
    pub uri: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SpotifyTransferParams {
    /// A device id from spotify_devices.
    pub device_id: String,
    /// Start playing on the device (default false).
    pub play: Option<bool>,
}

/// `(offset, limit)` for a paged browse call.
pub fn page(offset: Option<u32>, limit: Option<u32>) -> (u32, u32) {
    (
        offset.unwrap_or(0),
        limit.unwrap_or(PAGE_LIMIT).clamp(1, PAGE_MAX),
    )
}

/// Search every kind unless some are named; at most 10 results per kind.
pub fn search_op(params: SpotifySearchParams) -> SpotifyOp {
    let kinds = match params.types {
        Some(types) if !types.is_empty() => types.into_iter().map(SpotifyKind::from).collect(),
        _ => vec![
            SpotifyKind::Track,
            SpotifyKind::Album,
            SpotifyKind::Artist,
            SpotifyKind::Playlist,
        ],
    };
    SpotifyOp::Search {
        query: params.query,
        kinds,
        offset: params.offset.unwrap_or(0),
        limit: params.limit.unwrap_or(SEARCH_MAX).clamp(1, SEARCH_MAX),
    }
}

/// `uri` plays that track; `context_uri` plays an album or playlist; neither resumes.
pub fn play_op(params: SpotifyPlayParams) -> Result<SpotifyOp, McpError> {
    let (uris, context_uri) = match (params.uri, params.context_uri) {
        (Some(_), Some(_)) => {
            return Err(McpError::invalid_params(
                "give uri or context_uri, not both",
                None,
            ));
        }
        (Some(uri), None) => (vec![uri], None),
        (None, context) => (Vec::new(), context),
    };
    Ok(SpotifyOp::Play {
        context_uri,
        uris,
        offset: params.offset,
    })
}

/// A volume between 0.0 and 1.0, the same scale as the local volume.
pub fn volume(volume: f32) -> Result<f32, McpError> {
    if (0.0..=1.0).contains(&volume) {
        Ok(volume)
    } else {
        Err(McpError::invalid_params(
            "volume must be between 0.0 and 1.0",
            None,
        ))
    }
}

/// A Spotify failure as a tool error. The kind goes in the error's `data`, so
/// agents can branch on it; the message is the text the user would see.
pub fn error_of(error: ZniczError) -> McpError {
    match error {
        ZniczError::Spotify { kind, message } => {
            McpError::internal_error(message, Some(json!({ "kind": kind })))
        }
        other => McpError::internal_error(other.to_string(), None),
    }
}

pub fn unexpected_reply() -> McpError {
    McpError::internal_error("unexpected reply from the Spotify service", None)
}

pub fn login_label(login: &SpotifyLogin) -> &'static str {
    match login {
        SpotifyLogin::NotConfigured => "not_configured",
        SpotifyLogin::NotLoggedIn => "not_logged_in",
        SpotifyLogin::LoggingIn => "logging_in",
        SpotifyLogin::LoggedIn => "logged_in",
        SpotifyLogin::Failed(_) => "failed",
    }
}

/// Unix time in milliseconds, for the playback position.
pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// The `spotify_status` result. The position moves on from the last read
/// while playing, as the TUI shows it.
pub fn status_json(state: &SpotifyState, now_ms: u64) -> serde_json::Value {
    let device = state
        .playback
        .as_ref()
        .and_then(|p| p.device.as_ref())
        .or_else(|| state.devices.iter().find(|d| d.is_active))
        .map(|d| d.name.clone());
    let playing = state.playback.as_ref().and_then(|p| {
        let track = p.item.as_ref()?;
        Some(json!({
            "title": track.title,
            "artists": track.artists,
            "album": track.album,
            "is_playing": p.is_playing,
            "progress_ms": p.progress_at(now_ms),
            "duration_ms": track.duration_ms,
        }))
    });
    json!({
        "configured": state.login != SpotifyLogin::NotConfigured,
        "login": login_label(&state.login),
        "user": state.account.as_ref().map(|a| a.user.clone()),
        "premium": state.account.as_ref().and_then(|a| a.premium),
        "device": device,
        "playing": playing,
        "last_error": state.last_error,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use znicz_core::{
        SpotifyAccount, SpotifyDevice, SpotifyErrorKind, SpotifyPlayback, SpotifyTrack,
    };

    #[test]
    fn status_reports_the_login_the_account_and_the_moving_position() {
        let state = SpotifyState {
            login: SpotifyLogin::LoggedIn,
            account: Some(SpotifyAccount {
                user: "eve".into(),
                premium: Some(false),
            }),
            playback: Some(SpotifyPlayback {
                item: Some(SpotifyTrack {
                    title: "So What".into(),
                    artists: vec!["Miles Davis".into()],
                    album: "Kind of Blue".into(),
                    duration_ms: 10_000,
                    ..SpotifyTrack::default()
                }),
                is_playing: true,
                progress_ms: 4_000,
                read_at_ms: 1_000,
                device: Some(SpotifyDevice {
                    id: "d".into(),
                    name: "Desk".into(),
                    kind: "Computer".into(),
                    is_active: true,
                    volume_percent: None,
                }),
                ..SpotifyPlayback::default()
            }),
            ..SpotifyState::default()
        };
        let json = status_json(&state, 3_500);
        assert_eq!(json["configured"], true);
        assert_eq!(json["login"], "logged_in");
        assert_eq!(json["user"], "eve");
        assert_eq!(json["premium"], false);
        assert_eq!(json["device"], "Desk");
        assert_eq!(json["playing"]["title"], "So What");
        assert_eq!(json["playing"]["progress_ms"], 6_500);
        assert!(json["last_error"].is_null());
    }

    #[test]
    fn status_of_a_fresh_player_says_not_configured_with_nulls() {
        let json = status_json(&SpotifyState::default(), 0);
        assert_eq!(json["configured"], false);
        assert_eq!(json["login"], "not_configured");
        assert!(json["premium"].is_null());
        assert!(json["playing"].is_null());
    }

    #[test]
    fn a_spotify_error_carries_its_kind_in_the_tool_error_data() {
        let error = error_of(ZniczError::Spotify {
            kind: SpotifyErrorKind::PremiumRequired,
            message: "needs Spotify Premium".into(),
        });
        assert_eq!(error.message.as_ref(), "needs Spotify Premium");
        assert_eq!(error.data, Some(json!({ "kind": "premium_required" })));
    }

    #[test]
    fn a_track_uri_plays_that_track_and_both_uris_are_refused() {
        let track = play_op(SpotifyPlayParams {
            uri: Some("spotify:track:1".into()),
            context_uri: None,
            offset: None,
        })
        .unwrap();
        assert_eq!(
            track,
            SpotifyOp::Play {
                context_uri: None,
                uris: vec!["spotify:track:1".into()],
                offset: None,
            }
        );
        let album = play_op(SpotifyPlayParams {
            uri: None,
            context_uri: Some("spotify:album:2".into()),
            offset: Some(3),
        })
        .unwrap();
        assert_eq!(
            album,
            SpotifyOp::Play {
                context_uri: Some("spotify:album:2".into()),
                uris: vec![],
                offset: Some(3),
            }
        );
        let resume = play_op(SpotifyPlayParams {
            uri: None,
            context_uri: None,
            offset: None,
        })
        .unwrap();
        assert_eq!(
            resume,
            SpotifyOp::Play {
                context_uri: None,
                uris: vec![],
                offset: None,
            }
        );
        assert!(
            play_op(SpotifyPlayParams {
                uri: Some("a".into()),
                context_uri: Some("b".into()),
                offset: None,
            })
            .is_err()
        );
    }

    #[test]
    fn paging_has_defaults_and_clamps() {
        assert_eq!(page(None, None), (0, 20));
        assert_eq!(page(Some(3), Some(500)), (3, 50));
        assert_eq!(page(None, Some(0)), (0, 1));
    }

    #[test]
    fn search_covers_every_kind_by_default_and_at_most_ten_results() {
        let op = search_op(SpotifySearchParams {
            query: "blue".into(),
            types: None,
            limit: Some(99),
            offset: None,
        });
        assert_eq!(
            op,
            SpotifyOp::Search {
                query: "blue".into(),
                kinds: vec![
                    SpotifyKind::Track,
                    SpotifyKind::Album,
                    SpotifyKind::Artist,
                    SpotifyKind::Playlist,
                ],
                offset: 0,
                limit: 10,
            }
        );
        let albums = search_op(SpotifySearchParams {
            query: "blue".into(),
            types: Some(vec![SpotifyTypeArg::Album]),
            limit: None,
            offset: Some(10),
        });
        assert_eq!(
            albums,
            SpotifyOp::Search {
                query: "blue".into(),
                kinds: vec![SpotifyKind::Album],
                offset: 10,
                limit: 10,
            }
        );
    }

    #[test]
    fn volume_must_be_between_zero_and_one() {
        assert_eq!(volume(0.5).unwrap(), 0.5);
        assert!(volume(1.5).is_err());
        assert!(volume(-0.1).is_err());
        assert!(volume(f32::NAN).is_err());
    }
}
```

- [x] **Step 2: Declare the module**

In `znicz-mcp/src/lib.rs`, add `mod spotify_json;` after `mod skills;` (the top of the file reads `mod server;`, `mod skills;`).

- [x] **Step 3: Run the tests to verify they pass**

Run: `cargo test -p znicz-mcp spotify_json -q`
Expected: PASS, 7 tests. If `SpotifyPlayback`, `SpotifyDevice` or `SpotifyAccount` need a different field, the compiler names it; the field names above are from `znicz-core/src/spotify_host.rs`.

- [x] **Step 4: Commit**

```bash
git add znicz-mcp/src/spotify_json.rs znicz-mcp/src/lib.rs
git commit -m "feat(mcp): Spotify tool arguments and JSON helpers

Co-Authored-By: Claude Haiku 5.5 <noreply@anthropic.com>"
```

---

### Task 7: The status, login and logout tools

Adds the Spotify connection helpers to `server.rs`, the `spotify_status`, `spotify_login_begin`, `spotify_login_complete` and `spotify_logout` tools, and the test fake used by every later task.

**Files:**
- Modify: `znicz-mcp/src/server.rs`

- [x] **Step 1: Add the imports**

In `server.rs`, change the `use znicz_core::{…}` block to add the Spotify types (keep the existing names):

```rust
use znicz_core::{
    AudioOutput, Command, IpcClient, LibraryHost, LibraryOp, LoadResult, PlaybackStatus,
    PlayerState, ScanSummary, SpotifyHost, SpotifyOp, SpotifyReply, SpotifyState, apply_to_player,
    copy_saved, list_saved, load_path, remove_saved, rename_saved, sanitize_stem, saved_path,
    wait_for_library, write_path,
};
```

and add below `use crate::skills::SkillRegistry;`:

```rust
use crate::spotify_json;
```

- [x] **Step 2: Add the connection helpers**

In `server.rs`, in the plain `impl ZniczMcpServer` block (the one with `fn live_state`), add these after `run_library_job_blocking`:

```rust
    /// One Spotify op on the Spotify connection, on a blocking thread: a
    /// Spotify call can take about 40 s, and the server must keep answering.
    async fn spotify_apply(&self, op: SpotifyOp) -> Result<SpotifyReply, McpError> {
        let spotify = self.spotify.clone();
        tokio::task::spawn_blocking(move || SpotifyHost::apply(&spotify, op))
            .await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?
            .map_err(spotify_json::error_of)
    }

    /// The Spotify snapshot. It sends `Touch` first, so the player polls fast
    /// for the next minute, as it does after any Spotify call.
    async fn spotify_status_state(&self) -> Result<SpotifyState, McpError> {
        let spotify = self.spotify.clone();
        tokio::task::spawn_blocking(move || {
            SpotifyHost::apply(&spotify, SpotifyOp::Touch)?;
            SpotifyHost::try_state(&spotify)
        })
        .await
        .map_err(|e| McpError::internal_error(e.to_string(), None))?
        .map_err(spotify_json::error_of)
    }

    fn spotify_status_result(
        state: &SpotifyState,
    ) -> Result<rmcp::model::CallToolResult, McpError> {
        Self::json_result(&spotify_json::status_json(state, spotify_json::now_ms()))
    }
```

- [x] **Step 3: Add the tools**

In `server.rs`, inside the `#[tool_router] impl ZniczMcpServer` block, add these methods directly after `enrich_metadata` (the last tool, just before that block's closing brace):

```rust
    #[tool(
        description = "Spotify status: configured, login, account, Premium, device, what is playing and the last error. Call this first"
    )]
    async fn spotify_status(&self) -> Result<rmcp::model::CallToolResult, McpError> {
        let state = self.spotify_status_state().await?;
        Self::spotify_status_result(&state)
    }

    #[tool(
        description = "Start a Spotify login. Returns authorize_url: give it to the user, who approves it in a browser"
    )]
    async fn spotify_login_begin(&self) -> Result<rmcp::model::CallToolResult, McpError> {
        match self.spotify_apply(SpotifyOp::BeginLogin).await? {
            SpotifyReply::AuthorizeUrl {
                url,
                redirect_uri,
                listening,
            } => Self::json_result(&serde_json::json!({
                "authorize_url": url,
                "redirect_uri": redirect_uri,
                "listening": listening,
                "next": "Give authorize_url to the user. If listening is true, the browser can redirect to redirect_uri and the login finishes by itself. Otherwise, or if that fails, the user pastes the address the browser lands on into spotify_login_complete.",
            })),
            _ => Err(spotify_json::unexpected_reply()),
        }
    }

    #[tool(
        description = "Finish a Spotify login with the address the browser landed on (the paste route)"
    )]
    async fn spotify_login_complete(
        &self,
        Parameters(params): Parameters<spotify_json::SpotifyLoginCompleteParams>,
    ) -> Result<rmcp::model::CallToolResult, McpError> {
        self.spotify_apply(SpotifyOp::CompleteLogin {
            redirect_url: params.redirect_url,
        })
        .await?;
        let state = self.spotify_status_state().await?;
        Self::spotify_status_result(&state)
    }

    #[tool(description = "Log out of Spotify and delete the stored login")]
    async fn spotify_logout(&self) -> Result<rmcp::model::CallToolResult, McpError> {
        self.spotify_apply(SpotifyOp::Logout).await?;
        let state = self.spotify_status_state().await?;
        Self::spotify_status_result(&state)
    }
```

- [x] **Step 4: Add the fake and the test harness**

In `server.rs`'s `tests` module, add the imports on a new line below `use znicz_core::{AudioConfig, ClientRole, IpcServer, PlaybackStatus, spawn_player};`:

```rust
    use znicz_core::{
        SpotifyAccount, SpotifyErrorKind, SpotifyLogin, SpotifyPage, ZniczError,
    };
```

Add these to the `tests` module (after `second_client`, from Task 5):

```rust
    /// Records every op it gets. It refuses playback as a free account would,
    /// and answers the account and login calls with fixed values.
    #[derive(Default)]
    struct FakeSpotify {
        ops: Mutex<Vec<SpotifyOp>>,
    }

    impl SpotifyHost for FakeSpotify {
        fn apply(&self, op: SpotifyOp) -> znicz_core::Result<SpotifyReply> {
            self.ops.lock().unwrap().push(op.clone());
            match op {
                SpotifyOp::Play { .. } => Err(ZniczError::Spotify {
                    kind: SpotifyErrorKind::PremiumRequired,
                    message: "needs Spotify Premium".into(),
                }),
                SpotifyOp::Search { .. } => Ok(SpotifyReply::Page {
                    page: SpotifyPage::default(),
                }),
                SpotifyOp::BeginLogin => Ok(SpotifyReply::AuthorizeUrl {
                    url: "https://accounts.spotify.com/authorize?fake".into(),
                    redirect_uri: "http://127.0.0.1:8898/callback".into(),
                    listening: true,
                }),
                _ => Ok(SpotifyReply::Done),
            }
        }

        fn state(&self) -> SpotifyState {
            SpotifyState::default()
        }

        fn try_state(&self) -> znicz_core::Result<SpotifyState> {
            Ok(SpotifyState {
                login: SpotifyLogin::LoggedIn,
                account: Some(SpotifyAccount {
                    user: "eve".into(),
                    premium: Some(true),
                }),
                ..SpotifyState::default()
            })
        }
    }

    /// A player that hosts `FakeSpotify`, and an MCP server with two
    /// connections to it. The fake is returned to read what it was sent.
    /// The temp dir comes first: a `let` pattern drops its bindings in reverse
    /// order, so the dir is dropped last, after the server it backs.
    fn spotify_server() -> (tempfile::TempDir, IpcServer, ZniczMcpServer, Arc<FakeSpotify>) {
        let _lock = env_lock();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ipc.toml");
        let (player, thread) = spawn_player(AudioConfig::default());
        std::mem::forget(thread);
        let fake = Arc::new(FakeSpotify::default());
        let ipc = IpcServer::start_with_session_and_library(
            player,
            &path,
            Duration::ZERO,
            dir.path().join("session.toml"),
            Duration::ZERO,
            None,
            Some(fake.clone() as Arc<dyn SpotifyHost>),
        )
        .expect("start ipc");
        let player_client = IpcClient::connect(&path, ClientRole::Agent).expect("connect");
        let spotify_client = second_client(dir.path());
        let server = ZniczMcpServer::new(player_client, spotify_client, Vec::new());
        (dir, ipc, server, fake)
    }

    fn json_of(result: &rmcp::model::CallToolResult) -> serde_json::Value {
        serde_json::from_str(&result_text(result)).expect("tool result is JSON")
    }
```

`Arc` and `Mutex` come from `use super::*;` (the server module imports `std::sync::{Arc, Mutex}`).

- [x] **Step 5: Write the tests**

Add to the `tests` module:

```rust
    #[tokio::test]
    async fn spotify_status_reads_the_snapshot_and_marks_spotify_in_use() {
        let (_dir, _ipc, server, fake) = spotify_server();
        let result = server.spotify_status().await.expect("status");
        let json = json_of(&result);
        assert_eq!(json["configured"], true);
        assert_eq!(json["login"], "logged_in");
        assert_eq!(json["user"], "eve");
        assert_eq!(json["premium"], true);
        assert!(
            fake.ops.lock().unwrap().contains(&SpotifyOp::Touch),
            "a status call marks Spotify use"
        );
    }

    #[tokio::test]
    async fn login_begin_hands_back_the_url_and_what_to_do_with_it() {
        let (_dir, _ipc, server, _fake) = spotify_server();
        let result = server.spotify_login_begin().await.expect("begin");
        let json = json_of(&result);
        assert_eq!(
            json["authorize_url"],
            "https://accounts.spotify.com/authorize?fake"
        );
        assert_eq!(json["redirect_uri"], "http://127.0.0.1:8898/callback");
        assert_eq!(json["listening"], true);
        assert!(json["next"].as_str().unwrap().contains("authorize_url"));
    }

    #[tokio::test]
    async fn login_complete_and_logout_send_their_ops_and_return_the_status() {
        let (_dir, _ipc, server, fake) = spotify_server();
        let result = server
            .spotify_login_complete(Parameters(spotify_json::SpotifyLoginCompleteParams {
                redirect_url: "http://127.0.0.1:8898/callback?code=x&state=y".into(),
            }))
            .await
            .expect("complete");
        assert_eq!(json_of(&result)["login"], "logged_in");
        server.spotify_logout().await.expect("logout");
        let ops = fake.ops.lock().unwrap();
        assert!(matches!(
            ops.iter().find(|op| matches!(op, SpotifyOp::CompleteLogin { .. })),
            Some(SpotifyOp::CompleteLogin { redirect_url })
                if redirect_url.contains("code=x")
        ));
        assert!(ops.contains(&SpotifyOp::Logout));
    }
```

- [x] **Step 6: Run the tests to verify they pass**

Run: `cargo test -p znicz-mcp spotify_ -q`
Expected: PASS for the three new tests.

- [x] **Step 7: Commit**

```bash
git add znicz-mcp/src/server.rs
git commit -m "feat(mcp): spotify_status, login and logout tools

Co-Authored-By: Claude Haiku 5.5 <noreply@anthropic.com>"
```

---

### Task 8: Search and browse tools

**Files:**
- Modify: `znicz-mcp/src/server.rs`

- [ ] **Step 1: Write the failing tests**

Add to the `tests` module of `server.rs`:

```rust
    #[tokio::test]
    async fn search_sends_one_op_with_every_kind_and_at_most_ten_results() {
        let (_dir, _ipc, server, fake) = spotify_server();
        let result = server
            .spotify_search(Parameters(spotify_json::SpotifySearchParams {
                query: "blue".into(),
                types: None,
                limit: Some(99),
                offset: None,
            }))
            .await
            .expect("search");
        assert_eq!(json_of(&result)["reply"], "page");
        let ops = fake.ops.lock().unwrap();
        assert!(
            matches!(ops.last(), Some(SpotifyOp::Search { limit: 10, .. })),
            "{ops:?}"
        );
    }

    #[tokio::test]
    async fn tracks_pass_the_uri_and_clamp_the_page() {
        let (_dir, _ipc, server, fake) = spotify_server();
        server
            .spotify_tracks(Parameters(spotify_json::SpotifyUriPagedParams {
                uri: "spotify:album:1".into(),
                offset: None,
                limit: Some(500),
            }))
            .await
            .expect("tracks");
        let ops = fake.ops.lock().unwrap();
        assert!(
            matches!(
                ops.last(),
                Some(SpotifyOp::Tracks { uri, limit: 50, .. }) if uri.as_str() == "spotify:album:1"
            ),
            "{ops:?}"
        );
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p znicz-mcp search_sends -q`
Expected: FAIL to compile (`spotify_search` and `spotify_tracks` do not exist).

- [ ] **Step 3: Add the helper and the tools**

In `server.rs`, in the plain `impl ZniczMcpServer` block, add after `spotify_status_result`:

```rust
    /// Run one Spotify op and return its reply as the tool result.
    async fn spotify_reply(&self, op: SpotifyOp) -> Result<rmcp::model::CallToolResult, McpError> {
        let reply = self.spotify_apply(op).await?;
        Self::json_result(&reply)
    }
```

In the `#[tool_router]` block, after `spotify_logout`, add:

```rust
    #[tool(
        description = "Search Spotify for tracks, albums, artists and playlists. Spotify returns at most 10 per kind; to page, search one kind at a time and use offset"
    )]
    async fn spotify_search(
        &self,
        Parameters(params): Parameters<spotify_json::SpotifySearchParams>,
    ) -> Result<rmcp::model::CallToolResult, McpError> {
        self.spotify_reply(spotify_json::search_op(params)).await
    }

    #[tool(description = "The user's Spotify playlists, a page at a time")]
    async fn spotify_playlists(
        &self,
        Parameters(params): Parameters<spotify_json::SpotifyPagedParams>,
    ) -> Result<rmcp::model::CallToolResult, McpError> {
        let (offset, limit) = spotify_json::page(params.offset, params.limit);
        self.spotify_reply(SpotifyOp::Playlists { offset, limit })
            .await
    }

    #[tool(description = "The user's liked songs, a page at a time")]
    async fn spotify_liked_songs(
        &self,
        Parameters(params): Parameters<spotify_json::SpotifyPagedParams>,
    ) -> Result<rmcp::model::CallToolResult, McpError> {
        let (offset, limit) = spotify_json::page(params.offset, params.limit);
        self.spotify_reply(SpotifyOp::LikedSongs { offset, limit })
            .await
    }

    #[tool(description = "The albums the user has saved, a page at a time")]
    async fn spotify_albums(
        &self,
        Parameters(params): Parameters<spotify_json::SpotifyPagedParams>,
    ) -> Result<rmcp::model::CallToolResult, McpError> {
        let (offset, limit) = spotify_json::page(params.offset, params.limit);
        self.spotify_reply(SpotifyOp::Albums { offset, limit })
            .await
    }

    #[tool(
        description = "Tracks of an album, or of a playlist the user owns or collaborates on. Pass its uri. Other playlists can still be played, but their tracks cannot be listed"
    )]
    async fn spotify_tracks(
        &self,
        Parameters(params): Parameters<spotify_json::SpotifyUriPagedParams>,
    ) -> Result<rmcp::model::CallToolResult, McpError> {
        let (offset, limit) = spotify_json::page(params.offset, params.limit);
        self.spotify_reply(SpotifyOp::Tracks {
            uri: params.uri,
            offset,
            limit,
        })
        .await
    }

    #[tool(description = "An artist's albums. Pass the artist's uri")]
    async fn spotify_artist_albums(
        &self,
        Parameters(params): Parameters<spotify_json::SpotifyUriPagedParams>,
    ) -> Result<rmcp::model::CallToolResult, McpError> {
        let (offset, limit) = spotify_json::page(params.offset, params.limit);
        self.spotify_reply(SpotifyOp::ArtistAlbums {
            uri: params.uri,
            offset,
            limit,
        })
        .await
    }
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p znicz-mcp -q`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add znicz-mcp/src/server.rs
git commit -m "feat(mcp): Spotify search and browse tools

Co-Authored-By: Claude Haiku 5.5 <noreply@anthropic.com>"
```

---

### Task 9: Playback and device tools

**Files:**
- Modify: `znicz-mcp/src/server.rs`

- [ ] **Step 1: Write the failing tests**

Add to the `tests` module:

```rust
    #[tokio::test]
    async fn a_premium_refusal_comes_back_with_its_kind() {
        let (_dir, _ipc, server, _fake) = spotify_server();
        let err = server
            .spotify_play(Parameters(spotify_json::SpotifyPlayParams {
                uri: Some("spotify:track:1".into()),
                context_uri: None,
                offset: None,
            }))
            .await
            .expect_err("a free account cannot play");
        assert_eq!(err.message.as_ref(), "needs Spotify Premium");
        assert_eq!(err.data, Some(serde_json::json!({ "kind": "premium_required" })));
    }

    #[tokio::test]
    async fn volume_out_of_range_is_refused_before_it_is_sent() {
        let (_dir, _ipc, server, fake) = spotify_server();
        let err = server
            .spotify_set_volume(Parameters(spotify_json::SpotifyVolumeParams { volume: 1.5 }))
            .await
            .expect_err("out of range");
        assert_eq!(err.code, rmcp::model::ErrorCode::INVALID_PARAMS);
        assert!(fake.ops.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn transfer_without_play_only_moves_the_device() {
        let (_dir, _ipc, server, fake) = spotify_server();
        server
            .spotify_transfer(Parameters(spotify_json::SpotifyTransferParams {
                device_id: "dev1".into(),
                play: None,
            }))
            .await
            .expect("transfer");
        let ops = fake.ops.lock().unwrap();
        assert!(
            matches!(
                ops.last(),
                Some(SpotifyOp::Transfer { device_id, play: false })
                    if device_id.as_str() == "dev1"
            ),
            "{ops:?}"
        );
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p znicz-mcp a_premium_refusal -q`
Expected: FAIL to compile (`spotify_play` and the others do not exist).

- [ ] **Step 3: Add the tools**

In the `#[tool_router]` block, after `spotify_artist_albums`, add:

```rust
    #[tool(
        description = "Play on Spotify. uri plays one track; context_uri plays an album or playlist from offset; giving neither resumes. Starting Spotify pauses Znicz's own playback"
    )]
    async fn spotify_play(
        &self,
        Parameters(params): Parameters<spotify_json::SpotifyPlayParams>,
    ) -> Result<rmcp::model::CallToolResult, McpError> {
        let op = spotify_json::play_op(params)?;
        self.spotify_reply(op).await
    }

    #[tool(description = "Pause Spotify. Znicz's playback is not affected")]
    async fn spotify_pause(&self) -> Result<rmcp::model::CallToolResult, McpError> {
        self.spotify_reply(SpotifyOp::Pause).await
    }

    #[tool(description = "Next track on Spotify")]
    async fn spotify_next(&self) -> Result<rmcp::model::CallToolResult, McpError> {
        self.spotify_reply(SpotifyOp::Next).await
    }

    #[tool(description = "Previous track on Spotify")]
    async fn spotify_previous(&self) -> Result<rmcp::model::CallToolResult, McpError> {
        self.spotify_reply(SpotifyOp::Previous).await
    }

    #[tool(description = "Seek within the Spotify track, in milliseconds")]
    async fn spotify_seek(
        &self,
        Parameters(params): Parameters<spotify_json::SpotifySeekParams>,
    ) -> Result<rmcp::model::CallToolResult, McpError> {
        self.spotify_reply(SpotifyOp::Seek {
            position_ms: params.position_ms,
        })
        .await
    }

    #[tool(description = "Spotify volume between 0.0 and 1.0, the same scale as set_volume")]
    async fn spotify_set_volume(
        &self,
        Parameters(params): Parameters<spotify_json::SpotifyVolumeParams>,
    ) -> Result<rmcp::model::CallToolResult, McpError> {
        let volume = spotify_json::volume(params.volume)?;
        self.spotify_reply(SpotifyOp::SetVolume { volume }).await
    }

    #[tool(description = "Spotify repeat mode: off, track or context")]
    async fn spotify_set_repeat(
        &self,
        Parameters(params): Parameters<spotify_json::SpotifyRepeatParams>,
    ) -> Result<rmcp::model::CallToolResult, McpError> {
        self.spotify_reply(SpotifyOp::SetRepeat {
            mode: params.mode.into(),
        })
        .await
    }

    #[tool(description = "Turn Spotify shuffle on or off")]
    async fn spotify_set_shuffle(
        &self,
        Parameters(params): Parameters<spotify_json::SpotifyShuffleParams>,
    ) -> Result<rmcp::model::CallToolResult, McpError> {
        self.spotify_reply(SpotifyOp::SetShuffle { on: params.on })
            .await
    }

    #[tool(description = "Add a track to the Spotify queue")]
    async fn spotify_queue_add(
        &self,
        Parameters(params): Parameters<spotify_json::SpotifyQueueAddParams>,
    ) -> Result<rmcp::model::CallToolResult, McpError> {
        self.spotify_reply(SpotifyOp::QueueAdd { uri: params.uri })
            .await
    }

    #[tool(description = "Spotify devices the account can see, with their ids")]
    async fn spotify_devices(&self) -> Result<rmcp::model::CallToolResult, McpError> {
        self.spotify_reply(SpotifyOp::Devices).await
    }

    #[tool(
        description = "Move Spotify playback to a device from spotify_devices. play: true starts it there"
    )]
    async fn spotify_transfer(
        &self,
        Parameters(params): Parameters<spotify_json::SpotifyTransferParams>,
    ) -> Result<rmcp::model::CallToolResult, McpError> {
        self.spotify_reply(SpotifyOp::Transfer {
            device_id: params.device_id,
            play: params.play.unwrap_or(false),
        })
        .await
    }
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p znicz-mcp -q`
Expected: PASS for the whole crate, including the three new tests.

- [ ] **Step 5: Commit**

```bash
git add znicz-mcp/src/server.rs
git commit -m "feat(mcp): Spotify playback, volume, queue and device tools

Co-Authored-By: Claude Haiku 5.5 <noreply@anthropic.com>"
```

---

### Task 10: The `znicz://spotify/now-playing` resource

**Files:**
- Modify: `znicz-mcp/src/server.rs`

`read_resource` needs an rmcp `RequestContext`, which tests cannot build, so this task has no unit test. The manual check in Task 15 reads the resource.

- [ ] **Step 1: Add the resource to the list**

In `server.rs`, in `fn list_resources`, add after the `znicz://stations` line:

```rust
            make_resource(
                "znicz://spotify/now-playing",
                "spotify-now-playing",
                "Spotify login, account, playback and devices",
            ),
```

- [ ] **Step 2: Serve it**

In `fn read_resource`, add this line directly after `let state = self.live_state();`:

```rust
        let spotify = self.spotify.clone();
```

and inside the `async move` block, directly after `if uri == "skill://index.json" { … }` (before `if let Some(file) = skills.get_file(&uri)`), add:

```rust
            if uri == "znicz://spotify/now-playing" {
                let snapshot = tokio::task::spawn_blocking(move || SpotifyHost::try_state(&spotify))
                    .await
                    .map_err(|e| McpError::internal_error(e.to_string(), None))?
                    .map_err(spotify_json::error_of)?;
                let text = serde_json::to_string_pretty(&snapshot)
                    .map_err(|e| McpError::internal_error(e.to_string(), None))?;
                return Ok(ReadResourceResult {
                    contents: vec![ResourceContents::TextResourceContents {
                        uri,
                        mime_type: Some("application/json".into()),
                        text,
                    }],
                });
            }
```

The snapshot is read with `try_state`, not with `apply`, so reading the resource does not mark Spotify as in use. Only the tools do.

- [ ] **Step 3: Build and test**

Run: `cargo test -p znicz-mcp -q && cargo clippy -p znicz-mcp -- -D warnings`
Expected: PASS. If clippy reports `redundant_clone` or a similar lint on `spotify`, keep the clone: `read_resource` returns a `'_` future that borrows `self`, so the clone is needed.

- [ ] **Step 4: Commit**

```bash
git add znicz-mcp/src/server.rs
git commit -m "feat(mcp): znicz://spotify/now-playing resource

Co-Authored-By: Claude Haiku 5.5 <noreply@anthropic.com>"
```

---

### Task 11: The bundled `spotify` skill

**Files:**
- Create: `znicz-mcp/skills/spotify/SKILL.md`
- Modify: `znicz-mcp/src/skills.rs` (the bundled-skills test)

- [ ] **Step 1: Write the failing test**

In `znicz-mcp/src/skills.rs`, replace `bundled_skills_loaded` with:

```rust
    #[test]
    fn bundled_skills_loaded() {
        let skills_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("skills");
        let registry = SkillRegistry::load(&[skills_dir]);
        assert!(registry.entries().len() >= 6);
        assert!(
            registry.entries().iter().any(|e| e.name == "spotify"),
            "the spotify skill is bundled"
        );
    }
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p znicz-mcp bundled_skills_loaded -q`
Expected: FAIL: "the spotify skill is bundled".

- [ ] **Step 3: Write the skill**

Create `znicz-mcp/skills/spotify/SKILL.md` with exactly this content:

````markdown
---
name: spotify
description: Control Spotify from znicz: log in, search, browse, play, pause, seek, set volume and move playback between devices. Use for any request that names Spotify, and for login or Premium questions. Requests that do not name Spotify are for the local library.
---

# Spotify

Spotify is a second source beside the local library. Its tools start with `spotify_`. Znicz controls the Spotify app on the user's account; it does not play Spotify audio itself.

## Always start with status

Call `spotify_status` first. It returns:

| Field | Meaning |
|-------|---------|
| `configured` | `false`: no client ID is set in `config.toml` |
| `login` | `not_configured`, `not_logged_in`, `logging_in`, `logged_in` or `failed` |
| `user` and `premium` | The account. `premium` is `null` until Spotify has refused a playback call, so it is not known yet |
| `device` | The device Spotify plays on, if any |
| `playing` | The current track and position, or `null` |
| `last_error` | The last Spotify error text |

## Logging in

If `login` is not `logged_in`, log in:

1. Call `spotify_login_begin`. It returns `authorize_url`.
2. Give `authorize_url` to the user. They open it in a browser and approve.
3. Then one of two routes:
   - **Local:** the browser redirects to `redirect_uri` on the user's machine, and the login finishes by itself. Check with `spotify_status`.
   - **Headless** (SSH, or the redirect does not reach the player): the browser lands on a page that says it cannot connect. Ask the user to copy the full address from the address bar and paste it to you. Call `spotify_login_complete` with `redirect_url` set to that address.
4. `spotify_logout` removes the stored login.

If `login` is `not_configured`, the user must set `client_id` under `[spotify]` in `config.toml` and run `znicz spotify login`. You cannot set the client ID from here.

## Local library or Spotify?

A request that does not say "Spotify" is for the local library: use `search_library`, `play_playlist` and the other library tools. A request that says Spotify is for the `spotify_*` tools.

## Searching and browsing

- `spotify_search` returns at most 10 results per type. To page, search one type at a time and use `offset`.
- `spotify_playlists`, `spotify_liked_songs` and `spotify_albums` list the user's own items.
- `spotify_tracks { uri }` lists an album's tracks, or the tracks of a playlist. Spotify lists a playlist's tracks only when the user owns it or collaborates on it. Any other playlist can still be played with `spotify_play`, but its tracks cannot be listed.
- `spotify_artist_albums { uri }` lists an artist's albums. Artists have no top-tracks list here.
- Every item carries a `uri`. Pass it to the other tools.

## Playing and controlling

- `spotify_play { uri }` plays one track. `spotify_play { context_uri, offset? }` plays an album or playlist from `offset`. `spotify_play {}` resumes.
- Starting Spotify pauses Znicz's own playback. Starting Znicz playback pauses Spotify.
- `spotify_pause`, `spotify_next`, `spotify_previous`, `spotify_seek { position_ms }`, `spotify_set_volume { volume }` (0.0 to 1.0), `spotify_set_repeat { mode }` (off, track or context), `spotify_set_shuffle { on }`, and `spotify_queue_add { uri }`.
- Transport calls return `{"reply": "done"}`. Call `spotify_status` to see the new state.

## Devices

- `spotify_devices` lists the devices Spotify can see, with their ids.
- `spotify_transfer { device_id, play }` moves playback to a device. Use `play: true` to start it there.

## Errors

A failed Spotify tool returns its message and a `kind` in the error data. Act on the kind:

| kind | What to do |
|------|------------|
| `premium_required` | Tell the user that playback needs Spotify Premium. Search and browsing still work. |
| `no_active_device` | Ask the user to open Spotify on a phone or computer. Or call `spotify_devices`, pick one, and call `spotify_transfer` with `play: true`. |
| `not_configured` | Tell the user to set `client_id` under `[spotify]` in `config.toml` and run `znicz spotify login`. |
| `not_logged_in`, `login_expired` | Start the login steps above. |
| `wrong_address` | The pasted address is not from this login. Call `spotify_login_begin` again and ask for the new address. |
| `rate_limited` | Wait a little before the next call. Do not retry in a loop. |
| `unavailable` | Explain the message to the user. Spotify does not offer this for that item. |
| `network` | Spotify or the player did not answer. Try once more, then tell the user. If the message mentions an older player, the player must be restarted: ask the user to run `znicz player stop` and try again. |
| `no_service` | The running player has no Spotify service, perhaps an older version. Ask the user to run `znicz player stop` and try again. |
| `other` | Show the message. |

Some messages end with a hint for the terminal UI, such as "Enter still plays it" or "pick one with ,". Those keys do not exist here: use the tools above instead, and do not repeat the hint to the user.
````

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p znicz-mcp -q`
Expected: PASS, including `bundled_skills_loaded` and the existing frontmatter test.

- [ ] **Step 5: Commit**

```bash
git add znicz-mcp/skills/spotify/SKILL.md znicz-mcp/src/skills.rs
git commit -m "feat(mcp): bundled spotify skill

Co-Authored-By: Claude Haiku 5.5 <noreply@anthropic.com>"
```

---

### Task 12: Wiki (wiki-sync rule)

Behaviour, MCP tools, the resource, the skill and the error kinds changed, so the wiki changes in this same PR.

**Files:**
- Modify: `wiki/Architecture/MCP.md`
- Modify: `wiki/Architecture/Spotify.md`

- [ ] **Step 1: Add the skill to the list in `MCP.md`**

In `wiki/Architecture/MCP.md`, replace:

```
- `radio-streaming`
- `mcp-control`
```

with:

```
- `radio-streaming`
- `mcp-control`
- `spotify`
```

- [ ] **Step 2: Add a Spotify section to `MCP.md`**

In `wiki/Architecture/MCP.md`, insert this section directly before the line `## Cursor`:

```markdown
## Spotify tools

The `spotify_*` tools control Spotify through the player (see [Spotify](Spotify.md)).
`znicz mcp` opens a **second** IPC connection for them. A Spotify call can take
about 40 s, and sharing one connection would hold up the player tools. Each
Spotify tool runs on a blocking thread, as library scans do.

| Group | Tools |
| --- | --- |
| Account | `spotify_status`, `spotify_login_begin`, `spotify_login_complete`, `spotify_logout` |
| Browse | `spotify_search`, `spotify_playlists`, `spotify_liked_songs`, `spotify_albums`, `spotify_tracks`, `spotify_artist_albums` |
| Playback | `spotify_play`, `spotify_pause`, `spotify_next`, `spotify_previous`, `spotify_seek`, `spotify_set_volume`, `spotify_set_repeat`, `spotify_set_shuffle`, `spotify_queue_add` |
| Devices | `spotify_devices`, `spotify_transfer` |

The client ID cannot be set from MCP, and no tool opens a browser:
`spotify_login_begin` returns the URL, and the agent gives it to the user. The
user either lets the browser redirect to the player, or pastes the address the
browser lands on into `spotify_login_complete`.

**Errors** are MCP tool errors with the message, and a `kind` in the error's
`data`, for example `{"kind": "premium_required"}`. The kinds are
`not_configured`, `not_logged_in`, `login_expired`, `wrong_address`,
`premium_required`, `no_active_device`, `rate_limited`, `unavailable`, `network`,
`no_service` and `other`. The bundled `spotify` skill says what to do for each.

**Resource:** `znicz://spotify/now-playing` is the Spotify snapshot as JSON: login,
account, playback, devices and the last error. Reading it does not mark Spotify
as in use; only the tools do.

```

- [ ] **Step 3: Add typed errors to `Spotify.md`**

In `wiki/Architecture/Spotify.md`, append this to the end of the file (after the line `Tokens never appear in logs (`Token`'s `Debug` redacts them).`):

```markdown

### Typed errors

`SpotifyError` maps to a `SpotifyErrorKind`. The core error is
`ZniczError::Spotify { kind, message }`, and its text is the message. Over IPC the
kind is sent as `spotify_kind` next to the message, so the CLI and the MCP tools
act on the kind and show the message. The kinds are `not_configured`,
`not_logged_in`, `login_expired`, `wrong_address`, `premium_required`,
`no_active_device`, `rate_limited`, `unavailable`, `network`, `no_service` and
`other`.

- A player that does not answer within the 30 s read timeout is `network`, with
  the text "the player did not answer in time". The client reconnects and resends
  a change once first. A dropped connection is `network` too.
- `no_service` is a player without a Spotify service: an older player, or one
  started without a data folder for the token.
- A resent change keeps its kind, because the player stores the failure with it.
```

- [ ] **Step 4: Check the wiki for stale wording**

Run: `grep -n "Spotify { op }\|Spotify tools\|spotify_" wiki/Architecture/MCP.md wiki/Architecture/Spotify.md | head -20`
Expected: the new section and the typed-errors section appear; no line still says the MCP server has no Spotify tools.

- [ ] **Step 5: Commit**

```bash
git add wiki/Architecture/MCP.md wiki/Architecture/Spotify.md
git commit -m "docs(wiki): Spotify tools, resource, skill and typed errors

Co-Authored-By: Claude Haiku 5.5 <noreply@anthropic.com>"
```

---

### Task 13: Point the Part 2 manual-check note at this part

**Files:**
- Modify: `docs/superpowers/plans/2026-11-26-spotify-2-player.md`

- [ ] **Step 1: Update the Step 5 note**

In `docs/superpowers/plans/2026-11-26-spotify-2-player.md`, in Task 13 Step 5, replace:

```
Re-check this step in Part 3, once MCP `spotify_play` exists: start a local track in Znicz, call `spotify_play` through MCP, and the local track must pause while Spotify plays.
```

with:

```
This direction is checked in Part 3's manual check (its Step 5 in `docs/superpowers/plans/2026-11-26-spotify-3-mcp.md`): start a local track in Znicz, call `spotify_play` through MCP, and the local track must pause while Spotify plays.
```

- [ ] **Step 2: Commit**

```bash
git add docs/superpowers/plans/2026-11-26-spotify-2-player.md
git commit -m "docs(plan): Spotify part 2 step 5 is checked in part 3

Co-Authored-By: Claude Haiku 5.5 <noreply@anthropic.com>"
```

---

### Task 14: Full check

**Files:** none (verification; fix any finding in the file where it is).

- [ ] **Step 1: Run the whole workspace**

Run: `cargo test --workspace -q`
Expected: PASS.

- [ ] **Step 2: Clippy and formatting**

Run: `cargo clippy --workspace --all-targets -- -D warnings && cargo fmt --all --check`
Expected: both pass. If `fmt` reports differences, run `cargo fmt --all`, re-run the check, and commit the formatting as `style: cargo fmt`.

- [ ] **Step 3: Check nothing else builds the old variant**

Run: `grep -rn "ZniczError::Spotify(" --include=*.rs . | grep -v target`
Expected: no output.

Run: `grep -rn "IpcResponse::Err { message }" --include=*.rs znicz-core | grep -v target`
Expected: no output (every pattern now has `..` or `spotify_kind`).

- [ ] **Step 4: Confirm the working tree is clean**

Run: `git status --short`
Expected: no output. If a generated file (such as `Cargo.lock`) still shows, commit it with the task that changed it.

- [ ] **Step 5: Commit any fixes**

If steps 1–4 needed changes:

```bash
git add <the changed files>
git commit -m "fix(spotify): <what the check found>

Co-Authored-By: Claude Haiku 5.5 <noreply@anthropic.com>"
```

If nothing needed changing, there is no commit for this task. Mark its checkboxes in this plan and commit the plan on its own with `docs(plan): tick Spotify part 3 full check`.

---

### Task 15: Manual check

Run these with a real Spotify Premium account and a Development Mode app (client ID set under `[spotify]`, redirect URI `http://127.0.0.1:8898/callback` registered, the account added under User Management). Connect an MCP host, or the MCP inspector, to `znicz mcp`. No code is written in this task. Note any surprise and fix it in its own commit.

- [ ] **Step 1: Status**

Call `spotify_status`.

Expected: `configured: true`, `login: "logged_in"`, your name in `user`. `premium` is `true` or `null`. Once a playback call has been refused it is `false`.

- [ ] **Step 2: Search**

Call `spotify_search` with `query: "Kind of Blue"` and `types: ["album"]`.

Expected: a `page` reply with at most 10 items, each with a `uri` such as `spotify:album:…`.

- [ ] **Step 3: Play a track and an album**

Call `spotify_play` with a track `uri` from search. Then `spotify_status`: `playing.is_playing` is `true` and `playing.title` matches.

Call `spotify_play` with the album's `uri` as `context_uri`. Then `spotify_status` shows the album's first track.

- [ ] **Step 4: Pause, next, seek, volume**

Call `spotify_pause`; status shows `is_playing: false`. Call `spotify_play {}` to resume. Call `spotify_next`; the title changes. Call `spotify_seek { position_ms: 30000 }`; `progress_ms` is near 30 s. Call `spotify_set_volume { volume: 0.3 }` and check the device's `volume_percent` through `spotify_devices`.

Call `spotify_set_volume { volume: 1.5 }`. Expected: an error with the message about 0.0 and 1.0, and no call is sent.

- [ ] **Step 5: Znicz pauses when `spotify_play` starts**

Start a local track in Znicz first: call `play` with a file path from the library, and confirm it plays (`get_player_state` shows `status: "Playing"`). Then call `spotify_play { uri }` with a track.

Expected: the local track is paused (`get_player_state` shows `status: "Paused"`), and Spotify plays (`spotify_status`). This is the step deferred from Part 2, Task 13, Step 5.

- [ ] **Step 6: Devices and transfer**

Call `spotify_devices`. Note a device id that is not the active one. Call `spotify_transfer { device_id, play: true }`.

Expected: `spotify_status` shows that device in `device`, and it plays.

- [ ] **Step 7: A playlist you do not own**

Find a playlist that you do not own (search for a public playlist) and call `spotify_tracks { uri }` on it.

Expected: an error with `kind: "unavailable"` and the message "Spotify only lists the tracks of playlists you own or collaborate on; Enter still plays it" (the service's text, which has a terminal-UI hint that the skill tells agents to ignore). Then `spotify_play { context_uri: <that playlist> }` still plays it.

- [ ] **Step 8: Resource**

Read `znicz://spotify/now-playing` through the MCP host.

Expected: the same JSON as `spotify_status`'s data, with `login`, `account`, `playback`, `devices` and `last_error` keys.

- [ ] **Step 9: Commit**

Mark the checks that passed in this plan, then commit:

```bash
git add docs/superpowers/plans/2026-11-26-spotify-3-mcp.md
git commit -m "docs(plan): Spotify part 3 manual check passed

Co-Authored-By: Claude Haiku 5.5 <noreply@anthropic.com>"
```

If a check found a bug, fix it, run Task 14, and commit with `fix(spotify): <what the manual check found>` and the trailer.

---

## Self-review against the spec and the brief

| Brief item | Where |
| --- | --- |
| 1. `SpotifyErrorKind` (snake_case, eleven variants) | Task 1 Step 3 |
| 1. `ZniczError::Spotify { kind, message }`; Display is the message | Task 1 Step 4 |
| 1. Map every `SpotifyError` variant (the brief's eight, plus the other nine) | Task 1 Step 5, test `every_error_has_a_kind` |
| 1. `spotify_kind` on `IpcResponse::Err`, `serde(default, skip_serializing_if)`; old clients keep working | Task 2 Step 3 |
| 1. Server sets the kind; `IpcClient` rebuilds the typed error; `NO_SPOTIFY` gives `NoService` | Task 2 Steps 5–6, Task 1 `spotify_failure` |
| 1. Resent change keeps the kind (the brief did not ask; needed for correctness) | Task 2 Step 4, test `the_kind_survives_ipc_and_a_resend` |
| 1. A timeout is `Spotify { Network, "the player did not answer in time" }`, not the outdated hint | Task 3 Steps 3–6, test `a_timeout_is_a_network_error…` |
| 1. Every `ZniczError::Spotify` site fixed; CLI switched to the kind | Task 1 Step 6, Task 4, Task 14 Step 3 |
| 2. Second `IpcClient` for Spotify, not a clone; opened like the first | Task 5 (`run_mcp` calls `connect_agent` again) |
| 3. Every tool calls `SpotifyHost::apply`, path-qualified; blocking off the runtime | Tasks 7–9 (`spotify_apply`, `spawn_blocking`) |
| 3. `spotify_status` shape | Task 6 `status_json` |
| 3. `spotify_login_begin` returns `authorize_url`, `redirect_uri`, `listening`, and says what to do | Task 7 Step 3 |
| 3. `spotify_login_complete`, `spotify_logout` | Task 7 Step 3 |
| 3. `spotify_search` (types, limit default 10 max 10, offset) | Task 6 `search_op`, Task 8 |
| 3. `spotify_playlists`, `spotify_liked_songs`, `spotify_albums` (offset, limit) | Task 8 |
| 3. `spotify_tracks`, `spotify_artist_albums` (uri, offset, limit) | Task 8 |
| 3. `spotify_play` (uri → `uris`, context_uri, offset; none resumes) | Task 6 `play_op`, Task 9 |
| 3. `spotify_pause`, `spotify_next`, `spotify_previous`, `spotify_seek`, `spotify_set_volume` (0.0–1.0 checked), `spotify_set_repeat`, `spotify_set_shuffle`, `spotify_queue_add`, `spotify_devices`, `spotify_transfer` (play default false) | Task 9 |
| 3. Errors: message plus kind | Task 6 `error_of`, Task 9 test `a_premium_refusal…` |
| 3. No settable client ID; no browser | Task 7 (no tool opens one), wiki Task 12 |
| 4. `znicz://spotify/now-playing` as `SpotifyState` JSON | Task 10 |
| 5. Bundled `spotify` skill with each required section | Task 11 |
| 5. Skill included in the bundled-skills test | Task 11 Step 1 |
| 6. `MCP.md`: tools, resource, skill, second connection, error kinds | Task 12 Steps 1–2 |
| 6. `Spotify.md`: typed errors | Task 12 Step 3 |
| 6. Part 2 Task 13 Step 5 note | Task 13 |
| 7. Manual check list, including Znicz pausing on `spotify_play` | Task 15 |
| 8. Out of scope: TUI, version bump, CHANGELOG, README | Not planned here (see "Spec refinements") |
| Commit message `docs(plan): Spotify part 3, MCP tools` | Written when this plan is committed |

**Placeholder scan:** no `TBD`, no "handle edge cases", every code step has its code. Task 2 Step 3 says the compiler lists the `IpcResponse::Err` sites rather than printing all 21; that is a mechanical edit with a fixed rule, and Task 14 Step 3 greps for any site left behind.

**Type consistency:**
- `SpotifyErrorKind` is defined in Task 1 and used in Tasks 2–4, 6 and 7.
- `spotify_failure(message, Option<SpotifyErrorKind>)` and `unexpected_spotify()` are defined in Task 1, and used in Task 2 Step 6.
- `SpotifyFailure::{of, response}` and `no_spotify()` are defined in Task 2 Step 4, and used in Step 5.
- `RpcError::{TimedOut, Other}`, `RpcError::into_player_error`, `is_timeout`, `TIMED_OUT`, `RpcFailure::sent` and `RpcFailure::into_spotify_error` are defined in Task 3 Steps 3 and 5, and used in Steps 4, 5 and 6.
- `ZniczMcpServer::new(player, spotify, skills_dirs)` and `with_library(player, spotify, skills_dirs, library)` are defined in Task 5, and used in Task 5's test updates and Task 7's `spotify_server`.
- `spotify_json::{page, search_op, play_op, volume, error_of, unexpected_reply, login_label, now_ms, status_json}` and the `Spotify…Params` types are defined in Task 6. Every use in Tasks 7–10 matches those names.
- `ZniczMcpServer::{spotify_apply, spotify_status_state, spotify_status_result, spotify_reply}` are defined in Tasks 7 and 8, and used in Tasks 7–9.
- `FakeSpotify`, `spotify_server()`, `json_of()` and `second_client()` are defined in Tasks 5 and 7, and used in Tasks 7–9.
- `SpotifyPage` (`Default` derived) is used in the fake (Task 7). It derives `Default` in `spotify_host.rs` ✓.
