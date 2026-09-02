use std::collections::HashMap;
use std::io::Read;
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AudioAddictNetwork {
    RadioTunes,
    Di,
    RockRadio,
    JazzRadio,
    ClassicalRadio,
    ZenRadio,
}

impl AudioAddictNetwork {
    pub fn slug(self) -> &'static str {
        match self {
            Self::RadioTunes => "radiotunes",
            Self::Di => "di",
            Self::RockRadio => "rockradio",
            Self::JazzRadio => "jazzradio",
            Self::ClassicalRadio => "classicalradio",
            Self::ZenRadio => "zenradio",
        }
    }

    fn from_host(host: &str) -> Option<Self> {
        const PAIRS: &[(&str, AudioAddictNetwork)] = &[
            ("radiotunes.com", AudioAddictNetwork::RadioTunes),
            ("di.fm", AudioAddictNetwork::Di),
            ("rockradio.com", AudioAddictNetwork::RockRadio),
            ("jazzradio.com", AudioAddictNetwork::JazzRadio),
            ("classicalradio.com", AudioAddictNetwork::ClassicalRadio),
            ("zenradio.com", AudioAddictNetwork::ZenRadio),
        ];
        let host = host.trim().trim_end_matches('.').to_ascii_lowercase();
        for (domain, net) in PAIRS {
            if host == *domain || host.ends_with(&format!(".{domain}")) {
                return Some(*net);
            }
        }
        None
    }
}

const QUALITY_SUFFIXES: &[&str] = &["_aacplus", "_aac", "_premium", "_hi", "_med", "_low"];

pub fn parse_audioaddict_channel(stream_url: &str) -> Option<(AudioAddictNetwork, String)> {
    let url = stream_url.trim();
    let rest = url
        .strip_prefix("http://")
        .or_else(|| url.strip_prefix("https://"))?;
    let authority_path = rest.split_once('?').map(|(a, _)| a).unwrap_or(rest);
    let (authority, path) = match authority_path.split_once('/') {
        Some((a, p)) => (a, p),
        None => (authority_path, ""),
    };
    let host = authority
        .rsplit_once('@')
        .map(|(_, h)| h)
        .unwrap_or(authority);
    let host = host.rsplit_once(':').map(|(h, _)| h).unwrap_or(host);
    if host.starts_with('[') {
        return None;
    }
    let network = AudioAddictNetwork::from_host(host)?;
    let segment = path.split('/').next().unwrap_or("");
    if segment.is_empty() {
        return None;
    }
    let mut key = segment.to_string();
    for suffix in QUALITY_SUFFIXES {
        if let Some(stripped) = key.strip_suffix(suffix) {
            if !stripped.is_empty() {
                key = stripped.to_string();
                break;
            }
        }
    }
    Some((network, key))
}

const DEFAULT_ORIGIN: &str = "https://api.audioaddict.com";
const JSON_MAX: u64 = 1024 * 1024;
const CACHE_TTL: Duration = Duration::from_secs(15);

type ChannelCoverMap = HashMap<String, Option<String>>;
type CacheEntry = (Instant, ChannelCoverMap);
type CacheStore = HashMap<(String, AudioAddictNetwork), CacheEntry>;

static CACHE: LazyLock<Mutex<CacheStore>> = LazyLock::new(|| Mutex::new(HashMap::new()));

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_connect(Some(Duration::from_secs(8)))
        .build()
        .into()
}

fn normalize_art_url(raw: &str) -> Option<String> {
    let mut url = raw.trim().to_string();
    if url.is_empty() {
        return None;
    }
    if url.starts_with("//") {
        url = format!("https:{url}");
    }
    if let Some(idx) = url.rfind("{?") {
        if url[idx..].contains('}') {
            url.truncate(idx);
        }
    }
    let lower = url.to_ascii_lowercase();
    if lower.starts_with("http://") || lower.starts_with("https://") {
        Some(url)
    } else {
        None
    }
}

fn channel_id_string(value: &Value) -> Option<String> {
    match value {
        Value::Number(n) => Some(n.to_string()),
        Value::String(s) => Some(s.clone()),
        _ => None,
    }
}

#[cfg(test)]
pub fn join_audioaddict_art_url(
    channel_key: &str,
    currently_playing: &str,
    track_history: &str,
) -> Option<String> {
    let playing: Vec<Value> = serde_json::from_str(currently_playing).ok()?;
    let entry = playing
        .iter()
        .find(|v| v.get("channel_key").and_then(Value::as_str) == Some(channel_key))?;
    let channel_id = channel_id_string(entry.get("channel_id")?)?;
    let history: HashMap<String, Value> = serde_json::from_str(track_history).ok()?;
    let art_url = history.get(&channel_id)?.get("art_url")?.as_str()?;
    normalize_art_url(art_url)
}

