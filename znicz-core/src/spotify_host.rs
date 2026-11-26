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
    /// `None` until known: Spotify stopped reporting the plan in 2026, so it is
    /// learned from a playback refusal.
    pub premium: Option<bool>,
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

/// What went wrong with a Spotify request, so that agents and front ends can
/// act on it. The names are the `kind` in the MCP tool errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpotifyErrorKind {
    /// No client ID is set.
    NotConfigured,
    /// No login, or the login was removed.
    NotLoggedIn,
    /// The login was started too long ago.
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
    /// The player is an older version without Spotify support.
    OutdatedPlayer,
    /// Anything else, including a kind from a newer player.
    #[serde(other)]
    Other,
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
    /// Not meaningful for a search over several kinds; page one kind at a time (see `SpotifyOp::Search`).
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
    Configure {
        config: SpotifyConfig,
    },
    BeginLogin,
    /// Finish a login from a pasted redirect address (or its query).
    CompleteLogin {
        redirect_url: String,
    },
    CancelLogin,
    Logout,
    /// Keeps the fast poll going for another minute.
    Touch,
    /// `offset` and `limit` apply to each kind. With several kinds, `total` is the largest per-kind total, so page one kind at a time.
    Search {
        query: String,
        kinds: Vec<SpotifyKind>,
        #[serde(default)]
        offset: u32,
        limit: u32,
    },
    Playlists {
        offset: u32,
        limit: u32,
    },
    LikedSongs {
        offset: u32,
        limit: u32,
    },
    Albums {
        offset: u32,
        limit: u32,
    },
    /// An album's or playlist's tracks, or an artist's top tracks.
    Tracks {
        uri: String,
        offset: u32,
        limit: u32,
    },
    ArtistAlbums {
        uri: String,
        offset: u32,
        limit: u32,
    },
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
    Seek {
        position_ms: u64,
    },
    /// 0.0–1.0, like the local volume.
    SetVolume {
        volume: f32,
    },
    SetRepeat {
        mode: SpotifyRepeat,
    },
    SetShuffle {
        on: bool,
    },
    QueueAdd {
        uri: String,
    },
    Transfer {
        device_id: String,
        play: bool,
    },
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
    AuthorizeUrl {
        url: String,
        /// The redirect URI the Spotify app must have registered.
        redirect_uri: String,
        /// False when the callback listener could not bind its port: the user must paste the address.
        listening: bool,
    },
    Page {
        page: SpotifyPage,
    },
    Devices {
        devices: Vec<SpotifyDevice>,
    },
    Queue {
        current: Option<SpotifyTrack>,
        next: Vec<SpotifyTrack>,
    },
}

/// Implemented by the in-process `SpotifyService` and by the IPC client.
pub trait SpotifyHost: Send + Sync {
    fn apply(&self, op: SpotifyOp) -> Result<SpotifyReply>;
    fn state(&self) -> SpotifyState;

    /// Like `state`, but reports a host that cannot be reached.
    fn try_state(&self) -> Result<SpotifyState> {
        Ok(self.state())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixed(SpotifyState);

    impl SpotifyHost for Fixed {
        fn apply(&self, _op: SpotifyOp) -> Result<SpotifyReply> {
            Ok(SpotifyReply::Done)
        }
        fn state(&self) -> SpotifyState {
            self.0.clone()
        }
    }

    #[test]
    fn try_state_returns_the_state() {
        let state = SpotifyState {
            last_error: Some("busy".into()),
            ..SpotifyState::default()
        };
        let host = Fixed(state.clone());
        assert_eq!(host.try_state().unwrap(), state);
    }

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
            offset: 20,
            limit: 10,
        };
        let text = serde_json::to_string(&op).unwrap();
        assert!(text.contains(r#""op":"search""#), "{text}");
        assert_eq!(serde_json::from_str::<SpotifyOp>(&text).unwrap(), op);

        let reply = SpotifyReply::AuthorizeUrl {
            url: "u".into(),
            redirect_uri: "http://127.0.0.1:8898/callback".into(),
            listening: true,
        };
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

    #[test]
    fn an_unknown_error_kind_from_a_newer_player_reads_as_other() {
        assert_eq!(
            serde_json::from_str::<SpotifyErrorKind>("\"some_new_kind\"").unwrap(),
            SpotifyErrorKind::Other
        );
    }

    #[test]
    fn error_kinds_are_snake_case_on_the_wire() {
        let text = serde_json::to_string(&SpotifyErrorKind::PremiumRequired).unwrap();
        assert_eq!(text, r#""premium_required""#);
        assert_eq!(
            serde_json::from_str::<SpotifyErrorKind>(r#""no_active_device""#).unwrap(),
            SpotifyErrorKind::NoActiveDevice
        );
    }
}
