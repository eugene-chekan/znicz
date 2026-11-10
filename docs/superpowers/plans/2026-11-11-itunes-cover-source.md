# iTunes Cover Source Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add an iTunes Search cover source and make every search source retry with the lead artist of a multi-artist credit.

**Architecture:** `cover/title.rs` gains `lead_artist` and `artist_attempts`. Deezer, MusicBrainz, and the new `cover/itunes.rs` loop over the attempts. `default_sources()` lists `itunes` between `deezer` and `musicbrainz`.

**Tech Stack:** Rust, `ureq` 3, `serde_json`.

**Spec:** `docs/superpowers/specs/2026-11-11-itunes-cover-source-design.md`

**Every subagent:** first run `git branch --show-current`; it must print `feat/itunes-cover-source`. Stop if not.

---

### Task 1: Lead artist and attempts

**Files:** Modify `znicz-core/src/cover/title.rs`

- [ ] **Step 1: Failing tests** (add to the `tests` module)

```rust
    #[test]
    fn the_lead_artist_is_the_credit_up_to_the_first_joiner() {
        assert_eq!(
            lead_artist("Anna Maria Jopek & Friends with Pat Metheny"),
            "Anna Maria Jopek"
        );
        assert_eq!(lead_artist("Nils Landgren With Esbjörn Svensson"), "Nils Landgren");
        assert_eq!(lead_artist("Lizz Wright FEAT. Gregory Porter"), "Lizz Wright");
        assert_eq!(lead_artist("A ft. B"), "A");
        assert_eq!(lead_artist("A featuring B"), "A");
        assert_eq!(lead_artist("A feat B"), "A");
        assert_eq!(lead_artist("Chick Corea and Gary Burton"), "Chick Corea");
        assert_eq!(lead_artist("Trio X, Guest"), "Trio X");
        assert_eq!(lead_artist("A x B"), "A");
        assert_eq!(lead_artist("A + B"), "A");
        assert_eq!(lead_artist("Ben Webster"), "Ben Webster");
    }

    #[test]
    fn attempts_are_the_credit_then_the_lead_artist() {
        assert_eq!(
            artist_attempts("Anna Maria Jopek & Friends with Pat Metheny"),
            vec!["Anna Maria Jopek & Friends with Pat Metheny", "Anna Maria Jopek"]
        );
        assert_eq!(artist_attempts("Ben Webster"), vec!["Ben Webster"]);
    }
```

- [ ] **Step 2:** `cargo test -p znicz-core --lib cover::title` → compile errors (functions missing).

- [ ] **Step 3: Implement** (above `#[cfg(test)]`)

```rust
/// What joins several artists in one credit (`A & B`, `A feat. B`, …).
const CREDIT_JOINERS: [&str; 10] = [
    " & ", " and ", " with ", " feat. ", " feat ", " ft. ", " featuring ", ", ", " x ", " + ",
];

/// The first artist of a multi-artist credit: the text up to the earliest
/// joiner, case-insensitive. A credit without joiners is returned whole.
pub fn lead_artist(credit: &str) -> &str {
    // ASCII lowercasing keeps byte offsets, so positions map back to `credit`.
    let lower = credit.to_ascii_lowercase();
    CREDIT_JOINERS
        .iter()
        .filter_map(|joiner| lower.find(joiner))
        .min()
        .map_or(credit, |at| credit[..at].trim())
}

/// Artist texts to search with, in order: the full credit, then its lead
/// artist when that differs.
pub(crate) fn artist_attempts(credit: &str) -> Vec<&str> {
    let lead = lead_artist(credit);
    if lead.is_empty() || lead == credit {
        vec![credit]
    } else {
        vec![credit, lead]
    }
}
```

- [ ] **Step 4:** `cargo test -p znicz-core --lib cover::title` → all pass.
- [ ] **Step 5:** Commit `Add lead-artist attempts for cover search`.

---

### Task 2: Deezer and MusicBrainz retry with the lead artist

**Files:** Modify `znicz-core/src/cover/deezer.rs`, `znicz-core/src/cover/musicbrainz.rs`

- [ ] **Step 1: Failing tests**

`deezer.rs` tests:

