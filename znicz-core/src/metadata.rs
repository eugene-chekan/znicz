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
    /// Average audio bitrate in kilobits per second.
    pub audio_bitrate: Option<u32>,
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
        audio_bitrate: file_properties.audio_bitrate().filter(|kbps| *kbps > 0),
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
        artist: tag
            .artist()
            .map(|a| a.to_string())
            .filter(|s| !s.is_empty()),
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

/// Embedded picture bytes from a file. Empty / missing → `read_cover` returns `None`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoverArt {
    pub mime: String,
    pub bytes: Vec<u8>,
}

fn sniff_image_mime(bytes: &[u8]) -> String {
    if bytes.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]) {
        "image/png".into()
    } else if bytes.len() >= 3 && bytes[0] == 0xFF && bytes[1] == 0xD8 && bytes[2] == 0xFF {
        "image/jpeg".into()
    } else {
        "application/octet-stream".into()
    }
}

/// Front cover if present, else the first picture with data. Never errors to the caller.
pub fn read_cover(path: &Path) -> Option<CoverArt> {
    use lofty::picture::PictureType;

    let tagged = match Probe::open(path).and_then(|probe| probe.read()) {
        Ok(tagged) => tagged,
        Err(e) => {
            tracing::debug!(path = %path.display(), error = %e, "no readable cover");
            return None;
        }
    };

    let mut first: Option<CoverArt> = None;
    for tag in tagged.tags() {
        for pic in tag.pictures() {
            if pic.data().is_empty() {
                continue;
            }
            let mime = pic
                .mime_type()
                .map(|m| m.as_str().to_string())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| sniff_image_mime(pic.data()));
            let art = CoverArt {
                mime,
                bytes: pic.data().to_vec(),
            };
            if pic.pic_type() == PictureType::CoverFront {
                return Some(art);
            }
            if first.is_none() {
                first = Some(art);
            }
        }
    }
    first
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
        assert!(
            is_audio_file(Path::new("/music/a.FLAC")),
            "case insensitive"
        );
        assert!(is_audio_file(Path::new("/music/a.mp3")));
        assert!(!is_audio_file(Path::new("/music/cover.jpg")));
        assert!(!is_audio_file(Path::new("/music/notes.txt")));
        assert!(!is_audio_file(Path::new("/music/no-extension")));
    }

    const TINY_PNG: &[u8] = &[
        0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48,
        0x44, 0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x02, 0x00, 0x00,
        0x00, 0x90, 0x77, 0x53, 0xDE, 0x00, 0x00, 0x00, 0x0C, 0x49, 0x44, 0x41, 0x54, 0x08,
        0xD7, 0x63, 0xF8, 0xCF, 0xC0, 0x00, 0x00, 0x00, 0x03, 0x00, 0x01, 0x18, 0xDD, 0x8D,
        0xB4, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
    ];

    fn ffmpeg_available() -> bool {
        std::process::Command::new("ffmpeg")
            .arg("-version")
            .output()
            .map(|out| out.status.success())
            .unwrap_or(false)
    }

    fn write_silent_flac(path: &Path) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        let status = std::process::Command::new("ffmpeg")
            .args(["-y", "-loglevel", "error"])
            .args(["-f", "lavfi", "-i", "sine=frequency=440:duration=1"])
            .args(["-ac", "1", "-ar", "44100", "-c:a", "flac"])
            .arg(path)
            .status()
            .expect("run ffmpeg");
        assert!(status.success(), "ffmpeg failed for {}", path.display());
    }

    fn picture(pic_type: lofty::picture::PictureType) -> lofty::picture::Picture {
        use lofty::picture::{MimeType, Picture};
        Picture::unchecked(TINY_PNG.to_vec())
            .pic_type(pic_type)
            .mime_type(MimeType::Png)
            .build()
    }

    fn save_pictures(path: &Path, pics: Vec<lofty::picture::Picture>) {
        use lofty::config::WriteOptions;
        use lofty::prelude::*;
        use lofty::probe::Probe;
        use lofty::tag::Tag;

        let mut tagged = Probe::open(path).unwrap().read().unwrap();
        if tagged.primary_tag().is_none() {
            let tag_type = tagged.primary_tag_type();
            tagged.insert_tag(Tag::new(tag_type));
        }
        let tag = tagged.primary_tag_mut().expect("tag");
        for pic in pics {
            tag.push_picture(pic);
        }
        tag.save_to_path(path, WriteOptions::default())
            .expect("write pictures");
    }

    #[test]
    fn missing_file_has_no_cover() {
        assert!(read_cover(Path::new("/definitely/not/here.flac")).is_none());
    }

    #[test]
    fn file_without_pictures_has_no_cover() {
        if !ffmpeg_available() {
            eprintln!("ffmpeg not available, skipping");
            return;
        }
        let dir = std::env::temp_dir().join("znicz-cover-none");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("plain.flac");
        write_silent_flac(&path);
        assert!(read_cover(&path).is_none());
    }

    #[test]
    fn front_cover_is_preferred() {
        if !ffmpeg_available() {
            eprintln!("ffmpeg not available, skipping");
            return;
        }
        let dir = std::env::temp_dir().join("znicz-cover-front");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("two.flac");
        write_silent_flac(&path);
        save_pictures(
            &path,
            vec![
                picture(lofty::picture::PictureType::CoverBack),
                picture(lofty::picture::PictureType::CoverFront),
            ],
        );
        let cover = read_cover(&path).expect("front cover");
        assert_eq!(cover.bytes, TINY_PNG);
        assert_eq!(cover.mime, "image/png");
    }

    #[test]
    fn mp3_front_cover_is_read_when_ffmpeg_can_encode_mp3() {
        if !ffmpeg_available() {
            eprintln!("ffmpeg not available, skipping");
            return;
        }
        let dir = std::env::temp_dir().join("znicz-cover-mp3");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("front.mp3");
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        let status = std::process::Command::new("ffmpeg")
            .args(["-y", "-loglevel", "error"])
            .args(["-f", "lavfi", "-i", "sine=frequency=440:duration=1"])
            .args(["-ac", "1", "-ar", "44100", "-c:a", "libmp3lame"])
            .arg(&path)
            .status()
            .expect("run ffmpeg");
        if !status.success() {
            eprintln!("ffmpeg cannot encode mp3, skipping");
            return;
        }
        save_pictures(&path, vec![picture(lofty::picture::PictureType::CoverFront)]);
        let cover = read_cover(&path).expect("mp3 cover");
        assert_eq!(cover.bytes, TINY_PNG);
    }

    #[test]
    fn first_picture_is_used_when_there_is_no_front() {
        if !ffmpeg_available() {
            eprintln!("ffmpeg not available, skipping");
            return;
        }
        let dir = std::env::temp_dir().join("znicz-cover-first");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("back.flac");
        write_silent_flac(&path);
        save_pictures(
            &path,
            vec![picture(lofty::picture::PictureType::CoverBack)],
        );
        let cover = read_cover(&path).expect("back cover");
        assert_eq!(cover.bytes, TINY_PNG);
    }
}
