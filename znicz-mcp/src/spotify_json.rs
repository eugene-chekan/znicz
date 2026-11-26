//! Spotify tool arguments, and the JSON the Spotify tools return. The tools in
//! `server.rs` send the ops built here. Everything in this file is pure.
use std::time::{SystemTime, UNIX_EPOCH};

use rmcp::ErrorData as McpError;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::json;
use znicz_core::{SpotifyKind, SpotifyLogin, SpotifyOp, SpotifyRepeat, SpotifyState, ZniczError};

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
