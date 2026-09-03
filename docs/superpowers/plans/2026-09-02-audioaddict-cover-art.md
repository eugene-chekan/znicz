# AudioAddict Song Covers Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Show the current song cover for AudioAddict streams (RadioTunes, DI.FM, RockRadio, JazzRadio, ClassicalRadio, ZenRadio) in the TUI slot, looked up from the stream URL via public JSON.

**Architecture:** `znicz-core` parses `TrackInfo.url` into `(network, channel_key)` and, on the cover worker only, GETs `currently_playing` + `track_history` (15s per-network cache) to produce an `https` `art_url`. The TUI uses `CoverKey::AudioAddict` and existing `fetch_cover`. Player thread unchanged. No JPEG on IPC.

**Tech Stack:** Existing `ureq` and `serde_json` in `znicz-core`; existing `image` in `znicz-tui`. No new crates.

**Spec:** `docs/superpowers/specs/2026-09-02-audioaddict-cover-art-design.md`

## Global Constraints

- Version **0.4.1 → 0.4.2** in the same PR (compatible addition).
- No image bytes on JSON IPC or `PlayerState`.
- Player thread does not call AudioAddict HTTP, `fetch_cover`, or decode images.
- Channel comes from **`TrackInfo.url` only** (not `StreamTitle`). Ignore the query string: do not log it, cache it, or send it to the API.
- Cover order: ICY image → AudioAddict `art_url` → station `art` file → logo.
- JSON: origin `https://api.audioaddict.com`, connect timeout 8s, body cap **1 MiB** each, `debug` log on failure, no toast. JSON miss is not a permanent fail (retry after 15s TTL).
- Failed **image** URLs: existing process-lifetime `HashSet` (unchanged).
- Out of scope: MusicBrainz, filling `station.art` from channel logos, listen-key auth, new CLI/MCP tools, Icecast `icy-url` as a cover, disk cache, `cover://current`.
- Wiki matches the code in the same PR. No new TUI keys.
- Tests: loopback HTTP only. **No** live `api.audioaddict.com` in CI.

## File map

| File | Role |
| --- | --- |
| Create `znicz-core/src/audioaddict.rs` | Parse URL, join JSON, cached HTTP lookup |
| Modify `znicz-core/src/lib.rs` | `mod audioaddict`; re-export parse + lookup + network type |
| Modify `znicz-tui/src/cover.rs` | `CoverKey::AudioAddict`, three-way `pick_stream_cover`, stale refresh |
| Modify `znicz-tui/src/views/now_playing.rs` | Middle pick argument from `track.url` |
| Wiki + README + `Cargo.toml` | 0.4.2 |

---

### Task 1: Parse stream URL → network + channel key

**Files:**
- Create: `znicz-core/src/audioaddict.rs`
- Modify: `znicz-core/src/lib.rs`

**Interfaces:**
- Consumes: a stream URL string (`TrackInfo.url`)
- Produces: `pub enum AudioAddictNetwork { RadioTunes, Di, RockRadio, JazzRadio, ClassicalRadio, ZenRadio }` with `slug(self) -> &'static str`; `pub fn parse_audioaddict_channel(stream_url: &str) -> Option<(AudioAddictNetwork, String)>`

- [ ] **Step 1: Write the failing tests** in `znicz-core/src/audioaddict.rs` under `#[cfg(test)]`.

