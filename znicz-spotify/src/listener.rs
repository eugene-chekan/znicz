//! A one-shot HTTP listener on `127.0.0.1` for the login redirect. It hands
//! each `/callback` query to `on_query`; when that returns a page, the
//! listener answers with it and stops. It also stops when dropped.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

/// How often the accept loop checks whether it should stop.
const POLL: Duration = Duration::from_millis(100);

/// The previous listener on the same port may take one `POLL` to let go.
const BIND_RETRIES: u32 = 5;

pub(crate) struct CallbackListener {
    port: u16,
    stop: Arc<AtomicBool>,
}

impl CallbackListener {
    /// `port` 0 picks a free port (tests). `on_query` gets the query string
    /// of each `/callback` request and returns the page to show when the
    /// login is finished (successfully or not), or `None` to keep waiting.
    pub(crate) fn start<F>(port: u16, mut on_query: F) -> std::io::Result<Self>
    where
        F: FnMut(&str) -> Option<String> + Send + 'static,
    {
        let listener = bind(port)?;
        listener.set_nonblocking(true)?;
        let port = listener.local_addr()?.port();
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = stop.clone();
        thread::Builder::new()
            .name("spotify-login".into())
            .spawn(move || {
                while !stopped.load(Ordering::SeqCst) {
                    match listener.accept() {
                        Ok((stream, _)) => {
                            if handle(stream, &mut on_query) {
                                break;
                            }
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => thread::sleep(POLL),
                        Err(e) => {
                            tracing::debug!(error = %e, "login listener accept failed");
                            thread::sleep(POLL);
                        }
                    }
                }
            })?;
        Ok(Self { port, stop })
    }

    pub(crate) fn port(&self) -> u16 {
        self.port
    }
}

impl Drop for CallbackListener {
    /// Only signals the thread; it exits within `POLL`. Joining here could
    /// deadlock when the owner holds a lock that `on_query` needs.
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}

fn bind(port: u16) -> std::io::Result<TcpListener> {
    let mut attempt = 0;
    loop {
        match TcpListener::bind(("127.0.0.1", port)) {
            Ok(listener) => return Ok(listener),
            Err(e) if attempt < BIND_RETRIES && e.kind() == std::io::ErrorKind::AddrInUse => {
                attempt += 1;
                thread::sleep(POLL);
            }
            Err(e) => return Err(e),
        }
    }
}

/// Answer one request. `true` when the login is finished.
fn handle<F>(mut stream: TcpStream, on_query: &mut F) -> bool
where
    F: FnMut(&str) -> Option<String>,
{
    let _ = stream.set_nonblocking(false);
    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
    // Read the whole header block: closing with unread bytes would reset the
    // connection before the answer arrives.
    let mut buf = Vec::new();
    let mut chunk = [0u8; 1024];
    while !buf.windows(4).any(|w| w == b"\r\n\r\n") && buf.len() < 8192 {
        match stream.read(&mut chunk) {
            Ok(0) | Err(_) => break,
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
        }
    }
    let request = String::from_utf8_lossy(&buf);
    let target = request
        .lines()
        .next()
        .and_then(|line| line.split(' ').nth(1))
        .unwrap_or("");
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    if path != "/callback" {
        respond(&mut stream, 404, "Not found.");
        return false;
    }
    match on_query(query) {
        Some(page) => {
            respond(&mut stream, 200, &page);
            true
        }
        None => {
            respond(
                &mut stream,
                400,
                "This page is not from the current Znicz login.",
            );
            false
        }
    }
}

fn respond(stream: &mut TcpStream, status: u16, text: &str) {
    let body = format!(
        "<!doctype html><meta charset=\"utf-8\"><title>Znicz</title><p>{}</p>",
        escape(text)
    );
    let head = format!(
        "HTTP/1.1 {status} {}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        if status == 200 { "OK" } else { "Error" },
        body.len()
    );
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(body.as_bytes());
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    fn get(port: u16, target: &str) -> String {
        let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
        write!(stream, "GET {target} HTTP/1.1\r\nHost: x\r\n\r\n").unwrap();
        let mut reply = String::new();
        stream.read_to_string(&mut reply).unwrap();
        reply
    }

    #[test]
    fn hands_over_the_query_and_stops_when_done() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let log = seen.clone();
        let listener = CallbackListener::start(0, move |query| {
            log.lock().unwrap().push(query.to_string());
            if query.contains("state=right") {
                Some("Logged in. You can close this tab.".into())
            } else {
                None
            }
        })
        .unwrap();
        let port = listener.port();

        let wrong = get(port, "/callback?code=c&state=wrong");
        assert!(wrong.starts_with("HTTP/1.1 400"), "{wrong}");
        let other = get(port, "/favicon.ico");
        assert!(other.starts_with("HTTP/1.1 404"), "{other}");
        let done = get(port, "/callback?code=c&state=right");
        assert!(done.starts_with("HTTP/1.1 200"), "{done}");
        assert!(done.contains("You can close this tab."), "{done}");

        assert_eq!(
            *seen.lock().unwrap(),
            vec!["code=c&state=wrong", "code=c&state=right"]
        );
        // The listener has stopped: new connections are refused.
        thread::sleep(Duration::from_millis(300));
        assert!(TcpStream::connect(("127.0.0.1", port)).is_err());
    }

    #[test]
    fn stops_when_dropped() {
        let listener = CallbackListener::start(0, |_| None).unwrap();
        let port = listener.port();
        drop(listener);
        thread::sleep(Duration::from_millis(300));
        assert!(TcpStream::connect(("127.0.0.1", port)).is_err());
    }
}
