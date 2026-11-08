//! Which library rows stand for what is playing.

use std::path::PathBuf;

use znicz_core::TrackInfo;
use znicz_library::{AlbumSummary, ArtistSummary, Track};

/// The playing file, and the browse artist and album it belongs to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlayingRef {
    pub path: PathBuf,
    pub artist: Option<String>,
    pub album: Option<String>,
}

/// Case-insensitive for any script, not just ASCII.
fn same(a: &str, b: &str) -> bool {
    a.to_lowercase() == b.to_lowercase()
}

impl PlayingRef {
    /// From the library's own row: browse artist is the album artist, else the artist.
    pub fn from_track(track: &Track) -> Self {
        Self {
            path: track.path.clone(),
            artist: track.album_artist.clone().or_else(|| track.artist.clone()),
            album: track.album.clone(),
        }
    }

    /// From the file's tags, when the library does not know the file. Streams give `None`.
    pub fn from_info(info: &TrackInfo) -> Option<Self> {
        Some(Self {
            path: info.path.clone()?,
            artist: info
                .tags
                .album_artist
                .clone()
                .or_else(|| info.tags.artist.clone()),
            album: info.tags.album.clone(),
        })
    }

    pub fn is_track(&self, track: &Track) -> bool {
        track.path == self.path
    }

    pub fn is_artist(&self, artist: &ArtistSummary) -> bool {
        self.artist
            .as_deref()
            .is_some_and(|a| same(a, &artist.name))
    }

    pub fn is_album(&self, album: &AlbumSummary) -> bool {
        let Some(name) = self.album.as_deref() else {
            return false;
        };
        let artist_matches = match (album.album_artist.as_deref(), self.artist.as_deref()) {
            (Some(album_artist), Some(artist)) => same(album_artist, artist),
            _ => true,
        };
        same(name, &album.album) && artist_matches
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use znicz_core::TrackTags;

    fn summary(album: &str, artist: Option<&str>) -> AlbumSummary {
        AlbumSummary {
            album: album.into(),
            album_artist: artist.map(Into::into),
            year: None,
            track_count: 1,
            total_secs: None,
        }
    }

    fn playing() -> PlayingRef {
        PlayingRef {
            path: PathBuf::from("/music/a.flac"),
            artist: Some("Кино".into()),
            album: Some("Группа крови".into()),
        }
    }

    #[test]
    fn artist_and_album_match_ignoring_case_in_any_script() {
        let p = playing();
        assert!(p.is_artist(&ArtistSummary {
            name: "КИНО".into(),
            track_count: 1
        }));
        assert!(p.is_album(&summary("группа крови", Some("кино"))));
        assert!(
            p.is_album(&summary("Группа крови", None)),
            "no album artist"
        );
        assert!(!p.is_album(&summary("Группа крови", Some("Other"))));
    }

    #[test]
    fn a_stream_has_no_playing_ref() {
        let info = TrackInfo {
            path: None,
            url: Some("http://radio".into()),
            icy_stream_url: None,
            title: "Radio".into(),
            codec: "MP3".into(),
            sample_rate: 44_100,
            channels: 2,
            bits_per_sample: None,
            bitrate_kbps: None,
            duration: None,
            tags: TrackTags::default(),
        };
        assert_eq!(PlayingRef::from_info(&info), None);
    }
}
