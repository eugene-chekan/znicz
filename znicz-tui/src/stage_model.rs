//! The source-neutral model the Stage and the transport draw.
//!
//! The renderers read only these types, never `PlayerState`. Today the model is
//! built from the player by `from_player`.

use std::time::Duration;

use znicz_core::{
    PlaybackStatus, PlayerState, QueueItem, RepeatMode, SpotifyPlayback, SpotifyState,
    SpotifyTrack, Station, TrackInfo,
};

use crate::cover::CoverKey;
use crate::format;
use crate::spotify_mode::repeat_mode;
use crate::views::stage::stream_query;

/// How a piece of text is styled. The renderer maps each tone to a theme style.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Plain,
    Strong,
    Dim,
    Subtitle,
    Good,
    Warn,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StyledText {
    pub text: String,
    pub tone: Tone,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SeekModel {
    /// A position within a known or unknown length.
    Bar {
        position: Duration,
        total: Option<Duration>,
    },
    /// A live stream: the position only.
    Live { position: Duration },
}

impl SeekModel {
    pub fn position(&self) -> Duration {
        match self {
            SeekModel::Bar { position, .. } | SeekModel::Live { position } => *position,
        }
    }

    /// The length, `None` when unknown or live.
    pub fn total(&self) -> Option<Duration> {
        match self {
            SeekModel::Bar { total, .. } => *total,
            SeekModel::Live { .. } => None,
        }
    }

    /// The filled fraction of the bar: 0 when there is no length.
    pub fn ratio(&self) -> f64 {
        match self.total() {
            Some(total) if !total.is_zero() => self.position().as_secs_f64() / total.as_secs_f64(),
            _ => 0.0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VolumeModel {
    pub volume: f32,
    pub muted: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignalRow {
    pub label: &'static str,
    pub value: String,
    pub tone: Tone,
}

/// What the Stage shows instead of the details when nothing plays. The title
/// line is the model's `title`; this is the hint under it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmptyState {
    pub hint: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StageModel {
    /// The title line, not yet fitted to the width.
    pub title: String,
    /// Artist and album lines (file) or station and URL (stream), drawn under the title.
    pub head: Vec<StyledText>,
    /// `None` when nothing plays (no seek line is drawn).
    pub seek: Option<SeekModel>,
    pub status: PlaybackStatus,
    /// `None` when nothing plays (no volume line is drawn).
    pub volume: Option<VolumeModel>,
    pub repeat: RepeatMode,
    pub shuffle: bool,
    /// Empty when nothing plays. Otherwise the rows under the "Signal" heading.
    pub signal: Vec<SignalRow>,
    /// The path explanation, dropped first when rows run short.
    pub explanation: Option<String>,
    /// `None` when there is nothing to cover (nothing plays, or covers are off).
    pub cover: Option<CoverKey>,
    /// Shown instead of the signal when nothing plays.
    pub empty: Option<EmptyState>,
}

impl StageModel {
    pub fn from_player(
        state: &PlayerState,
        stations: &[Station],
        cover_search: bool,
        cover_on: bool,
    ) -> Self {
        let Some(track) = state.current_track.as_ref() else {
            return Self {
                title: "Nothing playing".into(),
                head: Vec::new(),
                seek: None,
                status: state.status,
                volume: None,
                repeat: state.repeat,
                shuffle: state.shuffle,
                signal: Vec::new(),
                explanation: None,
                cover: None,
                empty: Some(EmptyState {
                    hint: "Enter a track in Browse (v) to start".into(),
                }),
            };
        };
        let is_stream = track.url.is_some();
        let cover = cover_on.then(|| match track.path.as_ref() {
            Some(path) => CoverKey::File(path.clone()),
            None => CoverKey::Stream(stream_query(state, track, stations, cover_search)),
        });
        let (signal, explanation) = signal_rows(state, track);
        Self {
            title: track.title.clone(),
            head: if is_stream {
                stream_head(state, track)
            } else {
                file_head(track)
            },
            seek: Some(if is_stream {
                SeekModel::Live {
                    position: state.position,
                }
            } else {
                SeekModel::Bar {
                    position: state.position,
                    total: track.duration,
                }
            }),
            status: state.status,
            volume: Some(VolumeModel {
                volume: state.volume,
                muted: state.muted,
            }),
            repeat: state.repeat,
            shuffle: state.shuffle,
            signal,
            explanation,
            cover,
            empty: None,
        }
    }

    /// The Spotify Stage. `now_ms` is Unix milliseconds (for `progress_at`);
    /// `muted` is whether Spotify mode has `m` on.
    pub fn from_spotify(state: &SpotifyState, now_ms: u64, muted: bool) -> Self {
        let playback = state.playback.as_ref();
        let status = spotify_status(playback);
        let repeat = playback.map_or(RepeatMode::Off, |p| repeat_mode(p.repeat));
        let shuffle = playback.is_some_and(|p| p.shuffle);
        let item = playback.and_then(|p| p.item.as_ref());
        let (Some(playback), Some(item)) = (playback, item) else {
            return Self {
                title: SPOTIFY_NOTHING_TEXT.into(),
                head: Vec::new(),
                seek: None,
                status,
                volume: None,
                repeat,
                shuffle,
                signal: Vec::new(),
                explanation: None,
                cover: None,
                empty: Some(EmptyState {
                    hint: SPOTIFY_HINT_TEXT.into(),
                }),
            };
        };
        Self {
            title: item.title.clone(),
            head: spotify_head(item),
            seek: Some(SeekModel::Bar {
                position: Duration::from_millis(playback.progress_at(now_ms)),
                total: Some(Duration::from_millis(item.duration_ms)),
            }),
            status,
            volume: spotify_volume(playback, muted),
            repeat,
            shuffle,
            signal: spotify_signal(playback),
            explanation: Some(SPOTIFY_EXPLANATION.into()),
            cover: item.image_url.clone().map(CoverKey::Url),
            empty: None,
        }
    }
}

/// The title when nothing plays on Spotify.
pub const SPOTIFY_NOTHING_TEXT: &str = "Nothing playing on Spotify";
/// The hint under the title when nothing plays on Spotify.
pub const SPOTIFY_HINT_TEXT: &str = "/ search · O znicz";
/// The path explanation for Spotify Connect.
pub const SPOTIFY_EXPLANATION: &str =
    "decoded and played by the Spotify app on that device; Znicz does not touch the audio";

fn spotify_status(playback: Option<&SpotifyPlayback>) -> PlaybackStatus {
    match playback {
        Some(p) if p.is_playing => PlaybackStatus::Playing,
        Some(_) => PlaybackStatus::Paused,
        None => PlaybackStatus::Stopped,
    }
}

fn spotify_head(item: &SpotifyTrack) -> Vec<StyledText> {
    let mut out = Vec::new();
    let artist = item.artist_line();
    if !artist.is_empty() {
        out.push(StyledText {
            text: artist,
            tone: Tone::Subtitle,
        });
    }
    let mut parts = Vec::new();
    if !item.album.is_empty() {
        parts.push(item.album.clone());
    }
    if let Some(year) = item.year {
        parts.push(year.to_string());
    }
    if let Some(n) = item.track_number {
        parts.push(format!("track {n}"));
    }
    if !parts.is_empty() {
        out.push(StyledText {
            text: parts.join(" · "),
            tone: Tone::Dim,
        });
    }
    out
}

fn spotify_volume(playback: &SpotifyPlayback, muted: bool) -> Option<VolumeModel> {
    let percent = playback.device.as_ref()?.volume_percent?;
    Some(VolumeModel {
        volume: f32::from(percent) / 100.0,
        muted,
    })
}

/// The device the track plays on, as `<name> (<kind>)`.
fn spotify_device(playback: &SpotifyPlayback) -> String {
    match playback.device.as_ref() {
        Some(device) => format!("{} ({})", device.name, device.kind),
        None => "unknown device".into(),
    }
}

fn spotify_signal(playback: &SpotifyPlayback) -> Vec<SignalRow> {
    vec![
        SignalRow {
            label: "source",
            value: "spotify".into(),
            tone: Tone::Plain,
        },
        SignalRow {
            label: "device",
            value: spotify_device(playback),
            tone: Tone::Plain,
        },
        SignalRow {
            label: "path",
            value: "Spotify Connect".into(),
            tone: Tone::Good,
        },
    ]
}

/// The transport row: title, subtitle, seek, volume, toggles, and the signal
/// line (`source → device`) with its path badge.
#[derive(Debug, Clone, PartialEq)]
pub struct TransportModel {
    pub status: PlaybackStatus,
    /// Track title, or `Nothing playing`.
    pub title: String,
    /// `artist · album` for the middle of the row; `None` when there is none.
    pub subtitle: Option<String>,
    /// `None` when there is nothing to seek.
    pub seek: Option<SeekModel>,
    pub volume: Option<VolumeModel>,
    pub repeat: RepeatMode,
    pub shuffle: bool,
    /// The one-line signal as `(source, device)`. `None` when nothing plays.
    /// An empty device means no output is open: only the source is drawn.
    pub signal: Option<(String, String)>,
    /// `None` when nothing is playing or no output is open.
    pub path_badge: Option<(String, Tone)>,
}

impl TransportModel {
    pub fn from_player(state: &PlayerState) -> Self {
        let track = state.current_track.as_ref();
        let position = state.position;
        let seek = match track {
            Some(track) if track.url.is_some() => SeekModel::Live { position },
            _ => SeekModel::Bar {
                position,
                total: track.and_then(|t| t.duration),
            },
        };
        let signal = track.map(|track| {
            let source = track.format_description();
            match state.output.as_ref() {
                Some(output) => (
                    source,
                    format!(
                        "{} {}",
                        format::khz(output.sample_rate),
                        format::channel_name(output.channels)
                    ),
                ),
                None => (source, String::new()),
            }
        });
        let path_badge = state
            .output
            .as_ref()
            .filter(|_| state.status != PlaybackStatus::Stopped)
            .map(|output| {
                if output.bit_perfect {
                    ("● bit perfect".to_string(), Tone::Good)
                } else {
                    ("▲ resampled".to_string(), Tone::Warn)
                }
            });
        Self {
            status: state.status,
            title: track
                .map(|t| t.title.clone())
                .unwrap_or_else(|| "Nothing playing".into()),
            subtitle: track
                .and_then(|t| t.artist_album())
                .filter(|s| !s.is_empty()),
            seek: Some(seek),
            volume: Some(VolumeModel {
                volume: state.volume,
                muted: state.muted,
            }),
            repeat: state.repeat,
            shuffle: state.shuffle,
            signal,
            path_badge,
        }
    }
}

impl TransportModel {
    /// The Spotify transport row. `muted` is whether Spotify mode has `m` on.
    pub fn from_spotify(state: &SpotifyState, now_ms: u64, muted: bool) -> Self {
        let playback = state.playback.as_ref();
        let item = playback.and_then(|p| p.item.as_ref());
        let (title, subtitle, seek) = match (playback, item) {
            (Some(playback), Some(item)) => {
                let artist = item.artist_line();
                let subtitle = [artist.as_str(), item.album.as_str()]
                    .into_iter()
                    .filter(|s| !s.is_empty())
                    .collect::<Vec<_>>()
                    .join(" · ");
                (
                    item.title.clone(),
                    Some(subtitle).filter(|s| !s.is_empty()),
                    Some(SeekModel::Bar {
                        position: Duration::from_millis(playback.progress_at(now_ms)),
                        total: Some(Duration::from_millis(item.duration_ms)),
                    }),
                )
            }
            _ => (
                SPOTIFY_NOTHING_TEXT.to_string(),
                None,
                Some(SeekModel::Bar {
                    position: Duration::ZERO,
                    total: None,
                }),
            ),
        };
        let signal = playback.map(|p| ("spotify".to_string(), spotify_device(p)));
        let path_badge = playback.map(|_| ("Spotify Connect".to_string(), Tone::Good));
        Self {
            status: spotify_status(playback),
            title,
            subtitle,
            seek,
            volume: playback.and_then(|p| spotify_volume(p, muted)),
            repeat: playback.map_or(RepeatMode::Off, |p| repeat_mode(p.repeat)),
            shuffle: playback.is_some_and(|p| p.shuffle),
            signal,
            path_badge,
        }
    }
}

/// Artist (or album artist) and the album line, for a file.
fn file_head(track: &TrackInfo) -> Vec<StyledText> {
    let mut out = Vec::new();
    let artist = track
        .tags
        .artist
        .clone()
        .or_else(|| track.tags.album_artist.clone());
    if let Some(artist) = artist.filter(|a| !a.is_empty()) {
        out.push(StyledText {
            text: artist,
            tone: Tone::Subtitle,
        });
    }
    let mut parts = Vec::new();
    if let Some(album) = track.tags.album.clone().filter(|a| !a.is_empty()) {
        parts.push(album);
    }
    if let Some(year) = track.tags.year {
        parts.push(year.to_string());
    }
    if let Some(n) = track.tags.track_number {
        parts.push(format!("track {n}"));
    }
    if !parts.is_empty() {
        out.push(StyledText {
            text: parts.join(" · "),
            tone: Tone::Dim,
        });
    }
    out
}

/// The station name (from the queue row, when it differs from the title) and
/// the stream URL.
fn stream_head(state: &PlayerState, track: &TrackInfo) -> Vec<StyledText> {
    let mut out = Vec::new();
    let station = state
        .queue
        .get(state.queue_position)
        .and_then(|item| match item {
            QueueItem::Stream { name, .. } => Some(name.clone()),
            QueueItem::File { .. } => None,
        });
    if let Some(name) = station.filter(|n| *n != track.title) {
        out.push(StyledText {
            text: name,
            tone: Tone::Subtitle,
        });
    }
    if let Some(url) = track.url.as_deref() {
        out.push(StyledText {
            text: url.to_string(),
            tone: Tone::Dim,
        });
    }
    out
}

/// The signal rows (source, device, path) and the path explanation.
fn signal_rows(state: &PlayerState, track: &TrackInfo) -> (Vec<SignalRow>, Option<String>) {
    let source_label = if track.url.is_some() {
        "stream"
    } else {
        "file"
    };
    let mut rows = vec![SignalRow {
        label: source_label,
        value: track.format_description(),
        tone: Tone::Plain,
    }];
    let device = state
        .device_name
        .clone()
        .unwrap_or_else(|| "default device".to_string());
    let device = match state.output.as_ref() {
        Some(output) => format!("{device} · {}", device_stream(output)),
        None => format!("{device} · no stream open"),
    };
    rows.push(SignalRow {
        label: "device",
        value: device,
        tone: Tone::Plain,
    });
    let Some(output) = state.output.as_ref() else {
        rows.push(SignalRow {
            label: "path",
            value: "nothing to compare until a stream opens".into(),
            tone: Tone::Dim,
        });
        return (rows, None);
    };
    if output.bit_perfect {
        rows.push(SignalRow {
            label: "path",
            value: "● bit perfect".into(),
            tone: Tone::Good,
        });
        (
            rows,
            Some("the device took the file's own rate and channels".into()),
        )
    } else {
        rows.push(SignalRow {
            label: "path",
            value: "▲ resampled".into(),
            tone: Tone::Warn,
        });
        let explanation = format!(
            "file {} {} → device {} {}",
            format::khz(track.sample_rate),
            format::channel_name(track.channels),
            format::khz(output.sample_rate),
            format::channel_name(output.channels),
        );
        (rows, Some(explanation))
    }
}

fn device_stream(output: &znicz_core::OutputInfo) -> String {
    format!(
        "{} {} {}",
        format::khz(output.sample_rate),
        format::channel_name(output.channels),
        sample_format_label(&output.sample_format),
    )
}

fn sample_format_label(fmt: &str) -> &str {
    if fmt.is_empty() || fmt == "?" {
        "--"
    } else {
        fmt
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::views::stage::tests::{playing, stream};
    use znicz_core::{PlaybackStatus, PlayerState};

    #[test]
    fn a_file_model_has_the_artist_album_seek_and_signal() {
        let model = StageModel::from_player(&playing(false), &[], true, true);
        assert_eq!(model.title, "Sour Times");
        assert!(matches!(model.seek, Some(SeekModel::Bar { .. })));
        assert_eq!(model.signal[0].label, "file");
        assert!(matches!(model.cover, Some(CoverKey::File(_))));
    }

    #[test]
    fn a_stream_model_is_live_and_its_cover_is_a_stream_query() {
        let model = StageModel::from_player(&stream(), &[], true, true);
        assert!(matches!(model.seek, Some(SeekModel::Live { .. })));
        assert!(matches!(model.cover, Some(CoverKey::Stream(_))));
        assert_eq!(model.signal[0].label, "stream");
    }

    #[test]
    fn nothing_playing_has_an_empty_state_and_no_cover_or_seek() {
        let model = StageModel::from_player(&PlayerState::default(), &[], true, true);
        assert_eq!(model.title, "Nothing playing");
        assert!(model.seek.is_none() && model.volume.is_none() && model.cover.is_none());
        assert_eq!(
            model.empty.unwrap().hint,
            "Enter a track in Browse (v) to start"
        );
    }

    #[test]
    fn covers_off_gives_no_cover_key() {
        let model = StageModel::from_player(&playing(false), &[], true, false);
        assert!(model.cover.is_none());
    }

    #[test]
    fn the_transport_model_of_a_file_has_title_subtitle_and_seek() {
        let model = TransportModel::from_player(&playing(false));
        assert_eq!(model.title, "Sour Times");
        assert!(model.subtitle.is_some());
        assert!(matches!(model.seek, Some(SeekModel::Bar { .. })));
    }

    #[test]
    fn a_stopped_player_has_no_path_badge() {
        let mut state = playing(true);
        state.status = PlaybackStatus::Stopped;
        assert!(TransportModel::from_player(&state).path_badge.is_none());
    }

    use znicz_core::{
        SpotifyDevice, SpotifyLogin, SpotifyPlayback, SpotifyRepeat, SpotifyState, SpotifyTrack,
    };

    /// `So What` by Miles Davis, on Kitchen (Speaker), read at 1 000 000 ms
    /// with 83 s in; 562 s long.
    fn spotify(is_playing: bool) -> SpotifyState {
        SpotifyState {
            login: SpotifyLogin::LoggedIn,
            playback: Some(SpotifyPlayback {
                item: Some(SpotifyTrack {
                    uri: "spotify:track:1".into(),
                    title: "So What".into(),
                    artists: vec!["Miles Davis".into()],
                    album: "Kind of Blue".into(),
                    album_uri: "spotify:album:1".into(),
                    year: Some(1959),
                    track_number: Some(1),
                    duration_ms: 562_000,
                    image_url: Some("https://i.scdn.co/image/a".into()),
                }),
                unsupported: false,
                is_playing,
                progress_ms: 83_000,
                read_at_ms: 1_000_000,
                repeat: SpotifyRepeat::Off,
                shuffle: false,
                device: Some(SpotifyDevice {
                    id: "d1".into(),
                    name: "Kitchen".into(),
                    kind: "Speaker".into(),
                    is_active: true,
                    volume_percent: Some(60),
                }),
                context_uri: None,
            }),
            ..SpotifyState::default()
        }
    }

    #[test]
    fn a_spotify_model_has_the_path_signal_rows_and_the_explanation() {
        let model = StageModel::from_spotify(&spotify(true), 1_000_000, false);
        let labels: Vec<_> = model.signal.iter().map(|r| r.label).collect();
        let values: Vec<_> = model.signal.iter().map(|r| r.value.as_str()).collect();
        assert_eq!(labels, ["source", "device", "path"]);
        assert_eq!(values, ["spotify", "Kitchen (Speaker)", "Spotify Connect"]);
        assert_eq!(model.signal[2].tone, Tone::Good);
        assert_eq!(
            model.explanation.as_deref(),
            Some(
                "decoded and played by the Spotify app on that device; Znicz does not touch the audio"
            )
        );
    }

    #[test]
    fn nothing_playing_on_spotify_has_the_logo_text_and_hint() {
        let model = StageModel::from_spotify(&SpotifyState::default(), 0, false);
        assert_eq!(model.title, "Nothing playing on Spotify");
        assert_eq!(model.empty.unwrap().hint, "/ search · O znicz");
        assert!(model.cover.is_none());
        assert!(model.seek.is_none() && model.volume.is_none());
    }

    #[test]
    fn the_cover_is_the_track_image_url() {
        let model = StageModel::from_spotify(&spotify(true), 1_000_000, false);
        assert_eq!(
            model.cover,
            Some(CoverKey::Url("https://i.scdn.co/image/a".into()))
        );
    }

    #[test]
    fn a_paused_track_has_a_paused_status_and_a_path_badge() {
        let model = StageModel::from_spotify(&spotify(false), 1_000_000, false);
        assert_eq!(model.status, PlaybackStatus::Paused);
        let transport = TransportModel::from_spotify(&spotify(false), 1_000_000, false);
        assert_eq!(
            transport.path_badge,
            Some(("Spotify Connect".to_string(), Tone::Good))
        );
    }

    #[test]
    fn nothing_playing_on_spotify_has_no_path_badge_and_is_stopped() {
        let state = SpotifyState::default();
        let transport = TransportModel::from_spotify(&state, 0, false);
        assert_eq!(transport.status, PlaybackStatus::Stopped);
        assert!(transport.path_badge.is_none());
        assert!(transport.signal.is_none());
    }

    #[test]
    fn the_seek_is_progress_at_now_out_of_the_track_length() {
        // Read 83 s in at 1 000 000 ms; two seconds later it is 85 s in.
        let model = StageModel::from_spotify(&spotify(true), 1_002_000, false);
        assert_eq!(
            model.seek,
            Some(SeekModel::Bar {
                position: Duration::from_secs(85),
                total: Some(Duration::from_secs(562)),
            })
        );
    }

    #[test]
    fn the_head_is_the_artist_then_the_album_line() {
        let model = StageModel::from_spotify(&spotify(true), 1_000_000, false);
        assert_eq!(
            model.head,
            vec![
                StyledText {
                    text: "Miles Davis".into(),
                    tone: Tone::Subtitle,
                },
                StyledText {
                    text: "Kind of Blue · 1959 · track 1".into(),
                    tone: Tone::Dim,
                },
            ]
        );
    }

    #[test]
    fn the_volume_is_the_device_percent_and_shows_muted() {
        let model = StageModel::from_spotify(&spotify(true), 1_000_000, true);
        assert_eq!(
            model.volume,
            Some(VolumeModel {
                volume: 0.6,
                muted: true,
            })
        );
        let mut state = spotify(true);
        state
            .playback
            .as_mut()
            .unwrap()
            .device
            .as_mut()
            .unwrap()
            .volume_percent = None;
        assert!(
            StageModel::from_spotify(&state, 1_000_000, false)
                .volume
                .is_none()
        );
    }

    #[test]
    fn the_spotify_transport_has_the_title_subtitle_and_device_signal() {
        let model = TransportModel::from_spotify(&spotify(true), 1_000_000, false);
        assert_eq!(model.title, "So What");
        assert_eq!(
            model.subtitle.as_deref(),
            Some("Miles Davis · Kind of Blue")
        );
        assert_eq!(
            model.signal,
            Some(("spotify".into(), "Kitchen (Speaker)".into()))
        );
        assert_eq!(model.status, PlaybackStatus::Playing);
    }
}
