//! Icecast in-band metadata (`StreamTitle` every `icy-metaint` audio bytes).

use std::io::{self, Read};
use std::sync::{Arc, Mutex};

use crate::player::state::TrackInfo;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IcyTitle {
    Unset,
    Empty,
    Text(String),
}

impl IcyTitle {
    pub fn from_parsed(title: &str) -> Self {
        if title.is_empty() {
            Self::Empty
        } else {
            Self::Text(title.to_string())
        }
    }
}

/// First `StreamTitle='…';` in the block. `None` if that pattern is missing.
fn parse_icy_quoted(block: &[u8], prefix: &str) -> Option<String> {
    let text = String::from_utf8_lossy(block);
    let rest = text.split(prefix).nth(1)?;
    let (value, _) = rest.split_once("';")?;
    Some(value.to_string())
}

pub fn parse_stream_title(block: &[u8]) -> Option<String> {
    parse_icy_quoted(block, "StreamTitle='")
}

pub fn parse_stream_url(block: &[u8]) -> Option<String> {
    parse_icy_quoted(block, "StreamUrl='")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IcyUrl {
    Unset,
    Empty,
    Text(String),
}

impl IcyUrl {
    pub fn from_parsed(url: &str) -> Self {
        if url.is_empty() {
            Self::Empty
        } else {
            Self::Text(url.to_string())
        }
    }
}

pub fn apply_icy_url_to_track(track: &mut TrackInfo, icy: &IcyUrl) -> bool {
    match icy {
        IcyUrl::Unset => false,
        IcyUrl::Text(url) => {
            let changed = track.icy_stream_url.as_deref() != Some(url.as_str());
            if changed {
                track.icy_stream_url = Some(url.clone());
            }
            changed
        }
        IcyUrl::Empty => {
            let changed = track.icy_stream_url.is_some();
            if changed {
                track.icy_stream_url = None;
            }
            changed
        }
    }
}

/// Returns true when `track` was written.
pub fn apply_icy_to_track(track: &mut TrackInfo, icy: &IcyTitle, station_name: &str) -> bool {
    match icy {
        IcyTitle::Unset => false,
        IcyTitle::Text(song) => {
            let changed =
                track.title != *song || track.tags.title.as_deref() != Some(song.as_str());
            if changed {
                track.title = song.clone();
                track.tags.title = Some(song.clone());
            }
            changed
        }
        IcyTitle::Empty => {
            let changed = track.title != station_name || track.tags.title.is_some();
            if changed {
                track.title = station_name.to_string();
                track.tags.title = None;
            }
            changed
        }
    }
}

pub struct IcyStripRead<R> {
    inner: R,
    metaint: usize,
    audio_left: usize,
    title: Arc<Mutex<IcyTitle>>,
    url: Arc<Mutex<IcyUrl>>,
}

impl<R: Read> IcyStripRead<R> {
    pub fn new(
        inner: R,
        metaint: usize,
        title: Arc<Mutex<IcyTitle>>,
        url: Arc<Mutex<IcyUrl>>,
    ) -> Self {
        Self {
            inner,
            metaint,
            audio_left: metaint,
            title,
            url,
        }
    }

    fn skip_metadata(&mut self) -> io::Result<bool> {
        let mut len_buf = [0u8; 1];
        let n = self.inner.read(&mut len_buf)?;
        if n == 0 {
            return Ok(false);
        }
        let meta_len = usize::from(len_buf[0]) * 16;
        if meta_len == 0 {
            return Ok(true);
        }
        let mut meta = vec![0u8; meta_len];
        let mut got = 0;
        while got < meta_len {
            let n = self.inner.read(&mut meta[got..])?;
            if n == 0 {
                break;
            }
            got += n;
        }
        if got == meta_len {
            if let Some(parsed) = parse_stream_title(&meta) {
                *self.title.lock().unwrap() = IcyTitle::from_parsed(&parsed);
            }
            if let Some(parsed) = parse_stream_url(&meta) {
                *self.url.lock().unwrap() = IcyUrl::from_parsed(&parsed);
            }
        }
        Ok(true)
    }
}

impl<R: Read> Read for IcyStripRead<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        let mut written = 0;
        while written < buf.len() {
            if self.audio_left == 0 {
                if !self.skip_metadata()? {
                    break;
                }
                self.audio_left = self.metaint;
                continue;
            }
            let want = (buf.len() - written).min(self.audio_left);
            let n = self.inner.read(&mut buf[written..written + want])?;
            if n == 0 {
                break;
            }
            self.audio_left -= n;
            written += n;
        }
        Ok(written)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    #[test]
    fn parse_stream_title_reads_the_first_quoted_value() {
        assert_eq!(
            parse_stream_title(b"StreamTitle='Song';StreamUrl='http://x';"),
            Some("Song".into())
        );
        assert_eq!(
            parse_stream_title(b"StreamTitle='Artist - Track';"),
            Some("Artist - Track".into())
        );
        assert_eq!(parse_stream_title(b"StreamTitle='';"), Some("".into()));
        assert_eq!(parse_stream_title(b"StreamUrl='x';"), None);
        assert_eq!(parse_stream_title(b"StreamTitle='open"), None);
        assert_eq!(parse_stream_title(b"junk"), None);
    }

    #[test]
    fn parse_stream_url_reads_the_first_quoted_value_even_without_title() {
        assert_eq!(
            parse_stream_url(b"StreamTitle='Song';StreamUrl='http://x/cover.jpg';"),
            Some("http://x/cover.jpg".into())
        );
        assert_eq!(
            parse_stream_url(b"StreamUrl='https://cdn.example/art';"),
            Some("https://cdn.example/art".into())
        );
        assert_eq!(parse_stream_url(b"StreamUrl='';"), Some("".into()));
        assert_eq!(parse_stream_url(b"StreamTitle='Song';"), None);
        assert_eq!(parse_stream_url(b"StreamUrl='open"), None);
    }

    #[test]
    fn strip_read_keeps_audio_when_the_block_has_title_and_url() {
        let audio = b"ABCDEFGHIJKLMNOPQRSTUVWX".to_vec();
        let block = icy_block("StreamTitle='Song';StreamUrl='http://x/a.png';");
        let mut body = Vec::new();
        body.extend_from_slice(&audio[..16]);
        body.extend_from_slice(&block);
        body.extend_from_slice(&audio[16..]);
        let title = Arc::new(Mutex::new(IcyTitle::Unset));
        let url = Arc::new(Mutex::new(IcyUrl::Unset));
        let mut reader = IcyStripRead::new(std::io::Cursor::new(body), 16, title.clone(), url.clone());
        let mut out = Vec::new();
        reader.read_to_end(&mut out).unwrap();
        assert_eq!(out, audio);
        assert_eq!(*title.lock().unwrap(), IcyTitle::Text("Song".into()));
        assert_eq!(*url.lock().unwrap(), IcyUrl::Text("http://x/a.png".into()));
    }

    #[test]
    fn apply_icy_url_to_track_text_empty_and_unset() {
        let mut track = TrackInfo {
            path: None,
            url: Some("http://x".into()),
            icy_stream_url: None,
            title: "Station".into(),
            codec: "MP3".into(),
            sample_rate: 44100,
            channels: 2,
            bits_per_sample: None,
            bitrate_kbps: None,
            duration: None,
            tags: Default::default(),
        };
        assert!(!apply_icy_url_to_track(&mut track, &IcyUrl::Unset));
        assert!(track.icy_stream_url.is_none());

        assert!(apply_icy_url_to_track(
            &mut track,
            &IcyUrl::Text("https://a/b.png".into())
        ));
        assert_eq!(track.icy_stream_url.as_deref(), Some("https://a/b.png"));
        assert!(!apply_icy_url_to_track(
            &mut track,
            &IcyUrl::Text("https://a/b.png".into())
        ));

        assert!(apply_icy_url_to_track(&mut track, &IcyUrl::Empty));
        assert!(track.icy_stream_url.is_none());
    }

    fn icy_block(payload: &str) -> Vec<u8> {
        let mut bytes = payload.as_bytes().to_vec();
        let padded = bytes.len().div_ceil(16) * 16;
        bytes.resize(padded, 0);
        let mut out = vec![(padded / 16) as u8];
        out.extend(bytes);
        out
    }

    #[test]
    fn strip_read_drops_metadata_and_keeps_audio() {
        let audio = b"ABCDEFGHIJKLMNOPQRSTUVWX".to_vec();
        let block = icy_block("StreamTitle='Song';");
        let mut body = Vec::new();
        body.extend_from_slice(&audio[..16]);
        body.extend_from_slice(&block);
        body.extend_from_slice(&audio[16..]);
        let title = Arc::new(Mutex::new(IcyTitle::Unset));
        let url = Arc::new(Mutex::new(IcyUrl::Unset));
        let mut reader = IcyStripRead::new(std::io::Cursor::new(body), 16, title.clone(), url);
        let mut out = Vec::new();
        reader.read_to_end(&mut out).unwrap();
        assert_eq!(out, audio);
        assert_eq!(*title.lock().unwrap(), IcyTitle::Text("Song".into()));
    }

    #[test]
    fn empty_stream_title_is_empty_state() {
        let audio = b"0123456789abcdef".to_vec();
        let block = icy_block("StreamTitle='';");
        let mut body = audio.clone();
        body.extend_from_slice(&block);
        let title = Arc::new(Mutex::new(IcyTitle::Unset));
        let url = Arc::new(Mutex::new(IcyUrl::Unset));
        let mut reader = IcyStripRead::new(std::io::Cursor::new(body), 16, title.clone(), url);
        let mut out = Vec::new();
        reader.read_to_end(&mut out).unwrap();
        assert_eq!(out, audio);
        assert_eq!(*title.lock().unwrap(), IcyTitle::Empty);
    }

    #[test]
    fn junk_metadata_does_not_change_title() {
        let audio = b"0123456789abcdef".to_vec();
        let block = icy_block("not-stream-title");
        let mut body = audio.clone();
        body.extend_from_slice(&block);
        let title = Arc::new(Mutex::new(IcyTitle::Text("Keep".into())));
        let url = Arc::new(Mutex::new(IcyUrl::Unset));
        let mut reader = IcyStripRead::new(std::io::Cursor::new(body), 16, title.clone(), url);
        let mut out = Vec::new();
        reader.read_to_end(&mut out).unwrap();
        assert_eq!(out, audio);
        assert_eq!(*title.lock().unwrap(), IcyTitle::Text("Keep".into()));
    }

    #[test]
    fn apply_icy_to_track_text_empty_and_unset() {
        let mut track = TrackInfo {
            path: None,
            url: Some("http://x".into()),
            icy_stream_url: None,
            title: "Station".into(),
            codec: "MP3".into(),
            sample_rate: 44100,
            channels: 2,
            bits_per_sample: None,
            bitrate_kbps: None,
            duration: None,
            tags: Default::default(),
        };
        assert!(!apply_icy_to_track(&mut track, &IcyTitle::Unset, "Station"));
        assert_eq!(track.title, "Station");
        assert!(track.tags.title.is_none());

        assert!(apply_icy_to_track(
            &mut track,
            &IcyTitle::Text("Song".into()),
            "Station"
        ));
        assert_eq!(track.title, "Song");
        assert_eq!(track.tags.title.as_deref(), Some("Song"));
        assert!(!apply_icy_to_track(
            &mut track,
            &IcyTitle::Text("Song".into()),
            "Station"
        ));

        assert!(apply_icy_to_track(&mut track, &IcyTitle::Empty, "Station"));
        assert_eq!(track.title, "Station");
        assert!(track.tags.title.is_none());
    }
}
