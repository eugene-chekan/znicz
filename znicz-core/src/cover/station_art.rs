//! The station's local `art` image from `stations.toml`.

use crate::cover::{CoverSource, Found, StreamQuery};
use crate::metadata::{CoverArt, sniff_image_mime};

pub struct StationArtSource;

impl CoverSource for StationArtSource {
    fn name(&self) -> &'static str {
        "station_art"
    }

    fn key(&self, query: &StreamQuery) -> Option<String> {
        query
            .station_art
            .as_ref()
            .map(|path| path.display().to_string())
    }

    fn find(&self, query: &StreamQuery) -> Found {
        let Some(path) = query.station_art.as_ref() else {
            return Found::NotFound;
        };
        match std::fs::read(path) {
            Ok(bytes) if !bytes.is_empty() => Found::Art(CoverArt {
                mime: sniff_image_mime(&bytes),
                bytes,
            }),
            Ok(_) => Found::NotFound,
            Err(e) => {
                tracing::debug!(path = %path.display(), error = %e, "station art unreadable");
                Found::NotFound
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cover::{Found, StreamQuery};
    use std::path::PathBuf;

    fn query(art: Option<PathBuf>) -> StreamQuery {
        StreamQuery {
            stream_url: "http://radio/stream".into(),
            station_name: "Radio".into(),
            icy_title: None,
            icy_url: None,
            station_art: art,
            search: true,
        }
    }

    #[test]
    fn a_readable_file_is_the_cover() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("art.png");
        std::fs::write(&path, [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]).unwrap();
        let q = query(Some(path.clone()));

        assert_eq!(StationArtSource.key(&q), Some(path.display().to_string()));
        match StationArtSource.find(&q) {
            Found::Art(art) => assert_eq!(art.mime, "image/png"),
            other => panic!("expected art, got {other:?}"),
        }
    }

    #[test]
    fn a_missing_file_is_not_found_and_no_art_does_not_apply() {
        let q = query(Some(PathBuf::from("/nowhere/art.png")));
        assert_eq!(StationArtSource.find(&q), Found::NotFound);
        assert_eq!(StationArtSource.key(&query(None)), None);
    }
}
