//! AudioAddict networks (RadioTunes, DI.FM, RockRadio, …): the channel's
//! current song cover from their API.

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
    fn lookups_map_onto_found() {
        use crate::audioaddict::AudioAddictLookup;
        assert_eq!(
            found(AudioAddictLookup::Found("http://x".into())),
            Found::Url("http://x".into())
        );
        assert_eq!(found(AudioAddictLookup::NoArt), Found::NotFound);
        assert_eq!(found(AudioAddictLookup::RefreshFailed), Found::Unavailable);
    }
}
