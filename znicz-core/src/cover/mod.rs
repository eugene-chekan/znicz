//! Cover art for radio streams: a chain of sources tried in order.

pub mod audioaddict;
pub(crate) mod http;
pub mod icy_url;
pub mod station_art;
pub mod title;

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
                    Found::Url(url) => {
                        self.remember(source.as_ref(), key.clone(), Answer::Url(url))
                    }
                    Found::Art(art) => {
                        self.remember(source.as_ref(), key.clone(), Answer::Art(art))
                    }
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
        self.answers
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
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

        assert_eq!(
            covers.resolve(&query(Some("A - B"))),
            Resolution::Found(art(1))
        );
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
        assert_eq!(
            covers.resolve(&query(Some("A - B"))),
            Resolution::Found(art(7))
        );
    }

    #[test]
    fn a_url_that_fails_falls_through_and_is_remembered() {
        let (broken, broken_calls) = Fake::new("broken", || Found::Url("http://img/404".into()));
        let (next, _) = Fake::new("next", || Found::Art(art(3)));
        let covers = covers(vec![Box::new(broken), Box::new(next)]);

        assert_eq!(
            covers.resolve(&query(Some("A - B"))),
            Resolution::Found(art(3))
        );
        assert_eq!(
            covers.resolve(&query(Some("A - B"))),
            Resolution::Found(art(3))
        );
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
        assert_eq!(
            calls.load(Ordering::SeqCst),
            before + 1,
            "the oldest answer was dropped"
        );
    }
}