fn fetch_json(origin: &str, path: &str) -> Option<String> {
    let url = format!("{origin}{path}");
    let response = agent().get(&url).call().ok()?;
    if !response.status().is_success() {
        return None;
    }
    let mut body = Vec::new();
    let mut limited = response.into_body().into_reader().take(JSON_MAX + 1);
    limited.read_to_end(&mut body).ok()?;
    if body.is_empty() || body.len() as u64 > JSON_MAX {
        return None;
    }
    String::from_utf8(body).ok()
}

fn rebuild_channel_map(currently_playing: &str, track_history: &str) -> Option<ChannelCoverMap> {
    let playing: Vec<Value> = serde_json::from_str(currently_playing).ok()?;
    let history: HashMap<String, Value> = serde_json::from_str(track_history).ok()?;
    let mut map = ChannelCoverMap::new();
    for entry in playing {
        let Some(channel_key) = entry.get("channel_key").and_then(Value::as_str) else {
            continue;
        };
        let Some(channel_id) = entry.get("channel_id").and_then(channel_id_string) else {
            continue;
        };
        let art = history
            .get(&channel_id)
            .and_then(|v| v.get("art_url"))
            .and_then(|v| v.as_str())
            .and_then(normalize_art_url);
        map.insert(channel_key.to_string(), art);
    }
    Some(map)
}