```rust
#[test]
fn parse_radiotunes_hi_strips_suffix_and_ignores_query() {
    let (net, key) = parse_audioaddict_channel(
        "http://prem2.radiotunes.com:80/datempolounge_hi?listenkey",
    )
    .expect("radiotunes");
    assert_eq!(net, AudioAddictNetwork::RadioTunes);
    assert_eq!(key, "datempolounge");
}

#[test]
fn parse_rockradio_path_without_suffix() {
    let (net, key) = parse_audioaddict_channel("http://prem2.rockradio.com:80/metal")
        .expect("rockradio");
    assert_eq!(net, AudioAddictNetwork::RockRadio);
    assert_eq!(key, "metal");
}

#[test]
fn parse_di_fm_hi() {
    let (net, key) = parse_audioaddict_channel(
        "http://prem2.di.fm:80/lofiloungenchill_hi?listenkey",
    )
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
    let (_, key) = parse_audioaddict_channel("https://listen.jazzradio.com/cool_aacplus")
        .expect("jazz");
    assert_eq!(key, "cool");
    let (_, key) = parse_audioaddict_channel("https://listen.classicalradio.com/baroque_aac")
        .expect("classical");
    assert_eq!(key, "baroque");
}

#[test]
fn parse_unknown_suffix_stays() {
    let (_, key) = parse_audioaddict_channel("https://prem1.zenradio.com/foo_bar")
        .expect("zen");
    assert_eq!(key, "foo_bar");
}

#[test]
fn parse_host_is_domain_or_subdomain_not_suffix_spam() {
    assert!(parse_audioaddict_channel("http://notactuallyradiotunes.com/x").is_none());
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p znicz-core --offline parse_radiotunes_hi -- --nocapture`

Expected: FAIL (`parse_audioaddict_channel` / `AudioAddictNetwork` missing).

- [ ] **Step 3: Write minimal implementation**

```rust
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
    let host = authority.rsplit_once('@').map(|(_, h)| h).unwrap_or(authority);
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
```

In `lib.rs`: `mod audioaddict;` and `pub use audioaddict::{parse_audioaddict_channel, AudioAddictNetwork};`

Host match is `host == domain || host.ends_with(".{domain}")` so `notactuallyradiotunes.com` is not RadioTunes.

- [ ] **Step 4: Run tests**

Run: `cargo test -p znicz-core --offline parse_radiotunes_hi parse_rockradio parse_di_fm parse_rejects parse_aacplus parse_unknown_suffix parse_host_is_domain -- --nocapture`

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add znicz-core/src/audioaddict.rs znicz-core/src/lib.rs
git commit -m "$(cat <<'EOF'
Parse AudioAddict network and channel key from a stream URL.

EOF
)"
```

---

### Task 2: Join JSON and fetch `art_url` with a 15s cache

**Files:**
- Modify: `znicz-core/src/audioaddict.rs`
- Modify: `znicz-core/src/lib.rs` (re-export `audioaddict_cover_url`)

**Interfaces:**
- Consumes: `AudioAddictNetwork`, `channel_key`, JSON bodies
- Produces: `pub fn join_audioaddict_art_url(channel_key: &str, currently_playing: &str, track_history: &str) -> Option<String>`, `pub fn audioaddict_cover_url(network: AudioAddictNetwork, channel_key: &str) -> Option<String>`, `pub fn audioaddict_cache_fresh(network: AudioAddictNetwork) -> bool`, `pub(crate)` or `pub` `audioaddict_cover_url_at(network, channel_key, origin: &str)` for tests. Production origin: `https://api.audioaddict.com`.

- [ ] **Step 1: Write the failing tests** in the same `mod tests`.

```rust
const PLAYING: &str = r#"[{"channel_id":48,"channel_key":"datempolounge","track":{"id":15865}}]"#;
const HISTORY: &str = r#"{"48":{"art_url":"//cdn-images.audioaddict.com/a/f/9/a/4/7/af9a470e98f03d6a87a6e72bc0f8a204.jpg","type":"track"}}"#;

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
    assert!(join_audioaddict_art_url("datempolounge", PLAYING, r#"{"48":{"art_url":""}}"#).is_none());
}

#[test]
fn loopback_cover_url_is_some_then_cached() {
    let playing = PLAYING.as_bytes();
    let history = HISTORY.as_bytes();
    let (origin, hits) = serve_audioaddict_api(playing, history);
    let url = audioaddict_cover_url_at(
        AudioAddictNetwork::RadioTunes,
        "datempolounge",
        &origin,
    );
    assert_eq!(
        url.as_deref(),
        Some("https://cdn-images.audioaddict.com/a/f/9/a/4/7/af9a470e98f03d6a87a6e72bc0f8a204.jpg")
    );
    let first_hits = hits.load(std::sync::atomic::Ordering::SeqCst);
    assert_eq!(first_hits, 2);
    let again = audioaddict_cover_url_at(
        AudioAddictNetwork::RadioTunes,
        "datempolounge",
        &origin,
    );
    assert_eq!(again, url);
    assert_eq!(hits.load(std::sync::atomic::Ordering::SeqCst), first_hits);
}

#[test]
fn loopback_404_is_none() {
    let (origin, _hits) = serve_audioaddict_status(404);
    assert!(audioaddict_cover_url_at(AudioAddictNetwork::Di, "trance", &origin).is_none());
}
```

