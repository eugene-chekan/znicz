# Spotify control, part 1: the `znicz-spotify` crate — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A tested `znicz-spotify` crate that logs in to Spotify with PKCE, talks to the Web API, polls playback, and implements a `SpotifyHost` trait that `znicz-core` defines. Nothing in the player, TUI or MCP uses it yet (parts 2–5).

**Architecture:** `znicz-core` gets the shared, serialisable types (`SpotifyOp`, `SpotifyReply`, `SpotifyState`, …) and the `SpotifyHost` trait, mirroring `LibraryHost`. The new crate holds pure pieces (PKCE, redirect parsing, JSON → types, status → error), a thin blocking HTTP layer over `ureq`, a `Client` that owns the token and the login session, a one-shot callback listener, and `SpotifyService`, a worker thread that polls and answers ops.

**Tech Stack:** Rust 2024 (MSRV 1.85: no `let` chains), `ureq` 3 (blocking, `http_status_as_error(false)`), `serde_json`, `sha2`, `base64`, `getrandom` 0.3, `percent-encoding`, `dirs`, `thiserror`, `tracing`; `tempfile` for tests.

**Spec:** `docs/superpowers/specs/2026-11-26-spotify-control-design.md`.

**Part map** (each part is its own plan, written when the previous one is done):

1. This plan: core types plus the `znicz-spotify` crate.
2. Player, IPC, the pause rule, `znicz spotify login|logout|status`, `[spotify]` config.
3. MCP tools, resource and skill.
4. TUI Settings → Spotify tab and the login panel.
5. TUI `StageModel` / `TransportModel` split and Spotify mode (`O`).

**Spec refinement:** `open_browser` is only read by front ends, so it is not part of `SpotifyConfig` (the player never opens a browser). Parts 2 and 4 read it from `config.toml`.

**Spotify's February 2026 rules** (found during Task 9, in force for Development Mode apps since March 9, 2026; see the [changelog](https://developer.spotify.com/documentation/web-api/references/changes/february-2026)): `/playlists/{id}/tracks` became `/playlists/{id}/items` (rows hold `item`) and lists only playlists the user owns or collaborates on; `/artists/{id}/top-tracks` is gone; `/me` no longer returns `product`, so `SpotifyAccount.premium` is `Option<bool>`, learned from a playback answer; search returns at most 10 per type. Commit `6f77f27` adapted Tasks 1, 7, 8 and 9 (their code blocks below are the original plan); Tasks 12 and 13 below already include the change.

**Conventions for every task:**
- Run commands from the repo root `/home/eugene/projects/znicz`.
- Commit messages end with the line `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.
- Never chain `git commit … && git push`.

---

## File structure

| File | Responsibility |
| --- | --- |
| `znicz-core/src/spotify_host.rs` (new) | Shared types and the `SpotifyHost` trait |
| `znicz-core/src/error.rs` | `ZniczError::Spotify` |
| `znicz-core/src/lib.rs` | Exports |
| `Cargo.toml` (root) | Workspace member and new workspace deps |
| `znicz-spotify/Cargo.toml` (new) | Crate manifest |
| `znicz-spotify/src/lib.rs` | Module list and public exports |
| `znicz-spotify/src/error.rs` | `SpotifyError` with the user-facing messages |
| `znicz-spotify/src/pkce.rs` | Verifier, challenge, `state` |
| `znicz-spotify/src/redirect.rs` | Parse a pasted redirect address or callback query |
| `znicz-spotify/src/token.rs` | `Token`, its file (atomic, `0600`), default path |
| `znicz-spotify/src/http.rs` | `Http` transport, status → `SpotifyError`, test server |
| `znicz-spotify/src/auth.rs` | Authorize URL, code exchange, refresh |
| `znicz-spotify/src/model.rs` | Web API JSON → core types |
| `znicz-spotify/src/api.rs` | One method per Web API endpoint |
| `znicz-spotify/src/client.rs` | Config, token, login session, refresh-and-retry |
| `znicz-spotify/src/listener.rs` | One-shot `127.0.0.1` callback listener |
| `znicz-spotify/src/service.rs` | `SpotifyService`: worker thread, polling, `SpotifyHost` |
| `wiki/Architecture/Spotify.md` (new), `wiki/Architecture/Overview.md`, `wiki/_Sidebar.md` | Docs |

---

### Task 1: Shared Spotify types in `znicz-core`

**Files:**
- Create: `znicz-core/src/spotify_host.rs`
- Modify: `znicz-core/src/error.rs`, `znicz-core/src/lib.rs`

- [x] **Step 1: Add the error variant**

In `znicz-core/src/error.rs`, after the `Library(String)` variant, add:

```rust
    /// A Spotify request failed; the message is shown as is.
    #[error("{0}")]
    Spotify(String),
```

- [x] **Step 2: Write `spotify_host.rs` with its tests**

Create `znicz-core/src/spotify_host.rs`:

```rust
//! What a Spotify controller offers to clients: the operations, the snapshot
//! they read back, and the trait both the in-process service and the IPC
//! client implement. The Web API code lives in `znicz-spotify`.

use serde::{Deserialize, Serialize};

use crate::error::Result;

