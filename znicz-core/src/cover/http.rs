//! HTTP for cover sources: one agent, a JSON GET, and a test server.

use std::io::Read;
use std::time::Duration;

use serde_json::Value;

const JSON_MAX: u64 = 1024 * 1024;

pub(crate) const USER_AGENT: &str = concat!(
    "znicz/",
    env!("CARGO_PKG_VERSION"),
    " ( https://github.com/eugene-chekan/znicz )"
);

pub(crate) fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_connect(Some(Duration::from_secs(8)))
        .timeout_global(Some(Duration::from_secs(15)))
        .user_agent(USER_AGENT)
        .build()
        .into()
}

/// The outcome of a GET: a JSON body, an HTTP status the source may care
/// about, or a transport failure.
pub(crate) enum Fetched {
    Json(Value),
    Status(u16),
    Failed,
}

pub(crate) fn get_json(url: &str, query: &[(&str, &str)]) -> Fetched {
    let mut request = agent().get(url);
    for (name, value) in query {
        request = request.query(*name, *value);
    }
    let response = match request.call() {
        Ok(response) => response,
        Err(ureq::Error::StatusCode(code)) => return Fetched::Status(code),
        Err(e) => {
            tracing::debug!(url, error = %e, "cover source request failed");
            return Fetched::Failed;
        }
    };
    let mut body = Vec::new();
    let mut limited = response.into_body().into_reader().take(JSON_MAX + 1);
    if limited.read_to_end(&mut body).is_err() || body.len() as u64 > JSON_MAX {
        return Fetched::Failed;
    }
    match serde_json::from_slice(&body) {
        Ok(json) => Fetched::Json(json),
        Err(_) => Fetched::Failed,
    }
}

/// Rate limits and server trouble are worth retrying; other refusals are not.
pub(crate) fn retryable(status: u16) -> bool {
    status == 429 || status >= 500
}

/// Status of a GET, body unread. `None` on a transport failure.
pub(crate) fn probe(url: &str) -> Option<u16> {
    match agent().get(url).call() {
        Ok(response) => Some(response.status().as_u16()),
        Err(ureq::Error::StatusCode(code)) => Some(code),
        Err(_) => None,
    }
}

#[cfg(test)]
pub(crate) mod test_server {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};
    use std::thread;

    /// Serves each request with `handler(request_line)`; returns the origin and
    /// the raw requests seen (request line plus headers).
    pub(crate) fn serve<F>(handler: F) -> (String, Arc<Mutex<Vec<String>>>)
    where
        F: Fn(&str) -> (u16, Vec<u8>) + Send + 'static,
    {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let seen = Arc::new(Mutex::new(Vec::new()));
        let log = seen.clone();
        thread::spawn(move || {
            for mut stream in listener.incoming().flatten() {
                let mut buf = [0u8; 8192];
                let n = stream.read(&mut buf).unwrap_or(0);
                let request = String::from_utf8_lossy(&buf[..n]).into_owned();
                let line = request.lines().next().unwrap_or("").to_string();
                log.lock().unwrap().push(request);
                let (status, body) = handler(&line);
                let reason = match status {
                    200 => "OK",
                    403 => "Forbidden",
                    404 => "Not Found",
                    503 => "Service Unavailable",
                    _ => "Internal Server Error",
                };
                let header = format!(
                    "HTTP/1.1 {status} {reason}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(header.as_bytes());
                let _ = stream.write_all(&body);
            }
        });
        (origin, seen)
    }
}
