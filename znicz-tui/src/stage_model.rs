//! The source-neutral model the Stage and the transport draw.
//!
//! The renderers read only these types, never `PlayerState`. Today the model is
//! built from the player by `from_player`.

use std::time::Duration;

use znicz_core::{PlaybackStatus, PlayerState, QueueItem, RepeatMode, Station, TrackInfo};

use crate::cover::CoverKey;
use crate::format;
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
    use znicz_core::PlayerState;

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
}