/// Where the login stands.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", content = "reason", rename_all = "snake_case")]
pub enum SpotifyLogin {
    /// No client ID is set.
    #[default]
    NotConfigured,
    NotLoggedIn,
    /// A login was started and waits for the redirect.
    LoggingIn,
    LoggedIn,
    /// The last login failed; the reason is shown to the user.
    Failed(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpotifyAccount {
    /// Display name, else the user id.
    pub user: String,
    pub premium: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpotifyDevice {
    pub id: String,
    pub name: String,
    /// Spotify's device type: `Computer`, `Smartphone`, `Speaker`, …
    pub kind: String,
    pub is_active: bool,
    pub volume_percent: Option<u8>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpotifyTrack {
    pub uri: String,
    pub title: String,
    pub artists: Vec<String>,
    pub album: String,
    pub album_uri: String,
    pub year: Option<u16>,
    pub track_number: Option<u32>,
    pub duration_ms: u64,
    /// The largest album image up to 640 px wide.
    pub image_url: Option<String>,
}

impl SpotifyTrack {
    pub fn artist_line(&self) -> String {
        self.artists.join(", ")
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpotifyRepeat {
    #[default]
    Off,
    /// Repeat the current track.
    Track,
    /// Repeat the album or playlist.
    Context,
}

impl SpotifyRepeat {
    pub fn as_api(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Track => "track",
            Self::Context => "context",
        }
    }

    pub fn from_api(text: &str) -> Self {
        match text {
            "track" => Self::Track,
            "context" => Self::Context,
            _ => Self::Off,
        }
    }

    /// The order Znicz's `e` key cycles: off, all, one.
    pub fn next(self) -> Self {
        match self {
            Self::Off => Self::Context,
            Self::Context => Self::Track,
            Self::Track => Self::Off,
        }
    }
}

/// What Spotify is playing, as read at `read_at_ms`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpotifyPlayback {
    pub item: Option<SpotifyTrack>,
    /// Something plays that Znicz does not show (an episode, an ad).
    pub unsupported: bool,
    pub is_playing: bool,
    pub progress_ms: u64,
    /// Unix time in milliseconds when `progress_ms` was read.
    pub read_at_ms: u64,
    pub repeat: SpotifyRepeat,
    pub shuffle: bool,
    pub device: Option<SpotifyDevice>,
    pub context_uri: Option<String>,
}

impl SpotifyPlayback {
    /// The position at `now_ms`, moved on from the last read while playing
    /// and capped at the track length.
    pub fn progress_at(&self, now_ms: u64) -> u64 {
        if !self.is_playing {
            return self.progress_ms;
        }
        let moved = self.progress_ms + now_ms.saturating_sub(self.read_at_ms);
        match &self.item {
            Some(track) if track.duration_ms > 0 => moved.min(track.duration_ms),
            _ => moved,
        }
    }
}

/// Everything a front end can see about Spotify.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpotifyState {
    pub login: SpotifyLogin,
    pub account: Option<SpotifyAccount>,
    pub playback: Option<SpotifyPlayback>,
    pub devices: Vec<SpotifyDevice>,
    pub last_error: Option<String>,
}

impl SpotifyState {
    pub fn is_playing(&self) -> bool {
        self.playback.as_ref().is_some_and(|p| p.is_playing)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpotifyKind {
    Track,
    Album,
    Artist,
    Playlist,
}

impl SpotifyKind {
    pub fn as_api(self) -> &'static str {
        match self {
            Self::Track => "track",
            Self::Album => "album",
            Self::Artist => "artist",
            Self::Playlist => "playlist",
        }
    }
}

/// One row of a browse or search result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpotifyItem {
    pub kind: SpotifyKind,
    pub uri: String,
    pub name: String,
    /// Artists for a track or album, the owner for a playlist, empty for an artist.
    pub subtitle: String,
    pub image_url: Option<String>,
    /// Full details when `kind` is `Track`.
    pub track: Option<SpotifyTrack>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpotifyPage {
    pub items: Vec<SpotifyItem>,
    pub offset: u32,
    pub total: u32,
}

impl SpotifyPage {
    pub fn has_more(&self) -> bool {
        (self.offset as usize + self.items.len()) < self.total as usize
    }
}

/// The player-side settings: everything the service needs to log in and play.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpotifyConfig {
    pub client_id: String,
    pub redirect_port: u16,
    /// Preferred device name, used when nothing is active. Empty: none.
    pub device: String,
}

impl SpotifyConfig {
    pub const DEFAULT_PORT: u16 = 8898;

    pub fn redirect_uri(&self) -> String {
        format!("http://127.0.0.1:{}/callback", self.redirect_port)
    }

    pub fn is_configured(&self) -> bool {
        !self.client_id.is_empty()
    }
}

impl Default for SpotifyConfig {
    fn default() -> Self {
        Self {
            client_id: String::new(),
            redirect_port: Self::DEFAULT_PORT,
            device: String::new(),
        }
    }
}

/// A Spotify client ID is 32 hex characters.
pub fn valid_client_id(text: &str) -> bool {
    text.len() == 32 && text.bytes().all(|b| b.is_ascii_hexdigit())
}

/// A request to the Spotify service.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum SpotifyOp {
    Configure { config: SpotifyConfig },
    BeginLogin,
    /// Finish a login from a pasted redirect address (or its query).
    CompleteLogin { redirect_url: String },
    CancelLogin,
    Logout,
    /// Keeps the fast poll going for another minute.
    Touch,
    Search { query: String, kinds: Vec<SpotifyKind>, limit: u32 },
    Playlists { offset: u32, limit: u32 },
    LikedSongs { offset: u32, limit: u32 },
    Albums { offset: u32, limit: u32 },
    /// An album's or playlist's tracks, or an artist's top tracks.
    Tracks { uri: String, offset: u32, limit: u32 },
    ArtistAlbums { uri: String, offset: u32, limit: u32 },
    Devices,
    Queue,
    /// `context_uri` (+ `offset`) plays an album or playlist from a track;
    /// `uris` (+ `offset`) plays those tracks; neither resumes.
    Play {
        context_uri: Option<String>,
        uris: Vec<String>,
        offset: Option<u32>,
    },
    Pause,
    Next,
    Previous,
    Seek { position_ms: u64 },
    /// 0.0–1.0, like the local volume.
    SetVolume { volume: f32 },
    SetRepeat { mode: SpotifyRepeat },
    SetShuffle { on: bool },
    QueueAdd { uri: String },
    Transfer { device_id: String, play: bool },
}

impl SpotifyOp {
    /// Whether this op starts or resumes Spotify playback, so the player
    /// pauses its own engine first.
    pub fn starts_playback(&self) -> bool {
        matches!(self, Self::Play { .. } | Self::Transfer { play: true, .. })
    }

    /// Whether this op only reads. Reads need no change id over IPC.
    pub fn is_read(&self) -> bool {
        matches!(
            self,
            Self::Touch
                | Self::Search { .. }
                | Self::Playlists { .. }
                | Self::LikedSongs { .. }
                | Self::Albums { .. }
                | Self::Tracks { .. }
                | Self::ArtistAlbums { .. }
                | Self::Devices
                | Self::Queue
        )
    }
}

/// The answer to a `SpotifyOp`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "reply", rename_all = "snake_case")]
pub enum SpotifyReply {
    Done,
    AuthorizeUrl { url: String },
    Page { page: SpotifyPage },
    Devices { devices: Vec<SpotifyDevice> },
    Queue {
        current: Option<SpotifyTrack>,
        next: Vec<SpotifyTrack>,
    },
}

/// Implemented by the in-process `SpotifyService` and by the IPC client.
pub trait SpotifyHost: Send + Sync {
    fn apply(&self, op: SpotifyOp) -> Result<SpotifyReply>;
    fn state(&self) -> SpotifyState;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track(duration_ms: u64) -> SpotifyTrack {
        SpotifyTrack {
            duration_ms,
            ..SpotifyTrack::default()
        }
    }

    #[test]
    fn progress_moves_on_while_playing_and_stops_at_the_end() {
        let playback = SpotifyPlayback {
            item: Some(track(10_000)),
            is_playing: true,
            progress_ms: 4_000,
            read_at_ms: 1_000,
            ..SpotifyPlayback::default()
        };
        assert_eq!(playback.progress_at(3_500), 6_500);
        assert_eq!(playback.progress_at(60_000), 10_000);
        assert_eq!(playback.progress_at(500), 4_000, "a clock behind the read");
    }

    #[test]
    fn progress_holds_while_paused() {
        let playback = SpotifyPlayback {
            item: Some(track(10_000)),
            is_playing: false,
            progress_ms: 4_000,
            read_at_ms: 1_000,
            ..SpotifyPlayback::default()
        };
        assert_eq!(playback.progress_at(9_000), 4_000);
    }

    #[test]
    fn repeat_cycles_off_all_one() {
        assert_eq!(SpotifyRepeat::Off.next(), SpotifyRepeat::Context);
        assert_eq!(SpotifyRepeat::Context.next(), SpotifyRepeat::Track);
        assert_eq!(SpotifyRepeat::Track.next(), SpotifyRepeat::Off);
        assert_eq!(SpotifyRepeat::from_api("context"), SpotifyRepeat::Context);
        assert_eq!(SpotifyRepeat::from_api("nonsense"), SpotifyRepeat::Off);
    }

    #[test]
    fn client_ids_are_32_hex_characters() {
        assert!(valid_client_id("0123456789abcdef0123456789ABCDEF"));
        assert!(!valid_client_id("0123456789abcdef"));
        assert!(!valid_client_id("0123456789abcdef0123456789abcdeg"));
    }

    #[test]
    fn redirect_uri_uses_the_port() {
        let config = SpotifyConfig {
            redirect_port: 9000,
            ..SpotifyConfig::default()
        };
        assert_eq!(config.redirect_uri(), "http://127.0.0.1:9000/callback");
        assert!(!config.is_configured());
    }

    #[test]
    fn play_and_transfer_with_play_start_playback() {
        let play = SpotifyOp::Play {
            context_uri: None,
            uris: vec![],
            offset: None,
        };
        assert!(play.starts_playback());
        assert!(
            SpotifyOp::Transfer {
                device_id: "d".into(),
                play: true
            }
            .starts_playback()
        );
        assert!(
            !SpotifyOp::Transfer {
                device_id: "d".into(),
                play: false
            }
            .starts_playback()
        );
        assert!(!SpotifyOp::Pause.starts_playback());
    }

    #[test]
    fn reads_are_told_apart_from_changes() {
        assert!(SpotifyOp::Devices.is_read());
        assert!(SpotifyOp::Touch.is_read());
        assert!(!SpotifyOp::Next.is_read());
        assert!(!SpotifyOp::Logout.is_read());
    }

    #[test]
    fn ops_and_replies_round_trip_as_json() {
        let op = SpotifyOp::Search {
            query: "kind of blue".into(),
            kinds: vec![SpotifyKind::Album, SpotifyKind::Track],
            limit: 10,
        };
        let text = serde_json::to_string(&op).unwrap();
        assert!(text.contains(r#""op":"search""#), "{text}");
        assert_eq!(serde_json::from_str::<SpotifyOp>(&text).unwrap(), op);

        let reply = SpotifyReply::AuthorizeUrl { url: "u".into() };
        let text = serde_json::to_string(&reply).unwrap();
        assert_eq!(serde_json::from_str::<SpotifyReply>(&text).unwrap(), reply);

        let state = SpotifyState {
            login: SpotifyLogin::Failed("nope".into()),
            ..SpotifyState::default()
        };
        let text = serde_json::to_string(&state).unwrap();
        assert!(text.contains(r#""state":"failed""#), "{text}");
        assert_eq!(serde_json::from_str::<SpotifyState>(&text).unwrap(), state);
    }

    #[test]
    fn pages_know_whether_more_rows_exist() {
        let item = SpotifyItem {
            kind: SpotifyKind::Track,
            uri: "spotify:track:1".into(),
            name: "x".into(),
            subtitle: String::new(),
            image_url: None,
            track: None,
        };
        let page = SpotifyPage {
            items: vec![item.clone(), item],
            offset: 0,
            total: 3,
        };
        assert!(page.has_more());
        let last = SpotifyPage { offset: 1, ..page };
        assert!(!last.has_more());
    }
}
```

- [x] **Step 3: Export the module**

In `znicz-core/src/lib.rs`, add `pub mod spotify_host;` after `pub mod session;`, and after the `pub use session::{…};` block add:

```rust
pub use spotify_host::{
    SpotifyAccount, SpotifyConfig, SpotifyDevice, SpotifyHost, SpotifyItem, SpotifyKind,
    SpotifyLogin, SpotifyOp, SpotifyPage, SpotifyPlayback, SpotifyRepeat, SpotifyReply,
    SpotifyState, SpotifyTrack, valid_client_id,
};
```

- [x] **Step 4: Run the tests**

Run: `cargo test -p znicz-core spotify_host`
Expected: 9 tests pass.

- [x] **Step 5: Commit**

```bash
git add znicz-core/src/spotify_host.rs znicz-core/src/error.rs znicz-core/src/lib.rs
git commit -m "feat(core): Spotify ops, state and the SpotifyHost trait"
```

---

### Task 2: Crate scaffold and `SpotifyError`

**Files:**
- Modify: `Cargo.toml` (root)
- Create: `znicz-spotify/Cargo.toml`, `znicz-spotify/src/lib.rs`, `znicz-spotify/src/error.rs`

- [x] **Step 1: Workspace entries**

In the root `Cargo.toml`, change `members` to:

```toml
members = ["znicz-core", "znicz-library", "znicz-spotify", "znicz-tui", "znicz-mcp", "znicz"]
```

and add to `[workspace.dependencies]` after `ureq = "3"`:

```toml
sha2 = "0.10"
base64 = "0.22"
getrandom = "0.3"
percent-encoding = "2"
```

- [x] **Step 2: Crate manifest**

Create `znicz-spotify/Cargo.toml`:

```toml
[package]
name = "znicz-spotify"
version.workspace = true
edition.workspace = true
license.workspace = true
authors.workspace = true
rust-version.workspace = true
repository.workspace = true
publish = false

[dependencies]
znicz-core = { path = "../znicz-core" }
ureq.workspace = true
serde.workspace = true
serde_json.workspace = true
thiserror.workspace = true
tracing.workspace = true
sha2.workspace = true
base64.workspace = true
getrandom.workspace = true
percent-encoding.workspace = true
dirs.workspace = true

[dev-dependencies]
tempfile.workspace = true
```

- [x] **Step 3: Write `error.rs` with tests**

Create `znicz-spotify/src/error.rs`:

```rust
//! Every way a Spotify request can fail, worded for the user.

use znicz_core::ZniczError;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SpotifyError {
    #[error("Spotify is not configured: set a client ID in Settings → Spotify")]
    NotConfigured,
    #[error("not logged in to Spotify")]
    NotLoggedIn,
    #[error("no Spotify login is in progress")]
    NoLogin,
    #[error("login expired, press Enter to start again")]
    LoginExpired,
    #[error("that address is not from this login")]
    WrongAddress,
    #[error("Spotify login was declined: {0}")]
    Declined(String),
    #[error(
        "Spotify refused the client ID or redirect URI: register exactly {redirect_uri} in your Spotify app"
    )]
    InvalidClient { redirect_uri: String },
    #[error("Spotify login expired")]
    Revoked,
    #[error("Spotify rejected the access token")]
    Unauthorized,
    #[error("needs Spotify Premium")]
    PremiumRequired,
    #[error("Spotify has no active device: open Spotify somewhere, or pick one with ,")]
    NoActiveDevice,
    #[error("Spotify is rate limiting requests; try again in {retry_after_secs} s")]
    RateLimited { retry_after_secs: u64 },
    #[error("Spotify error {status}: {message}")]
    Http { status: u16, message: String },
    #[error("could not reach Spotify: {0}")]
    Network(String),
    #[error("unexpected Spotify response: {0}")]
    Parse(String),
    #[error("{0}")]
    Io(String),
}

pub type Result<T> = std::result::Result<T, SpotifyError>;

impl From<SpotifyError> for ZniczError {
    fn from(error: SpotifyError) -> Self {
        ZniczError::Spotify(error.to_string())
    }
}

impl From<std::io::Error> for SpotifyError {
    fn from(error: std::io::Error) -> Self {
        SpotifyError::Io(error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_match_the_spec() {
        assert_eq!(
            SpotifyError::NotConfigured.to_string(),
            "Spotify is not configured: set a client ID in Settings → Spotify"
        );
        assert_eq!(SpotifyError::PremiumRequired.to_string(), "needs Spotify Premium");
        assert_eq!(
            SpotifyError::InvalidClient {
                redirect_uri: "http://127.0.0.1:8898/callback".into()
            }
            .to_string(),
            "Spotify refused the client ID or redirect URI: register exactly \
             http://127.0.0.1:8898/callback in your Spotify app"
        );
    }

    #[test]
    fn converts_to_a_core_error_with_the_same_text() {
        let core: ZniczError = SpotifyError::NoActiveDevice.into();
        assert_eq!(core.to_string(), SpotifyError::NoActiveDevice.to_string());
    }
}
```

- [x] **Step 4: `lib.rs`**

Create `znicz-spotify/src/lib.rs`:

```rust
//! Spotify control through the Web API: login (OAuth PKCE), browse and
//! playback calls, and a polling service the player process hosts.
//!
//! Spotify plays the audio; Znicz only sends commands and reads state.

mod error;

pub use error::{Result, SpotifyError};
```

Later tasks each add one `mod` line here; the final list is in Task 12.

- [x] **Step 5: Build and test**

Run: `cargo test -p znicz-spotify`
Expected: 2 tests pass. `Cargo.lock` gains the crate (sha2, base64, getrandom and percent-encoding are already in the lock file through other crates).

- [x] **Step 6: Commit**

```bash
git add Cargo.toml Cargo.lock znicz-spotify
git commit -m "feat(spotify): crate scaffold and SpotifyError"
```

---

### Task 3: PKCE

**Files:**
- Create: `znicz-spotify/src/pkce.rs`
- Modify: `znicz-spotify/src/lib.rs` (add `mod pkce;`)

- [x] **Step 1: Write the failing tests**

Create `znicz-spotify/src/pkce.rs` with only the tests:

```rust
//! PKCE (RFC 7636): a random verifier, its S256 challenge, and a random
//! `state` that ties a redirect to the login that asked for it.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn challenge_matches_rfc_7636_appendix_b() {
        assert_eq!(
            challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    #[test]
    fn verifiers_are_64_unreserved_characters() {
        let verifier = random_verifier();
        assert_eq!(verifier.len(), 64);
        assert!(
            verifier
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'),
            "{verifier}"
        );
    }

    #[test]
    fn states_differ() {
        assert_ne!(random_state(), random_state());
    }
}
```

Add `mod pkce;` to `lib.rs`.

- [x] **Step 2: Run to see it fail**

Run: `cargo test -p znicz-spotify pkce`
Expected: compile error, `challenge`, `random_verifier`, `random_state` not found.

- [x] **Step 3: Implement**

Above the tests in `pkce.rs`:

```rust
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use sha2::{Digest, Sha256};

/// 48 random bytes in base64url: 64 characters, all unreserved.
pub(crate) fn random_verifier() -> String {
    URL_SAFE_NO_PAD.encode(random_bytes::<48>())
}

pub(crate) fn random_state() -> String {
    URL_SAFE_NO_PAD.encode(random_bytes::<16>())
}

/// The S256 challenge: base64url of the verifier's SHA-256, no padding.
pub(crate) fn challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

fn random_bytes<const N: usize>() -> [u8; N] {
    let mut bytes = [0u8; N];
    getrandom::fill(&mut bytes).expect("the OS random source is available");
    bytes
}
```

- [x] **Step 4: Run the tests**

Run: `cargo test -p znicz-spotify pkce`
Expected: 3 pass. (`cargo build` warns that these are unused outside tests until Task 10; that is fine.)

- [x] **Step 5: Commit**

```bash
git add znicz-spotify/src/pkce.rs znicz-spotify/src/lib.rs
git commit -m "feat(spotify): PKCE verifier, challenge and state"
```

---

### Task 4: Parse a redirect address

**Files:**
- Create: `znicz-spotify/src/redirect.rs`
- Modify: `znicz-spotify/src/lib.rs` (add `mod redirect;`)

- [x] **Step 1: Write the failing tests**

Create `znicz-spotify/src/redirect.rs`:

```rust
//! Turn what the user pasted (the address the browser landed on, or just its
//! query) into the authorization code, checking it belongs to this login.

use crate::error::{Result, SpotifyError};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_a_full_address() {
        let code = code_from(
            "http://127.0.0.1:8898/callback?code=AQBx-1_2&state=abc",
            "abc",
        );
        assert_eq!(code, Ok("AQBx-1_2".to_string()));
    }

    #[test]
    fn reads_a_bare_query_with_or_without_question_mark() {
        assert_eq!(code_from("code=c1&state=s", "s"), Ok("c1".to_string()));
        assert_eq!(code_from("?state=s&code=c2", "s"), Ok("c2".to_string()));
        assert_eq!(code_from("  code=c3&state=s\n", "s"), Ok("c3".to_string()));
    }

    #[test]
    fn decodes_percent_escapes_and_drops_a_fragment() {
        assert_eq!(
            code_from("http://x/callback?code=a%2Bb&state=s%3D#_", "s="),
            Ok("a+b".to_string())
        );
    }

    #[test]
    fn rejects_another_logins_state_or_a_missing_code() {
        assert_eq!(
            code_from("code=c&state=other", "s"),
            Err(SpotifyError::WrongAddress)
        );
        assert_eq!(code_from("state=s", "s"), Err(SpotifyError::WrongAddress));
        assert_eq!(
            code_from("http://127.0.0.1:8898/callback", "s"),
            Err(SpotifyError::WrongAddress)
        );
        assert_eq!(code_from("", "s"), Err(SpotifyError::WrongAddress));
    }

    #[test]
    fn reports_a_declined_login() {
        assert_eq!(
            code_from("error=access_denied&state=s", "s"),
            Err(SpotifyError::Declined("access_denied".into()))
        );
    }
}
```

Add `mod redirect;` to `lib.rs`.

- [x] **Step 2: Run to see it fail**

Run: `cargo test -p znicz-spotify redirect`
Expected: compile error, `code_from` not found.

- [x] **Step 3: Implement**

Between the `use` line and the tests:

```rust
use percent_encoding::percent_decode_str;

/// The code in `input`, when its `state` is `expected_state`.
pub(crate) fn code_from(input: &str, expected_state: &str) -> Result<String> {
    let input = input.trim();
    let query = match input.split_once('?') {
        Some((_, query)) => query,
        None => input,
    };
    let query = query.split('#').next().unwrap_or("");

    let mut code = None;
    let mut state = None;
    let mut error = None;
    for pair in query.split('&') {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        let value = decode(value);
        match key {
            "code" => code = Some(value),
            "state" => state = Some(value),
            "error" => error = Some(value),
            _ => {}
        }
    }
    if state.as_deref() != Some(expected_state) {
        return Err(SpotifyError::WrongAddress);
    }
    if let Some(error) = error {
        return Err(SpotifyError::Declined(error));
    }
    match code {
        Some(code) if !code.is_empty() => Ok(code),
        _ => Err(SpotifyError::WrongAddress),
    }
}

fn decode(value: &str) -> String {
    let spaced = value.replace('+', " ");
    percent_decode_str(&spaced).decode_utf8_lossy().into_owned()
}
```

- [x] **Step 4: Run the tests**

Run: `cargo test -p znicz-spotify redirect`
Expected: 5 pass.

- [x] **Step 5: Commit**

```bash
git add znicz-spotify/src/redirect.rs znicz-spotify/src/lib.rs
git commit -m "feat(spotify): read the code from a pasted redirect address"
```

---

### Task 5: The token and its file

**Files:**
- Create: `znicz-spotify/src/token.rs`
- Modify: `znicz-spotify/src/lib.rs` (add `mod token;` and `pub use token::default_token_path;`)

- [x] **Step 1: Write the failing tests**

Create `znicz-spotify/src/token.rs`:

```rust
//! The OAuth token and `spotify_token.json`: written atomically, readable by
//! the owner only, and never printed.

use std::fmt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::Result;

#[cfg(test)]
mod tests {
    use super::*;

    fn token() -> Token {
        Token {
            access_token: "secret-access".into(),
            refresh_token: "secret-refresh".into(),
            expires_at: 1_000,
            scope: "user-read-private".into(),
        }
    }

    #[test]
    fn saves_and_loads() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("spotify_token.json");
        save(&path, &token()).unwrap();
        assert_eq!(load(&path).unwrap(), Some(token()));
    }

    #[cfg(unix)]
    #[test]
    fn the_file_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("spotify_token.json");
        save(&path, &token()).unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    #[test]
    fn a_missing_or_corrupt_file_means_logged_out() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("spotify_token.json");
        assert_eq!(load(&path).unwrap(), None);
        std::fs::write(&path, "{ not json").unwrap();
        assert_eq!(load(&path).unwrap(), None);
    }

    #[test]
    fn remove_ignores_a_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("spotify_token.json");
        remove(&path).unwrap();
        save(&path, &token()).unwrap();
        remove(&path).unwrap();
        assert!(!path.exists());
    }

    #[test]
    fn debug_hides_the_secrets() {
        let text = format!("{:?}", token());
        assert!(!text.contains("secret"), "{text}");
        assert!(text.contains("expires_at"), "{text}");
    }

    #[test]
    fn refreshes_a_minute_before_expiry() {
        let token = token();
        assert!(!token.needs_refresh(939));
        assert!(token.needs_refresh(940));
        assert!(token.needs_refresh(2_000));
    }
}
```

Add `mod token;` and `pub use token::default_token_path;` to `lib.rs`.

- [x] **Step 2: Run to see it fail**

Run: `cargo test -p znicz-spotify token`
Expected: compile errors (`Token`, `save`, `load`, `remove` missing).

- [x] **Step 3: Implement**

Between the `use` lines and the tests:

```rust
/// Refresh this many seconds before the access token expires.
const REFRESH_MARGIN_SECS: u64 = 60;

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Token {
    pub access_token: String,
    pub refresh_token: String,
    /// Unix seconds.
    pub expires_at: u64,
    #[serde(default)]
    pub scope: String,
}

impl Token {
    pub(crate) fn needs_refresh(&self, now_secs: u64) -> bool {
        now_secs + REFRESH_MARGIN_SECS >= self.expires_at
    }
}

impl fmt::Debug for Token {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Token")
            .field("access_token", &"<redacted>")
            .field("refresh_token", &"<redacted>")
            .field("expires_at", &self.expires_at)
            .field("scope", &self.scope)
            .finish()
    }
}

/// `spotify_token.json` in the data folder, unless `ZNICZ_SPOTIFY_TOKEN_PATH` is set.
pub fn default_token_path() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("ZNICZ_SPOTIFY_TOKEN_PATH") {
        if !path.is_empty() {
            return Some(PathBuf::from(path));
        }
    }
    dirs::data_dir().map(|dir| dir.join("znicz").join("spotify_token.json"))
}

/// `None` when there is no file, or it cannot be parsed (logged once).
pub(crate) fn load(path: &Path) -> Result<Option<Token>> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    match serde_json::from_str(&text) {
        Ok(token) => Ok(Some(token)),
        Err(e) => {
            tracing::warn!(path = %path.display(), error = %e, "Spotify token file unreadable; logged out");
            Ok(None)
        }
    }
}

/// Write to a temporary file with mode `0600`, then rename it over `path`.
pub(crate) fn save(path: &Path, token: &Token) -> Result<()> {
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir)?;
    }
    let text = serde_json::to_string_pretty(token)
        .map_err(|e| crate::error::SpotifyError::Io(e.to_string()))?;
    let mut tmp_name = path.file_name().unwrap_or_default().to_os_string();
    tmp_name.push(".tmp");
    let tmp = path.with_file_name(tmp_name);
    // `mode(0o600)` applies only on create, so never reuse a `.tmp` left by a crash.
    let _ = std::fs::remove_file(&tmp);
    let result = write_private(&tmp, text.as_bytes()).and_then(|()| std::fs::rename(&tmp, path));
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    Ok(result?)
}

pub(crate) fn remove(path: &Path) -> Result<()> {
    match std::fs::remove_file(path) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.into()),
        _ => Ok(()),
    }
}

fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    file.write_all(bytes)?;
    file.sync_all()
}
```

- [x] **Step 4: Run the tests**

Run: `cargo test -p znicz-spotify token`
Expected: 6 pass (5 on Windows).

- [x] **Step 5: Commit**

```bash
git add znicz-spotify/src/token.rs znicz-spotify/src/lib.rs
git commit -m "feat(spotify): token file, owner-only and redacted"
```

---

### Task 6: HTTP transport, error mapping and a test server

**Files:**
- Create: `znicz-spotify/src/http.rs`
- Modify: `znicz-spotify/src/lib.rs` (add `mod http;`)

- [x] **Step 1: Write the module with the test server and failing tests**

Create `znicz-spotify/src/http.rs`:

```rust
//! Blocking HTTP for the Web API and the accounts service, and how a
//! response's status becomes a `SpotifyError`.

use std::time::Duration;

use serde_json::Value;

use crate::error::{Result, SpotifyError};

pub(crate) const API_BASE: &str = "https://api.spotify.com/v1";
pub(crate) const ACCOUNTS_BASE: &str = "https://accounts.spotify.com";

/// Without `Retry-After`, wait this long after a 429.
const DEFAULT_RETRY_AFTER_SECS: u64 = 5;

const USER_AGENT: &str = concat!(
    "znicz/",
    env!("CARGO_PKG_VERSION"),
    " ( https://github.com/eugene-chekan/znicz )"
);

#[cfg(test)]
pub(crate) mod test_server {
    //! A tiny HTTP/1.1 server: one thread, one request per connection.

    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::sync::{Arc, Mutex};
    use std::thread;

    #[derive(Debug, Clone)]
    pub(crate) struct Request {
        pub method: String,
        /// Path and query, as sent.
        pub target: String,
        pub headers: Vec<(String, String)>,
        pub body: String,
    }

    impl Request {
        pub(crate) fn path(&self) -> &str {
            self.target.split('?').next().unwrap_or("")
        }

        pub(crate) fn query(&self, key: &str) -> Option<String> {
            let query = self.target.split_once('?')?.1;
            query.split('&').find_map(|pair| {
                let (k, v) = pair.split_once('=')?;
                (k == key).then(|| {
                    percent_encoding::percent_decode_str(v)
                        .decode_utf8_lossy()
                        .into_owned()
                })
            })
        }