```rust
fn serve_audioaddict_api(
    playing: &'static [u8],
    history: &'static [u8],
) -> (String, std::sync::Arc<std::sync::atomic::AtomicUsize>) {
    serve_audioaddict(move |path: &str| {
        if path.contains("currently_playing") {
            (200, playing)
        } else if path.contains("track_history") {
            (200, history)
        } else {
            (404, b"{}" as &[u8])
        }
    })
}

fn serve_audioaddict_status(status: u16) -> (String, std::sync::Arc<std::sync::atomic::AtomicUsize>) {
    serve_audioaddict(move |_path: &str| (status, b"{}" as &[u8]))
}
```

`serve_audioaddict` binds `127.0.0.1:0`, spawns a thread, reads the request line, bumps `AtomicUsize`, writes `HTTP/1.1 {status}` + `Content-Length` + body. Return `(format!("http://{addr}"), hits)`. Origin has **no** trailing slash. Do not hit `api.audioaddict.com`. PLAYING/HISTORY in `join_*` tests are `&str`; in `serve_audioaddict_api` use `PLAYING.as_bytes()` only if you change them to `'static` — simplest: duplicate the JSON as `const PLAYING_BYTES: &[u8] = PLAYING.as_bytes();` is not legal for `str` consts. Use `PLAYING.as_bytes()` with a server that **owns** `Vec<u8>` (copy the two bodies into the thread, like `cover_fetch::serve_once_owned`).

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p znicz-core --offline join_prefixes_https -- --nocapture`

Expected: FAIL (`join_audioaddict_art_url` missing).

- [ ] **Step 3: Write minimal implementation**

`join_audioaddict_art_url`: parse `currently_playing` as `Vec<Value>`, find `channel_key`, read `channel_id` (number or string). Parse `track_history` as `Map`. Look up `channel_id.to_string()`. Read `art_url` string. `normalize_art_url`: trim; if starts with `//` prefix `https:`; strip trailing `{?…}` (find last `{?` that has a closing `}`); require `http://` or `https://`.

HTTP:

```rust
const DEFAULT_ORIGIN: &str = "https://api.audioaddict.com";
const JSON_MAX: u64 = 1024 * 1024;
const CACHE_TTL: Duration = Duration::from_secs(15);

pub fn audioaddict_cover_url(network: AudioAddictNetwork, channel_key: &str) -> Option<String> {
    audioaddict_cover_url_at(network, channel_key, DEFAULT_ORIGIN)
}
```

Cache: `Mutex<HashMap<(String /*origin*/, AudioAddictNetwork), (Instant, HashMap<String, Option<String>>)>>`. If entry age `< 15s`, return `map.get(channel_key).cloned().flatten()`. Else GET `{origin}/v1/{slug}/currently_playing` and `{origin}/v1/{slug}/track_history` (8s connect timeout, same `ureq` agent pattern as `cover_fetch.rs`, `take(JSON_MAX + 1)`, oversize → fail). On success, rebuild the map for **every** `channel_key` in `currently_playing` (missing art → `None` in the map). Store `(Instant::now(), map)`. On HTTP/JSON failure: `tracing::debug!`, keep previous map if any, return `None` for this call.

`audioaddict_cache_fresh(network)`: true iff the **production origin** cache for that network exists and age `< 15s`. (TUI refresh uses this. Tests that only call `_at` with loopback do not need it.)

Do not log the stream URL query string. Log `network` slug + `channel_key` + error only.

- [ ] **Step 4: Run tests**

Run: `cargo test -p znicz-core --offline -- audioaddict -- --nocapture`

Expected: PASS (parse + join + loopback).

- [ ] **Step 5: Commit**

```bash
git add znicz-core/src/audioaddict.rs znicz-core/src/lib.rs
git commit -m "$(cat <<'EOF'
Look up AudioAddict song cover URLs from currently_playing and track_history.

EOF
)"
```

