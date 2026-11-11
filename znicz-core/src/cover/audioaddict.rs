//! AudioAddict networks (RadioTunes, DI.FM, RockRadio, …): the channel's
//! current song cover from their API.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::audioaddict::{
    audioaddict_cover_lookup_at, parse_audioaddict_channel, AudioAddictLookup, DEFAULT_ORIGIN,
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
        let Some(key) = self.key(query) else {
            return Found::NotFound;
        };
        let Some((network, channel)) = parse_audioaddict_channel(&query.stream_url) else {
            return Found::NotFound;
        };
        let lookup =
            audioaddict_cover_lookup_at(network, &channel, query.icy_title.as_deref(), origin);
        match lookup {
            AudioAddictLookup::Found(url) => {
                if let Ok(mut waiting) = self.waiting.lock() {
                    waiting.remove(&key);
                }
                Found::Url(url)
            }
            AudioAddictLookup::NoArt => Found::NotFound,
            AudioAddictLookup::RefreshFailed => Found::Unavailable,
            AudioAddictLookup::NotYet => self.wait(key, give_up),
        }
    }

    /// `Soon` while the channel may still catch up; `NotFound` once it has
    /// been waiting on this title for `give_up`.
    fn wait(&self, key: String, give_up: Duration) -> Found {
        let Ok(mut waiting) = self.waiting.lock() else {
            return Found::NotFound;
        };
        let now = Instant::now();
        waiting.retain(|_, since| now.duration_since(*since) < WAITING_KEEP);
        let since = *waiting.entry(key).or_insert(now);
        if now.duration_since(since) >= give_up {
            Found::NotFound
        } else {
            Found::Soon
        }
    }
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
        let title = query.icy_title.as_deref().unwrap_or("");
        Some(format!("{}:{channel}:{title}", network.slug()))
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
    use std::thread;
    use std::time::Duration;

    use super::*;
    use crate::audioaddict::test_server::{serve_api, ART, LAGGING, NOW, TITLE, TRACK};

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
