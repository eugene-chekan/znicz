use std::io::{self, Read};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use symphonia::core::io::MediaSource;

use crate::audio::icy::{IcyStripRead, IcyTitle};
use crate::audio::source::AudioSource;
use crate::error::{Result, ZniczError};
use crate::player::state::TrackInfo;

pub struct HttpStreamSource {
    name: String,
    url: String,
    icy_title: Arc<Mutex<IcyTitle>>,
}

impl HttpStreamSource {
    pub fn new(name: impl Into<String>, url: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            url: url.into(),
            icy_title: Arc::new(Mutex::new(IcyTitle::Unset)),
        }
    }
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_connect(Some(Duration::from_secs(8)))
        .build()
        .into()
}

fn icy_metaint(headers: &ureq::http::HeaderMap) -> Option<usize> {
    let value = headers.get("icy-metaint")?;
    let n: usize = value.to_str().ok()?.trim().parse().ok()?;
    (n > 0).then_some(n)
}

impl AudioSource for HttpStreamSource {
    fn path(&self) -> Option<&Path> {
        None
    }

    fn url(&self) -> Option<&str> {
        Some(&self.url)
    }

    fn title_hint(&self) -> &str {
        &self.name
    }

    fn read_info(&self) -> Result<TrackInfo> {
        Ok(TrackInfo {
            path: None,
            url: Some(self.url.clone()),
            title: self.name.clone(),
            codec: "Audio".into(),
            sample_rate: 0,
            channels: 0,
            bits_per_sample: None,
            bitrate_kbps: None,
            duration: None,
            tags: Default::default(),
        })
    }

    fn open_reader(&self) -> Result<Box<dyn MediaSource>> {
        let response = agent()
            .get(&self.url)
            .header("Icy-MetaData", "1")
            .call()
            .map_err(|e| ZniczError::Player(format!("http: {e}")))?;
        if !response.status().is_success() {
            return Err(ZniczError::Player(format!("http {}", response.status())));
        }
        let metaint = icy_metaint(response.headers());
        let reader = response.into_body().into_reader();
        let boxed: Box<dyn Read + Send> = match metaint {
            Some(n) => Box::new(IcyStripRead::new(reader, n, self.icy_title.clone())),
            None => Box::new(reader),
        };
        Ok(Box::new(UnseekableRead(Mutex::new(boxed))))
    }

    fn icy_title_slot(&self) -> Option<Arc<Mutex<IcyTitle>>> {
        Some(self.icy_title.clone())
    }
}

struct UnseekableRead(Mutex<Box<dyn Read + Send>>);

impl Read for UnseekableRead {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.0.lock().unwrap().read(buf)
    }
}

impl std::io::Seek for UnseekableRead {
    fn seek(&mut self, _pos: std::io::SeekFrom) -> io::Result<u64> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "stream is not seekable",
        ))
    }
}

impl MediaSource for UnseekableRead {
    fn is_seekable(&self) -> bool {
        false
    }

    fn byte_len(&self) -> Option<u64> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::mpsc;
    use std::thread;