---

### Task 3: TUI cover order and `CoverKey::AudioAddict`

**Files:**
- Modify: `znicz-tui/src/cover.rs`
- Modify: `znicz-tui/src/views/now_playing.rs`

**Interfaces:**
- Consumes: `parse_audioaddict_channel`, `audioaddict_cover_url`, `audioaddict_cache_fresh`, `fetch_cover`
- Produces: `CoverKey::AudioAddict { network: AudioAddictNetwork, channel: String }`; `pick_stream_cover(icy, audioaddict, station)`

- [ ] **Step 1: Write the failing tests** — replace `pick_stream_cover_prefers_icy_then_station_then_logo` and `no_path_is_the_logo` with three-argument calls; add:

```rust
#[test]
fn pick_stream_cover_prefers_icy_then_audioaddict_then_station() {
    let icy_img = Arc::new(DynamicImage::new_rgb8(2, 2));
    let aa_img = Arc::new(DynamicImage::new_rgb8(4, 4));
    let station_img = Arc::new(DynamicImage::new_rgb8(3, 3));
    let icy = CoverReady::Embedded(icy_img.clone());
    let aa = CoverReady::Embedded(aa_img.clone());
    let station = CoverReady::Embedded(station_img.clone());
    assert!(matches!(
        pick_stream_cover(icy.clone(), aa.clone(), station.clone()),
        CoverReady::Embedded(img) if Arc::ptr_eq(&img, &icy_img)
    ));
    assert!(matches!(
        pick_stream_cover(CoverReady::Pending, aa.clone(), station.clone()),
        CoverReady::Embedded(img) if Arc::ptr_eq(&img, &aa_img)
    ));
    assert!(matches!(
        pick_stream_cover(CoverReady::Logo, CoverReady::Pending, station.clone()),
        CoverReady::Embedded(img) if Arc::ptr_eq(&img, &station_img)
    ));
    assert!(matches!(
        pick_stream_cover(CoverReady::Logo, CoverReady::Logo, station),
        CoverReady::Embedded(img) if Arc::ptr_eq(&img, &station_img)
    ));
    assert_eq!(
        pick_stream_cover(CoverReady::Pending, CoverReady::Pending, CoverReady::Logo),
        CoverReady::Logo
    );
}
```

Also change `no_path_is_the_logo` to `pick_stream_cover(Logo, Logo, Logo)`.

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p znicz-tui --lib --offline pick_stream_cover_prefers_icy_then_audioaddict -- --nocapture`

Expected: FAIL (function still takes two arguments).

- [ ] **Step 3: Write minimal implementation**

`pick_stream_cover(icy, audioaddict, station)`: first `Embedded` in that order, else `Logo`.

```rust
CoverKey::AudioAddict {
    network: znicz_core::AudioAddictNetwork,
    channel: String,
}
```

`Slot { ready, refreshing: bool }` (default `refreshing: false`).

`get`:

- `Url` failed-set behaviour unchanged.
- If slot exists:
  - If `CoverKey::AudioAddict { network, .. }` and `!audioaddict_cache_fresh(*network)` and `!slot.refreshing`: set `refreshing = true`, clone key, drop guard, `send`, return **current** `ready` (Embedded stays on screen).
  - Else return `slot.ready`.
- If no slot: insert `Pending` (`refreshing: false`), send, return `Pending`.

Worker after `resolve_cover`: set `refreshing = false` when inserting the result.

`resolve_cover` for `AudioAddict`:

```rust
CoverKey::AudioAddict { network, channel } => {
    match znicz_core::audioaddict_cover_url(*network, channel) {
        Some(url) => match znicz_core::fetch_cover(&url) {
            Some(art) => match decode_capped(&art.bytes) {
                Some(img) => (CoverReady::Embedded(Arc::new(img)), None),
                None => {
                    tracing::debug!(url, "AudioAddict cover did not decode");
                    (CoverReady::Logo, Some(url))
                }
            },
            None => (CoverReady::Logo, Some(url)),
        },
        None => (CoverReady::Logo, None),
    }
}
```

JSON miss → Logo on this key, **not** added to the failed-URL set, so TTL retry can run.

`now_playing.rs` `render_cover` stream branch:

```rust
let aa_ready = match track
    .url
    .as_deref()
    .and_then(znicz_core::parse_audioaddict_channel)
{
    Some((network, channel)) => app.covers.get(CoverKey::AudioAddict { network, channel }),
    None => CoverReady::Logo,
};
pick_stream_cover(icy_ready, aa_ready, station_ready)
```

`cover_protocol = "off"` still returns `Logo` before any `get`.

- [ ] **Step 4: Run tests**

Run: `cargo test -p znicz-tui --offline -- pick_stream_cover -- --nocapture && cargo test --workspace --offline`

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add znicz-tui/src/cover.rs znicz-tui/src/views/now_playing.rs
git commit -m "$(cat <<'EOF'
Pick AudioAddict song covers after ICY images and before station art.

EOF
)"
```