        pub(crate) fn header(&self, name: &str) -> Option<&str> {
            self.headers
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(name))
                .map(|(_, v)| v.as_str())
        }
    }

    pub(crate) struct Response {
        pub status: u16,
        pub headers: Vec<(String, String)>,
        pub body: String,
    }

    impl Response {
        pub(crate) fn json(status: u16, body: &str) -> Self {
            Self {
                status,
                headers: vec![("Content-Type".into(), "application/json".into())],
                body: body.into(),
            }
        }

        pub(crate) fn empty(status: u16) -> Self {
            Self {
                status,
                headers: vec![],
                body: String::new(),
            }
        }

        pub(crate) fn with_header(mut self, name: &str, value: &str) -> Self {
            self.headers.push((name.into(), value.into()));
            self
        }
    }

    pub(crate) type Seen = Arc<Mutex<Vec<Request>>>;

    /// Serves every request with `handler`; returns the origin
    /// (`http://127.0.0.1:port`) and the requests seen so far.
    pub(crate) fn serve<F>(handler: F) -> (String, Seen)
    where
        F: Fn(&Request) -> Response + Send + 'static,
    {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let seen: Seen = Arc::new(Mutex::new(Vec::new()));
        let log = seen.clone();
        thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                if let Some(request) = read_request(&stream) {
                    log.lock().unwrap().push(request.clone());
                    write_response(stream, handler(&request));
                }
            }
        });
        (origin, seen)
    }

    pub(crate) fn read_request(mut stream: &TcpStream) -> Option<Request> {
        let mut data = Vec::new();
        let mut buf = [0u8; 4096];
        let head_end = loop {
            let n = stream.read(&mut buf).ok()?;
            if n == 0 {
                return None;
            }
            data.extend_from_slice(&buf[..n]);
            if let Some(i) = data.windows(4).position(|w| w == b"\r\n\r\n") {
                break i + 4;
            }
        };
        let head = String::from_utf8_lossy(&data[..head_end]).into_owned();
        let mut lines = head.split("\r\n");
        let mut first = lines.next()?.split(' ');
        let method = first.next()?.to_string();
        let target = first.next()?.to_string();
        let headers: Vec<(String, String)> = lines
            .filter_map(|line| {
                let (k, v) = line.split_once(':')?;
                Some((k.trim().to_string(), v.trim().to_string()))
            })
            .collect();
        let length = headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case("content-length"))
            .and_then(|(_, v)| v.parse::<usize>().ok())
            .unwrap_or(0);
        while data.len() < head_end + length {
            let n = stream.read(&mut buf).ok()?;
            if n == 0 {
                break;
            }
            data.extend_from_slice(&buf[..n]);
        }
        let body = String::from_utf8_lossy(&data[head_end..]).into_owned();
        Some(Request {
            method,
            target,
            headers,
            body,
        })
    }

    fn write_response(mut stream: TcpStream, response: Response) {
        let mut head = format!(
            "HTTP/1.1 {} X\r\nContent-Length: {}\r\nConnection: close\r\n",
            response.status,
            response.body.len()
        );
        for (name, value) in &response.headers {
            head.push_str(&format!("{name}: {value}\r\n"));
        }
        head.push_str("\r\n");
        let _ = stream.write_all(head.as_bytes());
        let _ = stream.write_all(response.body.as_bytes());
    }
}

#[cfg(test)]
mod tests {
    use super::test_server::{Response, serve};
    use super::*;

    fn reply(status: u16, body: &str) -> Reply {
        Reply {
            status,
            body: body.into(),
            retry_after: None,
        }
    }

    #[test]
    fn success_passes_through() {
        assert_eq!(check(reply(200, "{}")).unwrap().body, "{}");
        assert_eq!(check(reply(204, "")).unwrap().status, 204);
    }

    #[test]
    fn statuses_become_errors() {
        assert_eq!(check(reply(401, "")).unwrap_err(), SpotifyError::Unauthorized);
        let premium = r#"{"error":{"status":403,"message":"Player command failed: Premium required","reason":"PREMIUM_REQUIRED"}}"#;
        assert_eq!(check(reply(403, premium)).unwrap_err(), SpotifyError::PremiumRequired);
        let device = r#"{"error":{"status":404,"message":"Player command failed: No active device found","reason":"NO_ACTIVE_DEVICE"}}"#;
        assert_eq!(check(reply(404, device)).unwrap_err(), SpotifyError::NoActiveDevice);
        let limited = Reply {
            retry_after: Some(7),
            ..reply(429, "")
        };
        assert_eq!(
            check(limited).unwrap_err(),
            SpotifyError::RateLimited { retry_after_secs: 7 }
        );
        assert_eq!(
            check(reply(429, "")).unwrap_err(),
            SpotifyError::RateLimited { retry_after_secs: 5 }
        );
        let other = r#"{"error":{"status":500,"message":"Server error"}}"#;
        assert_eq!(
            check(reply(500, other)).unwrap_err(),
            SpotifyError::Http {
                status: 500,
                message: "Server error".into()
            }
        );
    }

