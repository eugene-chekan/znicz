//! Blocking HTTP for the Web API and the accounts service, and how a
//! response's status becomes a `SpotifyError`.

use std::time::Duration;

use serde_json::Value;

use crate::error::{Result, SpotifyError};

pub(crate) const API_BASE: &str = "https://api.spotify.com/v1";
pub(crate) const ACCOUNTS_BASE: &str = "https://accounts.spotify.com";

/// Without `Retry-After`, wait this long after a 429.
const DEFAULT_RETRY_AFTER_SECS: u64 = 5;

const USER_AGENT: &str = concat!(
    "znicz/",
    env!("CARGO_PKG_VERSION"),
    " ( https://github.com/eugene-chekan/znicz )"
);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Method {
    Get,
    Put,
    Post,
}

pub(crate) enum Body<'a> {
    None,
    Json(&'a Value),
    Form(&'a [(&'a str, &'a str)]),
}

/// A response read in full. Every status arrives here; `check` sorts them.
#[derive(Debug)]
pub(crate) struct Reply {
    pub status: u16,
    pub body: String,
    pub retry_after: Option<u64>,
}

pub(crate) struct Http {
    agent: ureq::Agent,
}

impl Http {
    pub(crate) fn new() -> Self {
        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(10)))
            .user_agent(USER_AGENT)
            .http_status_as_error(false)
            .build()
            .into();
        Self { agent }
    }

    pub(crate) fn send(
        &self,
        method: Method,
        url: &str,
        query: &[(&str, &str)],
        bearer: Option<&str>,
        body: Body<'_>,
    ) -> Result<Reply> {
        let auth = bearer.map(|token| format!("Bearer {token}"));
        let result = match method {
            Method::Get => {
                let mut request = self.agent.get(url).query_pairs(query.iter().copied());
                if let Some(auth) = &auth {
                    request = request.header("Authorization", auth);
                }
                request.call()
            }
            Method::Put | Method::Post => {
                let request = if method == Method::Put {
                    self.agent.put(url)
                } else {
                    self.agent.post(url)
                };
                let mut request = request.query_pairs(query.iter().copied());
                if let Some(auth) = &auth {
                    request = request.header("Authorization", auth);
                }
                match body {
                    Body::None => request.send(""),
                    Body::Json(value) => request
                        .header("Content-Type", "application/json")
                        .send(value.to_string()),
                    Body::Form(pairs) => request.send_form(pairs.iter().copied()),
                }
            }
        };
        let response = result.map_err(|e| SpotifyError::Network(e.to_string()))?;
        let status = response.status().as_u16();
        let retry_after = response
            .headers()
            .get("retry-after")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.trim().parse().ok());
        let body = response
            .into_body()
            .read_to_string()
            .map_err(|e| SpotifyError::Network(e.to_string()))?;
        Ok(Reply {
            status,
            body,
            retry_after,
        })
    }
}

/// Pass a 2xx through; turn anything else into the matching error.
pub(crate) fn check(reply: Reply) -> Result<Reply> {
    if (200..300).contains(&reply.status) {
        return Ok(reply);
    }
    let (message, reason) = error_detail(&reply.body);
    Err(match reply.status {
        401 => SpotifyError::Unauthorized,
        403 if reason == "PREMIUM_REQUIRED" || message.contains("Premium") => {
            SpotifyError::PremiumRequired
        }
        404 if reason == "NO_ACTIVE_DEVICE" || message.contains("No active device") => {
            SpotifyError::NoActiveDevice
        }
        429 => SpotifyError::RateLimited {
            retry_after_secs: reply.retry_after.unwrap_or(DEFAULT_RETRY_AFTER_SECS),
        },
        status => SpotifyError::Http { status, message },
    })
}

/// `{"error":{"message":…,"reason":…}}` → (message, reason).
fn error_detail(body: &str) -> (String, String) {
    let json: Value = serde_json::from_str(body).unwrap_or(Value::Null);
    let error = &json["error"];
    let message = error["message"].as_str().unwrap_or("").to_string();
    let reason = error["reason"].as_str().unwrap_or("").to_string();
    (message, reason)
}

#[cfg(test)]
pub(crate) mod test_server {
    //! A tiny HTTP/1.1 server: one thread, one request per connection.

    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::sync::{Arc, Mutex};
    use std::thread;