```rust
    const JOPEK: &str = "Anna Maria Jopek & Friends with Pat Metheny - Przypływ, odpływ";

    #[test]
    fn a_long_credit_is_retried_with_the_lead_artist() {
        let (origin, seen) = serve(|line: &str| {
            if line.contains("Friends") {
                (200, br#"{"data":[]}"#.to_vec())
            } else {
                (200, br#"{"data":[{"artist":{"name":"Pat Metheny"},
                    "album":{"cover_xl":"https://cdn/upojenie.jpg"}}]}"#.to_vec())
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
```

`musicbrainz.rs` tests (the MusicBrainz server answers by request line):

```rust
    #[test]
    fn a_long_credit_is_retried_with_the_lead_artist() {
        let (mb, seen) = serve(|line: &str| {
            if line.contains("Friends") {
                (200, br#"{"recordings":[]}"#.to_vec())
            } else {
                (200, br#"{"recordings":[{"score":100,
                    "artist-credit":[{"name":"Anna Maria Jopek"}],
                    "releases":[{"id":"upojenie"}]}]}"#.to_vec())
            }
        });
        let (caa, _) = serve(|_line: &str| (200, b"img".to_vec()));
        let source = MusicBrainzSource::at(&mb, &caa, Duration::ZERO);
        let found = source.find(&query(
            "Anna Maria Jopek & Friends with Pat Metheny - Przypływ, odpływ",
        ));
        assert!(matches!(found, Found::Url(url) if url.ends_with("/release/upojenie/front-500")));
        assert_eq!(seen.lock().unwrap().len(), 2);
    }
```

- [ ] **Step 2:** `cargo test -p znicz-core --lib cover::` → the new tests fail (one request only / NotFound).

- [ ] **Step 3: Implement**

Deezer: move the single search into a helper and loop over attempts. The hit filter still matches against the **full** credit.

```rust
use crate::cover::title::{artist_attempts, artist_matches, search_key, searchable};

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
                return Err(if retryable(code) { Found::Unavailable } else { Found::NotFound });
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

// in `impl CoverSource for DeezerSource`
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
```

MusicBrainz: same shape. Keep `release_ids(json, artist)` matching the full credit; build the Lucene query per attempt; each attempt calls `self.wait_turn()` before its request; the release probing loop runs for the first attempt that yields release ids.

```rust
    fn find(&self, query: &StreamQuery) -> Found {
        let Some((artist, title)) = searchable(query) else {
            return Found::NotFound;
        };
        for attempt in artist_attempts(artist) {
            let search = format!(
                "artist:\"{}\" AND recording:\"{}\"",
                quoted(attempt),
                quoted(title)
            );
            self.wait_turn();
            let url = format!("{}/ws/2/recording/", self.musicbrainz);
            let json = match get_json(&url, &[("query", &search), ("fmt", "json"), ("limit", "5")]) {
                Fetched::Json(json) => json,
                Fetched::Status(code) => {
                    tracing::debug!(source = "musicbrainz", code, "cover search refused");
                    return if retryable(code) { Found::Unavailable } else { Found::NotFound };
                }
                Fetched::Failed => return Found::Unavailable,
            };
            let ids = release_ids(&json, artist);
            if ids.is_empty() {
                continue;
            }
            for id in ids.into_iter().take(MAX_RELEASES) {
                let front = format!("{}/release/{id}/front-500", self.cover_art);
                match probe(&front) {
                    Some(200) => return Found::Url(front),
                    Some(code) if retryable(code) => return Found::Unavailable,
                    Some(_) => {}
                    None => return Found::Unavailable,
                }
            }
            return Found::NotFound;
        }
        Found::NotFound
    }
```

- [ ] **Step 4:** `cargo test -p znicz-core --lib cover::` → all pass. `cargo clippy -p znicz-core --all-targets -- -D warnings` clean.
- [ ] **Step 5:** Commit `Retry Deezer and MusicBrainz cover searches with the lead artist`.

---

### Task 3: iTunes source

**Files:** Create `znicz-core/src/cover/itunes.rs`; modify `cover/mod.rs` (`pub mod itunes;`, `default_sources`, chain-order test)

- [ ] **Step 1: Failing tests** — `itunes.rs`:

```rust
//! Apple's iTunes Search API by artist and title. No key; Apple serves the
//! artwork at any requested size, so 1000 px is asked for.

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

    const ART: &str = "https://is1-ssl.mzstatic.com/image/thumb/Music126/v4/24/e0/10/x/cover.jpg/100x100bb.jpg";

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
        assert_eq!(found, Found::Url(ART.replace("/100x100bb.", "/1000x1000bb.")));
        let request = seen.lock().unwrap()[0].clone();
        assert!(request.starts_with("GET /search?"), "{request}");
        assert!(request.contains("entity=song"), "{request}");
    }

    #[test]
    fn another_artist_or_no_results_is_not_found() {
        let (origin, _) = serve(results(
            r#"{"resultCount":1,"results":[{"artistName":"Stereo Total","artworkUrl100":"https://x/100x100bb.jpg"}]}"#,
        ));
        assert_eq!(ItunesSource::at(&origin).find(&query("Serotonal - Memory Leaves")), Found::NotFound);
        let (origin, _) = serve(results(r#"{"resultCount":0,"results":[]}"#));
        assert_eq!(ItunesSource::at(&origin).find(&query("Serotonal - Memory Leaves")), Found::NotFound);
    }

    #[test]
    fn rate_limits_are_unavailable_and_other_refusals_not_found() {
        let (origin, _) = serve(|_line: &str| (403, Vec::new()));
        assert_eq!(ItunesSource::at(&origin).find(&query("A - B")), Found::Unavailable);
        let (origin, _) = serve(|_line: &str| (404, Vec::new()));
        assert_eq!(ItunesSource::at(&origin).find(&query("A - B")), Found::NotFound);
    }

    #[test]
    fn a_long_credit_is_retried_with_the_lead_artist() {
        let (origin, seen) = serve(|line: &str| {
            if line.contains("Friends") {
                (200, br#"{"resultCount":0,"results":[]}"#.to_vec())
            } else {
                (200, br#"{"resultCount":1,"results":[{"artistName":"Anna Maria Jopek",
                    "artworkUrl100":"https://x/cover.jpg/100x100bb.jpg"}]}"#.to_vec())
            }
        });
        let found = ItunesSource::at(&origin)
            .find(&query("Anna Maria Jopek & Friends with Pat Metheny - Przypływ, odpływ"));
        assert_eq!(found, Found::Url("https://x/cover.jpg/1000x1000bb.jpg".into()));
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
```

Update the chain-order test in `cover/mod.rs` to
`["icy_url", "audioaddict", "deezer", "itunes", "musicbrainz", "station_art"]`.

The local test server's status reasons: add `403 => "Forbidden"` to the match in `http.rs` `test_server::serve` if it only knows 200/404/503.

- [ ] **Step 2:** `cargo test -p znicz-core --lib cover::` → compile errors (`ItunesSource` missing).

- [ ] **Step 3: Implement** (above the tests in `itunes.rs`)

```rust
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
```

`cover/mod.rs`: `pub mod itunes;`, and in `default_sources()` insert `Box::new(itunes::ItunesSource::new()),` after the Deezer line. Update the `default_sources` doc comment to mention iTunes.

- [ ] **Step 4:** `cargo test -p znicz-core --lib cover::` → all pass; clippy clean.
- [ ] **Step 5:** Commit `Add the iTunes Search cover source`.

---

### Task 4: Docs, live test, version

**Files:** `Cargo.toml`, `README.md`, `wiki/Architecture/Cover-Sources.md`, `wiki/Architecture/TUI.md`, `wiki/Domain/Formats-and-Metadata.md`, `znicz-core/tests/cover_live.rs`, this plan

- [ ] **Step 1:** Version `0.12.0` in the root `Cargo.toml`.
- [ ] **Step 2:** Docs: everywhere the chain order is listed (README features bullet and Stage sentence, `Cover-Sources.md`, `TUI.md`, `Formats-and-Metadata.md`), add iTunes after Deezer. `Cover-Sources.md`: an iTunes section (endpoint, 1000 px artwork, 403 = rate limit) and a "Long credits" paragraph (lead-artist retry, joiners list, matching against the full credit). README privacy sentence: titles go to Deezer, then Apple (iTunes) and MusicBrainz when earlier sources find nothing. `[covers] search` comment: `(Deezer, iTunes, MusicBrainz)`.
- [ ] **Step 3:** Live test: in `znicz-core/tests/cover_live.rs`, add a third query (FluxFM station, icy_title `Anna Maria Jopek & Friends with Pat Metheny - Przypływ, odpływ`) to `live_covers_for_known_stations`, asserting `Resolution::Found` with an image mime. Run `cargo test -p znicz-core --test cover_live -- --ignored --nocapture` and report.
- [ ] **Step 4:** `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`.
- [ ] **Step 5:** Tick this plan's boxes; commit `Document the iTunes cover source; bump to 0.12.0`.