    #[test]
    fn sends_bearer_query_and_reads_the_body() {
        let (origin, seen) = serve(|_| Response::json(200, r#"{"ok":true}"#));
        let http = Http::new();
        let reply = http
            .send(
                Method::Get,
                &format!("{origin}/v1/me"),
                &[("limit", "5")],
                Some("tok"),
                Body::None,
            )
            .unwrap();
        assert_eq!(reply.status, 200);
        assert_eq!(reply.body, r#"{"ok":true}"#);
        let request = seen.lock().unwrap()[0].clone();
        assert_eq!(request.method, "GET");
        assert_eq!(request.path(), "/v1/me");
        assert_eq!(request.query("limit").as_deref(), Some("5"));
        assert_eq!(request.header("authorization"), Some("Bearer tok"));
    }

    #[test]
    fn sends_json_form_and_empty_bodies() {
        let (origin, seen) = serve(|_| Response::empty(204));
        let http = Http::new();
        let json = serde_json::json!({"device_ids": ["d1"]});
        http.send(Method::Put, &format!("{origin}/a"), &[], None, Body::Json(&json))
            .unwrap();
        http.send(
            Method::Post,
            &format!("{origin}/b"),
            &[],
            None,
            Body::Form(&[("grant_type", "refresh_token")]),
        )
        .unwrap();
        http.send(Method::Put, &format!("{origin}/c"), &[], None, Body::None)
            .unwrap();
        let seen = seen.lock().unwrap();
        assert_eq!(seen[0].body, r#"{"device_ids":["d1"]}"#);
        assert_eq!(seen[0].header("content-type"), Some("application/json"));
        assert_eq!(seen[1].method, "POST");
        assert_eq!(seen[1].body, "grant_type=refresh_token");
        assert_eq!(seen[2].header("content-length"), Some("0"));
    }

    #[test]
    fn reads_retry_after() {
        let (origin, _) =
            serve(|_| Response::empty(429).with_header("Retry-After", "3"));
        let reply = Http::new()
            .send(Method::Get, &format!("{origin}/x"), &[], None, Body::None)
            .unwrap();
        assert_eq!(reply.status, 429);
        assert_eq!(reply.retry_after, Some(3));
    }

    #[test]
    fn a_closed_port_is_a_network_error() {
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let error = Http::new()
            .send(
                Method::Get,
                &format!("http://127.0.0.1:{port}/x"),
                &[],
                None,
                Body::None,
            )
            .unwrap_err();
        assert!(matches!(error, SpotifyError::Network(_)), "{error:?}");
    }
}
```

Add `mod http;` to `lib.rs`.

- [x] **Step 2: Run to see it fail**

Run: `cargo test -p znicz-spotify http`
Expected: compile errors (`Http`, `Reply`, `Method`, `Body`, `check` missing).

- [x] **Step 3: Implement**

Between `USER_AGENT` and `mod test_server`:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Method {
    Get,
    Put,
    Post,
}

pub(crate) enum Body<'a> {
    None,
    Json(&'a Value),
    Form(&'a [(&'a str, &'a str)]),
}

/// A response read in full. Every status arrives here; `check` sorts them.
#[derive(Debug)]
pub(crate) struct Reply {
    pub status: u16,
    pub body: String,
    pub retry_after: Option<u64>,
}

pub(crate) struct Http {
    agent: ureq::Agent,
}

impl Http {
    pub(crate) fn new() -> Self {
        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(10)))
            .user_agent(USER_AGENT)
            .http_status_as_error(false)
            .build()
            .into();
        Self { agent }
    }

    pub(crate) fn send(
        &self,
        method: Method,
        url: &str,
        query: &[(&str, &str)],
        bearer: Option<&str>,
        body: Body<'_>,
    ) -> Result<Reply> {
        let auth = bearer.map(|token| format!("Bearer {token}"));
        let result = match method {
            Method::Get => {
                let mut request = self.agent.get(url).query_pairs(query.iter().copied());
                if let Some(auth) = &auth {
                    request = request.header("Authorization", auth);
                }
                request.call()
            }
            Method::Put | Method::Post => {
                let request = if method == Method::Put {
                    self.agent.put(url)
                } else {
                    self.agent.post(url)
                };
                let mut request = request.query_pairs(query.iter().copied());
                if let Some(auth) = &auth {
                    request = request.header("Authorization", auth);
                }
                match body {
                    Body::None => request.send(""),
                    Body::Json(value) => request
                        .header("Content-Type", "application/json")
                        .send(value.to_string()),
                    Body::Form(pairs) => request.send_form(pairs.iter().copied()),
                }
            }
        };
        let response = result.map_err(|e| SpotifyError::Network(e.to_string()))?;
        let status = response.status().as_u16();
        let retry_after = response
            .headers()
            .get("retry-after")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.trim().parse().ok());
        let body = response
            .into_body()
            .read_to_string()
            .map_err(|e| SpotifyError::Network(e.to_string()))?;
        Ok(Reply {
            status,
            body,
            retry_after,
        })
    }
}

/// Pass a 2xx through; turn anything else into the matching error.
pub(crate) fn check(reply: Reply) -> Result<Reply> {
    if (200..300).contains(&reply.status) {
        return Ok(reply);
    }
    let (message, reason) = error_detail(&reply.body);
    Err(match reply.status {
        401 => SpotifyError::Unauthorized,
        403 if reason == "PREMIUM_REQUIRED" || message.contains("Premium") => {
            SpotifyError::PremiumRequired
        }
        404 if reason == "NO_ACTIVE_DEVICE" || message.contains("No active device") => {
            SpotifyError::NoActiveDevice
        }
        429 => SpotifyError::RateLimited {
            retry_after_secs: reply.retry_after.unwrap_or(DEFAULT_RETRY_AFTER_SECS),
        },
        status => SpotifyError::Http { status, message },
    })
}

/// `{"error":{"message":…,"reason":…}}` → (message, reason).
fn error_detail(body: &str) -> (String, String) {
    let json: Value = serde_json::from_str(body).unwrap_or(Value::Null);
    let error = &json["error"];
    let message = error["message"].as_str().unwrap_or("").to_string();
    let reason = error["reason"].as_str().unwrap_or("").to_string();
    (message, reason)
}
```

If `read_to_string` does not resolve on the owned body, bind it first: `let mut body = response.into_body(); let body = body.read_to_string()…`. ureq 3.4 has both an owned and a `&mut` form.

- [x] **Step 4: Run the tests**

Run: `cargo test -p znicz-spotify http`
Expected: 6 pass. If `sends_json_form_and_empty_bodies` fails because the empty PUT has no `content-length: 0`, change `Body::None => request.send("")` to `request.header("Content-Length", "0").send_empty()` and rerun. Spotify answers 411 to a PUT or POST without a length.

- [x] **Step 5: Commit**

```bash
git add znicz-spotify/src/http.rs znicz-spotify/src/lib.rs
git commit -m "feat(spotify): HTTP transport and status-to-error mapping"
```

---

### Task 7: Authorize URL, code exchange and refresh

**Files:**
- Create: `znicz-spotify/src/auth.rs`
- Modify: `znicz-spotify/src/lib.rs` (add `mod auth;`)

- [x] **Step 1: Write the failing tests**

Create `znicz-spotify/src/auth.rs`:

```rust
//! The accounts service: where the user approves, and how a code or a
//! refresh token becomes an access token.

use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use serde_json::Value;

use crate::error::{Result, SpotifyError};
use crate::http::{Body, Http, Method, Reply};
use crate::token::Token;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::test_server::{Response, serve};

    #[test]
    fn authorize_url_carries_every_parameter() {
        let url = authorize_url(
            "https://accounts.spotify.com",
            "cid",
            "http://127.0.0.1:8898/callback",
            "chal",
            "st",
        );
        assert!(url.starts_with("https://accounts.spotify.com/authorize?"), "{url}");
        assert!(url.contains("client_id=cid"));
        assert!(url.contains("response_type=code"));
        assert!(url.contains("redirect_uri=http%3A%2F%2F127.0.0.1%3A8898%2Fcallback"));
        assert!(url.contains("code_challenge_method=S256"));
        assert!(url.contains("code_challenge=chal"));
        assert!(url.contains("state=st"));
        assert!(url.contains("scope=user-read-private%20user-read-playback-state"));
    }

    #[test]
    fn exchanges_a_code() {
        let (origin, seen) = serve(|_| {
            Response::json(
                200,
                r#"{"access_token":"a1","token_type":"Bearer","expires_in":3600,"refresh_token":"r1","scope":"s"}"#,
            )
        });
        let token = exchange_code(
            &Http::new(),
            &origin,
            "cid",
            "code1",
            "verifier1",
            "http://127.0.0.1:8898/callback",
            1_000,
        )
        .unwrap();
        assert_eq!(token.access_token, "a1");
        assert_eq!(token.refresh_token, "r1");
        assert_eq!(token.expires_at, 4_600);
        let request = seen.lock().unwrap()[0].clone();
        assert_eq!(request.method, "POST");
        assert_eq!(request.path(), "/api/token");
        for part in [
            "grant_type=authorization_code",
            "code=code1",
            "code_verifier=verifier1",
            "client_id=cid",
            "redirect_uri=http%3A%2F%2F127.0.0.1%3A8898%2Fcallback",
        ] {
            assert!(request.body.contains(part), "{part} in {}", request.body);
        }
    }

    #[test]
    fn refresh_keeps_the_old_refresh_token_when_none_comes_back() {
        let (origin, seen) =
            serve(|_| Response::json(200, r#"{"access_token":"a2","expires_in":60}"#));
        let token = refresh(&Http::new(), &origin, "cid", "r-old", 10).unwrap();
        assert_eq!(token.access_token, "a2");
        assert_eq!(token.refresh_token, "r-old");
        assert_eq!(token.expires_at, 70);
        let body = seen.lock().unwrap()[0].body.clone();
        assert!(body.contains("grant_type=refresh_token"), "{body}");
        assert!(body.contains("refresh_token=r-old"), "{body}");
    }

    #[test]
    fn token_errors_become_spotify_errors() {
        let (origin, _) = serve(|request| {
            if request.body.contains("client_id=bad") {
                Response::json(400, r#"{"error":"invalid_client","error_description":"Invalid client"}"#)
            } else if request.body.contains("grant_type=refresh_token") {
                Response::json(400, r#"{"error":"invalid_grant","error_description":"Refresh token revoked"}"#)
            } else {
                Response::json(400, r#"{"error":"invalid_grant","error_description":"Invalid redirect URI"}"#)
            }
        });
        let http = Http::new();
        let uri = "http://127.0.0.1:8898/callback";
        assert_eq!(
            exchange_code(&http, &origin, "bad", "c", "v", uri, 0).unwrap_err(),
            SpotifyError::InvalidClient { redirect_uri: uri.into() }
        );
        assert_eq!(
            exchange_code(&http, &origin, "cid", "c", "v", uri, 0).unwrap_err(),
            SpotifyError::InvalidClient { redirect_uri: uri.into() }
        );
        assert_eq!(
            refresh(&http, &origin, "cid", "r", 0).unwrap_err(),
            SpotifyError::Revoked
        );
    }
}
```

Add `mod auth;` to `lib.rs`.

- [x] **Step 2: Run to see it fail**

Run: `cargo test -p znicz-spotify auth`
Expected: compile errors (`authorize_url`, `exchange_code`, `refresh` missing).

- [x] **Step 3: Implement**

Between the `use` lines and the tests:

```rust
pub(crate) const SCOPES: &str = "user-read-private user-read-playback-state \
user-modify-playback-state user-read-currently-playing user-library-read \
playlist-read-private playlist-read-collaborative";

/// Characters left as they are in a query value (RFC 3986 unreserved).
const QUERY_VALUE: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');

fn encode(value: &str) -> String {
    utf8_percent_encode(value, QUERY_VALUE).to_string()
}

pub(crate) fn authorize_url(
    accounts_base: &str,
    client_id: &str,
    redirect_uri: &str,
    challenge: &str,
    state: &str,
) -> String {
    format!(
        "{accounts_base}/authorize?client_id={}&response_type=code&redirect_uri={}\
         &code_challenge_method=S256&code_challenge={}&state={}&scope={}",
        encode(client_id),
        encode(redirect_uri),
        encode(challenge),
        encode(state),
        encode(SCOPES),
    )
}

pub(crate) fn exchange_code(
    http: &Http,
    accounts_base: &str,
    client_id: &str,
    code: &str,
    verifier: &str,
    redirect_uri: &str,
    now_secs: u64,
) -> Result<Token> {
    let form = [
        ("grant_type", "authorization_code"),
        ("code", code),
        ("redirect_uri", redirect_uri),
        ("client_id", client_id),
        ("code_verifier", verifier),
    ];
    let reply = post_token(http, accounts_base, &form)?;
    if reply.status != 200 {
        let (error, description) = token_error(&reply);
        return Err(match error.as_str() {
            "invalid_client" => SpotifyError::InvalidClient {
                redirect_uri: redirect_uri.into(),
            },
            "invalid_grant" if description.to_lowercase().contains("redirect") => {
                SpotifyError::InvalidClient {
                    redirect_uri: redirect_uri.into(),
                }
            }
            "invalid_grant" => SpotifyError::LoginExpired,
            _ => token_http_error(&reply, &description),
        });
    }
    token_from(&reply, None, now_secs)
}

pub(crate) fn refresh(
    http: &Http,
    accounts_base: &str,
    client_id: &str,
    refresh_token: &str,
    now_secs: u64,
) -> Result<Token> {
    let form = [
        ("grant_type", "refresh_token"),
        ("refresh_token", refresh_token),
        ("client_id", client_id),
    ];
    let reply = post_token(http, accounts_base, &form)?;
    if reply.status != 200 {
        let (error, description) = token_error(&reply);
        return Err(match error.as_str() {
            "invalid_grant" | "invalid_client" => SpotifyError::Revoked,
            _ => token_http_error(&reply, &description),
        });
    }
    token_from(&reply, Some(refresh_token), now_secs)
}

fn post_token(http: &Http, accounts_base: &str, form: &[(&str, &str)]) -> Result<Reply> {
    http.send(
        Method::Post,
        &format!("{accounts_base}/api/token"),
        &[],
        None,
        Body::Form(form),
    )
}

fn token_from(reply: &Reply, old_refresh: Option<&str>, now_secs: u64) -> Result<Token> {
    let json: Value =
        serde_json::from_str(&reply.body).map_err(|e| SpotifyError::Parse(e.to_string()))?;
    let access_token = json["access_token"]
        .as_str()
        .ok_or_else(|| SpotifyError::Parse("token response without access_token".into()))?;
    let refresh_token = json["refresh_token"]
        .as_str()
        .or(old_refresh)
        .ok_or_else(|| SpotifyError::Parse("token response without refresh_token".into()))?;
    Ok(Token {
        access_token: access_token.into(),
        refresh_token: refresh_token.into(),
        expires_at: now_secs + json["expires_in"].as_u64().unwrap_or(3600),
        scope: json["scope"].as_str().unwrap_or("").into(),
    })
}

/// `{"error":"invalid_grant","error_description":"…"}` → (error, description).
fn token_error(reply: &Reply) -> (String, String) {
    let json: Value = serde_json::from_str(&reply.body).unwrap_or(Value::Null);
    (
        json["error"].as_str().unwrap_or("").into(),
        json["error_description"].as_str().unwrap_or("").into(),
    )
}

fn token_http_error(reply: &Reply, description: &str) -> SpotifyError {
    if reply.status == 429 {
        return SpotifyError::RateLimited {
            retry_after_secs: reply.retry_after.unwrap_or(5),
        };
    }
    SpotifyError::Http {
        status: reply.status,
        message: description.into(),
    }
}
```

- [x] **Step 4: Run the tests**

Run: `cargo test -p znicz-spotify auth`
Expected: 4 pass.

- [x] **Step 5: Commit**

```bash
git add znicz-spotify/src/auth.rs znicz-spotify/src/lib.rs
git commit -m "feat(spotify): authorize URL, code exchange and refresh"
```

---

### Task 8: Web API JSON → core types

**Files:**
- Create: `znicz-spotify/src/model.rs`
- Modify: `znicz-spotify/src/lib.rs` (add `mod model;`)

- [x] **Step 1: Write the failing tests**

Create `znicz-spotify/src/model.rs`:

```rust
//! Read Web API JSON into the core types. Lenient: a missing field becomes a
//! default, and a row that is not a usable item (a removed track, a local
//! file in a playlist) is skipped.

use serde_json::Value;
use znicz_core::{
    SpotifyAccount, SpotifyDevice, SpotifyItem, SpotifyKind, SpotifyPage, SpotifyPlayback,
    SpotifyRepeat, SpotifyTrack,
};

#[cfg(test)]
mod tests {
    use super::*;

    const TRACK: &str = r#"{
        "type": "track", "uri": "spotify:track:t1", "name": "So What",
        "duration_ms": 562000, "track_number": 1,
        "artists": [{"name": "Miles Davis"}, {"name": "John Coltrane"}],
        "album": {
            "uri": "spotify:album:a1", "name": "Kind of Blue", "release_date": "1959-08-17",
            "images": [
                {"url": "https://i/1280", "width": 1280, "height": 1280},
                {"url": "https://i/640", "width": 640, "height": 640},
                {"url": "https://i/64", "width": 64, "height": 64}
            ]
        }
    }"#;

    fn json(text: &str) -> Value {
        serde_json::from_str(text).unwrap()
    }

    #[test]
    fn reads_a_track() {
        let track = track(&json(TRACK)).unwrap();
        assert_eq!(track.uri, "spotify:track:t1");
        assert_eq!(track.title, "So What");
        assert_eq!(track.artists, vec!["Miles Davis", "John Coltrane"]);
        assert_eq!(track.album, "Kind of Blue");
        assert_eq!(track.album_uri, "spotify:album:a1");
        assert_eq!(track.year, Some(1959));
        assert_eq!(track.track_number, Some(1));
        assert_eq!(track.duration_ms, 562_000);
        assert_eq!(track.image_url.as_deref(), Some("https://i/640"));
    }

    #[test]
    fn picks_the_first_image_when_widths_are_unknown() {
        let images = json(r#"[{"url": "https://a", "width": null}, {"url": "https://b"}]"#);
        assert_eq!(image(&images).as_deref(), Some("https://a"));
        assert_eq!(image(&json("[]")), None);
    }

    #[test]
    fn reads_playback() {
        let text = format!(
            r#"{{
                "device": {{"id": "d1", "name": "Kitchen", "type": "Speaker", "is_active": true, "volume_percent": 70}},
                "repeat_state": "context", "shuffle_state": true, "is_playing": true,
                "progress_ms": 62000, "currently_playing_type": "track",
                "context": {{"uri": "spotify:album:a1"}},
                "item": {TRACK}
            }}"#
        );
        let playback = playback(&json(&text), 5_000).unwrap();
        assert!(playback.is_playing);
        assert!(!playback.unsupported);
        assert_eq!(playback.progress_ms, 62_000);
        assert_eq!(playback.read_at_ms, 5_000);
        assert_eq!(playback.repeat, SpotifyRepeat::Context);
        assert!(playback.shuffle);
        assert_eq!(playback.device.unwrap().name, "Kitchen");
        assert_eq!(playback.context_uri.as_deref(), Some("spotify:album:a1"));
        assert_eq!(playback.item.unwrap().title, "So What");
    }

    #[test]
    fn an_episode_is_unsupported_and_nothing_playing_is_none() {
        let episode = json(
            r#"{"is_playing": true, "progress_ms": 1, "currently_playing_type": "episode", "item": null}"#,
        );
        let playback = playback(&episode, 0).unwrap();
        assert!(playback.unsupported);
        assert_eq!(playback.item, None);
        assert_eq!(super::playback(&Value::Null, 0), None);
    }

    #[test]
    fn reads_devices_and_skips_restricted_ones() {
        let devices = devices(&json(
            r#"{"devices": [
                {"id": "d1", "name": "Laptop", "type": "Computer", "is_active": false, "volume_percent": 50},
                {"id": null, "name": "Restricted", "type": "Speaker", "is_active": false}
            ]}"#,
        ));
        assert_eq!(
            devices,
            vec![SpotifyDevice {
                id: "d1".into(),
                name: "Laptop".into(),
                kind: "Computer".into(),
                is_active: false,
                volume_percent: Some(50),
            }]
        );
    }

    #[test]
    fn reads_the_account() {
        let account = account(&json(r#"{"id": "u1", "display_name": "Eugene", "product": "premium"}"#));
        assert_eq!(account, SpotifyAccount { user: "Eugene".into(), premium: true });
        let free = super::account(&json(r#"{"id": "u2", "display_name": null, "product": "free"}"#));
        assert_eq!(free, SpotifyAccount { user: "u2".into(), premium: false });
    }

    #[test]
    fn reads_saved_tracks_and_skips_empty_rows() {
        let text = format!(
            r#"{{"offset": 50, "total": 120, "items": [
                {{"added_at": "x", "track": {TRACK}}},
                {{"added_at": "y", "track": null}}
            ]}}"#
        );
        let page = page(&json(&text));
        assert_eq!(page.offset, 50);
        assert_eq!(page.total, 120);
        assert_eq!(page.items.len(), 1);
        let item = &page.items[0];
        assert_eq!(item.kind, SpotifyKind::Track);
        assert_eq!(item.name, "So What");
        assert_eq!(item.subtitle, "Miles Davis, John Coltrane");
        assert!(item.track.is_some());
    }

    #[test]
    fn reads_albums_playlists_and_artists() {
        let page = page(&json(
            r#"{"offset": 0, "total": 3, "items": [
                {"album": {"type": "album", "uri": "spotify:album:a1", "name": "Kind of Blue",
                           "artists": [{"name": "Miles Davis"}], "images": []}},
                {"type": "playlist", "uri": "spotify:playlist:p1", "name": "Jazz",
                 "owner": {"display_name": "Eugene"}, "images": null},
                {"type": "artist", "uri": "spotify:artist:r1", "name": "Miles Davis", "images": []}
            ]}"#,
        ));
        let kinds: Vec<_> = page.items.iter().map(|i| i.kind).collect();
        assert_eq!(
            kinds,
            vec![SpotifyKind::Album, SpotifyKind::Playlist, SpotifyKind::Artist]
        );
        assert_eq!(page.items[0].subtitle, "Miles Davis");
        assert_eq!(page.items[1].subtitle, "Eugene");
        assert_eq!(page.items[2].subtitle, "");
    }

    #[test]
    fn search_results_keep_the_requested_kind_order() {
        let text = format!(
            r#"{{
                "tracks": {{"items": [{TRACK}], "total": 1}},
                "albums": {{"items": [{{"type": "album", "uri": "spotify:album:a1", "name": "Kind of Blue", "artists": [], "images": []}}], "total": 1}}
            }}"#
        );
        let page = search(&json(&text), &[SpotifyKind::Album, SpotifyKind::Track]);
        let kinds: Vec<_> = page.items.iter().map(|i| i.kind).collect();
        assert_eq!(kinds, vec![SpotifyKind::Album, SpotifyKind::Track]);
        assert_eq!(page.total, 2);
    }

    #[test]
    fn reads_the_queue() {
        let text = format!(r#"{{"currently_playing": {TRACK}, "queue": [{TRACK}, {{"type": "episode"}}]}}"#);
        let (current, next) = queue(&json(&text));
        assert_eq!(current.unwrap().title, "So What");
        assert_eq!(next.len(), 1);
    }
}
```

Add `mod model;` to `lib.rs`.

- [x] **Step 2: Run to see it fail**

Run: `cargo test -p znicz-spotify model`
Expected: compile errors (`track`, `image`, `playback`, … missing).

- [x] **Step 3: Implement**

Between the `use` lines and the tests:

```rust
/// Spotify album and playlist images go up to 640 px; the TUI covers are
/// never bigger.
const MAX_IMAGE_WIDTH: u64 = 640;

fn text(value: &Value) -> String {
    value.as_str().unwrap_or("").to_string()
}

fn names(list: &Value) -> Vec<String> {
    list.as_array()
        .map(|items| items.iter().map(|a| text(&a["name"])).collect())
        .unwrap_or_default()
}

/// The widest image no wider than 640 px; else the first one.
pub(crate) fn image(images: &Value) -> Option<String> {
    let list = images.as_array()?;
    let fitting = list
        .iter()
        .filter(|i| i["width"].as_u64().is_some_and(|w| w <= MAX_IMAGE_WIDTH))
        .max_by_key(|i| i["width"].as_u64().unwrap_or(0));
    fitting
        .or_else(|| list.first())
        .and_then(|i| i["url"].as_str())
        .map(str::to_string)
}

/// A track object. `None` for anything else (an episode, null).
pub(crate) fn track(value: &Value) -> Option<SpotifyTrack> {
    let kind = value["type"].as_str().unwrap_or("track");
    if !value.is_object() || kind != "track" {
        return None;
    }
    let uri = value["uri"].as_str()?;
    let album = &value["album"];
    Some(SpotifyTrack {
        uri: uri.into(),
        title: text(&value["name"]),
        artists: names(&value["artists"]),
        album: text(&album["name"]),
        album_uri: text(&album["uri"]),
        year: album["release_date"]
            .as_str()
            .and_then(|d| d.get(..4))
            .and_then(|y| y.parse().ok()),
        track_number: value["track_number"].as_u64().map(|n| n as u32),
        duration_ms: value["duration_ms"].as_u64().unwrap_or(0),
        image_url: image(&album["images"]),
    })
}

/// A browse row from a track, album, artist or playlist object.
pub(crate) fn item(value: &Value) -> Option<SpotifyItem> {
    let kind = match value["type"].as_str() {
        Some("track") => SpotifyKind::Track,
        None if value.get("album").is_some() => SpotifyKind::Track,
        Some("album") => SpotifyKind::Album,
        Some("artist") => SpotifyKind::Artist,
        Some("playlist") => SpotifyKind::Playlist,
        _ => return None,
    };
    let uri = value["uri"].as_str()?.to_string();
    let name = text(&value["name"]);
    Some(match kind {
        SpotifyKind::Track => {
            let track = track(value)?;
            SpotifyItem {
                kind,
                uri,
                name,
                subtitle: track.artist_line(),
                image_url: track.image_url.clone(),
                track: Some(track),
            }
        }
        SpotifyKind::Album => SpotifyItem {
            kind,
            uri,
            name,
            subtitle: names(&value["artists"]).join(", "),
            image_url: image(&value["images"]),
            track: None,
        },
        SpotifyKind::Artist => SpotifyItem {
            kind,
            uri,
            name,
            subtitle: String::new(),
            image_url: image(&value["images"]),
            track: None,
        },
        SpotifyKind::Playlist => SpotifyItem {
            kind,
            uri,
            name,
            subtitle: text(&value["owner"]["display_name"]),
            image_url: image(&value["images"]),
            track: None,
        },
    })
}

/// A paging object. Saved tracks and albums wrap each row as
/// `{"track": …}` / `{"album": …}`; playlists as `{"track": …}` or `{"item": …}`.
pub(crate) fn page(value: &Value) -> SpotifyPage {
    let items = value["items"]
        .as_array()
        .map(|rows| {
            rows.iter()
                .filter_map(|row| {
                    // Wrapper rows (`{"added_at": …, "track": …}`) have no `type`;
                    // a track object has an `album` key, so check `type` first.
                    if row.get("type").is_some() {
                        return item(row);
                    }
                    ["track", "album", "item"]
                        .iter()
                        .find(|key| row.get(**key).is_some())
                        .and_then(|key| item(&row[*key]))
                })
                .collect()
        })
        .unwrap_or_default();
    SpotifyPage {
        items,
        offset: value["offset"].as_u64().unwrap_or(0) as u32,
        total: value["total"].as_u64().unwrap_or(0) as u32,
    }
}

/// A plain list of track objects (top tracks) as one page.
pub(crate) fn track_list(list: &Value) -> SpotifyPage {
    let items: Vec<SpotifyItem> = list
        .as_array()
        .map(|rows| rows.iter().filter_map(item).collect())
        .unwrap_or_default();
    SpotifyPage {
        total: items.len() as u32,
        items,
        offset: 0,
    }
}

/// `/search` groups results by kind; keep the order the caller asked for.
pub(crate) fn search(value: &Value, kinds: &[SpotifyKind]) -> SpotifyPage {
    let items: Vec<SpotifyItem> = kinds
        .iter()
        .flat_map(|kind| page(&value[format!("{}s", kind.as_api())]).items)
        .collect();
    SpotifyPage {
        total: items.len() as u32,
        items,
        offset: 0,
    }
}

pub(crate) fn device(value: &Value) -> Option<SpotifyDevice> {
    Some(SpotifyDevice {
        id: value["id"].as_str()?.to_string(),
        name: text(&value["name"]),
        kind: text(&value["type"]),
        is_active: value["is_active"].as_bool().unwrap_or(false),
        volume_percent: value["volume_percent"].as_u64().map(|v| v.min(100) as u8),
    })
}

pub(crate) fn devices(value: &Value) -> Vec<SpotifyDevice> {
    value["devices"]
        .as_array()
        .map(|list| list.iter().filter_map(device).collect())
        .unwrap_or_default()
}

/// `/me/player`. `None` when nothing is playing (the API answers 204).
pub(crate) fn playback(value: &Value, now_ms: u64) -> Option<SpotifyPlayback> {
    if !value.is_object() {
        return None;
    }
    let item = track(&value["item"]);
    let kind = value["currently_playing_type"].as_str().unwrap_or("track");
    Some(SpotifyPlayback {
        unsupported: item.is_none() && (kind != "track" || !value["item"].is_null()),
        item,
        is_playing: value["is_playing"].as_bool().unwrap_or(false),
        progress_ms: value["progress_ms"].as_u64().unwrap_or(0),
        read_at_ms: now_ms,
        repeat: SpotifyRepeat::from_api(value["repeat_state"].as_str().unwrap_or("off")),
        shuffle: value["shuffle_state"].as_bool().unwrap_or(false),
        device: device(&value["device"]),
        context_uri: value["context"]["uri"].as_str().map(str::to_string),
    })
}

pub(crate) fn account(value: &Value) -> SpotifyAccount {
    let user = value["display_name"]
        .as_str()
        .filter(|name| !name.is_empty())
        .or_else(|| value["id"].as_str())
        .unwrap_or("")
        .to_string();
    SpotifyAccount {
        user,
        premium: value["product"].as_str() == Some("premium"),
    }
}

pub(crate) fn queue(value: &Value) -> (Option<SpotifyTrack>, Vec<SpotifyTrack>) {
    let next = value["queue"]
        .as_array()
        .map(|list| list.iter().filter_map(track).collect())
        .unwrap_or_default();
    (track(&value["currently_playing"]), next)
}
```

A typed `"track"` without an `album` key (album track rows) is still a track; the album fields stay empty and `api::tracks` fills them (Task 9).

- [x] **Step 4: Run the tests**

Run: `cargo test -p znicz-spotify model`
Expected: 10 pass.

- [x] **Step 5: Commit**

```bash
git add znicz-spotify/src/model.rs znicz-spotify/src/lib.rs
git commit -m "feat(spotify): read Web API JSON into core types"
```

---

### Task 9: Web API endpoints

**Files:**
- Create: `znicz-spotify/src/api.rs`
- Modify: `znicz-spotify/src/lib.rs` (add `mod api;`)

- [x] **Step 1: Write the failing tests**

Create `znicz-spotify/src/api.rs`:

```rust
//! One method per Web API endpoint Znicz uses. Each call is a single HTTP
//! request (two for album tracks) with the caller's access token.

use serde_json::{Value, json};
use znicz_core::{
    SpotifyAccount, SpotifyDevice, SpotifyKind, SpotifyPage, SpotifyPlayback, SpotifyRepeat,
    SpotifyTrack,
};

use crate::error::{Result, SpotifyError};
use crate::http::{Body, Http, Method, check};
use crate::model;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::test_server::{Response, serve};

    fn api_for<'a>(http: &'a Http, base: &'a str) -> Api<'a> {
        Api {
            http,
            base,
            token: "tok",
        }
    }

    #[test]
    fn splits_uris() {
        assert_eq!(split_uri("spotify:album:abc"), Some((SpotifyKind::Album, "abc")));
        assert_eq!(split_uri("spotify:playlist:p"), Some((SpotifyKind::Playlist, "p")));
        assert_eq!(split_uri("spotify:artist:r"), Some((SpotifyKind::Artist, "r")));
        assert_eq!(split_uri("spotify:show:s"), None);
        assert_eq!(split_uri("nonsense"), None);
    }

    #[test]
    fn play_sends_context_and_offset() {
        let (origin, seen) = serve(|_| Response::empty(204));
        let http = Http::new();
        let base = format!("{origin}/v1");
        api_for(&http, &base)
            .play(Some("d1"), Some("spotify:album:a1"), &[], Some(3))
            .unwrap();
        api_for(&http, &base)
            .play(None, None, &["spotify:track:t1".to_string()], Some(0))
            .unwrap();
        api_for(&http, &base).play(None, None, &[], None).unwrap();
        let seen = seen.lock().unwrap();
        assert_eq!(seen[0].method, "PUT");
        assert_eq!(seen[0].path(), "/v1/me/player/play");
        assert_eq!(seen[0].query("device_id").as_deref(), Some("d1"));
        let body: Value = serde_json::from_str(&seen[0].body).unwrap();
        assert_eq!(body, json!({"context_uri": "spotify:album:a1", "offset": {"position": 3}}));
        let body: Value = serde_json::from_str(&seen[1].body).unwrap();
        assert_eq!(body, json!({"uris": ["spotify:track:t1"], "offset": {"position": 0}}));
        assert_eq!(seen[2].body, "", "a resume sends no body");
    }

    #[test]
    fn transport_calls_use_the_documented_endpoints() {
        let (origin, seen) = serve(|_| Response::empty(204));
        let http = Http::new();
        let base = format!("{origin}/v1");
        let api = api_for(&http, &base);
        api.pause(None).unwrap();
        api.next(None).unwrap();
        api.previous(None).unwrap();
        api.seek(None, 61_000).unwrap();
        api.volume(None, 0.5).unwrap();
        api.repeat(None, SpotifyRepeat::Context).unwrap();
        api.shuffle(None, true).unwrap();
        api.queue_add(None, "spotify:track:t1").unwrap();
        api.transfer("d2", true).unwrap();
        let seen = seen.lock().unwrap();
        let lines: Vec<String> = seen
            .iter()
            .map(|r| format!("{} {}", r.method, r.path()))
            .collect();
        assert_eq!(
            lines,
            vec![
                "PUT /v1/me/player/pause",
                "POST /v1/me/player/next",
                "POST /v1/me/player/previous",
                "PUT /v1/me/player/seek",
                "PUT /v1/me/player/volume",
                "PUT /v1/me/player/repeat",
                "PUT /v1/me/player/shuffle",
                "POST /v1/me/player/queue",
                "PUT /v1/me/player",
            ]
        );
        assert_eq!(seen[3].query("position_ms").as_deref(), Some("61000"));
        assert_eq!(seen[4].query("volume_percent").as_deref(), Some("50"));
        assert_eq!(seen[5].query("state").as_deref(), Some("context"));
        assert_eq!(seen[6].query("state").as_deref(), Some("true"));
        assert_eq!(seen[7].query("uri").as_deref(), Some("spotify:track:t1"));
        let body: Value = serde_json::from_str(&seen[8].body).unwrap();
        assert_eq!(body, json!({"device_ids": ["d2"], "play": true}));
        assert_eq!(seen[0].header("authorization"), Some("Bearer tok"));
    }

    #[test]
    fn nothing_playing_is_none() {
        let (origin, _) = serve(|_| Response::empty(204));
        let http = Http::new();
        let base = format!("{origin}/v1");
        assert_eq!(api_for(&http, &base).playback(0).unwrap(), None);
    }

    #[test]
    fn album_tracks_get_the_albums_details() {
        let (origin, _) = serve(|request| match request.path() {
            "/v1/albums/a1" => Response::json(
                200,
                r#"{"uri": "spotify:album:a1", "name": "Kind of Blue", "release_date": "1959",
                    "images": [{"url": "https://i/640", "width": 640}]}"#,
            ),
            "/v1/albums/a1/tracks" => Response::json(
                200,
                r#"{"offset": 0, "total": 1, "items": [
                    {"type": "track", "uri": "spotify:track:t1", "name": "So What",
                     "duration_ms": 1, "track_number": 1, "artists": [{"name": "Miles Davis"}]}
                ]}"#,
            ),
            _ => Response::empty(404),
        });
        let http = Http::new();
        let base = format!("{origin}/v1");
        let page = api_for(&http, &base).tracks("spotify:album:a1", 0, 50).unwrap();
        let track = page.items[0].track.clone().unwrap();
        assert_eq!(track.album, "Kind of Blue");
        assert_eq!(track.year, Some(1959));
        assert_eq!(track.image_url.as_deref(), Some("https://i/640"));
        assert_eq!(page.items[0].image_url.as_deref(), Some("https://i/640"));
    }

    #[test]
    fn an_unknown_uri_is_refused() {
        let http = Http::new();
        let error = api_for(&http, "http://127.0.0.1:1")
            .tracks("spotify:show:s", 0, 50)
            .unwrap_err();
        assert!(matches!(error, SpotifyError::Parse(_)), "{error:?}");
    }
}
```

Add `mod api;` to `lib.rs`.

- [x] **Step 2: Run to see it fail**

Run: `cargo test -p znicz-spotify api`
Expected: compile errors (`Api`, `split_uri` missing).

- [x] **Step 3: Implement**

Between the `use` lines and the tests:

```rust
pub(crate) struct Api<'a> {
    pub http: &'a Http,
    /// `https://api.spotify.com/v1`, or a test server.
    pub base: &'a str,
    pub token: &'a str,
}

/// `spotify:album:ID` → (Album, ID). Only albums, playlists, artists and tracks.
pub(crate) fn split_uri(uri: &str) -> Option<(SpotifyKind, &str)> {
    let mut parts = uri.split(':');
    if parts.next()? != "spotify" {
        return None;
    }
    let kind = match parts.next()? {
        "album" => SpotifyKind::Album,
        "playlist" => SpotifyKind::Playlist,
        "artist" => SpotifyKind::Artist,
        "track" => SpotifyKind::Track,
        _ => return None,
    };
    let id = parts.next().filter(|id| !id.is_empty())?;
    Some((kind, id))
}

fn device_query(device_id: Option<&str>) -> Vec<(&str, &str)> {
    device_id.map(|id| vec![("device_id", id)]).unwrap_or_default()
}

impl Api<'_> {
    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base)
    }

