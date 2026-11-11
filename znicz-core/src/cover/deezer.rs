//! Deezer search by artist and title. No key needed; covers up to 1000 px.

use serde_json::Value;

use crate::cover::http::{Fetched, get_json, retryable};
use crate::cover::title::{artist_attempts, artist_matches, search_key, searchable};
use crate::cover::{CoverSource, Found, StreamQuery};

const ORIGIN: &str = "https://api.deezer.com";

pub struct DeezerSource {
    origin: String,
}

impl DeezerSource {
    pub fn new() -> Self {
        Self::at(ORIGIN)
    }

    /// A source talking to another origin (tests).
    pub fn at(origin: &str) -> Self {
        Self {
            origin: origin.trim_end_matches('/').to_string(),
        }
    }
}

impl Default for DeezerSource {
    fn default() -> Self {
        Self::new()
    }
}

impl DeezerSource {
    /// One search: `Ok(Some(url))` on a matching hit, `Ok(None)` when nothing
    /// matches, `Err(found)` when the source must stop with `found`.
    fn search(&self, text: &str, credit: &str) -> Result<Option<String>, Found> {
        let json = match get_json(
            &format!("{}/search", self.origin),
            &[("q", text), ("limit", "5")],
        ) {
            Fetched::Json(json) => json,
            Fetched::Status(code) => {
                tracing::debug!(source = "deezer", code, "cover search refused");
                return Err(if retryable(code) {
                    Found::Unavailable
                } else {
                    Found::NotFound
                });
            }
            Fetched::Failed => return Err(Found::Unavailable),
        };
        // Deezer reports errors (quota, bad query) as HTTP 200 with an `error` key.
        let hits = json.get("data").and_then(Value::as_array);
        if json.get("error").is_some() || hits.is_none() {
            return Err(Found::Unavailable);
        }
        Ok(hits
            .into_iter()
            .flatten()
            .filter(|hit| {
                hit.pointer("/artist/name")
                    .and_then(Value::as_str)
                    .is_some_and(|name| artist_matches(credit, name))
            })
            .find_map(cover_of))
    }
}

fn cover_of(hit: &Value) -> Option<String> {
    let album = hit.get("album")?;
    ["cover_xl", "cover_big"]
        .iter()
        .find_map(|size| album.get(*size).and_then(Value::as_str))
        .filter(|url| !url.is_empty())
        .map(str::to_string)
}

impl CoverSource for DeezerSource {
    fn name(&self) -> &'static str {
        "deezer"
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
    use crate::cover::StreamQuery;
    use crate::cover::http::test_server::serve;

    fn query(title: Option<&str>, search: bool) -> StreamQuery {
        StreamQuery {
            stream_url: "https://channels.fluxfm.de/jazz-schwarzenstein/stream.mp3".into(),
            station_name: "FluxFM - Jazzradio Schwarzenstein".into(),
            icy_title: title.map(str::to_string),
            icy_url: None,
            station_art: None,
            search,
        }
    }

    const WEBSTER: &str = "Ben Webster - Willow Weep For Me";

    fn body(json: &'static str) -> impl Fn(&str) -> (u16, Vec<u8>) {
        move |_line: &str| (200, json.as_bytes().to_vec())
    }

    #[test]
    fn a_matching_artist_gives_the_xl_cover() {
        let (origin, seen) = serve(body(
            r#"{"data":[{"title":"Willow Weep For Me","artist":{"name":"Ben Webster"},
                "album":{"cover_big":"https://cdn/500.jpg","cover_xl":"https://cdn/1000.jpg"}}]}"#,
        ));
        let source = DeezerSource::at(&origin);
        assert_eq!(
            source.find(&query(Some(WEBSTER), true)),
            Found::Url("https://cdn/1000.jpg".into())
        );
        let request = seen.lock().unwrap()[0].clone();
        assert!(request.starts_with("GET /search?"), "{request}");
        assert!(request.contains("q=Ben"), "{request}");
        assert!(request.contains("limit=5"), "{request}");
        assert!(!request.contains("artist%3A"), "{request}");
    }

    #[test]
    fn an_error_body_is_unavailable() {
        let (origin, _) = serve(body(
            r#"{"error":{"type":"Exception","message":"Quota limit exceeded","code":4}}"#,
        ));
        assert_eq!(
            DeezerSource::at(&origin).find(&query(Some(WEBSTER), true)),
            Found::Unavailable
        );
    }

    #[test]
    fn a_hit_by_another_artist_is_skipped() {
        let (origin, _) = serve(body(
            r#"{"data":[
                {"artist":{"name":"Stereo Total"},"album":{"cover_xl":"https://cdn/wrong.jpg"}},
                {"artist":{"name":"Ben Webster"},"album":{"cover_big":"https://cdn/500.jpg"}}]}"#,
        ));
        assert_eq!(
            DeezerSource::at(&origin).find(&query(Some(WEBSTER), true)),
            Found::Url("https://cdn/500.jpg".into()),
            "falls back to cover_big when cover_xl is missing"
        );
    }

    #[test]
    fn no_hits_is_not_found() {
        let (origin, _) = serve(body(r#"{"data":[],"total":0}"#));
        assert_eq!(
            DeezerSource::at(&origin).find(&query(Some("Serotonal - Memory Leaves"), true)),
            Found::NotFound
        );
    }

    #[test]
    fn a_server_error_is_unavailable() {
        let (origin, _) = serve(|_line: &str| (500, b"{}".to_vec()));
        assert_eq!(
            DeezerSource::at(&origin).find(&query(Some(WEBSTER), true)),
            Found::Unavailable
        );
    }

    #[test]
    fn applies_only_with_search_on_and_a_split_title() {
        let source = DeezerSource::new();
        assert_eq!(
            source.key(&query(Some(WEBSTER), true)),
            Some("ben webster|willow weep for me".into())
        );
        assert_eq!(source.key(&query(Some(WEBSTER), false)), None);
        assert_eq!(source.key(&query(Some("Jazz all night"), true)), None);
        assert_eq!(source.key(&query(None, true)), None);
    }

    const JOPEK: &str = "Anna Maria Jopek & Friends with Pat Metheny - Przypływ, odpływ";

    #[test]
    fn a_long_credit_is_retried_with_the_lead_artist() {
        let (origin, seen) = serve(|line: &str| {
            if line.contains("Friends") {
                (200, br#"{"data":[]}"#.to_vec())
            } else {
                (
                    200,
                    br#"{"data":[{"artist":{"name":"Pat Metheny"},
                    "album":{"cover_xl":"https://cdn/upojenie.jpg"}}]}"#
                        .to_vec(),
                )
            }
        });
        let found = DeezerSource::at(&origin).find(&query(Some(JOPEK), true));
        assert_eq!(found, Found::Url("https://cdn/upojenie.jpg".into()));
        let requests = seen.lock().unwrap();
        assert_eq!(requests.len(), 2);
        assert!(!requests[1].lines().next().unwrap().contains("Friends"));
    }

    #[test]
    fn a_single_artist_is_searched_once() {
        let (origin, seen) = serve(body(r#"{"data":[]}"#));
        DeezerSource::at(&origin).find(&query(Some(WEBSTER), true));
        assert_eq!(seen.lock().unwrap().len(), 1);
    }
}
