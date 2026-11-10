//! The station's own ICY `StreamUrl`, when it points at an image.

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
        assert_eq!(
            IcyUrlSource.key(&q),
            Some("https://img.example/now.jpg".into())
        );
        assert_eq!(
            IcyUrlSource.find(&q),
            Found::Url("https://img.example/now.jpg".into())
        );
    }

    #[test]
    fn no_or_non_http_stream_url_does_not_apply() {
        assert_eq!(IcyUrlSource.key(&query(None)), None);
        assert_eq!(IcyUrlSource.key(&query(Some("www.example.com"))), None);
    }
}
