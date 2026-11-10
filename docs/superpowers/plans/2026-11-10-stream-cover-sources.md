# Stream Cover Sources Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the hard-wired stream cover lookups with a chain of `CoverSource`s in `znicz-core`, and add a Deezer + MusicBrainz artist/title search so ordinary Icecast/Shoutcast stations get per-track covers.

**Architecture:** `znicz-core/src/cover/` holds the `CoverSource` trait, one file per source, and a `StreamCovers` resolver that tries the sources in order and remembers answers. The TUI's cover worker owns one `StreamCovers`; Stage builds one `StreamQuery` per stream and ICY title and asks `CoverCache` for `CoverKey::Stream(query)`.

**Tech Stack:** Rust 2021, `ureq` 3 (blocking HTTP), `serde_json`, new `unicode-normalization`, `image` (TUI decoding), `ratatui-image`.

**Spec:** `docs/superpowers/specs/2026-11-10-stream-cover-sources-design.md`

**Deviations from the spec, decided while planning:**

- `CoverSource` gains `fn remembers(&self) -> bool { true }`. AudioAddict returns `false`: it keeps its own 15 s channel map, and a remembered answer would freeze the cover of a stream without ICY titles.
- `StreamCovers::resolve` returns `Resolution { Found(CoverArt), Nothing, Unsure }`. `Unsure` (some source was `Unavailable` and none found an image) lets the TUI keep the image it already shows, as `AudioAddictLookup::RefreshFailed` does today.
- MusicBrainz probes `front-500` for up to three releases and returns `Found::Url` of the first that answers 200; the resolver downloads it (following the redirect).
- A URL that downloads but does not decode as an image shows the logo (decoding happens in the TUI); later sources are not tried for it. Downloads that fail fall through to the next source as specified.

**Every subagent:** first run `git branch --show-current` and confirm it prints `feat/stream-cover-sources`. Stop and report if not.

---

## File structure

| File | Responsibility |
| --- | --- |
| `znicz-core/src/cover/mod.rs` (new) | `StreamQuery`, `Found`, `CoverSource`, `Resolution`, `StreamCovers`, `default_sources()` |
| `znicz-core/src/cover/title.rs` (new) | `split_icy_title`, `fold`, `artist_matches` |
| `znicz-core/src/cover/http.rs` (new) | shared `ureq` agent, JSON GET helper, `#[cfg(test)]` local HTTP server |
| `znicz-core/src/cover/icy_url.rs` (new) | `IcyUrlSource` |
| `znicz-core/src/cover/audioaddict.rs` (new) | `AudioAddictSource` (adapter over `crate::audioaddict`) |
| `znicz-core/src/cover/deezer.rs` (new) | `DeezerSource` |
| `znicz-core/src/cover/musicbrainz.rs` (new) | `MusicBrainzSource` |
| `znicz-core/src/cover/station_art.rs` (new) | `StationArtSource` |
| `znicz-core/src/lib.rs` | `pub mod cover;` + re-exports |
| `znicz-core/src/station.rs` | `Station.cover_search` |
| `znicz-tui/src/cover.rs` | `CoverKey::{File, Stream}`, worker uses `StreamCovers`, 15 s refresh for title-less streams |
| `znicz-tui/src/views/stage.rs` | `stream_query`, keep-previous-cover, drop `pick_stream_cover` |
| `znicz-tui/src/app.rs` | `last_stream_cover` field |
| `znicz-tui/src/tui_config.rs` | `TuiConfig.cover_search` |
| `znicz/src/main.rs` | `[covers] search` config section |
| docs | README, wiki pages, version |

---

### Task 0: Branch up to date

- [x] **Step 1: Rebase on main**

PRs #65 and #66 change `ipc.rs`, `cover.rs` (`fill_cover_slot` takes a background and `see_through`), and `stage.rs`. This branch only has the spec and plan commits.

```bash
git fetch origin
git rebase origin/main
cargo test -p znicz-tui --lib cover 2>&1 | grep "test result"
```

Expected: rebase succeeds with no conflicts (only docs on this branch); tests pass.

---

### Task 1: Title helpers

**Files:**
- Modify: `Cargo.toml` (workspace deps), `znicz-core/Cargo.toml`
- Create: `znicz-core/src/cover/mod.rs`, `znicz-core/src/cover/title.rs`
- Modify: `znicz-core/src/lib.rs`

- [x] **Step 1: Add the dependency**

Root `Cargo.toml`, under `[workspace.dependencies]` (keep alphabetical order with neighbours):

```toml
unicode-normalization = "0.1"
```

`znicz-core/Cargo.toml`, under `[dependencies]`:

```toml
unicode-normalization.workspace = true
```

- [x] **Step 2: Create the module skeleton**

`znicz-core/src/cover/mod.rs`:

```rust
//! Cover art for radio streams: a chain of sources tried in order.

pub mod title;
```

`znicz-core/src/lib.rs`: add `pub mod cover;` after `mod audioaddict;`.

- [x] **Step 3: Write the failing tests**

`znicz-core/src/cover/title.rs`:

```rust
//! Reading `Artist - Title` out of an ICY StreamTitle, and loose name matching.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_artist_and_title() {
        assert_eq!(
            split_icy_title("Ben Webster - Willow Weep For Me", "FluxFM"),
            Some(("Ben Webster", "Willow Weep For Me"))
        );
        assert_eq!(
            split_icy_title("Serotonal – Memory Leaves", "HEAVY-MUSIC.NET"),
            Some(("Serotonal", "Memory Leaves"))
        );
    }

    #[test]
    fn the_first_separator_wins() {
        assert_eq!(split_icy_title("A - B - C", "X"), Some(("A", "B - C")));
    }

    #[test]
    fn titles_that_are_not_artist_and_title_do_not_split() {
        assert_eq!(split_icy_title("Live from Berlin", "X"), None);
        assert_eq!(split_icy_title(" - Song", "X"), None);
        assert_eq!(split_icy_title("Artist - ", "X"), None);
        assert_eq!(split_icy_title("Radio X - Live", "radio x - live"), None);
    }

    #[test]
    fn fold_ignores_case_accents_and_spacing() {
        assert_eq!(fold("  Björk   Guðmundsdóttir "), "bjork guðmundsdottir");
        assert_eq!(fold("SIGUR RÓS"), "sigur ros");
    }

    #[test]
    fn artists_match_loosely() {
        assert!(artist_matches("bjork", "Björk"));
        assert!(artist_matches("Ben Webster", "Ben Webster Quartet"));
        assert!(artist_matches("Ben Webster Quartet", "Ben Webster"));
        assert!(!artist_matches("Serotonal", "Stereo Total"));
        assert!(!artist_matches("", "Anyone"));
    }
}
```

Note: `ð` has no decomposition, so it stays; that is fine.

- [x] **Step 4: Run to see them fail**

Run: `cargo test -p znicz-core --lib cover::title`
Expected: compile errors, `split_icy_title`, `fold`, `artist_matches` not found.

- [x] **Step 5: Implement**

Above the tests in `title.rs`:

```rust
use unicode_normalization::char::is_combining_mark;
use unicode_normalization::UnicodeNormalization;

const SEPARATORS: [&str; 2] = [" - ", " – "];

/// Split an ICY title into `(artist, title)` on the first ` - ` or ` – `.
///
/// `None` when there is no separator, either side is empty, or the whole
/// title is just the station name.
pub fn split_icy_title<'a>(title: &'a str, station_name: &str) -> Option<(&'a str, &'a str)> {
    let title = title.trim();
    if fold(title) == fold(station_name) {
        return None;
    }
    let (at, len) = SEPARATORS
        .iter()
        .filter_map(|sep| title.find(sep).map(|at| (at, sep.len())))
        .min_by_key(|(at, _)| *at)?;
    let artist = title[..at].trim();
    let song = title[at + len..].trim();
    (!artist.is_empty() && !song.is_empty()).then_some((artist, song))
}

/// Lowercase, accents removed, whitespace collapsed: a key for loose matching.
pub fn fold(text: &str) -> String {
    let plain: String = text
        .nfkd()
        .filter(|c| !is_combining_mark(*c))
        .flat_map(char::to_lowercase)
        .collect();
    plain.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Whether a search hit's artist is the artist we asked for: folded, and
/// either name contains the other (`Ben Webster` / `Ben Webster Quartet`).
pub fn artist_matches(wanted: &str, found: &str) -> bool {
    let (wanted, found) = (fold(wanted), fold(found));
    !wanted.is_empty() && !found.is_empty() && (wanted.contains(&found) || found.contains(&wanted))
}
```

- [x] **Step 6: Run the tests**

Run: `cargo test -p znicz-core --lib cover::title`
Expected: 5 passed.

- [x] **Step 7: Commit**

```bash
git add Cargo.toml Cargo.lock znicz-core/Cargo.toml znicz-core/src/lib.rs znicz-core/src/cover
git commit -m "Add ICY title splitting and loose artist matching for cover search"
```

---

### Task 2: The source trait and resolver

**Files:**
- Modify: `znicz-core/src/cover/mod.rs`

- [x] **Step 1: Write the failing tests**

