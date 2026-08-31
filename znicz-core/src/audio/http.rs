use std::io::{self, Read};
use std::path::Path;
use std::sync::Mutex;
use std::time::Duration;

use symphonia::core::io::MediaSource;

use crate::audio::source::AudioSource;
use crate::error::{Result, ZniczError};
use crate::player::state::TrackInfo;

pub struct HttpStreamSource {
    name: String,
    url: String,
}

impl HttpStreamSource {
    pub fn new(name: impl Into<String>, url: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            url: url.into(),
        }
    }
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_connect(Some(Duration::from_secs(8)))
        .build()
        .into()
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
            .call()
            .map_err(|e| ZniczError::Player(format!("http: {e}")))?;
        if !response.status().is_success() {
            return Err(ZniczError::Player(format!(
                "http {}",
                response.status()
            )));
        }
        let reader = response.into_body().into_reader();
        Ok(Box::new(UnseekableRead(Mutex::new(Box::new(reader)))))
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
    use std::thread;

    fn serve_once(body: &'static [u8], content_type: &str) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let header = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buf = [0u8; 2048];
            let n = stream.read(&mut buf).unwrap();
            let req = String::from_utf8_lossy(&buf[..n]);
            assert!(!req.to_lowercase().contains("icy-metadata"));
            let _ = stream.write_all(header.as_bytes());
            let _ = stream.write_all(body);
        });
        format!("http://{addr}/stream")
    }

    #[test]
    fn open_reader_returns_the_http_body() {
        let url = serve_once(b"hello-stream", "application/octet-stream");
        let source = HttpStreamSource::new("Test", url);
        let mut reader = source.open_reader().unwrap();
        let mut buf = Vec::new();
        reader.read_to_end(&mut buf).unwrap();
        assert_eq!(buf, b"hello-stream");
        assert!(!reader.is_seekable());
    }

    #[test]
    fn a_non_audio_body_fails_to_decode() {
        let url = serve_once(b"<html>not audio</html>", "text/html");
        let source = HttpStreamSource::new("Bad", url);
        let err = match crate::audio::source::AudioDecoder::open(&source) {
            Err(e) => e,
            Ok(_) => panic!("Expected error"),
        };
        assert!(
            err.to_string().contains("decode") || err.to_string().contains("probe"),
            "{err}"
        );
    }
}
