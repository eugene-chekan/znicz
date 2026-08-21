//! Reading tags from audio files.
//!
//! Tags are the text stored inside a music file: title, artist, album, and so
//! on. Symphonia gives us the technical side (sample rate, codec). Lofty is
//! better at tags, so we use it for the human-readable part.
//!
//! Nothing here fails a track: if tags are missing or unreadable we fall back
//! to the file name, because playing music matters more than metadata.

use std::path::Path;
use std::time::Duration;

use lofty::file::{AudioFile, TaggedFileExt};
use lofty::prelude::{Accessor, ItemKey};
use lofty::probe::Probe;
use serde::{Deserialize, Serialize};

/// Audio file extensions we try to read.
pub const AUDIO_EXTENSIONS: &[&str] = &[
    "flac", "wav", "mp3", "m4a", "mp4", "aac", "ogg", "oga", "opus", "aiff", "aif", "wv", "ape",
    "alac", "wma",
];

/// True when the extension looks like audio we might play.
pub fn is_audio_file(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_ascii_lowercase())
        .map(|ext| AUDIO_EXTENSIONS.contains(&ext.as_str()))
        .unwrap_or(false)
}

/// Technical details, read from tags rather than by decoding.
///
/// Much faster than a full decoder probe, which matters when scanning
/// thousands of files.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AudioProperties {
    pub duration: Option<Duration>,
    pub sample_rate: Option<u32>,
    pub channels: Option<u16>,
    pub bits_per_sample: Option<u32>,
}

/// Tags plus technical details for one file.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileMetadata {
    pub tags: TrackTags,
    pub properties: AudioProperties,
}

/// Human-readable tags for one track.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrackTags {
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub album_artist: Option<String>,
    pub genre: Option<String>,
    pub year: Option<u32>,
    pub track_number: Option<u32>,
    pub disc_number: Option<u32>,
}

impl TrackTags {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    /// Artist and album as one line, skipping the parts we do not have.
    pub fn summary(&self) -> Option<String> {
        match (self.artist.as_deref(), self.album.as_deref()) {
            (Some(artist), Some(album)) => Some(format!("{artist} — {album}")),
            (Some(artist), None) => Some(artist.to_string()),
            (None, Some(album)) => Some(album.to_string()),
            (None, None) => None,
        }
    }
}

/// The name to show when a file has no title tag: its file stem.
pub fn title_from_path(path: &Path) -> String {
    path.file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("Unknown")
        .to_string()
}

/// Read tags from a file.
///
/// Returns empty tags rather than an error when the file has none, is an
/// unsupported container, or cannot be parsed.
pub fn read_tags(path: &Path) -> TrackTags {
    read_metadata(path).tags
}

/// Read tags and technical details in one pass.
pub fn read_metadata(path: &Path) -> FileMetadata {
    let tagged = match Probe::open(path).and_then(|probe| probe.read()) {
        Ok(tagged) => tagged,
        Err(e) => {
            tracing::debug!(path = %path.display(), error = %e, "no readable metadata");
            return FileMetadata::default();
        }
    };

    let file_properties = tagged.properties();
    let properties = AudioProperties {
        duration: Some(file_properties.duration()).filter(|d| !d.is_zero()),
        sample_rate: file_properties.sample_rate(),
        channels: file_properties.channels().map(u16::from),
        bits_per_sample: file_properties.bit_depth().map(u32::from),
    };

    let Some(tag) = tagged.primary_tag().or_else(|| tagged.first_tag()) else {
        return FileMetadata {
            tags: TrackTags::default(),
            properties,
        };
    };

    let text = |key: ItemKey| {
        tag.get_string(key)
            .map(str::to_string)
            .filter(|s| !s.is_empty())
    };

    let tags = TrackTags {
        title: tag.title().map(|t| t.to_string()).filter(|s| !s.is_empty()),
        artist: tag.artist().map(|a| a.to_string()).filter(|s| !s.is_empty()),
        album: tag.album().map(|a| a.to_string()).filter(|s| !s.is_empty()),
        album_artist: text(ItemKey::AlbumArtist),
        genre: tag.genre().map(|g| g.to_string()).filter(|s| !s.is_empty()),
        // Different containers store the date under different keys.
        year: text(ItemKey::Year)
            .or_else(|| text(ItemKey::RecordingDate))
            .or_else(|| text(ItemKey::OriginalReleaseDate))
            .and_then(parse_year),
        track_number: tag.track(),
        disc_number: tag.disk(),
    };

    FileMetadata { tags, properties }
}

/// Pull a four-digit year out of a date string such as "1979-10-05".
fn parse_year(value: String) -> Option<u32> {
    let digits: String = value.chars().take_while(|c| c.is_ascii_digit()).collect();
    digits.parse().ok().filter(|year| *year > 0 && *year < 3000)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn title_falls_back_to_file_stem() {
        let path = PathBuf::from("/music/Artist/01 - Song.flac");
        assert_eq!(title_from_path(&path), "01 - Song");
    }

    #[test]
    fn missing_file_yields_empty_tags() {
        let tags = read_tags(Path::new("/definitely/not/here.flac"));
        assert!(tags.is_empty(), "expected empty tags, got {tags:?}");
    }

    #[test]
    fn summary_skips_missing_parts() {
        let mut tags = TrackTags::default();
        assert_eq!(tags.summary(), None);

        tags.artist = Some("Portishead".into());
        assert_eq!(tags.summary().as_deref(), Some("Portishead"));

        tags.album = Some("Dummy".into());
        assert_eq!(tags.summary().as_deref(), Some("Portishead — Dummy"));

        tags.artist = None;
        assert_eq!(tags.summary().as_deref(), Some("Dummy"));
    }

    #[test]
    fn year_is_parsed_from_a_date() {
        assert_eq!(parse_year("1979-10-05".into()), Some(1979));
        assert_eq!(parse_year("1994".into()), Some(1994));
        assert_eq!(parse_year("unknown".into()), None);
    }

    #[test]
    fn audio_extensions_are_recognised() {
        assert!(is_audio_file(Path::new("/music/a.flac")));
        assert!(is_audio_file(Path::new("/music/a.FLAC")), "case insensitive");
        assert!(is_audio_file(Path::new("/music/a.mp3")));
        assert!(!is_audio_file(Path::new("/music/cover.jpg")));
        assert!(!is_audio_file(Path::new("/music/notes.txt")));
        assert!(!is_audio_file(Path::new("/music/no-extension")));
    }
}