    #[derive(Debug, Clone)]
    pub(crate) struct Request {
        pub method: String,
        /// Path and query, as sent.
        pub target: String,
        pub headers: Vec<(String, String)>,
        pub body: String,
    }

    impl Request {
        pub(crate) fn path(&self) -> &str {
            self.target.split('?').next().unwrap_or("")
        }

        pub(crate) fn query(&self, key: &str) -> Option<String> {
            let query = self.target.split_once('?')?.1;
            query.split('&').find_map(|pair| {
                let (k, v) = pair.split_once('=')?;
                (k == key).then(|| {
                    percent_encoding::percent_decode_str(v)
                        .decode_utf8_lossy()
                        .into_owned()
                })
            })
        }

        pub(crate) fn header(&self, name: &str) -> Option<&str> {
            self.headers
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(name))
                .map(|(_, v)| v.as_str())
        }
    }

    pub(crate) struct Response {
        pub status: u16,
        pub headers: Vec<(String, String)>,
        pub body: String,
    }

    impl Response {
        pub(crate) fn json(status: u16, body: &str) -> Self {
            Self {
                status,
                headers: vec![("Content-Type".into(), "application/json".into())],
                body: body.into(),
            }
        }

        pub(crate) fn empty(status: u16) -> Self {
            Self {
                status,
                headers: vec![],
                body: String::new(),
            }
        }

        pub(crate) fn with_header(mut self, name: &str, value: &str) -> Self {
            self.headers.push((name.into(), value.into()));
            self
        }
    }

    pub(crate) type Seen = Arc<Mutex<Vec<Request>>>;

    /// Serves every request with `handler`; returns the origin
    /// (`http://127.0.0.1:port`) and the requests seen so far.
    pub(crate) fn serve<F>(handler: F) -> (String, Seen)
    where
        F: Fn(&Request) -> Response + Send + 'static,
    {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let seen: Seen = Arc::new(Mutex::new(Vec::new()));
        let log = seen.clone();
        thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                if let Some(request) = read_request(&stream) {
                    log.lock().unwrap().push(request.clone());
                    write_response(stream, handler(&request));
                }
            }
        });
        (origin, seen)
    }

    pub(crate) fn read_request(mut stream: &TcpStream) -> Option<Request> {
        let mut data = Vec::new();
        let mut buf = [0u8; 4096];
        let head_end = loop {
            let n = stream.read(&mut buf).ok()?;
            if n == 0 {
                return None;
            }
            data.extend_from_slice(&buf[..n]);
            if let Some(i) = data.windows(4).position(|w| w == b"\r\n\r\n") {
                break i + 4;
            }
        };
        let head = String::from_utf8_lossy(&data[..head_end]).into_owned();
        let mut lines = head.split("\r\n");
        let mut first = lines.next()?.split(' ');
        let method = first.next()?.to_string();
        let target = first.next()?.to_string();
        let headers: Vec<(String, String)> = lines
            .filter_map(|line| {
                let (k, v) = line.split_once(':')?;
                Some((k.trim().to_string(), v.trim().to_string()))
            })
            .collect();
        let length = headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case("content-length"))
            .and_then(|(_, v)| v.parse::<usize>().ok())
            .unwrap_or(0);
        while data.len() < head_end + length {
            let n = stream.read(&mut buf).ok()?;
            if n == 0 {
                break;
            }
            data.extend_from_slice(&buf[..n]);
        }
        let body = String::from_utf8_lossy(&data[head_end..]).into_owned();
        Some(Request {
            method,
            target,
            headers,
            body,
        })
    }

    fn write_response(mut stream: TcpStream, response: Response) {
        let mut head = format!(
            "HTTP/1.1 {} X\r\nContent-Length: {}\r\nConnection: close\r\n",
            response.status,
            response.body.len()
        );
        for (name, value) in &response.headers {
            head.push_str(&format!("{name}: {value}\r\n"));
        }
        head.push_str("\r\n");
        let _ = stream.write_all(head.as_bytes());
        let _ = stream.write_all(response.body.as_bytes());
    }
}

#[cfg(test)]
mod tests {
    use super::test_server::{Response, serve};
    use super::*;

