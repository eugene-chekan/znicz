use std::collections::HashMap;
use std::io::Read;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::Value;

use crate::cover::title::fold;

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

pub(crate) const DEFAULT_ORIGIN: &str = "https://api.audioaddict.com";
const JSON_MAX: u64 = 1024 * 1024;
/// How long a channel list is trusted before it is fetched again.
const CHANNEL_IDS_TTL: Duration = Duration::from_secs(60 * 60);
/// A list younger than this is not refetched for an unknown channel key.
const UNKNOWN_KEY_RECHECK: Duration = Duration::from_secs(60);
/// Track covers kept in memory; the cache starts again when it is full.
const TRACK_COVERS_MAX: usize = 256;

/// Channel key → channel id.
type ChannelIds = HashMap<String, String>;
/// `(origin, network)` → when the channel list was fetched, and the list.
type ChannelIdStore = HashMap<(String, AudioAddictNetwork), (Instant, ChannelIds)>;
/// `(origin, network, track id)` → the track's cover, `None` when it has none.
type TrackCoverStore = HashMap<(String, AudioAddictNetwork, String), Option<String>>;

static CHANNEL_IDS: LazyLock<Mutex<ChannelIdStore>> = LazyLock::new(|| Mutex::new(HashMap::new()));
static TRACK_COVERS: LazyLock<Mutex<TrackCoverStore>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
static CACHE_BUSTER: AtomicU64 = AtomicU64::new(0);

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