    fn serve_once(body: &'static [u8], content_type: &str) -> (String, mpsc::Receiver<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let header = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buf = [0u8; 2048];
            let n = stream.read(&mut buf).unwrap();
            let req = String::from_utf8_lossy(&buf[..n]).into_owned();
            let _ = tx.send(req);
            let _ = stream.write_all(header.as_bytes());
            let _ = stream.write_all(body);
        });
        (format!("http://{addr}/stream"), rx)
    }

    #[test]
    fn open_reader_returns_the_http_body() {
        let (url, rx) = serve_once(b"hello-stream", "application/octet-stream");
        let source = HttpStreamSource::new("Test", url);
        let mut reader = source.open_reader().unwrap();
        let mut buf = Vec::new();
        reader.read_to_end(&mut buf).unwrap();
        assert_eq!(buf, b"hello-stream");
        assert!(!reader.is_seekable());
        let req = rx.recv().unwrap();
        assert!(req.to_lowercase().contains("icy-metadata"));
    }

    #[test]
    fn a_non_audio_body_fails_to_decode() {
        let (url, rx) = serve_once(b"<html>not audio</html>", "text/html");
        let source = HttpStreamSource::new("Bad", url);
        let err = match crate::audio::source::AudioDecoder::open(&source) {
            Err(e) => e,
            Ok(_) => panic!("Expected error"),
        };
        assert!(
            err.to_string().contains("decode") || err.to_string().contains("probe"),
            "{err}"
        );
        let req = rx.recv().unwrap();
        assert!(req.to_lowercase().contains("icy-metadata"));
    }

    fn serve_once_icy(
        body: Vec<u8>,
        content_type: &str,
        metaint: usize,
    ) -> (String, mpsc::Receiver<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let header = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nicy-metaint: {metaint}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buf = [0u8; 2048];
            let n = stream.read(&mut buf).unwrap();
            let req = String::from_utf8_lossy(&buf[..n]).into_owned();
            let _ = tx.send(req);
            let _ = stream.write_all(header.as_bytes());
            let _ = stream.write_all(&body);
        });
        (format!("http://{addr}/stream"), rx)
    }

    fn icy_block(payload: &str) -> Vec<u8> {
        let mut bytes = payload.as_bytes().to_vec();
        let padded = bytes.len().div_ceil(16) * 16;
        bytes.resize(padded, 0);
        let mut out = vec![(padded / 16) as u8];
        out.extend(bytes);
        out
    }

    fn silent_wav_bytes(sample_rate: u32, channels: u16, frames: u32) -> Vec<u8> {
        let bytes_per_frame = u32::from(channels) * 2;
        let data_size = frames * bytes_per_frame;
        let file_size = 36 + data_size;
        let mut buf = Vec::new();
        buf.extend_from_slice(b"RIFF");
        buf.extend_from_slice(&file_size.to_le_bytes());
        buf.extend_from_slice(b"WAVE");
        buf.extend_from_slice(b"fmt ");
        buf.extend_from_slice(&16u32.to_le_bytes());
        buf.extend_from_slice(&1u16.to_le_bytes());
        buf.extend_from_slice(&channels.to_le_bytes());
        buf.extend_from_slice(&sample_rate.to_le_bytes());
        buf.extend_from_slice(&(sample_rate * bytes_per_frame).to_le_bytes());
        buf.extend_from_slice(&(bytes_per_frame as u16).to_le_bytes());
        buf.extend_from_slice(&16u16.to_le_bytes());
        buf.extend_from_slice(b"data");
        buf.extend_from_slice(&data_size.to_le_bytes());
        buf.extend(vec![0u8; data_size as usize]);
        buf
    }

    #[test]
    fn open_reader_strips_icy_metadata_from_the_body() {
        use crate::audio::icy::IcyTitle;
        let audio = b"ABCDEFGHIJKLMNOPQRSTUVWX";
        let mut payload = audio[..16].to_vec();
        payload.extend_from_slice(&icy_block("StreamTitle='Hi';"));
        payload.extend_from_slice(&audio[16..]);
        let (url, rx) = serve_once_icy(payload, "application/octet-stream", 16);
        let source = HttpStreamSource::new("Test", url);
        let mut reader = source.open_reader().unwrap();
        let mut buf = Vec::new();
        reader.read_to_end(&mut buf).unwrap();
        assert_eq!(buf, audio);
        assert_eq!(
            source.icy_title_slot().unwrap().lock().unwrap().clone(),
            IcyTitle::Text("Hi".into())
        );
        let req = rx.recv().unwrap();
        assert!(req.to_lowercase().contains("icy-metadata"));
    }

    #[test]
    fn no_metaint_keeps_the_body_and_unset_title() {
        use crate::audio::icy::IcyTitle;
        let (url, rx) = serve_once(b"hello-stream", "application/octet-stream");
        let source = HttpStreamSource::new("Test", url);
        let mut reader = source.open_reader().unwrap();
        let mut buf = Vec::new();
        reader.read_to_end(&mut buf).unwrap();
        assert_eq!(buf, b"hello-stream");
        assert_eq!(
            source.icy_title_slot().unwrap().lock().unwrap().clone(),
            IcyTitle::Unset
        );
        assert!(rx.recv().unwrap().to_lowercase().contains("icy-metadata"));
    }

    #[test]
    fn decoder_sees_stream_title_from_icy_blocks() {
        use crate::audio::icy::IcyTitle;
        use crate::audio::source::AudioDecoder;
        let wav = silent_wav_bytes(44_100, 2, 256);
        let mut body = wav[..44].to_vec();
        body.extend_from_slice(&icy_block("StreamTitle='Song';"));
        body.extend_from_slice(&wav[44..]);
        let (url, _rx) = serve_once_icy(body, "audio/wav", 44);
        let source = HttpStreamSource::new("Station", url);
        let (mut decoder, info) = AudioDecoder::open(&source).unwrap();
        assert_eq!(info.title, "Station");
        let _ = decoder.decode_next();
        assert_eq!(decoder.icy_title(), IcyTitle::Text("Song".into()));
    }
}
