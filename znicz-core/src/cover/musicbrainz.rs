//! MusicBrainz recording search, then the release's front cover from the
//! Cover Art Archive. Finds niche artists Deezer does not know.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde_json::Value;

use crate::cover::deezer::{search_key, searchable};
use crate::cover::http::{get_json, probe, Fetched};
use crate::cover::title::artist_matches;
use crate::cover::{CoverSource, Found, StreamQuery};

const MUSICBRAINZ: &str = "https://musicbrainz.org";
const COVER_ART_ARCHIVE: &str = "https://coverartarchive.org";
/// MusicBrainz asks for at most one request per second.
const MIN_GAP: Duration = Duration::from_secs(1);
const MIN_SCORE: u64 = 90;
const MAX_RELEASES: usize = 3;

pub struct MusicBrainzSource {
    musicbrainz: String,
    cover_art: String,
    min_gap: Duration,
    last_request: Mutex<Option<Instant>>,
}

impl MusicBrainzSource {
    pub fn new() -> Self {
        Self::at(MUSICBRAINZ, COVER_ART_ARCHIVE, MIN_GAP)
    }

    /// A source talking to other origins with another request gap (tests).
    pub fn at(musicbrainz: &str, cover_art: &str, min_gap: Duration) -> Self {
        Self {
            musicbrainz: musicbrainz.trim_end_matches('/').to_string(),
            cover_art: cover_art.trim_end_matches('/').to_string(),
            min_gap,
            last_request: Mutex::new(None),
        }
    }

    /// Sleep until `min_gap` has passed since the last MusicBrainz request.
    fn wait_turn(&self) {
        let mut last = self.last_request.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(previous) = *last {
            let since = previous.elapsed();
            if since < self.min_gap {
                std::thread::sleep(self.min_gap - since);
            }
        }
        *last = Some(Instant::now());
    }
}

impl Default for MusicBrainzSource {
    fn default() -> Self {
        Self::new()
    }
}

/// Release ids of well-scored recordings by `artist`, in order, deduplicated.
fn release_ids(json: &Value, artist: &str) -> Vec<String> {
    let mut ids: Vec<String> = Vec::new();
    let recordings = json.get("recordings").and_then(Value::as_array);
    for recording in recordings.into_iter().flatten() {
        let score = recording.get("score").and_then(Value::as_u64).unwrap_or(0);
        let by_artist = recording
            .get("artist-credit")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|credit| credit.get("name").and_then(Value::as_str))
            .any(|name| artist_matches(artist, name));
        if score < MIN_SCORE || !by_artist {
            continue;
        }
        let releases = recording.get("releases").and_then(Value::as_array);
        for id in releases
            .into_iter()
            .flatten()
            .filter_map(|release| release.get("id").and_then(Value::as_str))
        {
            if !ids.iter().any(|seen| seen == id) {
                ids.push(id.to_string());
            }
        }
    }
    ids
}