fn refresh_cache(network: AudioAddictNetwork, origin: &str) -> Option<ChannelCoverMap> {
    let slug = network.slug();
    let playing = fetch_json(origin, &format!("/v1/{slug}/currently_playing"))?;
    let history = fetch_json(origin, &format!("/v1/{slug}/track_history"))?;
    rebuild_channel_map(&playing, &history)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AudioAddictLookup {
    Found(String),
    NoArt,
    RefreshFailed,
}

pub fn audioaddict_cover_url(network: AudioAddictNetwork, channel_key: &str) -> Option<String> {
    match audioaddict_cover_lookup(network, channel_key) {
        AudioAddictLookup::Found(url) => Some(url),
        AudioAddictLookup::NoArt | AudioAddictLookup::RefreshFailed => None,
    }
}

pub fn audioaddict_cover_lookup(
    network: AudioAddictNetwork,
    channel_key: &str,
) -> AudioAddictLookup {
    audioaddict_cover_lookup_at(network, channel_key, DEFAULT_ORIGIN)
}

#[cfg_attr(not(test), allow(dead_code))]
pub fn audioaddict_cover_url_at(
    network: AudioAddictNetwork,
    channel_key: &str,
    origin: &str,
) -> Option<String> {
    match audioaddict_cover_lookup_at(network, channel_key, origin) {
        AudioAddictLookup::Found(url) => Some(url),
        AudioAddictLookup::NoArt | AudioAddictLookup::RefreshFailed => None,
    }
}

fn lookup_channel(map: &ChannelCoverMap, channel_key: &str) -> AudioAddictLookup {
    match map.get(channel_key) {
        Some(Some(url)) => AudioAddictLookup::Found(url.clone()),
        Some(None) | None => AudioAddictLookup::NoArt,
    }
}

pub fn audioaddict_cover_lookup_at(
    network: AudioAddictNetwork,
    channel_key: &str,
    origin: &str,
) -> AudioAddictLookup {
    let origin = origin.trim_end_matches('/');
    let cache_key = (origin.to_string(), network);
    let needs_refresh = {
        let Ok(cache) = CACHE.lock() else {
            return AudioAddictLookup::RefreshFailed;
        };
        let now = Instant::now();
        cache
            .get(&cache_key)
            .is_none_or(|(fetched_at, _)| now.duration_since(*fetched_at) >= CACHE_TTL)
    };
    if needs_refresh {
        let refresh_result = refresh_cache(network, origin);
        let now = Instant::now();
        let Ok(mut cache) = CACHE.lock() else {
            return AudioAddictLookup::RefreshFailed;
        };
        match refresh_result {
            Some(map) => {
                cache.insert(cache_key.clone(), (now, map));
            }
            None => {
                tracing::debug!(
                    network = network.slug(),
                    channel_key,
                    "audioaddict cover refresh failed"
                );
                match cache.get_mut(&cache_key) {
                    Some((fetched_at, _)) => *fetched_at = now,
                    None => {
                        cache.insert(cache_key.clone(), (now, ChannelCoverMap::new()));
                    }
                }
                return AudioAddictLookup::RefreshFailed;
            }
        }
    }
    let Ok(cache) = CACHE.lock() else {
        return AudioAddictLookup::RefreshFailed;
    };
    cache
        .get(&cache_key)
        .map(|(_, map)| lookup_channel(map, channel_key))
        .unwrap_or(AudioAddictLookup::NoArt)
}

#[cfg(test)]
fn expire_audioaddict_cache_at(network: AudioAddictNetwork, origin: &str) {
    let Ok(mut cache) = CACHE.lock() else {
        return;
    };
    let cache_key = (origin.trim_end_matches('/').to_string(), network);
    if let Some((fetched_at, _)) = cache.get_mut(&cache_key) {
        *fetched_at = Instant::now() - CACHE_TTL - Duration::from_secs(1);
    }
}

pub fn audioaddict_cache_fresh(network: AudioAddictNetwork) -> bool {
    let Ok(cache) = CACHE.lock() else {
        return false;
    };
    let cache_key = (DEFAULT_ORIGIN.to_string(), network);
    cache
        .get(&cache_key)
        .is_some_and(|(fetched_at, _)| fetched_at.elapsed() < CACHE_TTL)
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::thread;

    use super::{
        audioaddict_cover_url_at, expire_audioaddict_cache_at, join_audioaddict_art_url,
        parse_audioaddict_channel, AudioAddictNetwork,
    };

    const PLAYING: &str =
        r#"[{"channel_id":48,"channel_key":"datempolounge","track":{"id":15865}}]"#;
    const HISTORY: &str = r#"{"48":{"art_url":"//cdn-images.audioaddict.com/a/f/9/a/4/7/af9a470e98f03d6a87a6e72bc0f8a204.jpg","type":"track"}}"#;

    fn serve_audioaddict<F>(handler: F) -> (String, Arc<AtomicUsize>)
    where
        F: Fn(&str) -> (u16, Vec<u8>) + Send + 'static,
    {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let hits = Arc::new(AtomicUsize::new(0));
        let hits_clone = hits.clone();
        thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let mut stream = stream;
                let mut buf = [0u8; 4096];
                let n = stream.read(&mut buf).unwrap_or(0);
                let req = String::from_utf8_lossy(&buf[..n]);
                let path = req.lines().next().unwrap_or("");
                hits_clone.fetch_add(1, Ordering::SeqCst);
                let (status, body) = handler(path);
                let status_text = match status {
                    200 => "200 OK",
                    404 => "404 Not Found",
                    _ => "500 Internal Server Error",
                };
                let header = format!(
                    "HTTP/1.1 {status_text}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(header.as_bytes());
                let _ = stream.write_all(&body);
            }
        });
        (format!("http://{addr}"), hits)
    }

    fn serve_audioaddict_api(
        playing: &'static [u8],
        history: &'static [u8],
    ) -> (String, Arc<AtomicUsize>) {
        let playing = playing.to_vec();
        let history = history.to_vec();
        serve_audioaddict(move |path: &str| {
            if path.contains("currently_playing") {
                (200, playing.clone())
            } else if path.contains("track_history") {
                (200, history.clone())
            } else {
                (404, b"{}".to_vec())
            }
        })
    }

    fn serve_audioaddict_status(status: u16) -> (String, Arc<AtomicUsize>) {
        serve_audioaddict(move |_path: &str| (status, b"{}".to_vec()))
    }

    #[test]
    fn join_prefixes_https_and_strips_template() {
        assert_eq!(
            join_audioaddict_art_url("datempolounge", PLAYING, HISTORY).as_deref(),
            Some("https://cdn-images.audioaddict.com/a/f/9/a/4/7/af9a470e98f03d6a87a6e72bc0f8a204.jpg")
        );
        let history = r#"{"48":{"art_url":"//cdn-images.audioaddict.com/x.jpg{?size,height,width,quality,pad}"}}"#;
        assert_eq!(
            join_audioaddict_art_url("datempolounge", PLAYING, history).as_deref(),
            Some("https://cdn-images.audioaddict.com/x.jpg")
        );
    }

    #[test]
    fn join_missing_channel_or_art_is_none() {
        assert!(join_audioaddict_art_url("nope", PLAYING, HISTORY).is_none());
        assert!(join_audioaddict_art_url("datempolounge", PLAYING, r#"{"48":{}}"#).is_none());
        assert!(
            join_audioaddict_art_url("datempolounge", PLAYING, r#"{"48":{"art_url":""}}"#)
                .is_none()
        );
    }

    #[test]
    fn loopback_cover_url_is_some_then_cached() {
        let playing = PLAYING.as_bytes();
        let history = HISTORY.as_bytes();
        let (origin, hits) = serve_audioaddict_api(playing, history);
        let url =
            audioaddict_cover_url_at(AudioAddictNetwork::RadioTunes, "datempolounge", &origin);
        assert_eq!(
            url.as_deref(),
            Some("https://cdn-images.audioaddict.com/a/f/9/a/4/7/af9a470e98f03d6a87a6e72bc0f8a204.jpg")
        );
        let first_hits = hits.load(Ordering::SeqCst);
        assert_eq!(first_hits, 2);
        let again =
            audioaddict_cover_url_at(AudioAddictNetwork::RadioTunes, "datempolounge", &origin);
        assert_eq!(again, url);
        assert_eq!(hits.load(Ordering::SeqCst), first_hits);
    }

    #[test]
    fn loopback_404_is_none() {
        let (origin, _hits) = serve_audioaddict_status(404);
        assert!(audioaddict_cover_url_at(AudioAddictNetwork::Di, "trance", &origin).is_none());
    }

    #[test]
    fn stale_refresh_failure_returns_none() {
        let playing = PLAYING.as_bytes().to_vec();
        let history = HISTORY.as_bytes().to_vec();
        let requests = Arc::new(AtomicUsize::new(0));
        let requests_clone = requests.clone();
        let (origin, _hits) = serve_audioaddict(move |path: &str| {
            let n = requests_clone.fetch_add(1, Ordering::SeqCst);
            if n < 2 {
                if path.contains("currently_playing") {
                    (200, playing.clone())
                } else if path.contains("track_history") {
                    (200, history.clone())
                } else {
                    (404, b"{}".to_vec())
                }
            } else if n < 4 {
                if path.contains("currently_playing") {
                    (200, playing.clone())
                } else {
                    (404, b"{}".to_vec())
                }
            } else {
                (404, b"{}".to_vec())
            }
        });
        let url =
            audioaddict_cover_url_at(AudioAddictNetwork::RadioTunes, "datempolounge", &origin);
        assert!(url.is_some());
        expire_audioaddict_cache_at(AudioAddictNetwork::RadioTunes, &origin);
        let hits_before_stale = requests.load(Ordering::SeqCst);
        assert!(
            audioaddict_cover_url_at(AudioAddictNetwork::RadioTunes, "datempolounge", &origin)
                .is_none()
        );
        let hits_after_stale_fail = requests.load(Ordering::SeqCst);
        assert_eq!(hits_after_stale_fail - hits_before_stale, 2);
        let again =
            audioaddict_cover_url_at(AudioAddictNetwork::RadioTunes, "datempolounge", &origin);
        assert_eq!(again, url);
        assert_eq!(requests.load(Ordering::SeqCst), hits_after_stale_fail);
    }

    #[test]
    fn parse_radiotunes_hi_strips_suffix_and_ignores_query() {
        let (net, key) =
            parse_audioaddict_channel("http://prem2.radiotunes.com:80/datempolounge_hi?listenkey")
                .expect("radiotunes");
        assert_eq!(net, AudioAddictNetwork::RadioTunes);
        assert_eq!(key, "datempolounge");
    }

    #[test]
    fn parse_rockradio_path_without_suffix() {
        let (net, key) =
            parse_audioaddict_channel("http://prem2.rockradio.com:80/metal").expect("rockradio");
        assert_eq!(net, AudioAddictNetwork::RockRadio);
        assert_eq!(key, "metal");
    }

    #[test]
    fn parse_di_fm_hi() {
        let (net, key) =
            parse_audioaddict_channel("http://prem2.di.fm:80/lofiloungenchill_hi?listenkey")
                .expect("di");
        assert_eq!(net, AudioAddictNetwork::Di);
        assert_eq!(key, "lofiloungenchill");
    }

    #[test]
    fn parse_rejects_unknown_host_and_non_http() {
        assert!(parse_audioaddict_channel("https://example.com/x").is_none());
        assert!(parse_audioaddict_channel("file:///tmp/x").is_none());
        assert!(parse_audioaddict_channel("").is_none());
    }

    #[test]
    fn parse_aacplus_wins_over_aac() {
        let (_, key) =
            parse_audioaddict_channel("https://listen.jazzradio.com/cool_aacplus").expect("jazz");
        assert_eq!(key, "cool");
        let (_, key) = parse_audioaddict_channel("https://listen.classicalradio.com/baroque_aac")
            .expect("classical");
        assert_eq!(key, "baroque");
    }

    #[test]
    fn parse_unknown_suffix_stays() {
        let (_, key) =
            parse_audioaddict_channel("https://prem1.zenradio.com/foo_bar").expect("zen");
        assert_eq!(key, "foo_bar");
    }

    #[test]
    fn parse_host_is_domain_or_subdomain_not_suffix_spam() {
        assert!(parse_audioaddict_channel("http://notactuallyradiotunes.com/x").is_none());
    }
}