Replace `znicz-core/src/cover/mod.rs` with the module doc, `pub mod title;`, and this test module (implementation comes in Step 3):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    fn art(byte: u8) -> CoverArt {
        CoverArt {
            mime: "image/png".into(),
            bytes: vec![byte],
        }
    }

    fn query(title: Option<&str>) -> StreamQuery {
        StreamQuery {
            stream_url: "http://radio.example/stream".into(),
            station_name: "Radio".into(),
            icy_title: title.map(str::to_string),
            icy_url: None,
            station_art: None,
            search: true,
        }
    }

    /// Answers with `found` and counts calls to `find`.
    struct Fake {
        name: &'static str,
        applies: bool,
        found: fn() -> Found,
        calls: Arc<AtomicUsize>,
        remembers: bool,
    }

    impl Fake {
        fn new(name: &'static str, found: fn() -> Found) -> (Self, Arc<AtomicUsize>) {
            let calls = Arc::new(AtomicUsize::new(0));
            let fake = Self {
                name,
                applies: true,
                found,
                calls: calls.clone(),
                remembers: true,
            };
            (fake, calls)
        }
    }

    impl CoverSource for Fake {
        fn name(&self) -> &'static str {
            self.name
        }
        fn key(&self, query: &StreamQuery) -> Option<String> {
            self.applies
                .then(|| query.icy_title.clone().unwrap_or_default())
        }
        fn find(&self, _query: &StreamQuery) -> Found {
            self.calls.fetch_add(1, Ordering::SeqCst);
            (self.found)()
        }
        fn remembers(&self) -> bool {
            self.remembers
        }
    }

    fn fetch_ok(url: &str) -> Option<CoverArt> {
        (url == "http://img/ok").then(|| art(7))
    }

    fn covers(sources: Vec<Box<dyn CoverSource>>) -> StreamCovers {
        StreamCovers::with_fetcher(sources, fetch_ok)
    }

    #[test]
    fn the_first_source_with_an_image_wins() {
        let (none, _) = Fake::new("none", || Found::NotFound);
        let (first, _) = Fake::new("first", || Found::Art(art(1)));
        let (second, second_calls) = Fake::new("second", || Found::Art(art(2)));
        let covers = covers(vec![Box::new(none), Box::new(first), Box::new(second)]);

        assert_eq!(covers.resolve(&query(Some("A - B"))), Resolution::Found(art(1)));
        assert_eq!(second_calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn a_source_that_does_not_apply_is_skipped() {
        let (mut skipped, skipped_calls) = Fake::new("skipped", || Found::Art(art(1)));
        skipped.applies = false;
        let (used, _) = Fake::new("used", || Found::Art(art(2)));
        let covers = covers(vec![Box::new(skipped), Box::new(used)]);

        assert_eq!(covers.resolve(&query(None)), Resolution::Found(art(2)));
        assert_eq!(skipped_calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn not_found_is_remembered_per_key() {
        let (source, calls) = Fake::new("s", || Found::NotFound);
        let covers = covers(vec![Box::new(source)]);

        assert_eq!(covers.resolve(&query(Some("A - B"))), Resolution::Nothing);
        assert_eq!(covers.resolve(&query(Some("A - B"))), Resolution::Nothing);
        assert_eq!(calls.load(Ordering::SeqCst), 1);

        covers.resolve(&query(Some("C - D")));
        assert_eq!(calls.load(Ordering::SeqCst), 2, "a new key asks again");
    }

    #[test]
    fn unavailable_is_not_remembered_and_makes_the_answer_unsure() {
        let (source, calls) = Fake::new("s", || Found::Unavailable);
        let covers = covers(vec![Box::new(source)]);

        assert_eq!(covers.resolve(&query(Some("A - B"))), Resolution::Unsure);
        assert_eq!(covers.resolve(&query(Some("A - B"))), Resolution::Unsure);
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn a_source_that_does_not_remember_is_asked_every_time() {
        let (mut source, calls) = Fake::new("s", || Found::NotFound);
        source.remembers = false;
        let covers = covers(vec![Box::new(source)]);

        covers.resolve(&query(None));
        covers.resolve(&query(None));
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn a_url_is_downloaded() {
        let (source, _) = Fake::new("s", || Found::Url("http://img/ok".into()));
        let covers = covers(vec![Box::new(source)]);
        assert_eq!(covers.resolve(&query(Some("A - B"))), Resolution::Found(art(7)));
    }

    #[test]
    fn a_url_that_fails_falls_through_and_is_remembered() {
        let (broken, broken_calls) = Fake::new("broken", || Found::Url("http://img/404".into()));
        let (next, _) = Fake::new("next", || Found::Art(art(3)));
        let covers = covers(vec![Box::new(broken), Box::new(next)]);

        assert_eq!(covers.resolve(&query(Some("A - B"))), Resolution::Found(art(3)));
        assert_eq!(covers.resolve(&query(Some("A - B"))), Resolution::Found(art(3)));
        assert_eq!(broken_calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn the_answer_cache_is_capped() {
        let (source, calls) = Fake::new("s", || Found::NotFound);
        let covers = covers(vec![Box::new(source)]);

        covers.resolve(&query(Some("first")));
        for n in 0..ANSWER_CAP {
            covers.resolve(&query(Some(&format!("t{n}"))));
        }
        let before = calls.load(Ordering::SeqCst);
        covers.resolve(&query(Some("first")));
        assert_eq!(calls.load(Ordering::SeqCst), before + 1, "the oldest answer was dropped");
    }
}
```

- [x] **Step 2: Run to see them fail**

Run: `cargo test -p znicz-core --lib cover::tests`
Expected: compile errors (`StreamQuery`, `Found`, `CoverSource`, … not found).

- [x] **Step 3: Implement**

In `znicz-core/src/cover/mod.rs`, between `pub mod title;` and the tests:

```rust
use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::Mutex;

use crate::metadata::CoverArt;

/// How many source answers are remembered for the session.
pub const ANSWER_CAP: usize = 256;

/// Everything a source may need to find a stream's cover.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct StreamQuery {
    pub stream_url: String,
    pub station_name: String,
    /// The ICY StreamTitle, when the station sent one.
    pub icy_title: Option<String>,
    /// The ICY StreamUrl, when the station sent one.
    pub icy_url: Option<String>,
    /// The station's local `art` file.
    pub station_art: Option<PathBuf>,
    /// Artist / title search allowed (global and per-station switches).
    pub search: bool,
}

/// One source's answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Found {
    /// An image URL to download.
    Url(String),
    /// Image bytes already in hand.
    Art(CoverArt),
    /// This source has nothing for this query.
    NotFound,
    /// The source could not answer (network, rate limit). Not remembered.
    Unavailable,
}

/// A place covers come from. Add a source by implementing this in a new file
/// under `cover/` and listing it in [`default_sources`].
pub trait CoverSource: Send + Sync {
    /// Short stable name, used in cache keys and logs.
    fn name(&self) -> &'static str;
    /// What this source's answer depends on, or `None` when it does not apply
    /// to this query. Queries with the same key share one remembered answer.
    fn key(&self, query: &StreamQuery) -> Option<String>;
    /// Look the cover up. Runs on the cover worker thread; may block.
    fn find(&self, query: &StreamQuery) -> Found;
    /// Whether answers may be remembered for the session. Sources with their
    /// own short-lived cache (AudioAddict) return `false`.
    fn remembers(&self) -> bool {
        true
    }
}

/// The outcome of [`StreamCovers::resolve`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolution {
    Found(CoverArt),
    /// Every applicable source answered and none had an image.
    Nothing,
    /// No image, and at least one source could not answer: keep what is shown.
    Unsure,
}

#[derive(Debug, Clone)]
enum Answer {
    Url(String),
    Art(CoverArt),
    Nothing,
}

type AnswerKey = (&'static str, String);

#[derive(Default)]
struct Answers {
    map: HashMap<AnswerKey, Answer>,
    order: VecDeque<AnswerKey>,
}

impl Answers {
    fn get(&self, key: &AnswerKey) -> Option<Answer> {
        self.map.get(key).cloned()
    }

    fn put(&mut self, key: AnswerKey, answer: Answer) {
        if self.map.insert(key.clone(), answer).is_none() {
            self.order.push_back(key);
        }
        while self.order.len() > ANSWER_CAP {
            if let Some(old) = self.order.pop_front() {
                self.map.remove(&old);
            }
        }
    }
}

/// Tries cover sources in order and remembers their answers.
pub struct StreamCovers {
    sources: Vec<Box<dyn CoverSource>>,
    answers: Mutex<Answers>,
    fetch: fn(&str) -> Option<CoverArt>,
}

impl StreamCovers {
    pub fn new(sources: Vec<Box<dyn CoverSource>>) -> Self {
        Self::with_fetcher(sources, crate::cover_fetch::fetch_cover)
    }

    /// Like [`StreamCovers::new`] with a different image downloader (tests).
    pub fn with_fetcher(
        sources: Vec<Box<dyn CoverSource>>,
        fetch: fn(&str) -> Option<CoverArt>,
    ) -> Self {
        Self {
            sources,
            answers: Mutex::default(),
            fetch,
        }
    }

    /// Try each applicable source in order; the first image that downloads wins.
    pub fn resolve(&self, query: &StreamQuery) -> Resolution {
        let mut unsure = false;
        for source in &self.sources {
            let Some(key) = source.key(query) else {
                continue;
            };
            let key = (source.name(), key);
            let answer = match self.remembered(source.as_ref(), &key) {
                Some(answer) => answer,
                None => match source.find(query) {
                    Found::Unavailable => {
                        tracing::debug!(source = source.name(), "cover source unavailable");
                        unsure = true;
                        continue;
                    }
                    Found::NotFound => self.remember(source.as_ref(), key.clone(), Answer::Nothing),
                    Found::Url(url) => self.remember(source.as_ref(), key.clone(), Answer::Url(url)),
                    Found::Art(art) => self.remember(source.as_ref(), key.clone(), Answer::Art(art)),
                },
            };
            match answer {
                Answer::Nothing => {}
                Answer::Art(art) => return Resolution::Found(art),
                Answer::Url(url) => match (self.fetch)(&url) {
                    Some(art) => return Resolution::Found(art),
                    None => {
                        tracing::debug!(source = source.name(), url, "cover download failed");
                        self.remember(source.as_ref(), key, Answer::Nothing);
                    }
                },
            }
        }
        if unsure {
            Resolution::Unsure
        } else {
            Resolution::Nothing
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Answers> {
        self.answers.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn remembered(&self, source: &dyn CoverSource, key: &AnswerKey) -> Option<Answer> {
        source.remembers().then(|| self.lock().get(key)).flatten()
    }

    fn remember(&self, source: &dyn CoverSource, key: AnswerKey, answer: Answer) -> Answer {
        if source.remembers() {
            self.lock().put(key, answer.clone());
        }
        answer
    }
}
```

`CoverArt` (in `metadata.rs`) already derives `Debug, Clone, PartialEq, Eq`. Check `cover_fetch` is reachable as `crate::cover_fetch::fetch_cover` (it is a private module of the crate; `pub fn fetch_cover` is fine).

- [x] **Step 4: Run the tests**

Run: `cargo test -p znicz-core --lib cover::tests`
Expected: 8 passed.

- [x] **Step 5: Commit**

```bash
git add znicz-core/src/cover/mod.rs
git commit -m "Add the cover source trait and the stream cover resolver"
```

---

### Task 3: Shared HTTP helpers and the simple sources

**Files:**
- Create: `znicz-core/src/cover/http.rs`, `icy_url.rs`, `station_art.rs`, `audioaddict.rs` (under `cover/`)
- Modify: `znicz-core/src/cover/mod.rs` (module lines)

- [x] **Step 1: HTTP helpers and test server**

`znicz-core/src/cover/http.rs`:

```rust
//! HTTP for cover sources: one agent, a JSON GET, and a test server.

use std::io::Read;
use std::time::Duration;

use serde_json::Value;

const JSON_MAX: u64 = 1024 * 1024;

pub(crate) const USER_AGENT: &str = concat!(
    "znicz/",
    env!("CARGO_PKG_VERSION"),
    " ( https://github.com/eugene-chekan/znicz )"
);

pub(crate) fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_connect(Some(Duration::from_secs(8)))
        .timeout_global(Some(Duration::from_secs(15)))
        .user_agent(USER_AGENT)
        .build()
        .into()
}

/// The outcome of a GET: a JSON body, an HTTP status the source may care
/// about, or a transport failure.
pub(crate) enum Fetched {
    Json(Value),
    Status(u16),
    Failed,
}

pub(crate) fn get_json(url: &str, query: &[(&str, &str)]) -> Fetched {
    let mut request = agent().get(url);
    for (name, value) in query {
        request = request.query(*name, *value);
    }
    let response = match request.call() {
        Ok(response) => response,
        Err(ureq::Error::StatusCode(code)) => return Fetched::Status(code),
        Err(e) => {
            tracing::debug!(url, error = %e, "cover source request failed");
            return Fetched::Failed;
        }
    };
    let mut body = Vec::new();
    let mut limited = response.into_body().into_reader().take(JSON_MAX + 1);
    if limited.read_to_end(&mut body).is_err() || body.len() as u64 > JSON_MAX {
        return Fetched::Failed;
    }
    match serde_json::from_slice(&body) {
        Ok(json) => Fetched::Json(json),
        Err(_) => Fetched::Failed,
    }
}

/// Status of a GET, body unread. `None` on a transport failure.
pub(crate) fn probe(url: &str) -> Option<u16> {
    match agent().get(url).call() {
        Ok(response) => Some(response.status().as_u16()),
        Err(ureq::Error::StatusCode(code)) => Some(code),
        Err(_) => None,
    }
}

#[cfg(test)]
pub(crate) mod test_server {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};
    use std::thread;

    /// Serves each request with `handler(request_line)`; returns the origin and
    /// the raw requests seen (request line plus headers).
    pub(crate) fn serve<F>(handler: F) -> (String, Arc<Mutex<Vec<String>>>)
    where
        F: Fn(&str) -> (u16, Vec<u8>) + Send + 'static,
    {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let seen = Arc::new(Mutex::new(Vec::new()));
        let log = seen.clone();
        thread::spawn(move || {
            for mut stream in listener.incoming().flatten() {
                let mut buf = [0u8; 8192];
                let n = stream.read(&mut buf).unwrap_or(0);
                let request = String::from_utf8_lossy(&buf[..n]).into_owned();
                let line = request.lines().next().unwrap_or("").to_string();
                log.lock().unwrap().push(request);
                let (status, body) = handler(&line);
                let reason = match status {
                    200 => "OK",
                    404 => "Not Found",
                    503 => "Service Unavailable",
                    _ => "Internal Server Error",
                };
                let header = format!(
                    "HTTP/1.1 {status} {reason}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(header.as_bytes());
                let _ = stream.write_all(&body);
            }
        });
        (origin, seen)
    }
}
```

Check against the `ureq` 3 API in the lockfile: `ureq::Error::StatusCode(u16)` exists and is returned for 4xx/5xx by default; `config_builder().user_agent(..)` and `.timeout_global(..)` exist; `RequestBuilder::query(name, value)` percent-encodes. If a name differs, adapt and note it in the commit message.

- [x] **Step 2: Write the failing tests for the three simple sources**

`znicz-core/src/cover/icy_url.rs`:

```rust
//! The station's own ICY `StreamUrl`, when it points at an image.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cover::{Found, StreamQuery};

    fn query(icy_url: Option<&str>) -> StreamQuery {
        StreamQuery {
            stream_url: "http://radio/stream".into(),
            station_name: "Radio".into(),
            icy_title: None,
            icy_url: icy_url.map(str::to_string),
            station_art: None,
            search: true,
        }
    }

    #[test]
    fn an_http_stream_url_is_the_cover() {
        let q = query(Some("https://img.example/now.jpg"));
        assert_eq!(IcyUrlSource.key(&q), Some("https://img.example/now.jpg".into()));
        assert_eq!(IcyUrlSource.find(&q), Found::Url("https://img.example/now.jpg".into()));
    }

    #[test]
    fn no_or_non_http_stream_url_does_not_apply() {
        assert_eq!(IcyUrlSource.key(&query(None)), None);
        assert_eq!(IcyUrlSource.key(&query(Some("www.example.com"))), None);
    }
}
```

`znicz-core/src/cover/station_art.rs`:

```rust
//! The station's local `art` image from `stations.toml`.

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
        let path = std::env::temp_dir().join(format!("znicz-art-{}.png", std::process::id()));
        std::fs::write(&path, [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]).unwrap();
        let q = query(Some(path.clone()));

        assert_eq!(StationArtSource.key(&q), Some(path.display().to_string()));
        match StationArtSource.find(&q) {
            Found::Art(art) => assert_eq!(art.mime, "image/png"),
            other => panic!("expected art, got {other:?}"),
        }
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_missing_file_is_not_found_and_no_art_does_not_apply() {
        let q = query(Some(PathBuf::from("/nowhere/art.png")));
        assert_eq!(StationArtSource.find(&q), Found::NotFound);
        assert_eq!(StationArtSource.key(&query(None)), None);
    }
}
```

`znicz-core/src/cover/audioaddict.rs`:

```rust
//! AudioAddict networks (RadioTunes, DI.FM, RockRadio, …): the channel's
//! current song cover from their API.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cover::StreamQuery;

    fn query(url: &str, title: Option<&str>) -> StreamQuery {
        StreamQuery {
            stream_url: url.into(),
            station_name: "Radio".into(),
            icy_title: title.map(str::to_string),
            icy_url: None,
            station_art: None,
            search: true,
        }
    }

    #[test]
    fn applies_to_audioaddict_streams_only() {
        let source = AudioAddictSource::new();
        let key = source
            .key(&query("http://prem2.radiotunes.com/datempolounge_hi", Some("A - B")))
            .expect("applies");
        assert!(key.contains("datempolounge"), "{key}");
        assert!(key.contains("A - B"), "a new title is a new key: {key}");
        assert_eq!(source.key(&query("http://stream.heavy-music.net:8176/stream", None)), None);
    }

    #[test]
    fn answers_are_not_remembered() {
        assert!(!AudioAddictSource::new().remembers());
    }

    #[test]
    fn lookups_map_onto_found() {
        use crate::audioaddict::AudioAddictLookup;
        assert_eq!(found(AudioAddictLookup::Found("http://x".into())), Found::Url("http://x".into()));
        assert_eq!(found(AudioAddictLookup::NoArt), Found::NotFound);
        assert_eq!(found(AudioAddictLookup::RefreshFailed), Found::Unavailable);
    }
}
```

In `cover/mod.rs`, under `pub mod title;`:

```rust
pub mod audioaddict;
pub(crate) mod http;
pub mod icy_url;
pub mod station_art;
```

- [x] **Step 3: Run to see them fail**

Run: `cargo test -p znicz-core --lib cover::`
Expected: compile errors for `IcyUrlSource`, `StationArtSource`, `AudioAddictSource`, `found`.

- [x] **Step 4: Implement**

`icy_url.rs`, above the tests:

```rust
use crate::cover::{CoverSource, Found, StreamQuery};
use crate::player::state::is_http_url;

pub struct IcyUrlSource;

impl CoverSource for IcyUrlSource {
    fn name(&self) -> &'static str {
        "icy_url"
    }

    fn key(&self, query: &StreamQuery) -> Option<String> {
        query
            .icy_url
            .as_deref()
            .map(str::trim)
            .filter(|url| is_http_url(url))
            .map(str::to_string)
    }

    fn find(&self, query: &StreamQuery) -> Found {
        match self.key(query) {
            Some(url) => Found::Url(url),
            None => Found::NotFound,
        }
    }
}
```

(`is_http_url` is `pub fn` in `player/state.rs`; if `player::state` is private to the crate path, use `crate::player::state::is_http_url` as written — it is inside the crate.)

`station_art.rs`, above the tests:

```rust
use crate::cover::{CoverSource, Found, StreamQuery};
use crate::metadata::{sniff_image_mime, CoverArt};

pub struct StationArtSource;

impl CoverSource for StationArtSource {
    fn name(&self) -> &'static str {
        "station_art"
    }

    fn key(&self, query: &StreamQuery) -> Option<String> {
        query.station_art.as_ref().map(|path| path.display().to_string())
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
```

`audioaddict.rs`, above the tests:

```rust
use crate::audioaddict::{audioaddict_cover_lookup, parse_audioaddict_channel, AudioAddictLookup};
use crate::cover::{CoverSource, Found, StreamQuery};

pub struct AudioAddictSource;

impl AudioAddictSource {
    pub fn new() -> Self {
        Self
    }
}

impl Default for AudioAddictSource {
    fn default() -> Self {
        Self::new()
    }
}

fn found(lookup: AudioAddictLookup) -> Found {
    match lookup {
        AudioAddictLookup::Found(url) => Found::Url(url),
        AudioAddictLookup::NoArt => Found::NotFound,
        AudioAddictLookup::RefreshFailed => Found::Unavailable,
    }
}

impl CoverSource for AudioAddictSource {
    fn name(&self) -> &'static str {
        "audioaddict"
    }

    fn key(&self, query: &StreamQuery) -> Option<String> {
        let (network, channel) = parse_audioaddict_channel(&query.stream_url)?;
        let title = query.icy_title.as_deref().unwrap_or("");
        Some(format!("{}:{channel}:{title}", network.slug()))
    }

    fn find(&self, query: &StreamQuery) -> Found {
        match parse_audioaddict_channel(&query.stream_url) {
            Some((network, channel)) => found(audioaddict_cover_lookup(network, &channel)),
            None => Found::NotFound,
        }
    }

    /// The AudioAddict module keeps its own 15 s channel map.
    fn remembers(&self) -> bool {
        false
    }
}
```

`sniff_image_mime` is `pub(crate)` in `metadata.rs` — reachable from the crate.

- [x] **Step 5: Run the tests**

Run: `cargo test -p znicz-core --lib cover::`
Expected: all cover tests pass (5 + 8 + 7 new).

- [x] **Step 6: Commit**

```bash
git add znicz-core/src/cover
git commit -m "Add ICY URL, station art, and AudioAddict cover sources"
```

---

### Task 4: Deezer source

**Files:**
- Create: `znicz-core/src/cover/deezer.rs`
- Modify: `znicz-core/src/cover/mod.rs` (`pub mod deezer;`)

- [x] **Step 1: Write the failing tests**

`znicz-core/src/cover/deezer.rs`:

```rust
//! Deezer search by artist and title. No key needed; covers up to 1000 px.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cover::http::test_server::serve;
    use crate::cover::StreamQuery;

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
        assert!(request.contains("Webster"), "{request}");
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
}
```

Add `pub mod deezer;` to `cover/mod.rs`.

- [x] **Step 2: Run to see them fail**

Run: `cargo test -p znicz-core --lib cover::deezer`
Expected: compile errors (`DeezerSource` not found).

- [x] **Step 3: Implement**

Above the tests:

```rust
use serde_json::Value;

use crate::cover::http::{get_json, Fetched};
use crate::cover::title::{artist_matches, fold, split_icy_title};
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

/// `(artist, title)` when search is allowed and the ICY title splits.
pub(crate) fn searchable(query: &StreamQuery) -> Option<(&str, &str)> {
    if !query.search {
        return None;
    }
    split_icy_title(query.icy_title.as_deref()?, &query.station_name)
}

/// `artist|title`, folded: the cache key shared by the search sources.
pub(crate) fn search_key(query: &StreamQuery) -> Option<String> {
    let (artist, title) = searchable(query)?;
    Some(format!("{}|{}", fold(artist), fold(title)))
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
        let q = format!("artist:\"{artist}\" track:\"{title}\"");
        let json = match get_json(&format!("{}/search", self.origin), &[("q", &q), ("limit", "5")]) {
            Fetched::Json(json) => json,
            Fetched::Status(_) | Fetched::Failed => return Found::Unavailable,
        };
        let hits = json.get("data").and_then(Value::as_array);
        hits.into_iter()
            .flatten()
            .filter(|hit| {
                hit.pointer("/artist/name")
                    .and_then(Value::as_str)
                    .is_some_and(|name| artist_matches(artist, name))
            })
            .find_map(cover_of)
            .map_or(Found::NotFound, Found::Url)
    }
}
```

- [x] **Step 4: Run the tests**

Run: `cargo test -p znicz-core --lib cover::deezer`
Expected: 5 passed.

- [x] **Step 5: Commit**

```bash
git add znicz-core/src/cover
git commit -m "Add the Deezer artist/title cover source"
```

---

### Task 5: MusicBrainz + Cover Art Archive source

**Files:**
- Create: `znicz-core/src/cover/musicbrainz.rs`
- Modify: `znicz-core/src/cover/mod.rs` (`pub mod musicbrainz;`)

- [x] **Step 1: Write the failing tests**

`znicz-core/src/cover/musicbrainz.rs`:

```rust
//! MusicBrainz recording search, then the release's front cover from the
//! Cover Art Archive. Finds niche artists Deezer does not know.

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
        assert_eq!(wrong.find(&query("Serotonal - Memory Leaves")), Found::NotFound);
        let low = servers(SEROTONAL, "low-score");
        assert_eq!(low.find(&query("Serotonal - Memory Leaves")), Found::NotFound);
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
        assert_eq!(source.find(&query("Serotonal - Memory Leaves")), Found::Unavailable);
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
```

Add `pub mod musicbrainz;` to `cover/mod.rs`.

- [x] **Step 2: Run to see them fail**

Run: `cargo test -p znicz-core --lib cover::musicbrainz`
Expected: compile errors (`MusicBrainzSource` not found).

- [x] **Step 3: Implement**

Above the tests:

```rust
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
            Fetched::Status(_) | Fetched::Failed => return Found::Unavailable,
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
```

`probe` follows redirects (ureq default), so a CAA 307 to archive.org reports the final 200.

- [x] **Step 4: Run the tests**

Run: `cargo test -p znicz-core --lib cover::musicbrainz`
Expected: 6 passed.

- [x] **Step 5: Commit**

```bash
git add znicz-core/src/cover
git commit -m "Add the MusicBrainz and Cover Art Archive cover source"
```

---

### Task 6: Default chain, exports, and `Station.cover_search`

**Files:**
- Modify: `znicz-core/src/cover/mod.rs`, `znicz-core/src/lib.rs`, `znicz-core/src/station.rs`
- Modify: every `Station { … }` literal (see Step 5)

- [x] **Step 1: Write the failing tests**

In `cover/mod.rs` tests:

```rust
    #[test]
    fn the_default_chain_runs_in_the_documented_order() {
        let names: Vec<&str> = default_sources().iter().map(|s| s.name()).collect();
        assert_eq!(
            names,
            ["icy_url", "audioaddict", "deezer", "musicbrainz", "station_art"]
        );
    }
```

In `station.rs` tests:

```rust
    #[test]
    fn cover_search_defaults_on_and_is_written_only_when_off() {
        let file: StationsFile =
            toml::from_str("[[station]]\nname = \"A\"\nurl = \"http://a\"\n").unwrap();
        assert!(file.station[0].cover_search);

        let mut off = file.station[0].clone();
        off.cover_search = false;
        let text = toml::to_string(&StationsFile { station: vec![off] }).unwrap();
        assert!(text.contains("cover_search = false"), "{text}");

        let on = toml::to_string(&StationsFile { station: file.station }).unwrap();
        assert!(!on.contains("cover_search"), "{on}");
    }
```

- [x] **Step 2: Run to see them fail**

Run: `cargo test -p znicz-core --lib default_chain cover_search_defaults`
Expected: compile errors (`default_sources`, `cover_search` not found).

- [x] **Step 3: Implement the chain and exports**

`cover/mod.rs`, after `StreamCovers`:

```rust
/// The built-in chain: the station's own image URL, the AudioAddict API,
/// artist / title search (Deezer, then MusicBrainz), the station's local art.
pub fn default_sources() -> Vec<Box<dyn CoverSource>> {
    vec![
        Box::new(icy_url::IcyUrlSource),
        Box::new(audioaddict::AudioAddictSource::new()),
        Box::new(deezer::DeezerSource::new()),
        Box::new(musicbrainz::MusicBrainzSource::new()),
        Box::new(station_art::StationArtSource),
    ]
}
```

`lib.rs`, next to the other `pub use` lines:

```rust
pub use cover::{default_sources, CoverSource, Found, Resolution, StreamCovers, StreamQuery};
```

- [x] **Step 4: Implement `cover_search`**

`station.rs`:

```rust
fn yes() -> bool {
    true
}

fn is_true(value: &bool) -> bool {
    *value
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Station {
    pub name: String,
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub art: Option<PathBuf>,
    /// Look the playing track's cover up by artist and title (Deezer,
    /// MusicBrainz). `false` keeps this station's titles private.
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    pub cover_search: bool,
}
```

- [x] **Step 5: Fix every `Station` literal**

Run: `cargo build --workspace --all-targets 2>&1 | grep -A3 "missing field \`cover_search\`"`

Each error points at a `Station { name, url, art }` literal (about 24, in `znicz-core/src/station.rs`, `znicz-core/tests/{queue,stream}.rs`, `znicz/src/main.rs`, `znicz-tui/tests/{keys,render}.rs`, `znicz-tui/examples/preview.rs`). Add `cover_search: true,` after the `art` field in each. Where a function copies a station (`station.rs` copy/rename helpers such as `last.art = src.art;`), also copy `cover_search`:

```rust
        last.cover_search = src.cover_search;
```

Re-run until the build is clean: `cargo build --workspace --all-targets`.

- [x] **Step 6: Run the tests**

Run: `cargo test -p znicz-core`
Expected: all pass.

- [x] **Step 7: Commit**

```bash
git add -A
git commit -m "Wire the default cover chain and add a per-station cover_search switch"
```

---

### Task 7: TUI cover cache on the chain

**Files:**
- Modify: `znicz-tui/src/cover.rs`

- [x] **Step 1: Write the failing tests**

In `cover.rs` tests, remove `pick_stream_cover_prefers_icy_then_audioaddict_then_station` and `a_failed_url_is_not_fetched_again` (and the `serve_counting_html` helper if nothing else uses it). Add:

```rust
    use znicz_core::{CoverArt, CoverSource, Found, StreamCovers, StreamQuery};

    const PNG_1X1: &[u8] = &[
        0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44,
        0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x02, 0x00, 0x00, 0x00, 0x90,
        0x77, 0x53, 0xDE, 0x00, 0x00, 0x00, 0x0C, 0x49, 0x44, 0x41, 0x54, 0x08, 0xD7, 0x63, 0xF8,
        0xCF, 0xC0, 0x00, 0x00, 0x00, 0x03, 0x00, 0x01, 0x67, 0xF0, 0xF7, 0xE7, 0x00, 0x00, 0x00,
        0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
    ];

    /// Always finds the 1×1 PNG and counts lookups.
    struct Counting(std::sync::Arc<std::sync::atomic::AtomicUsize>);

    impl CoverSource for Counting {
        fn name(&self) -> &'static str {
            "counting"
        }
        fn key(&self, query: &StreamQuery) -> Option<String> {
            Some(query.icy_title.clone().unwrap_or_default())
        }
        fn find(&self, _query: &StreamQuery) -> Found {
            self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Found::Art(CoverArt {
                mime: "image/png".into(),
                bytes: PNG_1X1.to_vec(),
            })
        }
        fn remembers(&self) -> bool {
            false
        }
    }

    fn stream_query(title: Option<&str>) -> StreamQuery {
        StreamQuery {
            stream_url: "http://radio/stream".into(),
            station_name: "Radio".into(),
            icy_title: title.map(str::to_string),
            icy_url: None,
            station_art: None,
            search: true,
        }
    }

    fn wait_ready(cache: &CoverCache, key: &CoverKey) -> CoverReady {
        let start = std::time::Instant::now();
        loop {
            let ready = cache.get(key.clone());
            if ready != CoverReady::Pending || start.elapsed() > Duration::from_secs(5) {
                return ready;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn a_stream_key_resolves_through_the_chain() {
        let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let cache = CoverCache::with_streams(StreamCovers::new(vec![Box::new(Counting(calls))]));
        let key = CoverKey::Stream(stream_query(Some("A - B")));
        assert!(matches!(wait_ready(&cache, &key), CoverReady::Embedded(_)));
    }

    #[test]
    fn only_streams_without_a_title_are_refreshed() {
        let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let cache = CoverCache::with_streams(StreamCovers::new(vec![Box::new(Counting(
            calls.clone(),
        ))]))
        .with_stream_refresh(Duration::from_millis(50));
        let titled = CoverKey::Stream(stream_query(Some("A - B")));
        let untitled = CoverKey::Stream(stream_query(None));
        wait_ready(&cache, &titled);
        wait_ready(&cache, &untitled);
        let after_first = calls.load(std::sync::atomic::Ordering::SeqCst);
        assert_eq!(after_first, 2);

        std::thread::sleep(Duration::from_millis(80));
        assert!(matches!(cache.get(titled.clone()), CoverReady::Embedded(_)));
        assert!(
            matches!(cache.get(untitled.clone()), CoverReady::Embedded(_)),
            "a refreshing entry keeps its image"
        );
        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 3);
    }
```

- [x] **Step 2: Run to see them fail**

Run: `cargo test -p znicz-tui --lib cover`
Expected: compile errors (`CoverKey::Stream`, `with_streams`, `with_stream_refresh` not found).

- [x] **Step 3: Implement**

In `znicz-tui/src/cover.rs`:

1. Replace `CoverKey`:

```rust
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum CoverKey {
    /// A local file's embedded picture.
    File(PathBuf),
    /// A radio stream, resolved through the cover source chain.
    Stream(znicz_core::StreamQuery),
}
```

2. Add a constant and fields:

```rust
/// How often a stream without ICY titles is looked up again (the AudioAddict
/// channel map lifetime), so its cover follows the playing track.
const STREAM_REFRESH: Duration = Duration::from_secs(15);
```

`Slot` gains `resolved_at: Instant`. `Map` drops the failed-URL `HashSet`:

```rust
type Map = Arc<Mutex<(HashMap<CoverKey, Slot>, VecDeque<CoverKey>)>>;

pub struct CoverCache {
    map: Map,
    requests: Sender<CoverKey>,
    logo: Arc<DynamicImage>,
    stream_refresh: Duration,
}
```

3. Constructors: `new()` calls `Self::with_streams(StreamCovers::new(znicz_core::default_sources()))`. `with_streams(streams: StreamCovers)` holds today's body of `new` (logo, map, worker thread), with the worker owning `streams`, and sets `stream_refresh: STREAM_REFRESH`. Add:

```rust
    /// Use another refresh interval for title-less streams (tests).
    pub fn with_stream_refresh(mut self, every: Duration) -> Self {
        self.stream_refresh = every;
        self
    }
```

4. The worker loop calls `resolve_cover(&key, &streams)`; on `Update` it inserts `Slot { ready, refreshing: false, resolved_at: Instant::now() }`; on `KeepPrevious` it clears `refreshing`, sets `resolved_at = Instant::now()`, and, if the slot is still `Pending`, sets it to `Logo`.

5. `get`:

```rust
    pub fn get(&self, key: CoverKey) -> CoverReady {
        let mut guard = self.map.lock().unwrap();
        let (cache, _) = &mut *guard;
        if let Some(slot) = cache.get_mut(&key) {
            let refresh = matches!(&key, CoverKey::Stream(query) if query.icy_title.is_none())
                && !slot.refreshing
                && slot.resolved_at.elapsed() >= self.stream_refresh;
            if refresh {
                slot.refreshing = true;
                let ready = slot.ready.clone();
                drop(guard);
                self.requests.send(key).ok();
                return ready;
            }
            return slot.ready.clone();
        }
        cache.insert(
            key.clone(),
            Slot {
                ready: CoverReady::Pending,
                refreshing: false,
                resolved_at: Instant::now(),
            },
        );
        drop(guard);
        self.requests.send(key).ok();
        CoverReady::Pending
    }
```

6. `resolve_cover`:

```rust
fn resolve_cover(key: &CoverKey, streams: &znicz_core::StreamCovers) -> ResolveResult {
    let ready = |bytes: &[u8]| match decode_capped(bytes) {
        Some(img) => CoverReady::Embedded(Arc::new(img)),
        None => CoverReady::Logo,
    };
    match key {
        CoverKey::File(path) => ResolveResult::Update {
            ready: znicz_core::read_cover(path).map_or(CoverReady::Logo, |art| ready(&art.bytes)),
        },
        CoverKey::Stream(query) => match streams.resolve(query) {
            znicz_core::Resolution::Found(art) => ResolveResult::Update {
                ready: ready(&art.bytes),
            },
            znicz_core::Resolution::Nothing => ResolveResult::Update {
                ready: CoverReady::Logo,
            },
            znicz_core::Resolution::Unsure => ResolveResult::KeepPrevious,
        },
    }
}
```

`ResolveResult::Update` loses its `failed_url` field.

7. Delete `pick_stream_cover`. Imports: add `std::time::{Duration, Instant}`, drop `HashSet`.

- [x] **Step 4: Run the tests**

Run: `cargo test -p znicz-tui --lib cover`
Expected: cover tests pass. `views/stage.rs` will not compile yet if the crate is built as a whole; if `cargo test --lib` fails on `stage.rs` only, continue to Task 8 before running, and run Step 4 there.

- [x] **Step 5: Commit (together with Task 8 if stage.rs blocks the build)**

```bash
git add znicz-tui/src/cover.rs
git commit -m "Resolve stream covers through the core cover chain"
```

---

### Task 8: Stage builds one query and keeps the previous cover

**Files:**
- Modify: `znicz-tui/src/views/stage.rs`, `znicz-tui/src/app.rs`, `znicz-tui/src/tui_config.rs`

- [x] **Step 1: Write the failing tests**

In `stage.rs` tests:

```rust
    fn stream_state(title: &str) -> PlayerState {
        let mut state = PlayerState::default();
        state.queue = vec![QueueItem::Stream {
            name: "HEAVY-MUSIC.NET".into(),
            url: "http://stream.heavy-music.net:8176/stream".into(),
        }];
        state.current_track = Some(TrackInfo {
            path: None,
            url: Some("http://stream.heavy-music.net:8176/stream".into()),
            icy_stream_url: None,
            title: title.into(),
            codec: "MP3".into(),
            sample_rate: 44_100,
            channels: 2,
            bits_per_sample: None,
            bitrate_kbps: None,
            duration: None,
            tags: Default::default(),
        });
        state
    }

    fn station(cover_search: bool) -> znicz_core::Station {
        znicz_core::Station {
            name: "HEAVY-MUSIC.NET".into(),
            url: "http://stream.heavy-music.net:8176/stream".into(),
            art: Some("/art/heavy.png".into()),
            cover_search,
        }
    }

    #[test]
    fn a_stream_query_carries_the_icy_title_station_and_switches() {
        let state = stream_state("Serotonal - Memory Leaves");
        let track = state.current_track.as_ref().unwrap();
        let q = stream_query(&state, track, &[station(true)], true);
        assert_eq!(q.station_name, "HEAVY-MUSIC.NET");
        assert_eq!(q.icy_title.as_deref(), Some("Serotonal - Memory Leaves"));
        assert_eq!(q.station_art.as_deref(), Some(std::path::Path::new("/art/heavy.png")));
        assert!(q.search);

        assert!(!stream_query(&state, track, &[station(false)], true).search);
        assert!(!stream_query(&state, track, &[station(true)], false).search);
    }

    #[test]
    fn a_title_that_is_the_station_name_is_no_icy_title() {
        let state = stream_state("HEAVY-MUSIC.NET");
        let track = state.current_track.as_ref().unwrap();
        assert_eq!(stream_query(&state, track, &[], true).icy_title, None);
    }

    #[test]
    fn the_previous_stream_cover_shows_while_the_next_is_pending() {
        let img = CoverReady::Embedded(Arc::new(image::DynamicImage::new_rgb8(1, 1)));
        let mut last = None;
        let url = "http://radio/stream";

        assert_eq!(keep_previous(&mut last, url, img.clone()), img);
        assert_eq!(keep_previous(&mut last, url, CoverReady::Pending), img);
        assert_eq!(
            keep_previous(&mut last, "http://other/stream", CoverReady::Pending),
            CoverReady::Pending,
            "a new station starts from the logo"
        );
        assert_eq!(keep_previous(&mut last, url, CoverReady::Logo), CoverReady::Logo);
    }
```

(Adjust the `TrackInfo` literal to the struct's current fields if they differ; check `znicz-core/src/player/state.rs`.)

- [x] **Step 2: Run to see them fail**

Run: `cargo test -p znicz-tui --lib stage`
Expected: compile errors (`stream_query`, `keep_previous` not found).

- [x] **Step 3: Config switch**

`tui_config.rs`: add to `TuiConfig`

```rust
    /// Look stream covers up by artist and title (Deezer, MusicBrainz).
    pub cover_search: bool,
```

and `cover_search: true` in `Default`. `znicz/src/main.rs`:

```rust
#[derive(Debug, Deserialize, Default)]
struct CoversSection {
    /// Artist / title cover search for streams. Default on.
    search: Option<bool>,
}
```

Add `covers: CoversSection,` to `Config`, and `cover_search: config.covers.search.unwrap_or(true),` in `tui_config`. Fix any other `TuiConfig { … }` literal the compiler reports.

- [x] **Step 4: Stage**

In `app.rs`, add to `App`:

```rust
    /// The last stream cover shown and its stream URL, drawn while the next
    /// title's cover is still loading.
    pub(crate) last_stream_cover: Option<(String, crate::cover::CoverReady)>,
```

initialised to `None`.

In `stage.rs`, replace the import `use crate::cover::{pick_stream_cover, CoverKey, CoverReady};` with `use crate::cover::{CoverKey, CoverReady};` and add:

```rust
/// The cover question for the playing stream.
pub(crate) fn stream_query(
    state: &PlayerState,
    track: &TrackInfo,
    stations: &[znicz_core::Station],
    search: bool,
) -> znicz_core::StreamQuery {
    let stream_url = track.url.clone().unwrap_or_default();
    let station_name = match state.queue.get(state.queue_position) {
        Some(QueueItem::Stream { name, .. }) => name.clone(),
        _ => String::new(),
    };
    let station = stations.iter().find(|s| s.url == stream_url);
    let icy_title = Some(track.title.clone()).filter(|title| *title != station_name);
    znicz_core::StreamQuery {
        icy_title,
        icy_url: track.icy_stream_url.clone(),
        station_art: station.and_then(|s| s.art.clone()),
        search: search && station.is_none_or(|s| s.cover_search),
        stream_url,
        station_name,
    }
}

/// Show the last stream cover while the same stream's next cover is pending.
pub(crate) fn keep_previous(
    last: &mut Option<(String, CoverReady)>,
    stream_url: &str,
    ready: CoverReady,
) -> CoverReady {
    match &ready {
        CoverReady::Embedded(_) => {
            *last = Some((stream_url.to_string(), ready.clone()));
            ready
        }
        CoverReady::Pending => match last {
            Some((url, previous)) if url == stream_url => previous.clone(),
            _ => ready,
        },
        CoverReady::Logo => {
            *last = None;
            ready
        }
    }
}
```

In `render_cover`, replace the whole `else { … pick_stream_cover(…) }` stream branch with:

```rust
            } else {
                let query = stream_query(state, track, &app.stations, app.tui.cover_search);
                let stream_url = query.stream_url.clone();
                let ready = app.covers.get(CoverKey::Stream(query));
                keep_previous(&mut app.last_stream_cover, &stream_url, ready)
            }
```

- [x] **Step 5: Run the tests**

Run: `cargo test -p znicz-tui`
Expected: all pass (including Task 7's cover tests).

- [x] **Step 6: Commit**

```bash
git add -A
git commit -m "Stage asks for one stream cover and keeps the last one while loading"
```

---

### Task 9: Docs, version, full check

**Files:**
- Modify: `Cargo.toml` (version), `README.md`, `wiki/Architecture/TUI.md`, `wiki/Domain/Formats-and-Metadata.md`, `wiki/Plans/Roadmap.md`
- Create: `wiki/Architecture/Cover-Sources.md`
- Modify: the plan's checkboxes

- [x] **Step 1: Version**

Root `Cargo.toml` `[workspace.package] version = "0.11.0"`.

- [x] **Step 2: README**

- Features list: replace the cover bullet's stream part with: stream covers from the ICY image, AudioAddict, an artist / title search (Deezer, then MusicBrainz and the Cover Art Archive), or the station's art, else the logo.
- Config section: document

```toml
[covers]
search = true   # look stream covers up by artist / title (Deezer, MusicBrainz)
```

and the per-station `cover_search = false` in `stations.toml`.
- One privacy sentence: with search on, the playing stream's ICY title is sent to Deezer and, when Deezer has no match, to MusicBrainz.
- Replace the Stage sentence "streams use an ICY image when one decodes, else an AudioAddict song cover on those networks, else station art, else the Znicz logo" with the new order.

- [x] **Step 3: Wiki**

- New `wiki/Architecture/Cover-Sources.md`: the chain and its order; `StreamQuery`, `Found`, `CoverSource` (with `remembers`), `Resolution`; how answers are remembered (256, session, `Unavailable` not kept, a failed download counts as not found); the 15 s refresh for title-less streams; the switches; MusicBrainz etiquette (User-Agent, one request per second). A section **Adding a source**: implement `CoverSource` in `znicz-core/src/cover/<name>.rs`, give it an `at(origin)` constructor for tests, test it against `cover::http::test_server::serve`, list it in `default_sources()` in the right place.
- `Architecture/TUI.md` cover paragraph: streams resolve through `CoverKey::Stream` and the chain; link the new page; the previous cover stays while the next loads.
- `Domain/Formats-and-Metadata.md`: in the radio section, the cover order and `cover_search`.
- `Plans/Roadmap.md`: under Phase 6, note that the MusicBrainz client for stream covers exists (`znicz-core/src/cover/musicbrainz.rs`) and can be reused.
- Link the new page from wherever the other Architecture pages are listed (check `wiki/Home.md` or the sidebar file).

- [x] **Step 4: Full check**

```bash
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Expected: clean; all tests pass.

- [x] **Step 5: Live smoke check (network)**

```bash
cargo run -p znicz -- station play "HEAVY-MUSIC.NET"
```

Then open the TUI (`cargo run -p znicz`), press `v`, and confirm a cover appears within a few seconds for both HEAVY-MUSIC.NET and FluxFM (`R` to pick a station). If a cover does not appear, run with `RUST_LOG=znicz_core::cover=debug` and read `~/.cache/znicz/znicz-session.log`. Report what was seen; do not block the PR on it if the network is unavailable.

- [x] **Step 6: Tick this plan's boxes and commit**

```bash
git add -A
git commit -m "Document stream cover sources; bump to 0.11.0"
```