    /// GET and parse; an empty body (204) is `Value::Null`.
    fn get(&self, path: &str, query: &[(&str, &str)]) -> Result<Value> {
        let reply = check(self.http.send(
            Method::Get,
            &self.url(path),
            query,
            Some(self.token),
            Body::None,
        )?)?;
        if reply.body.trim().is_empty() {
            return Ok(Value::Null);
        }
        serde_json::from_str(&reply.body).map_err(|e| SpotifyError::Parse(e.to_string()))
    }

    fn send(&self, method: Method, path: &str, query: &[(&str, &str)], body: Option<&Value>) -> Result<()> {
        let body = match body {
            Some(value) => Body::Json(value),
            None => Body::None,
        };
        check(self.http.send(method, &self.url(path), query, Some(self.token), body)?)?;
        Ok(())
    }

    pub(crate) fn me(&self) -> Result<SpotifyAccount> {
        Ok(model::account(&self.get("/me", &[])?))
    }

    pub(crate) fn playback(&self, now_ms: u64) -> Result<Option<SpotifyPlayback>> {
        Ok(model::playback(&self.get("/me/player", &[])?, now_ms))
    }

    pub(crate) fn devices(&self) -> Result<Vec<SpotifyDevice>> {
        Ok(model::devices(&self.get("/me/player/devices", &[])?))
    }

    pub(crate) fn queue(&self) -> Result<(Option<SpotifyTrack>, Vec<SpotifyTrack>)> {
        Ok(model::queue(&self.get("/me/player/queue", &[])?))
    }

    pub(crate) fn search(&self, query: &str, kinds: &[SpotifyKind], limit: u32) -> Result<SpotifyPage> {
        let types: Vec<&str> = kinds.iter().map(|k| k.as_api()).collect();
        let types = types.join(",");
        let limit = limit.clamp(1, 50).to_string();
        let json = self.get("/search", &[("q", query), ("type", &types), ("limit", &limit)])?;
        Ok(model::search(&json, kinds))
    }

    fn paged(&self, path: &str, offset: u32, limit: u32) -> Result<SpotifyPage> {
        let offset = offset.to_string();
        let limit = limit.clamp(1, 50).to_string();
        Ok(model::page(&self.get(path, &[("offset", &offset), ("limit", &limit)])?))
    }

    pub(crate) fn playlists(&self, offset: u32, limit: u32) -> Result<SpotifyPage> {
        self.paged("/me/playlists", offset, limit)
    }

    pub(crate) fn liked_songs(&self, offset: u32, limit: u32) -> Result<SpotifyPage> {
        self.paged("/me/tracks", offset, limit)
    }

    pub(crate) fn albums(&self, offset: u32, limit: u32) -> Result<SpotifyPage> {
        self.paged("/me/albums", offset, limit)
    }

    /// Album or playlist tracks, or an artist's top tracks (one page).
    pub(crate) fn tracks(&self, uri: &str, offset: u32, limit: u32) -> Result<SpotifyPage> {
        let unknown = || SpotifyError::Parse(format!("{uri} is not an album, playlist or artist"));
        let (kind, id) = split_uri(uri).ok_or_else(unknown)?;
        match kind {
            SpotifyKind::Album => {
                // Album track rows carry no album; fill it from the album itself.
                let album = self.get(&format!("/albums/{id}"), &[])?;
                let mut page = self.paged(&format!("/albums/{id}/tracks"), offset, limit);
                if let Ok(page) = &mut page {
                    fill_album(page, &album);
                }
                page
            }
            SpotifyKind::Playlist => self.paged(&format!("/playlists/{id}/tracks"), offset, limit),
            SpotifyKind::Artist => {
                let json = self.get(&format!("/artists/{id}/top-tracks"), &[("market", "from_token")])?;
                Ok(model::track_list(&json["tracks"]))
            }
            SpotifyKind::Track => Err(unknown()),
        }
    }

    pub(crate) fn artist_albums(&self, uri: &str, offset: u32, limit: u32) -> Result<SpotifyPage> {
        match split_uri(uri) {
            Some((SpotifyKind::Artist, id)) => {
                let offset = offset.to_string();
                let limit = limit.clamp(1, 50).to_string();
                let json = self.get(
                    &format!("/artists/{id}/albums"),
                    &[("include_groups", "album,single"), ("offset", &offset), ("limit", &limit)],
                )?;
                Ok(model::page(&json))
            }
            _ => Err(SpotifyError::Parse(format!("{uri} is not an artist"))),
        }
    }

    pub(crate) fn play(
        &self,
        device_id: Option<&str>,
        context_uri: Option<&str>,
        uris: &[String],
        offset: Option<u32>,
    ) -> Result<()> {
        let mut body = serde_json::Map::new();
        if let Some(context) = context_uri {
            body.insert("context_uri".into(), json!(context));
        } else if !uris.is_empty() {
            body.insert("uris".into(), json!(uris));
        }
        if let (false, Some(position)) = (body.is_empty(), offset) {
            body.insert("offset".into(), json!({ "position": position }));
        }
        let payload = Value::Object(body);
        let payload = payload.as_object().is_some_and(|b| !b.is_empty()).then_some(&payload);
        self.send(Method::Put, "/me/player/play", &device_query(device_id), payload)
    }

    pub(crate) fn pause(&self, device_id: Option<&str>) -> Result<()> {
        self.send(Method::Put, "/me/player/pause", &device_query(device_id), None)
    }

    pub(crate) fn next(&self, device_id: Option<&str>) -> Result<()> {
        self.send(Method::Post, "/me/player/next", &device_query(device_id), None)
    }

    pub(crate) fn previous(&self, device_id: Option<&str>) -> Result<()> {
        self.send(Method::Post, "/me/player/previous", &device_query(device_id), None)
    }

    pub(crate) fn seek(&self, device_id: Option<&str>, position_ms: u64) -> Result<()> {
        let position = position_ms.to_string();
        let mut query = vec![("position_ms", position.as_str())];
        query.extend(device_query(device_id));
        self.send(Method::Put, "/me/player/seek", &query, None)
    }

    /// `volume` is 0.0–1.0.
    pub(crate) fn volume(&self, device_id: Option<&str>, volume: f32) -> Result<()> {
        let percent = ((volume.clamp(0.0, 1.0) * 100.0).round() as u8).to_string();
        let mut query = vec![("volume_percent", percent.as_str())];
        query.extend(device_query(device_id));
        self.send(Method::Put, "/me/player/volume", &query, None)
    }

    pub(crate) fn repeat(&self, device_id: Option<&str>, mode: SpotifyRepeat) -> Result<()> {
        let mut query = vec![("state", mode.as_api())];
        query.extend(device_query(device_id));
        self.send(Method::Put, "/me/player/repeat", &query, None)
    }

    pub(crate) fn shuffle(&self, device_id: Option<&str>, on: bool) -> Result<()> {
        let mut query = vec![("state", if on { "true" } else { "false" })];
        query.extend(device_query(device_id));
        self.send(Method::Put, "/me/player/shuffle", &query, None)
    }

    pub(crate) fn queue_add(&self, device_id: Option<&str>, uri: &str) -> Result<()> {
        let mut query = vec![("uri", uri)];
        query.extend(device_query(device_id));
        self.send(Method::Post, "/me/player/queue", &query, None)
    }

    pub(crate) fn transfer(&self, device_id: &str, play: bool) -> Result<()> {
        let body = json!({ "device_ids": [device_id], "play": play });
        self.send(Method::Put, "/me/player", &[], Some(&body))
    }
}

/// Copy the album's name, URI, year and image into each track row.
fn fill_album(page: &mut SpotifyPage, album: &Value) {
    let name = album["name"].as_str().unwrap_or("").to_string();
    let uri = album["uri"].as_str().unwrap_or("").to_string();
    let year = album["release_date"]
        .as_str()
        .and_then(|d| d.get(..4))
        .and_then(|y| y.parse().ok());
    let image = model::image(&album["images"]);
    for item in &mut page.items {
        item.image_url = image.clone();
        if let Some(track) = &mut item.track {
            track.album = name.clone();
            track.album_uri = uri.clone();
            track.year = year;
            track.image_url = image.clone();
        }
    }
}
```

- [x] **Step 4: Run the tests**

Run: `cargo test -p znicz-spotify`
Expected: everything so far passes (api: 6).

- [x] **Step 5: Commit**

```bash
git add znicz-spotify/src/api.rs znicz-spotify/src/lib.rs
git commit -m "feat(spotify): Web API endpoints for browse and playback"
```

---

### Task 10: `Client`: config, token, login session, refresh-and-retry

**Files:**
- Create: `znicz-spotify/src/client.rs`
- Modify: `znicz-spotify/src/lib.rs` (add `mod client;`)

- [ ] **Step 1: Write the failing tests**

Create `znicz-spotify/src/client.rs`:

```rust
//! Holds what a logged-in session needs: the settings, the token (in memory
//! and on disk), and a login in progress. Every API call goes through `call`,
//! which refreshes the token before it expires and once more on a 401.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use znicz_core::SpotifyConfig;

use crate::api::Api;
use crate::auth;
use crate::error::{Result, SpotifyError};
use crate::http::{ACCOUNTS_BASE, API_BASE, Http};
use crate::pkce;
use crate::redirect;
use crate::token::{self, Token};

#[cfg(test)]
mod tests {
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

    use super::*;
    use crate::http::test_server::{Response, serve};

    const CLIENT_ID: &str = "0123456789abcdef0123456789abcdef";

    struct Fixture {
        client: Client,
        now: Arc<AtomicU64>,
        token_path: PathBuf,
        _dir: tempfile::TempDir,
    }

