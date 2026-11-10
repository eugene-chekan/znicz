//! Apple's iTunes Search API by artist and title. No key; Apple serves the
//! artwork at any requested size, so 1000 px is asked for.

use serde_json::Value;

use crate::cover::http::{get_json, retryable, Fetched};
use crate::cover::title::{artist_attempts, artist_matches, search_key, searchable};
use crate::cover::{CoverSource, Found, StreamQuery};

const ORIGIN: &str = "https://itunes.apple.com";

pub struct ItunesSource {
    origin: String,
}

impl ItunesSource {
    pub fn new() -> Self {
        Self::at(ORIGIN)
    }

    /// A source talking to another origin (tests).
    pub fn at(origin: &str) -> Self {
        Self {
            origin: origin.trim_end_matches('/').to_string(),
        }
    }

    /// One search; see `DeezerSource::search` for the result shape.
    fn search(&self, text: &str, credit: &str) -> Result<Option<String>, Found> {
        let url = format!("{}/search", self.origin);
        let json = match get_json(&url, &[("term", text), ("entity", "song"), ("limit", "5")]) {
            Fetched::Json(json) => json,
            Fetched::Status(code) => {
                tracing::debug!(source = "itunes", code, "cover search refused");
                // iTunes answers 403 when it rate-limits a client.
                return Err(if code == 403 || retryable(code) {
                    Found::Unavailable
                } else {
                    Found::NotFound
                });
            }
            Fetched::Failed => return Err(Found::Unavailable),
        };
        let Some(results) = json.get("results").and_then(Value::as_array) else {
            return Err(Found::Unavailable);
        };
        Ok(results
            .iter()
            .filter(|hit| {
                hit.get("artistName")
                    .and_then(Value::as_str)
                    .is_some_and(|name| artist_matches(credit, name))
            })
            .find_map(|hit| hit.get("artworkUrl100").and_then(Value::as_str))
            .map(large_artwork))
    }
}

impl Default for ItunesSource {
    fn default() -> Self {
        Self::new()
    }
}

/// Apple serves artwork at the size named in the URL; ask for 1000 px.
fn large_artwork(url: &str) -> String {
    url.replace("/100x100bb.", "/1000x1000bb.")
}

impl CoverSource for ItunesSource {
    fn name(&self) -> &'static str {
        "itunes"
    }

    fn key(&self, query: &StreamQuery) -> Option<String> {
        search_key(query)
    }

    fn find(&self, query: &StreamQuery) -> Found {
        let Some((artist, title)) = searchable(query) else {
            return Found::NotFound;
        };
        for attempt in artist_attempts(artist) {
            match self.search(&format!("{attempt} {title}"), artist) {
                Ok(Some(url)) => return Found::Url(url),
                Ok(None) => {}
                Err(found) => return found,
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

    fn query(title: &str) -> StreamQuery {
        StreamQuery {
            stream_url: "https://channels.fluxfm.de/jazz-schwarzenstein/stream.mp3".into(),
            station_name: "FluxFM - Jazzradio Schwarzenstein".into(),
            icy_title: Some(title.into()),
            icy_url: None,
            station_art: None,
            search: true,
        }
    }

    const ART: &str =
        "https://is1-ssl.mzstatic.com/image/thumb/Music126/v4/24/e0/10/x/cover.jpg/100x100bb.jpg";

    fn results(json: &'static str) -> impl Fn(&str) -> (u16, Vec<u8>) {
        move |_line: &str| (200, json.as_bytes().to_vec())
    }

    #[test]
    fn a_matching_artist_gives_the_1000px_artwork() {
        let (origin, seen) = serve(results(
            r#"{"resultCount":1,"results":[{"artistName":"Ben Webster","trackName":"Willow Weep for Me",
                "artworkUrl100":"https://is1-ssl.mzstatic.com/image/thumb/Music126/v4/24/e0/10/x/cover.jpg/100x100bb.jpg"}]}"#,
        ));
        let found = ItunesSource::at(&origin).find(&query("Ben Webster - Willow Weep For Me"));
        assert_eq!(
            found,
            Found::Url(ART.replace("/100x100bb.", "/1000x1000bb."))
        );
        let request = seen.lock().unwrap()[0].clone();
        assert!(request.starts_with("GET /search?"), "{request}");
        assert!(request.contains("entity=song"), "{request}");
    }

    #[test]
    fn another_artist_or_no_results_is_not_found() {
        let (origin, _) = serve(results(
            r#"{"resultCount":1,"results":[{"artistName":"Stereo Total","artworkUrl100":"https://x/100x100bb.jpg"}]}"#,
        ));
        assert_eq!(
            ItunesSource::at(&origin).find(&query("Serotonal - Memory Leaves")),
            Found::NotFound
        );
        let (origin, _) = serve(results(r#"{"resultCount":0,"results":[]}"#));
        assert_eq!(
            ItunesSource::at(&origin).find(&query("Serotonal - Memory Leaves")),
            Found::NotFound
        );
    }

    #[test]
    fn rate_limits_are_unavailable_and_other_refusals_not_found() {
        let (origin, _) = serve(|_line: &str| (403, Vec::new()));
        assert_eq!(
            ItunesSource::at(&origin).find(&query("A - B")),
            Found::Unavailable
        );
        let (origin, _) = serve(|_line: &str| (404, Vec::new()));
        assert_eq!(
            ItunesSource::at(&origin).find(&query("A - B")),
            Found::NotFound
        );
    }

    #[test]
    fn a_long_credit_is_retried_with_the_lead_artist() {
        let (origin, seen) = serve(|line: &str| {
            if line.contains("Friends") {
                (200, br#"{"resultCount":0,"results":[]}"#.to_vec())
            } else {
                (
                    200,
                    br#"{"resultCount":1,"results":[{"artistName":"Anna Maria Jopek",
                    "artworkUrl100":"https://x/cover.jpg/100x100bb.jpg"}]}"#
                        .to_vec(),
                )
            }
        });
        let found = ItunesSource::at(&origin).find(&query(
            "Anna Maria Jopek & Friends with Pat Metheny - Przypływ, odpływ",
        ));
        assert_eq!(
            found,
            Found::Url("https://x/cover.jpg/1000x1000bb.jpg".into())
        );
        assert_eq!(seen.lock().unwrap().len(), 2);
    }

    #[test]
    fn applies_like_the_other_search_sources() {
        assert_eq!(
            ItunesSource::new().key(&query("Ben Webster - Willow Weep For Me")),
            Some("ben webster|willow weep for me".into())
        );
        let mut off = query("Ben Webster - Willow Weep For Me");
        off.search = false;
        assert_eq!(ItunesSource::new().key(&off), None);
    }
}
