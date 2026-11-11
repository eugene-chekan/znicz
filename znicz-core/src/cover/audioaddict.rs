//! AudioAddict networks (RadioTunes, DI.FM, RockRadio, …): the channel's
//! current song cover from their API.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::audioaddict::{
    AudioAddictLookup, AudioAddictNetwork, DEFAULT_ORIGIN, audioaddict_cover_lookup_at,
    parse_audioaddict_channel,
};
use crate::cover::{CoverSource, Found, StreamQuery};

/// How long a channel that is still on the previous song is waited for.
const GIVE_UP_AFTER: Duration = Duration::from_secs(30);
/// Waits older than this are forgotten when a new one starts.
const WAITING_KEEP: Duration = Duration::from_secs(300);

pub struct AudioAddictSource {
    /// Channel + title → when it was first seen not yet on the title.
    waiting: Mutex<HashMap<String, Instant>>,
}

impl AudioAddictSource {
    pub fn new() -> Self {
        Self {
            waiting: Mutex::new(HashMap::new()),
        }
    }

    fn find_at(&self, query: &StreamQuery, origin: &str, give_up: Duration) -> Found {
        let Some((network, channel)) = parse_audioaddict_channel(&query.stream_url) else {
            return Found::NotFound;
        };
        let title = query.icy_title.as_deref();
        let key = wait_key(network, &channel, title);
        let lookup = audioaddict_cover_lookup_at(network, &channel, title, origin);
        if lookup != AudioAddictLookup::NotYet {
            self.forget(&key);
        }
        match lookup {
            AudioAddictLookup::Found(url) => Found::Url(url),
            AudioAddictLookup::NoArt => Found::NotFound,
            AudioAddictLookup::RefreshFailed => Found::Unavailable,
            AudioAddictLookup::NotYet => self.wait(key, give_up),
        }
    }

    fn forget(&self, key: &str) {
        if let Ok(mut waiting) = self.waiting.lock() {
            waiting.remove(key);
        }
    }

    /// `Soon` while the channel may still catch up; `NotFound` once it has
    /// been waiting on this title for `give_up`, after which the wait is
    /// forgotten so the same title starts over if it is asked for again.
    fn wait(&self, key: String, give_up: Duration) -> Found {
        let Ok(mut waiting) = self.waiting.lock() else {
            return Found::NotFound;
        };
        let now = Instant::now();
        waiting.retain(|_, since| now.duration_since(*since) < WAITING_KEEP);
        let since = *waiting.entry(key.clone()).or_insert(now);
        if now.duration_since(since) >= give_up {
            waiting.remove(&key);
            Found::NotFound
        } else {
            Found::Soon
        }
    }
}

/// The waiting key of a channel on a title. `key()` and `find_at` both use it.
fn wait_key(network: AudioAddictNetwork, channel: &str, title: Option<&str>) -> String {
    format!("{}:{channel}:{}", network.slug(), title.unwrap_or(""))
}

impl Default for AudioAddictSource {
    fn default() -> Self {
        Self::new()
    }
}

impl CoverSource for AudioAddictSource {
    fn name(&self) -> &'static str {
        "audioaddict"
    }

    fn key(&self, query: &StreamQuery) -> Option<String> {
        let (network, channel) = parse_audioaddict_channel(&query.stream_url)?;
        Some(wait_key(network, &channel, query.icy_title.as_deref()))
    }

    fn find(&self, query: &StreamQuery) -> Found {
        self.find_at(query, DEFAULT_ORIGIN, GIVE_UP_AFTER)
    }

    /// The channel's current track changes, so answers are not remembered.
    fn remembers(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::thread;
    use std::time::Duration;

    use super::*;
    use crate::audioaddict::test_server::{ART, LAGGING, NOW, TITLE, TRACK, serve, serve_api};

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

    const STREAM: &str = "http://prem2.radiotunes.com/smoothbeats_hi";

    #[test]
    fn applies_to_audioaddict_streams_only() {
        let source = AudioAddictSource::new();
        let key = source
            .key(&query(
                "http://prem2.radiotunes.com/datempolounge_hi",
                Some("A - B"),
            ))
            .expect("applies");
        assert!(key.contains("datempolounge"), "{key}");
        assert!(key.contains("A - B"), "a new title is a new key: {key}");
        assert_eq!(
            source.key(&query("http://stream.heavy-music.net:8176/stream", None)),
            None
        );
    }

    #[test]
    fn answers_are_not_remembered() {
        assert!(!AudioAddictSource::new().remembers());
    }

    #[test]
    fn lagging_channel_asks_soon_then_gives_up() {
        let (origin, _) = serve_api(LAGGING, TRACK);
        let source = AudioAddictSource::new();
        let stream = query(STREAM, Some(TITLE));
        let give_up = Duration::from_millis(100);
        assert_eq!(source.find_at(&stream, &origin, give_up), Found::Soon);
        thread::sleep(Duration::from_millis(300));
        assert_eq!(source.find_at(&stream, &origin, give_up), Found::NotFound);
    }

    #[test]
    fn a_give_up_starts_over_when_the_same_title_is_asked_again() {
        let (origin, _) = serve_api(LAGGING, TRACK);
        let source = AudioAddictSource::new();
        let stream = query(STREAM, Some(TITLE));
        let give_up = Duration::from_millis(100);
        assert_eq!(source.find_at(&stream, &origin, give_up), Found::Soon);
        thread::sleep(Duration::from_millis(300));
        assert_eq!(source.find_at(&stream, &origin, give_up), Found::NotFound);
        assert_eq!(source.find_at(&stream, &origin, give_up), Found::Soon);
    }

    #[test]
    fn a_failed_lookup_forgets_the_wait() {
        let failing = Arc::new(AtomicBool::new(false));
        let flag = failing.clone();
        let list = format!("[{LAGGING}]");
        let (origin, _) = serve(move |path: &str| {
            if path.contains("/currently_playing/channel/") {
                if flag.load(Ordering::SeqCst) {
                    (500, b"{}".to_vec())
                } else {
                    (200, LAGGING.as_bytes().to_vec())
                }
            } else if path.contains("/currently_playing") {
                (200, list.as_bytes().to_vec())
            } else {
                (404, b"{}".to_vec())
            }
        });
        let source = AudioAddictSource::new();
        let stream = query(STREAM, Some(TITLE));
        let give_up = Duration::from_millis(100);
        assert_eq!(source.find_at(&stream, &origin, give_up), Found::Soon);
        failing.store(true, Ordering::SeqCst);
        assert_eq!(
            source.find_at(&stream, &origin, give_up),
            Found::Unavailable
        );
        failing.store(false, Ordering::SeqCst);
        // The wait started before the failure; it must not count toward give-up.
        thread::sleep(Duration::from_millis(300));
        assert_eq!(source.find_at(&stream, &origin, give_up), Found::Soon);
    }

    #[test]
    fn track_on_the_stream_title_is_found() {
        let (origin, _) = serve_api(NOW, TRACK);
        let source = AudioAddictSource::new();
        let stream = query(STREAM, Some(TITLE));
        assert_eq!(
            source.find_at(&stream, &origin, Duration::from_millis(100)),
            Found::Url(ART.into())
        );
    }
}