---

### Task 4: Wiki, README, version 0.4.2

**Files:**
- Modify: `Cargo.toml` workspace version `0.4.1` → `0.4.2`
- Modify: `wiki/Plans/Phase-5-Album-Art.md`
- Modify: `wiki/Architecture/TUI.md`
- Modify: `wiki/Domain/Formats-and-Metadata.md`
- Modify: `wiki/Plans/Roadmap.md`
- Modify: `README.md`

**Interfaces:**
- Consumes: behaviour from Tasks 1–3
- Produces: docs and version 0.4.2

- [ ] **Step 1: No new behaviour tests.** Update copy to match the code.

`Cargo.toml`: `version = "0.4.2"`.

Phase-5 status line: Done **(0.4.2)**. What shipped: add that AudioAddict streams (RadioTunes, DI.FM, RockRadio, JazzRadio, ClassicalRadio, ZenRadio) show the current song cover from JSON `art_url`. Cover choice: ICY image, then AudioAddict, then station file, then logo. Milestones: add **5.5** checked — AudioAddict song covers (0.4.2). Leave disk cache and `cover://current` unchecked.

TUI.md streams sentence:

```
For streams, the cover slot uses an ICY `StreamUrl` image when one decodes,
else the AudioAddict current-song `art_url` (RadioTunes / DI.FM / RockRadio /
JazzRadio / ClassicalRadio / ZenRadio), else the station `art` file, else the
logo.
```

Formats-and-Metadata after the `StreamUrl` sentence:

```
RadioTunes, DI.FM, RockRadio, JazzRadio, ClassicalRadio, and ZenRadio do not
send `StreamUrl`. For those hosts the TUI looks up the channel from the stream
URL and uses AudioAddict `art_url` as the song cover.
```

Roadmap Phase 5 bullet: add `AudioAddict song covers for RadioTunes / DI.FM / RockRadio (0.4.2)`.

README transport sentence: streams use ICY image, else AudioAddict song cover on those networks, else station art, else the logo. Feature line: mention AudioAddict song covers.

- [ ] **Step 2: Format, test, clippy**

Run: `cargo fmt --all && cargo test --workspace --offline && cargo clippy --workspace --all-targets --offline -- -D warnings`

Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add Cargo.toml Cargo.lock wiki README.md
git commit -m "$(cat <<'EOF'
Document AudioAddict stream covers and ship them as 0.4.2.

EOF
)"
```

---

## Spec coverage

| Spec item | Task |
| --- | --- |
| Parse host + path; ignore query; quality suffixes | 1 |
| Domain-or-subdomain host match | 1 |
| Join `currently_playing` + `track_history`; `//` → `https:`; strip `{?…}` | 2 |
| 15s cache; 8s timeout; 1 MiB JSON; loopback tests | 2 |
| `audioaddict_cache_fresh` for TUI refresh | 2 |
| ICY → AudioAddict → station → logo; pending falls through | 3 |
| `CoverKey::AudioAddict`; worker `fetch_cover`; JSON miss not in failed-URL set | 3 |
| Stale refresh keeps previous Embedded | 3 |
| `cover_protocol = "off"` skips get | 3 (existing short-circuit) |
| Wiki + README + 0.4.2 | 4 |
| No live API in CI; no JPEG on IPC; player thread unchanged | 1–3 |
| Not MusicBrainz / channel logos / listen key / CLI-MCP | none |