/// A numeric id as text; the API sends some ids as strings.
fn id_string(value: &Value) -> Option<String> {
    match value {
        Value::Number(n) => Some(n.to_string()),
        Value::String(s) => Some(s.clone()),
        _ => None,
    }
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

/// A query value no earlier request used, so Cloudflare cannot answer from its cache.
fn cache_buster() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let seq = CACHE_BUSTER.fetch_add(1, Ordering::Relaxed);
    format!("{nanos}-{seq}")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AudioAddictLookup {
    Found(String),
    NoArt,
    RefreshFailed,
    /// The channel is still on another track than the stream title.
    NotYet,
}

pub fn audioaddict_cover_lookup(
    network: AudioAddictNetwork,
    channel_key: &str,
    title: Option<&str>,
) -> AudioAddictLookup {
    audioaddict_cover_lookup_at(network, channel_key, title, DEFAULT_ORIGIN)
}

/// The cover of the track `channel_key` is playing now. With a `title`, the
/// channel must be on that title, or the answer is `NotYet`.
pub fn audioaddict_cover_lookup_at(
    network: AudioAddictNetwork,
    channel_key: &str,
    title: Option<&str>,
    origin: &str,
) -> AudioAddictLookup {
    let origin = origin.trim_end_matches('/');
    let channel_id = match channel_id(network, channel_key, origin) {
        Ok(id) => id,
        Err(lookup) => return lookup,
    };
    let track = match current_track(network, &channel_id, origin) {
        Ok(track) => track,
        Err(lookup) => return lookup,
    };
    if let Some(title) = title {
        let current = format!("{} - {}", track.artist, track.title);
        if fold(&current) != fold(title) {
            return AudioAddictLookup::NotYet;
        }
    }
    track_cover(network, &track.id, origin)
}

/// The numeric id of `channel_key`, from the network's channel list. The list
/// is refreshed when it is old or lacks the key.
fn channel_id(
    network: AudioAddictNetwork,
    channel_key: &str,
    origin: &str,
) -> Result<String, AudioAddictLookup> {
    let cache_key = (origin.to_string(), network);
    if let Ok(store) = CHANNEL_IDS.lock() {
        if let Some((fetched_at, ids)) = store.get(&cache_key) {
            let age = fetched_at.elapsed();
            if age < CHANNEL_IDS_TTL {
                if let Some(id) = ids.get(channel_key) {
                    return Ok(id.clone());
                }
            }
            if age < UNKNOWN_KEY_RECHECK {
                return Err(AudioAddictLookup::NoArt);
            }
        }
    }
    let ids = fetch_channel_ids(network, origin).ok_or_else(|| {
        tracing::debug!(
            network = network.slug(),
            channel_key,
            "audioaddict channel list failed"
        );
        AudioAddictLookup::RefreshFailed
    })?;
    let id = ids.get(channel_key).cloned();
    if let Ok(mut store) = CHANNEL_IDS.lock() {
        store.insert(cache_key, (Instant::now(), ids));
    }
    id.ok_or(AudioAddictLookup::NoArt)
}

fn fetch_channel_ids(network: AudioAddictNetwork, origin: &str) -> Option<ChannelIds> {
    let body = fetch_json(origin, &format!("/v1/{}/currently_playing", network.slug()))?;
    let entries: Vec<Value> = serde_json::from_str(&body).ok()?;
    Some(
        entries
            .iter()
            .filter_map(|entry| {
                let key = entry.get("channel_key")?.as_str()?;
                let id = id_string(entry.get("channel_id")?)?;
                Some((key.to_string(), id))
            })
            .collect(),
    )
}

/// The track a channel is playing right now, read from the uncached endpoint.
struct CurrentTrack {
    id: String,
    artist: String,
    title: String,
}

fn current_track(
    network: AudioAddictNetwork,
    channel_id: &str,
    origin: &str,
) -> Result<CurrentTrack, AudioAddictLookup> {
    let path = format!(
        "/v1/{}/currently_playing/channel/{channel_id}?t={}",
        network.slug(),
        cache_buster()
    );
    let body = fetch_json(origin, &path).ok_or(AudioAddictLookup::RefreshFailed)?;
    let entry: Value = serde_json::from_str(&body).map_err(|_| AudioAddictLookup::RefreshFailed)?;
    let track = entry.get("track").ok_or(AudioAddictLookup::NoArt)?;
    let id = track
        .get("id")
        .and_then(id_string)
        .ok_or(AudioAddictLookup::NoArt)?;
    let text = |field: &str| {
        track
            .get(field)
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string()
    };
    Ok(CurrentTrack {
        id,
        artist: text("display_artist"),
        title: text("display_title"),
    })
}

/// The cover of a track from `tracks/{id}`, remembered per track.
fn track_cover(network: AudioAddictNetwork, track_id: &str, origin: &str) -> AudioAddictLookup {
    let key = (origin.to_string(), network, track_id.to_string());
    if let Ok(covers) = TRACK_COVERS.lock() {
        if let Some(cover) = covers.get(&key) {
            return cover
                .clone()
                .map_or(AudioAddictLookup::NoArt, AudioAddictLookup::Found);
        }
    }
    let Some(body) = fetch_json(origin, &format!("/v1/{}/tracks/{track_id}", network.slug()))
    else {
        return AudioAddictLookup::RefreshFailed;
    };
    let Ok(track) = serde_json::from_str::<Value>(&body) else {
        return AudioAddictLookup::RefreshFailed;
    };
    let art = track
        .get("asset_url")
        .and_then(Value::as_str)
        .and_then(normalize_art_url)
        .or_else(|| {
            track
                .get("images")
                .and_then(|images| images.get("default"))
                .and_then(Value::as_str)
                .and_then(normalize_art_url)
        });
    if let Ok(mut covers) = TRACK_COVERS.lock() {
        if covers.len() >= TRACK_COVERS_MAX {
            covers.clear();
        }
        covers.insert(key, art.clone());
    }
    art.map_or(AudioAddictLookup::NoArt, AudioAddictLookup::Found)
}

#[cfg(test)]
pub(crate) mod test_server {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};
    use std::thread;

    /// The request paths a test server has seen, in order.
    pub(crate) type Requests = Arc<Mutex<Vec<String>>>;

    /// The stream title of the Smooth Beats test channel.
    pub(crate) const TITLE: &str = "Else Born - This Time/Remember";
    /// The channel's `currently_playing` entry while it is on the stream title.
    pub(crate) const NOW: &str = r#"{"channel_id":407,"channel_key":"smoothbeats","track":{"id":1272704,"display_artist":"Else Born","display_title":"This Time/Remember","start_time":"2026-11-11T08:04:42-05:00","duration":241.0}}"#;
    /// The channel's `currently_playing` entry while it still plays the previous song.
    pub(crate) const LAGGING: &str = r#"{"channel_id":407,"channel_key":"smoothbeats","track":{"id":1111,"display_artist":"Parc","display_title":"4U","start_time":"2026-11-11T08:04:30-05:00","duration":200.0}}"#;
    /// `tracks/1272704`: the cover is `asset_url`.
    pub(crate) const TRACK: &str = r#"{"id":1272704,"display_artist":"Else Born","display_title":"This Time/Remember","asset_url":"//cdn-images.audioaddict.com/d/4/b/9/0/4/d4b904f817c57e4bfbe0a8bca9ffbb93.jpg","images":{"default":"//cdn-images.audioaddict.com/d/4/b/9/0/4/d4b904f817c57e4bfbe0a8bca9ffbb93.jpg{?size,height,width,quality,pad}"}}"#;
    /// The cover `TRACK` resolves to.
    pub(crate) const ART: &str =
        "https://cdn-images.audioaddict.com/d/4/b/9/0/4/d4b904f817c57e4bfbe0a8bca9ffbb93.jpg";

    /// Serves `handler(path) -> (status, body)` on a fresh loopback port.
    /// Returns the origin (`http://127.0.0.1:port`) and the recorded paths.
    pub(crate) fn serve<F>(handler: F) -> (String, Requests)
    where
        F: Fn(&str) -> (u16, Vec<u8>) + Send + 'static,
    {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("local addr");
        let requests: Requests = Arc::new(Mutex::new(Vec::new()));
        let seen = requests.clone();
        thread::spawn(move || {
            for mut stream in listener.incoming().flatten() {
                let mut buf = [0u8; 4096];
                let n = stream.read(&mut buf).unwrap_or(0);
                let request = String::from_utf8_lossy(&buf[..n]).into_owned();
                let path = request.split_whitespace().nth(1).unwrap_or("").to_string();
                let (status, body) = handler(&path);
                seen.lock().unwrap().push(path);
                let text = match status {
                    200 => "200 OK",
                    404 => "404 Not Found",
                    _ => "500 Internal Server Error",
                };
                let head = format!(
                    "HTTP/1.1 {text}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(head.as_bytes());
                let _ = stream.write_all(&body);
            }
        });
        (format!("http://{addr}"), requests)
    }

    /// How many recorded paths contain `needle`.
    pub(crate) fn count(requests: &Requests, needle: &str) -> usize {
        requests
            .lock()
            .unwrap()
            .iter()
            .filter(|path| path.contains(needle))
            .count()
    }

    /// A RadioTunes-like API: the channel list (one entry, `channel`), the
    /// channel's current track, and `track` for any `tracks/{id}`.
    pub(crate) fn serve_api(channel: &'static str, track: &'static str) -> (String, Requests) {
        let list = format!("[{channel}]");
        serve(move |path: &str| {
            if path.contains("/currently_playing/channel/") {
                (200, channel.as_bytes().to_vec())
            } else if path.contains("/currently_playing") {
                (200, list.as_bytes().to_vec())
            } else if path.contains("/tracks/") {
                (200, track.as_bytes().to_vec())
            } else {
                (404, b"{}".to_vec())
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::test_server::{ART, LAGGING, NOW, TITLE, TRACK, count, serve, serve_api};
    use super::{
        AudioAddictLookup, AudioAddictNetwork, audioaddict_cover_lookup_at,
        parse_audioaddict_channel,
    };

    /// Smooth Beats on the test server, with `title` (or none).
    fn lookup(origin: &str, title: Option<&str>) -> AudioAddictLookup {
        audioaddict_cover_lookup_at(AudioAddictNetwork::RadioTunes, "smoothbeats", title, origin)
    }

    #[test]
    fn channel_on_the_previous_track_is_not_yet() {
        let (origin, _) = serve_api(LAGGING, TRACK);
        assert_eq!(lookup(&origin, Some(TITLE)), AudioAddictLookup::NotYet);
    }

    #[test]
    fn channel_on_the_stream_title_gives_the_track_cover() {
        let (origin, _) = serve_api(NOW, TRACK);
        assert_eq!(
            lookup(&origin, Some("  else  born - THIS time/remember ")),
            AudioAddictLookup::Found(ART.into())
        );
    }

    #[test]
    fn a_backtick_in_the_stream_title_matches_an_apostrophe() {
        let now = r#"{"channel_id":407,"channel_key":"smoothbeats","track":{"id":1272704,"display_artist":"MAAD","display_title":"90's Love"}}"#;
        let (origin, _) = serve_api(now, TRACK);
        assert_eq!(
            lookup(&origin, Some("MAAD - 90`s Love")),
            AudioAddictLookup::Found(ART.into())
        );
    }

    #[test]
    fn images_default_template_is_stripped_and_https_added() {
        let track = r#"{"id":1272704,"images":{"default":"//cdn-images.audioaddict.com/x.jpg{?size,height,width,quality,pad}"}}"#;
        let (origin, _) = serve_api(NOW, track);
        assert_eq!(
            lookup(&origin, None),
            AudioAddictLookup::Found("https://cdn-images.audioaddict.com/x.jpg".into())
        );
    }

    #[test]
    fn track_cover_is_requested_once_per_track() {
        let (origin, requests) = serve_api(NOW, TRACK);
        for _ in 0..2 {
            assert_eq!(
                lookup(&origin, Some(TITLE)),
                AudioAddictLookup::Found(ART.into())
            );
        }
        assert_eq!(count(&requests, "/tracks/1272704"), 1);
    }

    #[test]
    fn no_title_gives_the_current_track_cover() {
        let (origin, _) = serve_api(LAGGING, TRACK);
        assert_eq!(lookup(&origin, None), AudioAddictLookup::Found(ART.into()));
    }

    #[test]
    fn failing_channel_request_is_refresh_failed() {
        let list = format!("[{NOW}]");
        let (origin, _) = serve(move |path: &str| {
            if path.contains("/currently_playing/channel/") {
                (500, b"{}".to_vec())
            } else if path.contains("/currently_playing") {
                (200, list.as_bytes().to_vec())
            } else {
                (404, b"{}".to_vec())
            }
        });
        assert_eq!(
            lookup(&origin, Some(TITLE)),
            AudioAddictLookup::RefreshFailed
        );
    }

    #[test]
    fn failing_track_request_is_refresh_failed_and_retried() {
        let track_calls = Arc::new(AtomicUsize::new(0));
        let calls = track_calls.clone();
        let list = format!("[{NOW}]");
        let (origin, _) = serve(move |path: &str| {
            if path.contains("/currently_playing/channel/") {
                (200, NOW.as_bytes().to_vec())
            } else if path.contains("/currently_playing") {
                (200, list.as_bytes().to_vec())
            } else if path.contains("/tracks/") {
                if calls.fetch_add(1, Ordering::SeqCst) == 0 {
                    (500, b"{}".to_vec())
                } else {
                    (200, TRACK.as_bytes().to_vec())
                }
            } else {
                (404, b"{}".to_vec())
            }
        });
        assert_eq!(
            lookup(&origin, Some(TITLE)),
            AudioAddictLookup::RefreshFailed
        );
        assert_eq!(
            lookup(&origin, Some(TITLE)),
            AudioAddictLookup::Found(ART.into())
        );
    }

    #[test]
    fn unknown_channel_is_no_art() {
        let (origin, _) = serve(|path: &str| {
            if path.contains("/currently_playing") && !path.contains("/channel/") {
                (
                    200,
                    br#"[{"channel_id":9,"channel_key":"other","track":{"id":1}}]"#.to_vec(),
                )
            } else {
                (404, b"{}".to_vec())
            }
        });
        assert_eq!(lookup(&origin, Some(TITLE)), AudioAddictLookup::NoArt);
    }

    #[test]
    fn unknown_key_does_not_refetch_the_channel_list_within_a_minute() {
        let (origin, requests) = serve_api(NOW, TRACK);
        for _ in 0..2 {
            assert_eq!(
                audioaddict_cover_lookup_at(
                    AudioAddictNetwork::RadioTunes,
                    "no-such-channel",
                    Some(TITLE),
                    &origin
                ),
                AudioAddictLookup::NoArt
            );
        }
        let list_requests = requests
            .lock()
            .unwrap()
            .iter()
            .filter(|path| path.contains("/currently_playing") && !path.contains("/channel/"))
            .count();
        assert_eq!(list_requests, 1);
    }

    #[test]
    fn each_channel_request_carries_a_new_cache_buster() {
        let (origin, requests) = serve_api(LAGGING, TRACK);
        let _ = lookup(&origin, Some(TITLE));
        let _ = lookup(&origin, Some(TITLE));
        let busters: Vec<String> = requests
            .lock()
            .unwrap()
            .iter()
            .filter(|path| path.contains("/currently_playing/channel/"))
            .filter_map(|path| path.split_once("?t=").map(|(_, t)| t.to_string()))
            .collect();
        assert_eq!(busters.len(), 2, "{busters:?}");
        assert!(!busters[0].is_empty());
        assert_ne!(busters[0], busters[1]);
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
