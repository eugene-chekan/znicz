use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// One track as stored in the library.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Track {
    pub id: i64,
    pub path: PathBuf,
    pub title: String,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub album_artist: Option<String>,
    pub genre: Option<String>,
    pub year: Option<u32>,
    pub track_number: Option<u32>,
    pub disc_number: Option<u32>,
    pub codec: Option<String>,
    pub sample_rate: Option<u32>,
    pub channels: Option<u16>,
    pub bits_per_sample: Option<u32>,
    pub duration_secs: Option<f64>,
}

impl Track {
    /// "Artist — Album", or whichever half exists.
    pub fn artist_album(&self) -> Option<String> {
        match (self.artist.as_deref(), self.album.as_deref()) {
            (Some(artist), Some(album)) => Some(format!("{artist} — {album}")),
            (Some(artist), None) => Some(artist.to_string()),
            (None, Some(album)) => Some(album.to_string()),
            (None, None) => None,
        }
    }
}

/// One album, grouped from the tracks that belong to it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AlbumSummary {
    pub album: String,
    pub album_artist: Option<String>,
    pub year: Option<u32>,
    pub track_count: u32,
    pub total_secs: Option<f64>,
}
