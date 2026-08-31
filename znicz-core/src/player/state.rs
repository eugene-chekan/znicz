use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::metadata::TrackTags;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PlaybackStatus {
    Stopped,
    Playing,
    Paused,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum QueueItem {
    File { path: PathBuf },
    Stream { name: String, url: String },
}

impl QueueItem {
    pub fn file(path: impl Into<PathBuf>) -> Self {
        Self::File { path: path.into() }
    }
    pub fn stream(name: impl Into<String>, url: impl Into<String>) -> Self {
        Self::Stream {
            name: name.into(),
            url: url.into(),
        }
    }
    pub fn as_path(&self) -> Option<&Path> {
        match self {
            Self::File { path } => Some(path),
            Self::Stream { .. } => None,
        }
    }
    pub fn is_stream(&self) -> bool {
        matches!(self, Self::Stream { .. })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrackInfo {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// Title tag when present, otherwise the file name.
    pub title: String,
    pub codec: String,
    pub sample_rate: u32,
    pub channels: u16,
    pub bits_per_sample: Option<u32>,
    /// Average audio bitrate in kilobits per second, when known.
    #[serde(default)]
    pub bitrate_kbps: Option<u32>,
    pub duration: Option<Duration>,
    /// Tags read from the file. Empty when the file carries none.
    #[serde(default)]
    pub tags: TrackTags,
}

impl TrackInfo {
    /// One line describing the audio itself, e.g. `FLAC 44.1 kHz 16-bit stereo`.
    ///
    /// Bit depth is left out when the codec does not have one (MP3, AAC, …).
    /// Bitrate is included when we have a number.
    pub fn format_description(&self) -> String {
        let mut parts = vec![self.codec.clone(), rate_label(self.sample_rate)];

        if let Some(bits) = self.bits_per_sample {
            parts.push(format!("{bits}-bit"));
        }
        if let Some(kbps) = self.bitrate_kbps {
            parts.push(format!("{kbps} kbps"));
        }
        parts.push(channel_label(self.channels));
        parts.join(" ")
    }

    pub fn artist(&self) -> Option<&str> {
        self.tags.artist.as_deref()
    }

    pub fn album(&self) -> Option<&str> {
        self.tags.album.as_deref()
    }

    /// "Artist — Album", or whichever half we have.
    pub fn artist_album(&self) -> Option<String> {
        self.tags.summary()
    }
}

/// Fractional rates matter here: 44.1 kHz must not read as 44 kHz.
fn rate_label(sample_rate: u32) -> String {
    let khz = sample_rate as f64 / 1000.0;
    if (khz.fract() * 10.0).round() == 0.0 {
        format!("{khz:.0} kHz")
    } else {
        format!("{khz:.1} kHz")
    }
}

fn channel_label(channels: u16) -> String {
    match channels {
        1 => "mono".to_string(),
        2 => "stereo".to_string(),
        n => format!("{n}ch"),
    }
}

/// What to do when the queue runs out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum RepeatMode {
    #[default]
    Off,
    /// Repeat the current track.
    One,
    /// Start the queue again from the top.
    All,
}

impl RepeatMode {
    /// Order used by the "cycle repeat" key.
    pub fn next(self) -> Self {
        match self {
            Self::Off => Self::All,
            Self::All => Self::One,
            Self::One => Self::Off,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::One => "one",
            Self::All => "all",
        }
    }
}

/// The stream Znicz actually opened on the sound device.
///
/// Compare this with the track to see whether playback is bit perfect.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutputInfo {
    pub sample_rate: u32,
    pub channels: u16,
    pub sample_format: String,
    /// True when the device took the file's own rate and channel count, so no
    /// resampling or channel remapping happens.
    pub bit_perfect: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlayerState {
    pub status: PlaybackStatus,
    pub current_track: Option<TrackInfo>,
    pub position: Duration,
    pub volume: f32,
    /// Silenced without losing the volume setting.
    #[serde(default)]
    pub muted: bool,
    pub device_id: Option<String>,
    pub device_name: Option<String>,
    /// Details of the open output stream, once playback has started.
    #[serde(default)]
    pub output: Option<OutputInfo>,
    pub queue: Vec<QueueItem>,
    pub queue_position: usize,
    #[serde(default)]
    pub repeat: RepeatMode,
    #[serde(default)]
    pub shuffle: bool,
}

impl Default for PlayerState {
    fn default() -> Self {
        Self {
            status: PlaybackStatus::Stopped,
            current_track: None,
            position: Duration::ZERO,
            volume: 1.0,
            muted: false,
            device_id: None,
            device_name: None,
            output: None,
            queue: Vec::new(),
            queue_position: 0,
            repeat: RepeatMode::default(),
            shuffle: false,
        }
    }
}

impl PlayerState {
    /// Volume actually sent to the device.
    pub fn effective_volume(&self) -> f32 {
        if self.muted {
            0.0
        } else {
            self.volume
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track(sample_rate: u32, channels: u16, bits: Option<u32>) -> TrackInfo {
        TrackInfo {
            path: Some(PathBuf::from("/music/a.flac")),
            url: None,
            title: "A".into(),
            codec: "FLAC".into(),
            sample_rate,
            channels,
            bits_per_sample: bits,
            bitrate_kbps: None,
            duration: None,
            tags: TrackTags::default(),
        }
    }

    #[test]
    fn format_description_keeps_the_fractional_rate() {
        // The most common audiophile rate is 44.1 kHz, not 44.
        assert_eq!(
            track(44_100, 2, Some(16)).format_description(),
            "FLAC 44.1 kHz 16-bit stereo"
        );
        assert_eq!(
            track(96_000, 2, Some(24)).format_description(),
            "FLAC 96 kHz 24-bit stereo"
        );
    }

    #[test]
    fn format_description_names_the_channel_layout() {
        assert!(track(48_000, 1, Some(16))
            .format_description()
            .ends_with("mono"));
        assert!(track(48_000, 2, Some(16))
            .format_description()
            .ends_with("stereo"));
        assert!(track(48_000, 6, Some(16))
            .format_description()
            .ends_with("6ch"));
    }

    #[test]
    fn format_description_omits_a_missing_bit_depth() {
        // MP3 and AAC have no PCM bit depth worth showing.
        let mut mp3 = track(44_100, 2, None);
        mp3.codec = "MP3".into();
        let description = mp3.format_description();
        assert_eq!(description, "MP3 44.1 kHz stereo");
        assert!(
            !description.contains("unknown"),
            "a missing depth must not be spelled out; got {description}"
        );
    }

    #[test]
    fn format_description_includes_bitrate_when_known() {
        let mut mp3 = track(44_100, 2, None);
        mp3.codec = "MP3".into();
        mp3.bitrate_kbps = Some(320);
        assert_eq!(mp3.format_description(), "MP3 44.1 kHz 320 kbps stereo");
    }

    #[test]
    fn repeat_cycles_through_every_mode() {
        let mut mode = RepeatMode::Off;
        mode = mode.next();
        assert_eq!(mode, RepeatMode::All);
        mode = mode.next();
        assert_eq!(mode, RepeatMode::One);
        mode = mode.next();
        assert_eq!(mode, RepeatMode::Off, "cycle must return to the start");
    }

    #[test]
    fn muting_silences_without_losing_the_setting() {
        let mut state = PlayerState {
            volume: 0.7,
            ..PlayerState::default()
        };
        assert_eq!(state.effective_volume(), 0.7);

        state.muted = true;
        assert_eq!(state.effective_volume(), 0.0);
        assert_eq!(state.volume, 0.7, "volume setting must be remembered");
    }

    #[test]
    fn queue_item_json_is_tagged_not_a_bare_path() {
        let file = QueueItem::file("/music/a.flac");
        let stream = QueueItem::stream("Example", "https://example.com/s");
        let file_json = serde_json::to_value(&file).unwrap();
        let stream_json = serde_json::to_value(&stream).unwrap();
        assert_eq!(file_json["kind"], "file");
        assert!(file_json["path"].as_str().unwrap().ends_with("a.flac"));
        assert_eq!(stream_json["kind"], "stream");
        assert_eq!(stream_json["name"], "Example");
        assert_eq!(stream_json["url"], "https://example.com/s");
    }

    #[test]
    fn track_info_stream_has_a_url_not_a_path() {
        let track = TrackInfo {
            path: None,
            url: Some("https://example.com/s".into()),
            title: "Example".into(),
            codec: "MP3".into(),
            sample_rate: 44_100,
            channels: 2,
            bits_per_sample: None,
            bitrate_kbps: None,
            duration: None,
            tags: TrackTags::default(),
        };
        let json = serde_json::to_value(&track).unwrap();
        assert!(json.get("path").is_none());
        assert_eq!(json["url"], "https://example.com/s");
        assert_eq!(json["title"], "Example");
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioDeviceInfo {
    pub id: String,
    pub name: String,
    pub is_default: bool,
}