    fn reply(status: u16, body: &str) -> Reply {
        Reply {
            status,
            body: body.into(),
            retry_after: None,
        }
    }

    #[test]
    fn success_passes_through() {
        assert_eq!(check(reply(200, "{}")).unwrap().body, "{}");
        assert_eq!(check(reply(204, "")).unwrap().status, 204);
    }

    #[test]
    fn statuses_become_errors() {
        assert_eq!(
            check(reply(401, "")).unwrap_err(),
            SpotifyError::Unauthorized
        );
        let premium = r#"{"error":{"status":403,"message":"Player command failed: Premium required","reason":"PREMIUM_REQUIRED"}}"#;
        assert_eq!(
            check(reply(403, premium)).unwrap_err(),
            SpotifyError::PremiumRequired
        );
        let device = r#"{"error":{"status":404,"message":"Player command failed: No active device found","reason":"NO_ACTIVE_DEVICE"}}"#;
        assert_eq!(
            check(reply(404, device)).unwrap_err(),
            SpotifyError::NoActiveDevice
        );
        let limited = Reply {
            retry_after: Some(7),
            ..reply(429, "")
        };
        assert_eq!(
            check(limited).unwrap_err(),
            SpotifyError::RateLimited {
                retry_after_secs: 7
            }
        );
        assert_eq!(
            check(reply(429, "")).unwrap_err(),
            SpotifyError::RateLimited {
                retry_after_secs: 5
            }
        );
        let other = r#"{"error":{"status":500,"message":"Server error"}}"#;
        assert_eq!(
            check(reply(500, other)).unwrap_err(),
            SpotifyError::Http {
                status: 500,
                message: "Server error".into()
            }
        );
    }

    #[test]
    fn sends_bearer_query_and_reads_the_body() {
        let (origin, seen) = serve(|_| Response::json(200, r#"{"ok":true}"#));
        let http = Http::new();
        let reply = http
            .send(
                Method::Get,
                &format!("{origin}/v1/me"),
                &[("limit", "5")],
                Some("tok"),
                Body::None,
            )
            .unwrap();
        assert_eq!(reply.status, 200);
        assert_eq!(reply.body, r#"{"ok":true}"#);
        let request = seen.lock().unwrap()[0].clone();
        assert_eq!(request.method, "GET");
        assert_eq!(request.path(), "/v1/me");
        assert_eq!(request.query("limit").as_deref(), Some("5"));
        assert_eq!(request.header("authorization"), Some("Bearer tok"));
    }

    #[test]
    fn sends_json_form_and_empty_bodies() {
        let (origin, seen) = serve(|_| Response::empty(204));
        let http = Http::new();
        let json = serde_json::json!({"device_ids": ["d1"]});
        http.send(
            Method::Put,
            &format!("{origin}/a"),
            &[],
            None,
            Body::Json(&json),
        )
        .unwrap();
        http.send(
            Method::Post,
            &format!("{origin}/b"),
            &[],
            None,
            Body::Form(&[("grant_type", "refresh_token")]),
        )
        .unwrap();
        http.send(Method::Put, &format!("{origin}/c"), &[], None, Body::None)
            .unwrap();
        let seen = seen.lock().unwrap();
        assert_eq!(seen[0].body, r#"{"device_ids":["d1"]}"#);
        assert_eq!(seen[0].header("content-type"), Some("application/json"));
        assert_eq!(seen[1].method, "POST");
        assert_eq!(seen[1].body, "grant_type=refresh_token");
        assert_eq!(seen[2].header("content-length"), Some("0"));
    }

    #[test]
    fn reads_retry_after() {
        let (origin, _) = serve(|_| Response::empty(429).with_header("Retry-After", "3"));
        let reply = Http::new()
            .send(Method::Get, &format!("{origin}/x"), &[], None, Body::None)
            .unwrap();
        assert_eq!(reply.status, 429);
        assert_eq!(reply.retry_after, Some(3));
    }

    #[test]
    fn a_closed_port_is_a_network_error() {
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let error = Http::new()
            .send(
                Method::Get,
                &format!("http://127.0.0.1:{port}/x"),
                &[],
                None,
                Body::None,
            )
            .unwrap_err();
        assert!(matches!(error, SpotifyError::Network(_)), "{error:?}");
    }
}