impl CoverSource for MusicBrainzSource {
    fn name(&self) -> &'static str {
        "musicbrainz"
    }

    fn key(&self, query: &StreamQuery) -> Option<String> {
        search_key(query)
    }

    fn find(&self, query: &StreamQuery) -> Found {
        let Some((artist, title)) = searchable(query) else {
            return Found::NotFound;
        };
        let search = format!("artist:\"{artist}\" AND recording:\"{title}\"");
        self.wait_turn();
        let url = format!("{}/ws/2/recording/", self.musicbrainz);
        let json = match get_json(&url, &[("query", &search), ("fmt", "json"), ("limit", "5")]) {
            Fetched::Json(json) => json,
            Fetched::Status(code) => {
                tracing::debug!(source = "musicbrainz", code, "cover search refused");
                return Found::Unavailable;
            }
            Fetched::Failed => return Found::Unavailable,
        };
        for id in release_ids(&json, artist).into_iter().take(MAX_RELEASES) {
            let front = format!("{}/release/{id}/front-500", self.cover_art);
            match probe(&front) {
                Some(200) => return Found::Url(front),
                Some(_) => {}
                None => return Found::Unavailable,
            }
        }
        Found::NotFound
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cover::http::test_server::serve;
    use crate::cover::StreamQuery;
    use std::time::Instant;

    fn query(title: &str) -> StreamQuery {
        StreamQuery {
            stream_url: "http://stream.heavy-music.net:8176/stream".into(),
            station_name: "HEAVY-MUSIC.NET".into(),
            icy_title: Some(title.into()),
            icy_url: None,
            station_art: None,
            search: true,
        }
    }

    const SEROTONAL: &str = r#"{"recordings":[
        {"score":100,"title":"Memory Leaves","artist-credit":[{"name":"Serotonal"}],
         "releases":[{"id":"no-art"},{"id":"63138efb"}]},
        {"score":95,"title":"Memory Leaves","artist-credit":[{"name":"Stereo Total"}],
         "releases":[{"id":"wrong-artist"}]},
        {"score":40,"title":"Memories","artist-credit":[{"name":"Serotonal"}],
         "releases":[{"id":"low-score"}]}]}"#;

    /// A MusicBrainz server answering `json`, and a CAA with art for `with_art`.
    fn servers(json: &'static str, with_art: &'static str) -> MusicBrainzSource {
        let (mb, _) = serve(move |_line: &str| (200, json.as_bytes().to_vec()));
        let (caa, _) = serve(move |line: &str| {
            if line.contains(&format!("/release/{with_art}/front-500")) {
                (200, b"img".to_vec())
            } else {
                (404, Vec::new())
            }
        });
        MusicBrainzSource::at(&mb, &caa, Duration::ZERO)
    }

    #[test]
    fn the_first_matching_release_with_art_wins() {
        let source = servers(SEROTONAL, "63138efb");
        match source.find(&query("Serotonal - Memory Leaves")) {
            Found::Url(url) => assert!(url.ends_with("/release/63138efb/front-500"), "{url}"),
            other => panic!("expected a url, got {other:?}"),
        }
    }

    #[test]
    fn other_artists_and_low_scores_are_ignored() {
        let wrong = servers(SEROTONAL, "wrong-artist");
        assert_eq!(
            wrong.find(&query("Serotonal - Memory Leaves")),
            Found::NotFound
        );
        let low = servers(SEROTONAL, "low-score");
        assert_eq!(
            low.find(&query("Serotonal - Memory Leaves")),
            Found::NotFound
        );
    }

    #[test]
    fn no_recordings_is_not_found() {
        let source = servers(r#"{"recordings":[]}"#, "x");
        assert_eq!(source.find(&query("Nobody - Nothing")), Found::NotFound);
    }

    #[test]
    fn a_busy_server_is_unavailable() {
        let (mb, _) = serve(|_line: &str| (503, Vec::new()));
        let source = MusicBrainzSource::at(&mb, "http://127.0.0.1:9", Duration::ZERO);
        assert_eq!(
            source.find(&query("Serotonal - Memory Leaves")),
            Found::Unavailable
        );
    }

    #[test]
    fn requests_identify_znicz() {
        let (mb, seen) = serve(|_line: &str| (200, br#"{"recordings":[]}"#.to_vec()));
        let source = MusicBrainzSource::at(&mb, "http://127.0.0.1:9", Duration::ZERO);
        source.find(&query("Serotonal - Memory Leaves"));
        let request = seen.lock().unwrap()[0].to_ascii_lowercase();
        assert!(request.starts_with("get /ws/2/recording/?"), "{request}");
        assert!(request.contains("user-agent: znicz/"), "{request}");
    }

    #[test]
    fn requests_are_spaced_out() {
        let (mb, _) = serve(|_line: &str| (200, br#"{"recordings":[]}"#.to_vec()));
        let source = MusicBrainzSource::at(&mb, "http://127.0.0.1:9", Duration::from_millis(300));
        let start = Instant::now();
        source.find(&query("A - B"));
        source.find(&query("C - D"));
        assert!(start.elapsed() >= Duration::from_millis(300));
    }
}
