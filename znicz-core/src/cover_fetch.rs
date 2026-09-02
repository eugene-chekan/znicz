use std::io::Read;
use std::time::Duration;

use crate::metadata::{sniff_image_mime, CoverArt};

const MAX_BYTES: u64 = 2 * 1024 * 1024;

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_connect(Some(Duration::from_secs(8)))
        .build()
        .into()
}

pub fn fetch_cover(url: &str) -> Option<CoverArt> {
    let lower = url.trim().to_ascii_lowercase();
    if !(lower.starts_with("http://") || lower.starts_with("https://")) {
        tracing::debug!(url, "cover fetch skipped (not http)");
        return None;
    }
    let response = match agent().get(url.trim()).call() {
        Ok(r) => r,
        Err(e) => {
            tracing::debug!(url, error = %e, "cover fetch failed");
            return None;
        }
    };
    if !response.status().is_success() {
        tracing::debug!(url, status = %response.status(), "cover fetch bad status");
        return None;
    }
    let header_mime = response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.split(';').next().unwrap_or(s).trim().to_string())
        .filter(|s| s.starts_with("image/"));
    let mut body = Vec::new();
    let mut limited = response.into_body().into_reader().take(MAX_BYTES + 1);
    if let Err(e) = limited.read_to_end(&mut body) {
        tracing::debug!(url, error = %e, "cover fetch body read failed");
        return None;
    }
    if body.is_empty() || body.len() as u64 > MAX_BYTES {
        tracing::debug!(url, bytes = body.len(), "cover fetch empty or oversize");
        return None;
    }
    let mime = header_mime.unwrap_or_else(|| sniff_image_mime(&body));
    Some(CoverArt { mime, bytes: body })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::mpsc;
    use std::thread;

    const TINY_PNG: &[u8] = &[
        0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44,
        0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x02, 0x00, 0x00, 0x00, 0x90,
        0x77, 0x53, 0xDE, 0x00, 0x00, 0x00, 0x0C, 0x49, 0x44, 0x41, 0x54, 0x08, 0xD7, 0x63, 0xF8,
        0xCF, 0xC0, 0x00, 0x00, 0x00, 0x03, 0x00, 0x01, 0x67, 0xF0, 0xF7, 0xE7, 0x00, 0x00, 0x00,
        0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
    ];

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

    fn serve_once_owned(body: Vec<u8>, content_type: &str) -> (String, mpsc::Receiver<String>) {
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
            let _ = stream.write_all(&body);
        });
        (format!("http://{addr}/stream"), rx)
    }

    #[test]
    fn file_url_is_none_and_does_not_need_a_server() {
        assert!(fetch_cover("file:///tmp/x.png").is_none());
        assert!(fetch_cover("not-a-url").is_none());
        assert!(fetch_cover("").is_none());
    }

    #[test]
    fn loopback_png_is_some() {
        let (url, _rx) = serve_once(TINY_PNG, "image/png");
        let art = fetch_cover(&url).expect("png");
        assert_eq!(art.mime, "image/png");
        assert_eq!(art.bytes, TINY_PNG);
    }

    #[test]
    fn html_body_still_returns_bytes_or_none_after_empty() {
        let (url, _rx) = serve_once(b"<html>nope</html>", "text/html");
        let art = fetch_cover(&url);
        match art {
            None => {}
            Some(a) => assert_eq!(a.mime, "application/octet-stream"),
        }
    }

    #[test]
    fn oversize_body_is_none() {
        let big = vec![0u8; (2 * 1024 * 1024) + 1];
        let (url, _rx) = serve_once_owned(big, "image/jpeg");
        assert!(fetch_cover(&url).is_none());
    }
}