    fn fixture(origin: &str, logged_in: Option<Token>) -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let token_path = dir.path().join("spotify_token.json");
        if let Some(token) = &logged_in {
            token::save(&token_path, token).unwrap();
        }
        let now = Arc::new(AtomicU64::new(1_000_000));
        let clock_now = now.clone();
        let clock: Clock = Arc::new(move || clock_now.load(Ordering::SeqCst));
        let config = SpotifyConfig {
            client_id: CLIENT_ID.into(),
            ..SpotifyConfig::default()
        };
        let endpoints = Endpoints {
            api: format!("{origin}/v1"),
            accounts: origin.to_string(),
        };
        Fixture {
            client: Client::new(config, token_path.clone(), endpoints, clock),
            now,
            token_path,
            _dir: dir,
        }
    }

    fn token(expires_at: u64) -> Token {
        Token {
            access_token: "old".into(),
            refresh_token: "r".into(),
            expires_at,
            scope: String::new(),
        }
    }

    fn token_reply() -> Response {
        Response::json(200, r#"{"access_token":"new","expires_in":3600,"refresh_token":"r2"}"#)
    }

    fn state_of(url: &str) -> String {
        url.split("state=").nth(1).unwrap().split('&').next().unwrap().to_string()
    }

    #[test]
    fn login_needs_a_client_id() {
        let mut f = fixture("http://127.0.0.1:1", None);
        f.client.set_config(SpotifyConfig::default());
        assert_eq!(f.client.begin_login().unwrap_err(), SpotifyError::NotConfigured);
    }

    #[test]
    fn a_pasted_address_finishes_the_login_and_saves_the_token() {
        let (origin, _) = serve(|_| token_reply());
        let mut f = fixture(&origin, None);
        assert!(!f.client.is_logged_in());
        let login = f.client.begin_login().unwrap();
        assert!(login.url.contains(&format!("client_id={CLIENT_ID}")));
        assert!(f.client.login_pending());
        let pasted = format!("http://127.0.0.1:8898/callback?code=c&state={}", state_of(&login.url));
        f.client.complete_login(&pasted).unwrap();
        assert!(f.client.is_logged_in());
        assert!(!f.client.login_pending());
        assert_eq!(token::load(&f.token_path).unwrap().unwrap().access_token, "new");
    }

    #[test]
    fn a_wrong_address_keeps_the_login_open() {
        let mut f = fixture("http://127.0.0.1:1", None);
        f.client.begin_login().unwrap();
        assert_eq!(
            f.client.complete_login("code=c&state=other").unwrap_err(),
            SpotifyError::WrongAddress
        );
        assert!(f.client.login_pending());
    }

    #[test]
    fn a_login_expires_after_ten_minutes() {
        let mut f = fixture("http://127.0.0.1:1", None);
        let login = f.client.begin_login().unwrap();
        f.now.fetch_add(10 * 60 * 1000 + 1, Ordering::SeqCst);
        assert!(!f.client.login_pending());
        let pasted = format!("code=c&state={}", state_of(&login.url));
        assert_eq!(f.client.complete_login(&pasted).unwrap_err(), SpotifyError::LoginExpired);
        assert_eq!(f.client.complete_login(&pasted).unwrap_err(), SpotifyError::NoLogin);
    }

    #[test]
    fn refreshes_before_the_token_expires() {
        let posts = Arc::new(AtomicUsize::new(0));
        let counter = posts.clone();
        let (origin, seen) = serve(move |request| {
            if request.path() == "/api/token" {
                counter.fetch_add(1, Ordering::SeqCst);
                token_reply()
            } else {
                Response::json(200, r#"{"id":"u","product":"premium"}"#)
            }
        });
        // 1_000_000 ms = 1_000 s; the token expires 30 s later, inside the margin.
        let mut f = fixture(&origin, Some(token(1_030)));
        f.client.call(|api| api.me()).unwrap();
        assert_eq!(posts.load(Ordering::SeqCst), 1);
        let me = seen.lock().unwrap().iter().find(|r| r.path() == "/v1/me").cloned().unwrap();
        assert_eq!(me.header("authorization"), Some("Bearer new"));
    }

    #[test]
    fn a_401_refreshes_once_and_retries() {
        let calls = Arc::new(Mutex::new(Vec::<String>::new()));
        let log = calls.clone();
        let (origin, _) = serve(move |request| {
            let auth = request.header("authorization").unwrap_or("").to_string();
            log.lock().unwrap().push(format!("{} {auth}", request.path()));
            match (request.path(), auth.as_str()) {
                ("/api/token", _) => token_reply(),
                ("/v1/me", "Bearer old") => Response::empty(401),
                _ => Response::json(200, r#"{"id":"u"}"#),
            }
        });
        let mut f = fixture(&origin, Some(token(999_999)));
        f.client.call(|api| api.me()).unwrap();
        assert_eq!(
            *calls.lock().unwrap(),
            vec!["/v1/me Bearer old", "/api/token ", "/v1/me Bearer new"]
        );
    }

    #[test]
    fn a_revoked_refresh_logs_out() {
        let (origin, _) = serve(|request| {
            if request.path() == "/api/token" {
                Response::json(400, r#"{"error":"invalid_grant"}"#)
            } else {
                Response::empty(401)
            }
        });
        let mut f = fixture(&origin, Some(token(999_999)));
        assert_eq!(f.client.call(|api| api.me()).unwrap_err(), SpotifyError::Revoked);
        assert!(!f.client.is_logged_in());
        assert!(!f.token_path.exists());
    }

    #[test]
    fn calls_need_a_login() {
        let mut f = fixture("http://127.0.0.1:1", None);
        assert_eq!(f.client.call(|api| api.me()).unwrap_err(), SpotifyError::NotLoggedIn);
    }

    #[test]
    fn a_new_client_id_logs_out() {
        let mut f = fixture("http://127.0.0.1:1", Some(token(999_999)));
        assert!(f.client.is_logged_in());
        f.client.set_config(SpotifyConfig {
            client_id: "ffffffffffffffffffffffffffffffff".into(),
            ..SpotifyConfig::default()
        });
        assert!(!f.client.is_logged_in());
        assert!(!f.token_path.exists());
    }

    #[test]
    fn logout_removes_the_token() {
        let mut f = fixture("http://127.0.0.1:1", Some(token(999_999)));
        f.client.logout().unwrap();
        assert!(!f.client.is_logged_in());
        assert!(!f.token_path.exists());
    }
}
```

Add `mod client;` to `lib.rs`.

- [ ] **Step 2: Run to see it fail**

Run: `cargo test -p znicz-spotify client`
Expected: compile errors (`Client`, `Clock`, `Endpoints` missing).

- [ ] **Step 3: Implement**

Between the `use` lines and the tests:

```rust
/// How long a started login waits for its redirect.
pub(crate) const LOGIN_TTL_MS: u64 = 10 * 60 * 1000;

/// Unix time in milliseconds. Tests inject their own.
pub(crate) type Clock = Arc<dyn Fn() -> u64 + Send + Sync>;

pub(crate) fn system_clock() -> Clock {
    Arc::new(|| {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0)
    })
}

#[derive(Debug, Clone)]
pub(crate) struct Endpoints {
    pub api: String,
    pub accounts: String,
}

impl Default for Endpoints {
    fn default() -> Self {
        Self {
            api: API_BASE.into(),
            accounts: ACCOUNTS_BASE.into(),
        }
    }
}

/// A login that waits for its redirect.
struct LoginSession {
    verifier: String,
    state: String,
    started_ms: u64,
}

/// What `begin_login` hands back.
pub(crate) struct BegunLogin {
    pub url: String,
    pub state: String,
}

pub(crate) struct Client {
    config: SpotifyConfig,
    token_path: PathBuf,
    endpoints: Endpoints,
    http: Http,
    clock: Clock,
    token: Option<Token>,
    login: Option<LoginSession>,
}

impl Client {
    pub(crate) fn new(
        config: SpotifyConfig,
        token_path: PathBuf,
        endpoints: Endpoints,
        clock: Clock,
    ) -> Self {
        let token = token::load(&token_path).unwrap_or_else(|e| {
            tracing::warn!(error = %e, "could not read the Spotify token");
            None
        });
        Self {
            config,
            token_path,
            endpoints,
            http: Http::new(),
            clock,
            token,
            login: None,
        }
    }

    pub(crate) fn now_ms(&self) -> u64 {
        (self.clock)()
    }

    pub(crate) fn config(&self) -> &SpotifyConfig {
        &self.config
    }

    /// A different client ID cannot refresh the old token, so it logs out.
    pub(crate) fn set_config(&mut self, config: SpotifyConfig) {
        if config.client_id != self.config.client_id {
            if let Err(e) = self.logout() {
                tracing::warn!(error = %e, "could not remove the Spotify token");
            }
        }
        if config.redirect_port != self.config.redirect_port {
            self.login = None;
        }
        self.config = config;
    }

    pub(crate) fn is_logged_in(&self) -> bool {
        self.config.is_configured() && self.token.is_some()
    }

    pub(crate) fn login_pending(&self) -> bool {
        self.login
            .as_ref()
            .is_some_and(|login| self.now_ms().saturating_sub(login.started_ms) <= LOGIN_TTL_MS)
    }

    pub(crate) fn begin_login(&mut self) -> Result<BegunLogin> {
        if !self.config.is_configured() {
            return Err(SpotifyError::NotConfigured);
        }
        let verifier = pkce::random_verifier();
        let state = pkce::random_state();
        let url = auth::authorize_url(
            &self.endpoints.accounts,
            &self.config.client_id,
            &self.config.redirect_uri(),
            &pkce::challenge(&verifier),
            &state,
        );
        self.login = Some(LoginSession {
            verifier,
            state: state.clone(),
            started_ms: self.now_ms(),
        });
        Ok(BegunLogin { url, state })
    }

    /// Finish the login from a pasted address or a callback query. A wrong
    /// address keeps the login open; any other outcome ends it.
    pub(crate) fn complete_login(&mut self, input: &str) -> Result<()> {
        let login = self.login.as_ref().ok_or(SpotifyError::NoLogin)?;
        if self.now_ms().saturating_sub(login.started_ms) > LOGIN_TTL_MS {
            self.login = None;
            return Err(SpotifyError::LoginExpired);
        }
        let code = match redirect::code_from(input, &login.state) {
            Err(SpotifyError::WrongAddress) => return Err(SpotifyError::WrongAddress),
            other => other,
        };
        let login = self.login.take().expect("checked above");
        let token = auth::exchange_code(
            &self.http,
            &self.endpoints.accounts,
            &self.config.client_id,
            &code?,
            &login.verifier,
            &self.config.redirect_uri(),
            self.now_ms() / 1000,
        )?;
        token::save(&self.token_path, &token)?;
        self.token = Some(token);
        Ok(())
    }

    pub(crate) fn cancel_login(&mut self) {
        self.login = None;
    }

    pub(crate) fn logout(&mut self) -> Result<()> {
        self.token = None;
        self.login = None;
        token::remove(&self.token_path)
    }

    /// Run `f` with a fresh access token: refresh first when it is about to
    /// expire, and refresh once more and retry when the API answers 401.
    pub(crate) fn call<T>(&mut self, f: impl Fn(&Api<'_>) -> Result<T>) -> Result<T> {
        if !self.config.is_configured() {
            return Err(SpotifyError::NotConfigured);
        }
        let expired = match &self.token {
            None => return Err(SpotifyError::NotLoggedIn),
            Some(token) => token.needs_refresh(self.now_ms() / 1000),
        };
        if expired {
            self.refresh()?;
        }
        match self.run(&f) {
            Err(SpotifyError::Unauthorized) => {
                self.refresh()?;
                self.run(&f)
            }
            result => result,
        }
    }

    fn run<T>(&self, f: &impl Fn(&Api<'_>) -> Result<T>) -> Result<T> {
        let token = self.token.as_ref().ok_or(SpotifyError::NotLoggedIn)?;
        let api = Api {
            http: &self.http,
            base: &self.endpoints.api,
            token: &token.access_token,
        };
        f(&api)
    }

    fn refresh(&mut self) -> Result<()> {
        let refresh_token = match &self.token {
            Some(token) => token.refresh_token.clone(),
            None => return Err(SpotifyError::NotLoggedIn),
        };
        match auth::refresh(
            &self.http,
            &self.endpoints.accounts,
            &self.config.client_id,
            &refresh_token,
            self.now_ms() / 1000,
        ) {
            Ok(token) => {
                token::save(&self.token_path, &token)?;
                self.token = Some(token);
                Ok(())
            }
            Err(SpotifyError::Revoked) => {
                self.logout()?;
                Err(SpotifyError::Revoked)
            }
            Err(e) => Err(e),
        }
    }
}
```

- [ ] **Step 4: Run the tests**

Run: `cargo test -p znicz-spotify client`
Expected: 10 pass.

- [ ] **Step 5: Commit**

```bash
git add znicz-spotify/src/client.rs znicz-spotify/src/lib.rs
git commit -m "feat(spotify): client with login session and token refresh"
```

---

### Task 11: Callback listener

**Files:**
- Create: `znicz-spotify/src/listener.rs`
- Modify: `znicz-spotify/src/lib.rs` (add `mod listener;`)

- [ ] **Step 1: Write the failing tests**

Create `znicz-spotify/src/listener.rs`:

```rust
//! A one-shot HTTP listener on `127.0.0.1` for the login redirect. It hands
//! each `/callback` query to `on_query`; when that returns a page, the
//! listener answers with it and stops. It also stops when dropped.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    fn get(port: u16, target: &str) -> String {
        let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
        write!(stream, "GET {target} HTTP/1.1\r\nHost: x\r\n\r\n").unwrap();
        let mut reply = String::new();
        stream.read_to_string(&mut reply).unwrap();
        reply
    }

    #[test]
    fn hands_over_the_query_and_stops_when_done() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let log = seen.clone();
        let listener = CallbackListener::start(0, move |query| {
            log.lock().unwrap().push(query.to_string());
            if query.contains("state=right") {
                Some("Logged in. You can close this tab.".into())
            } else {
                None
            }
        })
        .unwrap();
        let port = listener.port();

        let wrong = get(port, "/callback?code=c&state=wrong");
        assert!(wrong.starts_with("HTTP/1.1 400"), "{wrong}");
        let other = get(port, "/favicon.ico");
        assert!(other.starts_with("HTTP/1.1 404"), "{other}");
        let done = get(port, "/callback?code=c&state=right");
        assert!(done.starts_with("HTTP/1.1 200"), "{done}");
        assert!(done.contains("You can close this tab."), "{done}");

        assert_eq!(
            *seen.lock().unwrap(),
            vec!["code=c&state=wrong", "code=c&state=right"]
        );
        // The listener has stopped: new connections are refused.
        thread::sleep(Duration::from_millis(300));
        assert!(TcpStream::connect(("127.0.0.1", port)).is_err());
    }

    #[test]
    fn stops_when_dropped() {
        let listener = CallbackListener::start(0, |_| None).unwrap();
        let port = listener.port();
        drop(listener);
        thread::sleep(Duration::from_millis(300));
        assert!(TcpStream::connect(("127.0.0.1", port)).is_err());
    }
}
```

Add `mod listener;` to `lib.rs`.

- [ ] **Step 2: Run to see it fail**

Run: `cargo test -p znicz-spotify listener`
Expected: compile error, `CallbackListener` missing.

- [ ] **Step 3: Implement**

Between the `use` lines and the tests:

```rust
/// How often the accept loop checks whether it should stop.
const POLL: Duration = Duration::from_millis(100);

/// The previous listener on the same port may take one `POLL` to let go.
const BIND_RETRIES: u32 = 5;

pub(crate) struct CallbackListener {
    port: u16,
    stop: Arc<AtomicBool>,
}

impl CallbackListener {
    /// `port` 0 picks a free port (tests). `on_query` gets the query string
    /// of each `/callback` request and returns the page to show when the
    /// login is finished (successfully or not), or `None` to keep waiting.
    pub(crate) fn start<F>(port: u16, mut on_query: F) -> std::io::Result<Self>
    where
        F: FnMut(&str) -> Option<String> + Send + 'static,
    {
        let listener = bind(port)?;
        listener.set_nonblocking(true)?;
        let port = listener.local_addr()?.port();
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = stop.clone();
        thread::Builder::new()
            .name("spotify-login".into())
            .spawn(move || {
                while !stopped.load(Ordering::SeqCst) {
                    match listener.accept() {
                        Ok((stream, _)) => {
                            if handle(stream, &mut on_query) {
                                break;
                            }
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => thread::sleep(POLL),
                        Err(e) => {
                            tracing::debug!(error = %e, "login listener accept failed");
                            thread::sleep(POLL);
                        }
                    }
                }
            })?;
        Ok(Self { port, stop })
    }

    pub(crate) fn port(&self) -> u16 {
        self.port
    }
}

impl Drop for CallbackListener {
    /// Only signals the thread; it exits within `POLL`. Joining here could
    /// deadlock when the owner holds a lock that `on_query` needs.
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}

fn bind(port: u16) -> std::io::Result<TcpListener> {
    let mut attempt = 0;
    loop {
        match TcpListener::bind(("127.0.0.1", port)) {
            Ok(listener) => return Ok(listener),
            Err(e) if attempt < BIND_RETRIES && e.kind() == std::io::ErrorKind::AddrInUse => {
                attempt += 1;
                thread::sleep(POLL);
            }
            Err(e) => return Err(e),
        }
    }
}

/// Answer one request. `true` when the login is finished.
fn handle<F>(mut stream: TcpStream, on_query: &mut F) -> bool
where
    F: FnMut(&str) -> Option<String>,
{
    let _ = stream.set_nonblocking(false);
    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
    let mut buf = [0u8; 8192];
    let n = stream.read(&mut buf).unwrap_or(0);
    let request = String::from_utf8_lossy(&buf[..n]);
    let target = request
        .lines()
        .next()
        .and_then(|line| line.split(' ').nth(1))
        .unwrap_or("");
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    if path != "/callback" {
        respond(&mut stream, 404, "Not found.");
        return false;
    }
    match on_query(query) {
        Some(page) => {
            respond(&mut stream, 200, &page);
            true
        }
        None => {
            respond(&mut stream, 400, "This page is not from the current Znicz login.");
            false
        }
    }
}

fn respond(stream: &mut TcpStream, status: u16, text: &str) {
    let body = format!(
        "<!doctype html><meta charset=\"utf-8\"><title>Znicz</title><p>{}</p>",
        escape(text)
    );
    let head = format!(
        "HTTP/1.1 {status} {}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        if status == 200 { "OK" } else { "Error" },
        body.len()
    );
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(body.as_bytes());
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}
```

- [ ] **Step 4: Run the tests**

Run: `cargo test -p znicz-spotify listener`
Expected: 2 pass.

- [ ] **Step 5: Commit**

```bash
git add znicz-spotify/src/listener.rs znicz-spotify/src/lib.rs
git commit -m "feat(spotify): one-shot login callback listener"
```

---

### Task 12: `SpotifyService`

**Files:**
- Create: `znicz-spotify/src/service.rs`
- Modify: `znicz-spotify/src/lib.rs` (final form below)

- [ ] **Step 1: Write the service skeleton, the pure interval function and the tests**

Create `znicz-spotify/src/service.rs`:

```rust
//! The Spotify service the player process hosts: a worker thread that polls
//! playback, and `SpotifyHost::apply` for every op. One `Client` behind a
//! mutex; one `SpotifyState` snapshot that front ends read.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, Weak};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use znicz_core::{
    SpotifyConfig, SpotifyHost, SpotifyLogin, SpotifyOp, SpotifyReply, SpotifyState,
};

use crate::api::Api;
use crate::client::{Client, Clock, Endpoints, system_clock};
use crate::error::{Result, SpotifyError};
use crate::listener::CallbackListener;

/// Poll every second while someone is watching...
const FAST_POLL: Duration = Duration::from_secs(1);
/// ...and every 15 s otherwise.
const SLOW_POLL: Duration = Duration::from_secs(15);
/// "Watching": a `Touch` or any op within this window.
const ACTIVE_WINDOW_MS: u64 = 60_000;
/// Devices are re-read at most this often.
const DEVICES_EVERY_MS: u64 = 15_000;
/// A user action waits for a 429 at most this long before retrying once.
const MAX_RETRY_WAIT_SECS: u64 = 10;

/// How long to wait before the next poll. `None`: not logged in, wait for a wake.
fn poll_interval(logged_in: bool, now_ms: u64, last_touch_ms: u64) -> Option<Duration> {
    if !logged_in {
        return None;
    }
    if now_ms.saturating_sub(last_touch_ms) < ACTIVE_WINDOW_MS {
        Some(FAST_POLL)
    } else {
        Some(SLOW_POLL)
    }
}

pub struct ServiceOptions {
    pub token_path: PathBuf,
    /// Called before an op that starts Spotify playback, so the player can
    /// pause its own engine.
    pub on_start_playback: Box<dyn Fn() + Send + Sync>,
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicUsize;
    use std::time::Instant;

    use super::*;
    use crate::http::test_server::{Response, Seen, serve};
    use crate::token::{self, Token};

    const CLIENT_ID: &str = "0123456789abcdef0123456789abcdef";

    #[test]
    fn polls_fast_while_watched_and_not_at_all_when_logged_out() {
        assert_eq!(poll_interval(false, 0, 0), None);
        assert_eq!(poll_interval(true, 59_999, 0), Some(FAST_POLL));
        assert_eq!(poll_interval(true, 60_000, 0), Some(SLOW_POLL));
    }

    struct Fixture {
        service: SpotifyService,
        seen: Seen,
        starts: Arc<AtomicUsize>,
        _dir: tempfile::TempDir,
    }

    fn fixture<F>(logged_in: bool, device: &str, handler: F) -> Fixture
    where
        F: Fn(&crate::http::test_server::Request) -> Response + Send + 'static,
    {
        let (origin, seen) = serve(handler);
        let dir = tempfile::tempdir().unwrap();
        let token_path = dir.path().join("spotify_token.json");
        if logged_in {
            let token = Token {
                access_token: "a".into(),
                refresh_token: "r".into(),
                expires_at: u64::MAX / 2,
                scope: String::new(),
            };
            token::save(&token_path, &token).unwrap();
        }
        let starts = Arc::new(AtomicUsize::new(0));
        let counter = starts.clone();
        let config = SpotifyConfig {
            client_id: CLIENT_ID.into(),
            redirect_port: free_port(),
            device: device.into(),
        };
        let options = ServiceOptions {
            token_path,
            on_start_playback: Box::new(move || {
                counter.fetch_add(1, Ordering::SeqCst);
            }),
        };
        let endpoints = Endpoints {
            api: format!("{origin}/v1"),
            accounts: origin,
        };
        let service = SpotifyService::start_with(config, options, endpoints, system_clock());
        Fixture {
            service,
            seen,
            starts,
            _dir: dir,
        }
    }

    fn free_port() -> u16 {
        std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port()
    }

    fn api_default(request: &crate::http::test_server::Request) -> Response {
        match request.path() {
            "/api/token" => Response::json(
                200,
                r#"{"access_token":"a","expires_in":3600,"refresh_token":"r"}"#,
            ),
            "/v1/me" => Response::json(200, r#"{"id":"u","display_name":"Eugene","product":"premium"}"#),
            "/v1/me/player" if request.method == "GET" => Response::json(
                200,
                r#"{"is_playing":true,"progress_ms":5,"currently_playing_type":"track",
                    "item":{"type":"track","uri":"spotify:track:t","name":"So What","duration_ms":9,
                            "artists":[],"album":{"name":"Kind of Blue","images":[]}}}"#,
            ),
            "/v1/me/player/devices" => Response::json(
                200,
                r#"{"devices":[{"id":"k1","name":"Kitchen","type":"Speaker","is_active":false}]}"#,
            ),
            _ => Response::empty(204),
        }
    }

    fn wait_for(service: &SpotifyService, what: impl Fn(&SpotifyState) -> bool) -> SpotifyState {
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            let state = service.state();
            if what(&state) || Instant::now() > deadline {
                return state;
            }
            thread::sleep(Duration::from_millis(20));
        }
    }

    fn requests(seen: &Seen) -> Vec<String> {
        seen.lock()
            .unwrap()
            .iter()
            .map(|r| format!("{} {}", r.method, r.target))
            .collect()
    }

    fn play() -> SpotifyOp {
        SpotifyOp::Play {
            context_uri: Some("spotify:album:a1".into()),
            uris: vec![],
            offset: None,
        }
    }

    #[test]
    fn starts_logged_out_without_a_token_and_refuses_control() {
        let f = fixture(false, "", api_default);
        assert_eq!(f.service.state().login, SpotifyLogin::NotLoggedIn);
        let error = f.service.apply(SpotifyOp::Pause).unwrap_err();
        assert_eq!(error.to_string(), "not logged in to Spotify");
    }

    #[test]
    fn play_pauses_znicz_first_and_calls_the_api() {
        let f = fixture(true, "", api_default);
        f.service.apply(play()).unwrap();
        assert_eq!(f.starts.load(Ordering::SeqCst), 1);
        assert!(requests(&f.seen).contains(&"PUT /v1/me/player/play".to_string()));
        f.service.apply(SpotifyOp::Pause).unwrap();
        assert_eq!(f.starts.load(Ordering::SeqCst), 1, "pause does not count");
    }

    #[test]
    fn no_active_device_falls_back_to_the_preferred_one() {
        let f = fixture(true, "Kitchen", |request| {
            if request.target == "/v1/me/player/play" {
                Response::json(404, r#"{"error":{"status":404,"message":"No active device found","reason":"NO_ACTIVE_DEVICE"}}"#)
            } else {
                api_default(request)
            }
        });
        f.service.apply(play()).unwrap();
        let seen = requests(&f.seen);
        assert!(seen.contains(&"GET /v1/me/player/devices".to_string()), "{seen:?}");
        assert!(seen.contains(&"PUT /v1/me/player/play?device_id=k1".to_string()), "{seen:?}");
    }

    #[test]
    fn no_active_device_without_a_preference_is_reported() {
        let f = fixture(true, "", |request| {
            if request.path() == "/v1/me/player/play" {
                Response::json(404, r#"{"error":{"reason":"NO_ACTIVE_DEVICE"}}"#)
            } else {
                api_default(request)
            }
        });
        let error = f.service.apply(play()).unwrap_err();
        assert_eq!(error.to_string(), SpotifyError::NoActiveDevice.to_string());
        assert_eq!(f.service.state().last_error, Some(error.to_string()));
    }

    #[test]
    fn a_429_waits_and_retries_once() {
        let hits = Arc::new(AtomicUsize::new(0));
        let counter = hits.clone();
        let f = fixture(true, "", move |request| {
            if request.path() == "/v1/me/player/next" {
                if counter.fetch_add(1, Ordering::SeqCst) == 0 {
                    return Response::empty(429).with_header("Retry-After", "1");
                }
                return Response::empty(204);
            }
            api_default(request)
        });
        let started = Instant::now();
        f.service.apply(SpotifyOp::Next).unwrap();
        assert!(started.elapsed() >= Duration::from_millis(900));
        assert_eq!(hits.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn login_by_paste() {
        let f = fixture(false, "", api_default);
        let url = match f.service.apply(SpotifyOp::BeginLogin).unwrap() {
            SpotifyReply::AuthorizeUrl { url } => url,
            other => panic!("{other:?}"),
        };
        assert_eq!(f.service.state().login, SpotifyLogin::LoggingIn);
        let state = url.split("state=").nth(1).unwrap().split('&').next().unwrap();
        assert!(f
            .service
            .apply(SpotifyOp::CompleteLogin { redirect_url: "code=x&state=nope".into() })
            .is_err());
        assert_eq!(f.service.state().login, SpotifyLogin::LoggingIn, "a wrong paste keeps waiting");
        f.service
            .apply(SpotifyOp::CompleteLogin {
                redirect_url: format!("code=x&state={state}"),
            })
            .unwrap();
        let state = f.service.state();
        assert_eq!(state.login, SpotifyLogin::LoggedIn);
        assert_eq!(state.account.unwrap().user, "Eugene");
    }

    #[test]
    fn login_by_callback() {
        let f = fixture(false, "", api_default);
        let url = match f.service.apply(SpotifyOp::BeginLogin).unwrap() {
            SpotifyReply::AuthorizeUrl { url } => url,
            other => panic!("{other:?}"),
        };
        let state = url.split("state=").nth(1).unwrap().split('&').next().unwrap().to_string();
        let port = f.service.redirect_port();
        let mut stream = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
        use std::io::{Read, Write};
        write!(stream, "GET /callback?code=x&state={state} HTTP/1.1\r\n\r\n").unwrap();
        let mut page = String::new();
        stream.read_to_string(&mut page).unwrap();
        assert!(page.contains("You can close this tab"), "{page}");
        let state = wait_for(&f.service, |s| s.login == SpotifyLogin::LoggedIn);
        assert_eq!(state.login, SpotifyLogin::LoggedIn);
    }

    #[test]
    fn touch_polls_playback_at_once() {
        let f = fixture(true, "", api_default);
        f.service.apply(SpotifyOp::Touch).unwrap();
        let state = wait_for(&f.service, |s| s.playback.is_some());
        let playback = state.playback.unwrap();
        assert!(playback.is_playing);
        assert_eq!(playback.item.unwrap().title, "So What");
        assert_eq!(state.devices.len(), 1);
    }

    #[test]
    fn pause_if_playing_only_pauses_a_playing_spotify() {
        let f = fixture(true, "", api_default);
        f.service.pause_if_playing();
        assert!(!requests(&f.seen).contains(&"PUT /v1/me/player/pause".to_string()));
        f.service.apply(SpotifyOp::Touch).unwrap();
        wait_for(&f.service, |s| s.is_playing());
        f.service.pause_if_playing();
        assert!(requests(&f.seen).contains(&"PUT /v1/me/player/pause".to_string()));
    }

    #[test]
    fn a_playback_answer_tells_whether_the_account_has_premium() {
        let f = fixture(true, "", |request| match request.path() {
            "/v1/me" => Response::json(200, r#"{"id":"u","display_name":"Eugene"}"#),
            "/v1/me/player/next" => Response::json(
                403,
                r#"{"error":{"status":403,"message":"Player command failed: Premium required","reason":"PREMIUM_REQUIRED"}}"#,
            ),
            _ => api_default(request),
        });
        f.service.apply(SpotifyOp::Touch).unwrap();
        let state = wait_for(&f.service, |s| s.account.is_some());
        assert_eq!(state.account.unwrap().premium, None, "unknown until a refusal");
        f.service.apply(SpotifyOp::Next).unwrap_err();
        assert_eq!(f.service.state().account.unwrap().premium, Some(false));
        f.service.apply(SpotifyOp::Pause).unwrap();
        assert_eq!(f.service.state().account.unwrap().premium, Some(true));
    }

    #[test]
    fn logout_clears_the_state() {
        let f = fixture(true, "", api_default);
        f.service.apply(SpotifyOp::Touch).unwrap();
        wait_for(&f.service, |s| s.playback.is_some());
        f.service.apply(SpotifyOp::Logout).unwrap();
        let state = f.service.state();
        assert_eq!(state.login, SpotifyLogin::NotLoggedIn);
        assert_eq!(state.playback, None);
        assert_eq!(state.account, None);
    }

    #[test]
    fn configure_without_a_client_id_is_not_configured() {
        let f = fixture(true, "", api_default);
        f.service
            .apply(SpotifyOp::Configure { config: SpotifyConfig::default() })
            .unwrap();
        assert_eq!(f.service.state().login, SpotifyLogin::NotConfigured);
        let error = f.service.apply(SpotifyOp::Devices).unwrap_err();
        assert_eq!(error.to_string(), SpotifyError::NotConfigured.to_string());
    }
}
```

- [ ] **Step 2: Run to see it fail**

Run: `cargo test -p znicz-spotify service`
Expected: compile errors (`SpotifyService` missing).

- [ ] **Step 3: Implement the service**

Between `ServiceOptions` and the tests:

```rust
struct Shared {
    clock: Clock,
    client: Mutex<Client>,
    /// Bumped whenever the login ends, so a poll that started before a logout
    /// does not write its result into the cleared state.
    login_epoch: AtomicU64,
    state: Mutex<SpotifyState>,
    listener: Mutex<Option<CallbackListener>>,
    last_touch_ms: AtomicU64,
    last_devices_ms: AtomicU64,
    /// No polls before this time (after a 429).
    backoff_until_ms: AtomicU64,
    /// The last poll error shown, so the same one is not repeated every second.
    last_poll_error: Mutex<Option<String>>,
    wake: (Mutex<bool>, Condvar),
    stop: AtomicBool,
    on_start_playback: Box<dyn Fn() + Send + Sync>,
}

pub struct SpotifyService {
    shared: Arc<Shared>,
    worker: Option<JoinHandle<()>>,
}

impl SpotifyService {
    pub fn start(config: SpotifyConfig, options: ServiceOptions) -> Self {
        Self::start_with(config, options, Endpoints::default(), system_clock())
    }

    pub(crate) fn start_with(
        config: SpotifyConfig,
        options: ServiceOptions,
        endpoints: Endpoints,
        clock: Clock,
    ) -> Self {
        let client = Client::new(config, options.token_path, endpoints, clock.clone());
        let state = SpotifyState {
            login: login_of(&client),
            ..SpotifyState::default()
        };
        let shared = Arc::new(Shared {
            clock,
            client: Mutex::new(client),
            login_epoch: AtomicU64::new(0),
            state: Mutex::new(state),
            listener: Mutex::new(None),
            last_touch_ms: AtomicU64::new(0),
            last_devices_ms: AtomicU64::new(0),
            backoff_until_ms: AtomicU64::new(0),
            last_poll_error: Mutex::new(None),
            wake: (Mutex::new(false), Condvar::new()),
            stop: AtomicBool::new(false),
            on_start_playback: options.on_start_playback,
        });
        let worker_shared = shared.clone();
        let worker = thread::Builder::new()
            .name("spotify".into())
            .spawn(move || worker_shared.run())
            .expect("spawn the Spotify thread");
        Self {
            shared,
            worker: Some(worker),
        }
    }

    /// Pause Spotify if it is playing: Znicz is about to play.
    pub fn pause_if_playing(&self) {
        if !self.shared.state.lock().unwrap().is_playing() {
            return;
        }
        if let Err(e) = self.shared.control(&SpotifyOp::Pause) {
            tracing::warn!(error = %e, "could not pause Spotify");
        }
    }

    /// The port the login listener uses (tests).
    #[cfg(test)]
    fn redirect_port(&self) -> u16 {
        self.shared.client.lock().unwrap().config().redirect_port
    }
}

impl Drop for SpotifyService {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::SeqCst);
        self.shared.wake();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

impl SpotifyHost for SpotifyService {
    fn apply(&self, op: SpotifyOp) -> znicz_core::Result<SpotifyReply> {
        let result = self.shared.apply(op);
        if let Err(e) = &result {
            self.shared.state.lock().unwrap().last_error = Some(e.to_string());
        }
        Ok(result?)
    }

    fn state(&self) -> SpotifyState {
        self.shared.state.lock().unwrap().clone()
    }
}

fn login_of(client: &Client) -> SpotifyLogin {
    if !client.config().is_configured() {
        SpotifyLogin::NotConfigured
    } else if client.login_pending() {
        SpotifyLogin::LoggingIn
    } else if client.is_logged_in() {
        SpotifyLogin::LoggedIn
    } else {
        SpotifyLogin::NotLoggedIn
    }
}

impl Shared {
    /// Never takes the client lock, which a poll may hold for a whole request.
    fn now_ms(&self) -> u64 {
        (self.clock)()
    }

    fn wake(&self) {
        let (flag, condvar) = &self.wake;
        *flag.lock().unwrap() = true;
        condvar.notify_all();
    }

    fn touch(&self) {
        self.last_touch_ms.store(self.now_ms(), Ordering::SeqCst);
    }

    fn set_login(&self, login: SpotifyLogin) {
        let mut state = self.state.lock().unwrap();
        if login != SpotifyLogin::LoggedIn {
            self.login_epoch.fetch_add(1, Ordering::SeqCst);
            state.account = None;
            state.playback = None;
            state.devices.clear();
        }
        state.login = login;
    }

    fn apply(self: &Arc<Self>, op: SpotifyOp) -> Result<SpotifyReply> {
        self.touch();
        match op {
            SpotifyOp::Touch => {
                self.wake();
                Ok(SpotifyReply::Done)
            }
            SpotifyOp::Configure { config } => {
                let login = {
                    let mut client = self.client.lock().unwrap();
                    client.set_config(config);
                    login_of(&client)
                };
                self.listener.lock().unwrap().take();
                self.set_login(login);
                self.wake();
                Ok(SpotifyReply::Done)
            }
            SpotifyOp::BeginLogin => self.begin_login(),
            SpotifyOp::CompleteLogin { redirect_url } => {
                self.finish_login(&redirect_url)?;
                Ok(SpotifyReply::Done)
            }
            SpotifyOp::CancelLogin => {
                let login = {
                    let mut client = self.client.lock().unwrap();
                    client.cancel_login();
                    login_of(&client)
                };
                self.listener.lock().unwrap().take();
                self.set_login(login);
                Ok(SpotifyReply::Done)
            }
            SpotifyOp::Logout => {
                let result = self.client.lock().unwrap().logout();
                self.listener.lock().unwrap().take();
                let login = login_of(&self.client.lock().unwrap());
                self.set_login(login);
                result.map(|()| SpotifyReply::Done)
            }
            SpotifyOp::Search { query, kinds, limit } => self.page(|api| api.search(&query, &kinds, limit)),
            SpotifyOp::Playlists { offset, limit } => self.page(|api| api.playlists(offset, limit)),
            SpotifyOp::LikedSongs { offset, limit } => self.page(|api| api.liked_songs(offset, limit)),
            SpotifyOp::Albums { offset, limit } => self.page(|api| api.albums(offset, limit)),
            SpotifyOp::Tracks { uri, offset, limit } => self.page(|api| api.tracks(&uri, offset, limit)),
            SpotifyOp::ArtistAlbums { uri, offset, limit } => {
                self.page(|api| api.artist_albums(&uri, offset, limit))
            }
            SpotifyOp::Devices => {
                let devices = self.client.lock().unwrap().call(|api| api.devices())?;
                self.state.lock().unwrap().devices = devices.clone();
                Ok(SpotifyReply::Devices { devices })
            }
            SpotifyOp::Queue => {
                let (current, next) = self.client.lock().unwrap().call(|api| api.queue())?;
                Ok(SpotifyReply::Queue { current, next })
            }
            control => {
                self.control(&control)?;
                Ok(SpotifyReply::Done)
            }
        }
    }

    fn page(&self, f: impl Fn(&Api<'_>) -> Result<znicz_core::SpotifyPage>) -> Result<SpotifyReply> {
        let page = self.client.lock().unwrap().call(f)?;
        Ok(SpotifyReply::Page { page })
    }

    fn begin_login(self: &Arc<Self>) -> Result<SpotifyReply> {
        let (url, port) = {
            let mut client = self.client.lock().unwrap();
            let login = client.begin_login()?;
            (login.url, client.config().redirect_port)
        };
        // Before the listener starts: a fast callback must not be overwritten.
        self.set_login(SpotifyLogin::LoggingIn);
        let weak: Weak<Shared> = Arc::downgrade(self);
        let started = CallbackListener::start(port, move |query| {
            let shared = weak.upgrade()?;
            match shared.finish_login(query) {
                Ok(()) => Some("Logged in to Spotify. You can close this tab.".into()),
                Err(SpotifyError::WrongAddress) => None,
                Err(e) => Some(format!("Spotify login failed: {e}")),
            }
        });
        match started {
            Ok(listener) => *self.listener.lock().unwrap() = Some(listener),
            Err(e) => {
                tracing::warn!(port, error = %e, "login listener could not start; paste the address instead");
                self.state.lock().unwrap().last_error = Some(format!(
                    "port {port} is busy: paste the address from the browser instead"
                ));
            }
        }
        Ok(SpotifyReply::AuthorizeUrl { url })
    }

    /// Finish a login from a callback query or a pasted address.
    fn finish_login(&self, input: &str) -> Result<()> {
        let result = self.client.lock().unwrap().complete_login(input);
        match result {
            Ok(()) => {
                self.listener.lock().unwrap().take();
                self.set_login(SpotifyLogin::LoggedIn);
                self.refresh_account();
                self.wake();
                Ok(())
            }
            Err(SpotifyError::WrongAddress) => Err(SpotifyError::WrongAddress),
            Err(e) => {
                self.listener.lock().unwrap().take();
                self.set_login(SpotifyLogin::Failed(e.to_string()));
                Err(e)
            }
        }
    }

    fn refresh_account(&self) {
        match self.client.lock().unwrap().call(|api| api.me()) {
            Ok(account) => self.state.lock().unwrap().account = Some(account),
            Err(e) => tracing::warn!(error = %e, "could not read the Spotify account"),
        }
    }

    /// A playback command: pause Znicz first when it starts Spotify, fall
    /// back to the preferred device when none is active, and wait out one 429.
    fn control(&self, op: &SpotifyOp) -> Result<()> {
        if op.starts_playback() {
            (self.on_start_playback)();
        }
        let result = match self.control_once(op, None) {
            Err(SpotifyError::NoActiveDevice) => match self.preferred_device()? {
                Some(id) => self.control_once(op, Some(&id)),
                None => Err(SpotifyError::NoActiveDevice),
            },
            Err(SpotifyError::RateLimited { retry_after_secs }) if retry_after_secs <= MAX_RETRY_WAIT_SECS => {
                thread::sleep(Duration::from_secs(retry_after_secs));
                self.control_once(op, None)
            }
            other => other,
        };
        // Spotify no longer reports the plan, so a playback answer is how we learn it.
        match &result {
            Ok(()) => {
                self.set_premium(true);
                self.after_control(op);
                self.state.lock().unwrap().last_error = None;
                self.wake();
            }
            Err(SpotifyError::PremiumRequired) => self.set_premium(false),
            Err(_) => {}
        }
        result
    }

    fn set_premium(&self, premium: bool) {
        if let Some(account) = self.state.lock().unwrap().account.as_mut() {
            account.premium = Some(premium);
        }
    }

    fn control_once(&self, op: &SpotifyOp, device: Option<&str>) -> Result<()> {
        let mut client = self.client.lock().unwrap();
        client.call(|api| match op {
            SpotifyOp::Play { context_uri, uris, offset } => {
                api.play(device, context_uri.as_deref(), uris, *offset)
            }
            SpotifyOp::Pause => api.pause(device),
            SpotifyOp::Next => api.next(device),
            SpotifyOp::Previous => api.previous(device),
            SpotifyOp::Seek { position_ms } => api.seek(device, *position_ms),
            SpotifyOp::SetVolume { volume } => api.volume(device, *volume),
            SpotifyOp::SetRepeat { mode } => api.repeat(device, *mode),
            SpotifyOp::SetShuffle { on } => api.shuffle(device, *on),
            SpotifyOp::QueueAdd { uri } => api.queue_add(device, uri),
            SpotifyOp::Transfer { device_id, play } => api.transfer(device_id, *play),
            other => Err(SpotifyError::Parse(format!("{other:?} is not a playback command"))),
        })
    }

    /// The id of the device named in the settings, if it is online.
    fn preferred_device(&self) -> Result<Option<String>> {
        let mut client = self.client.lock().unwrap();
        let name = client.config().device.clone();
        if name.is_empty() {
            return Ok(None);
        }
        let devices = client.call(|api| api.devices())?;
        drop(client);
        let id = devices.iter().find(|d| d.name == name).map(|d| d.id.clone());
        self.state.lock().unwrap().devices = devices;
        Ok(id)
    }

    /// Show the obvious effect at once, before the next poll confirms it.
    fn after_control(&self, op: &SpotifyOp) {
        let now = self.now_ms();
        let mut state = self.state.lock().unwrap();
        let Some(playback) = state.playback.as_mut() else {
            return;
        };
        match op {
            SpotifyOp::Pause => {
                playback.progress_ms = playback.progress_at(now);
                playback.read_at_ms = now;
                playback.is_playing = false;
            }
            SpotifyOp::Seek { position_ms } => {
                playback.progress_ms = *position_ms;
                playback.read_at_ms = now;
            }
            SpotifyOp::SetRepeat { mode } => playback.repeat = *mode,
            SpotifyOp::SetShuffle { on } => playback.shuffle = *on,
            _ => {}
        }
    }

    fn run(self: Arc<Self>) {
        loop {
            let interval = {
                let client = self.client.lock().unwrap();
                poll_interval(
                    client.is_logged_in(),
                    client.now_ms(),
                    self.last_touch_ms.load(Ordering::SeqCst),
                )
            };
            {
                let (flag, condvar) = &self.wake;
                let mut woken = flag.lock().unwrap();
                if !*woken {
                    woken = match interval {
                        Some(interval) => condvar.wait_timeout(woken, interval).unwrap().0,
                        None => condvar.wait_timeout(woken, SLOW_POLL).unwrap().0,
                    };
                }
                *woken = false;
            }
            if self.stop.load(Ordering::SeqCst) {
                return;
            }
            if self.client.lock().unwrap().is_logged_in() {
                self.poll();
            }
        }
    }

    fn poll(&self) {
        let now = self.now_ms();
        if now < self.backoff_until_ms.load(Ordering::SeqCst) {
            return;
        }
        let epoch = self.login_epoch.load(Ordering::SeqCst);
        let need_account = self.state.lock().unwrap().account.is_none();
        if need_account {
            self.refresh_account();
        }
        let result = self.client.lock().unwrap().call(|api| api.playback(now));
        let result = result.and_then(|playback| {
            let due = now.saturating_sub(self.last_devices_ms.load(Ordering::SeqCst)) >= DEVICES_EVERY_MS;
            let devices = if due {
                self.last_devices_ms.store(now, Ordering::SeqCst);
                Some(self.client.lock().unwrap().call(|api| api.devices())?)
            } else {
                None
            };
            Ok((playback, devices))
        });
        if self.login_epoch.load(Ordering::SeqCst) != epoch {
            return;
        }
        match result {
            Ok((playback, devices)) => {
                let mut state = self.state.lock().unwrap();
                if self.login_epoch.load(Ordering::SeqCst) != epoch {
                    return;
                }
                state.playback = playback;
                if let Some(devices) = devices {
                    state.devices = devices;
                }
                let mut last = self.last_poll_error.lock().unwrap();
                if last.is_some() && state.last_error == *last {
                    state.last_error = None;
                }
                *last = None;
            }
            Err(SpotifyError::Revoked) => {
                self.set_login(SpotifyLogin::NotLoggedIn);
                self.state.lock().unwrap().last_error = Some(SpotifyError::Revoked.to_string());
            }
            Err(SpotifyError::RateLimited { retry_after_secs }) => {
                self.backoff_until_ms
                    .store(now + retry_after_secs * 1000, Ordering::SeqCst);
            }
            Err(e) => {
                let text = e.to_string();
                let mut last = self.last_poll_error.lock().unwrap();
                if last.as_deref() != Some(text.as_str()) {
                    tracing::warn!(error = %text, "Spotify poll failed");
                    self.state.lock().unwrap().last_error = Some(text.clone());
                    *last = Some(text);
                }
            }
        }
    }
}
```

**Lock order:** `client` before `state`, and never `listener` while holding `client`. `now_ms` reads the clock directly so it never waits for the client lock. Every method above takes the locks in that order or one at a time; keep it that way in later parts.

- [ ] **Step 4: Final `lib.rs`**

Replace `znicz-spotify/src/lib.rs` with:

```rust
//! Spotify control through the Web API: login (OAuth PKCE), browse and
//! playback calls, and a polling service the player process hosts.
//!
//! Spotify plays the audio; Znicz only sends commands and reads state.

mod api;
mod auth;
mod client;
mod error;
mod http;
mod listener;
mod model;
mod pkce;
mod redirect;
mod service;
mod token;

pub use error::{Result, SpotifyError};
pub use service::{ServiceOptions, SpotifyService};
pub use token::default_token_path;
```

- [ ] **Step 5: Run the tests**

Run: `cargo test -p znicz-spotify`
Expected: everything passes (service: 12). Run it three times to make sure the thread and timing tests are not flaky: `for i in 1 2 3; do cargo test -q -p znicz-spotify || break; done`.

- [ ] **Step 6: Lint**

Run: `cargo clippy -p znicz-spotify -p znicz-core --all-targets -- -D warnings` and `cargo fmt --all`.
Expected: no warnings. Fix any (for example `too_many_arguments` on `exchange_code`: add `#[allow(clippy::too_many_arguments)]` above it, since its parameters are the OAuth form fields).

- [ ] **Step 7: Commit**

```bash
git add znicz-spotify
git commit -m "feat(spotify): SpotifyService with polling, login and playback ops"
```

---

### Task 13: Docs and licences

**Files:**
- Create: `wiki/Architecture/Spotify.md`
- Modify: `wiki/Architecture/Overview.md`, `wiki/_Sidebar.md`, `THIRD-PARTY-LICENSES.html` (generated)

- [ ] **Step 1: Write `wiki/Architecture/Spotify.md`**

```markdown
# Spotify (`znicz-spotify`)

Znicz can control the Spotify app through the Spotify Web API (Spotify
Connect). Spotify decodes and plays the audio on whichever device it runs on;
Znicz only sends commands and reads what is playing.

## What you need

- **Spotify Premium.** The Web API's playback calls refuse free accounts
  (`needs Spotify Premium`). Browsing and search work without it. Spotify no
  longer reports an account's plan, so Znicz shows it only after the first
  playback call has been accepted or refused.
- **Your own Spotify app.** Spotify does not let open-source desktop apps
  share one client ID, so register a free app at
  <https://developer.spotify.com/dashboard>, add the redirect URI
  `http://127.0.0.1:8898/callback` (or your port), and give Znicz the client
  ID. No client secret is needed: login uses PKCE. The app's owner needs
  Premium, and an app in Development Mode allows at most 5 users (add each
  under *User Management*).

## Development Mode limits (February 2026)

Spotify's rules for Development Mode apps (in force since March 9, 2026)
shape what Znicz can browse:

- A playlist's tracks are listed only for playlists you own or collaborate on
  (`/playlists/{id}/items`). Others still play with `Enter`; opening one says
  so.
- Artists have no top-track list any more; an artist opens to their albums.
- Search returns at most 10 results per type.

## The crate

| Module | Job |
| --- | --- |
| `pkce` | Verifier, S256 challenge, `state` |
| `redirect` | Read the code from a pasted address or callback query |
| `token` | `spotify_token.json` (data folder, owner-only, written atomically) |
| `http` | Blocking `ureq` transport; status → `SpotifyError` |
| `auth` | Authorize URL, code exchange, refresh |
| `model` | Web API JSON → `znicz-core` types |
| `api` | One method per endpoint |
| `client` | Settings, token, login session; refresh before expiry and once on 401 |
| `listener` | One-shot `127.0.0.1` callback listener |
| `service` | `SpotifyService`: polling thread and `SpotifyHost` |

The shared types (`SpotifyOp`, `SpotifyReply`, `SpotifyState`, …) and the
`SpotifyHost` trait live in `znicz-core` (`spotify_host.rs`), like
`LibraryHost`, so core does not depend on this crate.

## Login

A login keeps a PKCE session for 10 minutes and starts a listener on
`127.0.0.1:<port>`. It finishes by whichever comes first:

1. the browser's redirect reaching the listener (a local session, or
   `ssh -L <port>:127.0.0.1:<port>`);
2. the address the browser landed on, pasted back (works anywhere; the
   browser shows "can't connect", which is expected).

The `state` in the address must match the session. A different client ID
logs out, because the old token cannot be refreshed with it.

## Polling

About every second while someone watches (a `Touch`, or any op in the last
minute), every 15 s otherwise, and not at all while logged out. Devices are
re-read at most every 15 s. After a 429 the service skips polls until
`Retry-After` has passed; a user command waits it out (up to 10 s) and retries
once. The same poll error is shown once, not every second.

## Errors

| Case | Message |
| --- | --- |
| No client ID | `Spotify is not configured: set a client ID in Settings → Spotify` |
| Refused client ID / redirect URI | names the exact URI to register |
| Revoked refresh token | `Spotify login expired`; the token file is deleted |
| 403 on playback | `needs Spotify Premium` |
| No active device | retries on the preferred device if set and online, else `Spotify has no active device: …` |

Tokens never appear in logs (`Token`'s `Debug` redacts them).
```

- [ ] **Step 2: Overview and sidebar**

In `wiki/Architecture/Overview.md`, change "Znicz is one workspace with five crates." to "Znicz is one workspace with six crates." and add under the diagram's explanation, after the paragraph about the library service:

```markdown
`znicz-spotify` controls the Spotify app through the Web API. The player
process will host its `SpotifyService` the way it hosts the library service;
see [Spotify](Spotify.md).
```

In `wiki/_Sidebar.md`, add a `[Spotify](Architecture/Spotify)` line after the Library entry, following that file's existing link format.

- [ ] **Step 3: Regenerate the licence list**

Run: `scripts/third-party-licenses.sh`
Expected: `THIRD-PARTY-LICENSES.html` changes only if the release build's dependency set changed. The crate is not linked into the `znicz` binary until part 2, so no change is expected yet; commit it if it did change.

- [ ] **Step 4: Full check**

Run: `cargo test --workspace -q && cargo clippy --workspace --all-targets -- -D warnings && cargo fmt --all --check`
Expected: all pass.

- [ ] **Step 5: Commit**

```bash
git add wiki THIRD-PARTY-LICENSES.html
git commit -m "docs(wiki): the znicz-spotify crate"
```

---

## Self-review against the spec

| Spec item | Where |
| --- | --- |
| New crate, blocking `ureq`, `auth` / `api` / `service` | Tasks 2–12 (`auth` split into `pkce`, `redirect`, `token`, `auth`, `listener`) |
| `SpotifyHost`, `SpotifyOp` in core | Task 1 |
| Token in data folder, atomic, `0600`, redacted | Task 5 |
| PKCE, scopes, 10 min session, listener on `127.0.0.1`, paste route, `state` check | Tasks 3, 4, 7, 10, 11, 12 |
| Refresh before expiry and once on 401; revoked → logged out | Task 10 |
| 403 Premium, 404 no device → preferred device, 429 backoff and one retry, network errors shown once | Tasks 6, 12 |
| Polling 1 s / 15 s / off | Task 12 |
| Pause rule hook (both directions) | Task 12 (`on_start_playback`, `pause_if_playing`); wired in part 2 |
| `Configure` without restart | Task 12 |
| IPC, CLI, config, MCP, TUI | Parts 2–5 |
| Version bump, CHANGELOG, README | Part 2 (first user-visible part) and the final PR |
