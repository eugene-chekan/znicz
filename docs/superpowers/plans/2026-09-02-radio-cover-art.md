# Radio and Stream Cover Art Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Show a local station picture in the TUI cover slot while a stream plays, and replace it with ICY `StreamUrl` when that URL decodes as an image.

**Architecture:** The player copies `StreamUrl` onto `TrackInfo.icy_stream_url` (a string, not bytes). The TUI `CoverCache` worker fetches that URL via `znicz_core::fetch_cover` and otherwise opens `Station.art` as an image file. The player thread never fetches or decodes pictures. JSON IPC still has no JPEG.

**Tech Stack:** Existing `ureq` in `znicz-core`, existing `image` in `znicz-tui`, lofty unused for station art.

**Spec:** `docs/superpowers/specs/2026-09-02-radio-cover-art-design.md`

## Global Constraints

- Version **0.4.0 → 0.4.1** in the same PR (compatible addition).
- No image bytes on JSON IPC or `PlayerState`. `icy_stream_url` is an optional string.
- Player thread does not call `fetch_cover` or decode images.
- Station `art` is a **local image file**, never `http://` / `https://`.
- Fetch: decode is the image check (no path-extension guess, no `HEAD` Content-Type gate). Connect timeout 8s, body cap 2 MiB, `http`/`https` only.
- Failed URLs: debug log, no toast; remember for the process lifetime.
- Out of scope: disk cache, MCP `cover://current`, `folder.jpg`, MusicBrainz, HLS/PLS/XSPF.
- Wiki matches the code in the same PR. No new TUI keys (radio form gains an `art:` field).
- Parked TUI `#5`–`#9` / `#22` / `#34` / `#36`, playlist `#18` / `#19`: untouched.

## File map

| File | Role |
| --- | --- |
| Modify `znicz-core/src/audio/icy.rs` | Parse `StreamUrl`, `IcyUrl`, strip-read slot, apply onto `TrackInfo` |
| Modify `znicz-core/src/player/state.rs` | `TrackInfo.icy_stream_url` |
| Modify `znicz-core/src/audio/http.rs` | Second ICY slot on `HttpStreamSource` |
| Modify `znicz-core/src/audio/source.rs` | `icy_url_slot` / `AudioDecoder::icy_url` |
| Modify `znicz-core/src/player/engine.rs` | Copy URL onto the current track |
| Create `znicz-core/src/cover_fetch.rs` | `fetch_cover` |
| Modify `znicz-core/src/lib.rs` | Re-export `fetch_cover`; `set_station_art` |
| Modify `znicz-core/src/station.rs` | `Station.art`, `set_art`, copy keeps art |
| Modify `znicz/src/main.rs` | `znicz station art` |
| Modify `znicz-mcp/src/server.rs` | `set_station_art` |
| Modify `znicz-mcp/skills/radio-streaming/SKILL.md` | Document the tool |
| Modify `znicz-tui/src/cover.rs` | Keys, fetch, station image, pick-while-pending |
| Modify `znicz-tui/src/views/now_playing.rs` | Stream cover choice |
| Modify `znicz-tui/src/app.rs` | Radio form third field |
| Modify `znicz-tui/src/views/radio.rs` | Draw `art:` |
| Modify `znicz-tui/src/keys.rs` | `e` help text |
| Wiki + README + `Cargo.toml` version | 0.4.1 |

Every `TrackInfo { ... }` literal in the repo must include `icy_stream_url: None` until serde-default exists **and** Rust struct update syntax is used. Prefer adding the field with `#[serde(default, skip_serializing_if = "Option::is_none")]` and filling literals.

Every `Station { name, url }` literal must include `art: None` unless the test sets art.

---

### Task 1: Parse ICY `StreamUrl` and put it on `TrackInfo`

**Files:**
- Modify: `znicz-core/src/audio/icy.rs`
- Modify: `znicz-core/src/player/state.rs`
- Modify: `znicz-core/src/audio/http.rs` (`IcyStripRead::new` fourth argument only)
- Modify: every `TrackInfo {` literal (`znicz-core/src/audio/source.rs`, `znicz-core/src/audio/http.rs`, `znicz-core/src/player/state.rs` tests, `znicz-core/tests/stream.rs`, `znicz-tui/tests/render.rs`, `znicz-tui/src/views/inspector.rs`, `znicz-tui/examples/preview.rs`)

**Interfaces:**
- Consumes: existing `parse_stream_title` / `IcyStripRead` / `IcyTitle`
- Produces: `pub fn parse_stream_url(block: &[u8]) -> Option<String>`, `pub enum IcyUrl { Unset, Empty, Text(String) }` with `from_parsed`, `IcyStripRead::new(inner, metaint, title: Arc<Mutex<IcyTitle>>, url: Arc<Mutex<IcyUrl>>)`, `pub fn apply_icy_url_to_track(track: &mut TrackInfo, icy: &IcyUrl) -> bool`, `TrackInfo.icy_stream_url: Option<String>`

- [ ] **Step 1: Write the failing tests** in `znicz-core/src/audio/icy.rs` `mod tests`.

Add next to `parse_stream_title_reads_the_first_quoted_value`:

```rust
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
```

Update existing `IcyStripRead::new(..., title)` call sites in this file to pass `Arc::new(Mutex::new(IcyUrl::Unset))`.

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p znicz-core --lib --offline parse_stream_url -- --nocapture`

Expected: FAIL compiling (`parse_stream_url` / `IcyUrl` / `icy_stream_url` missing) or FAIL assertion.

- [ ] **Step 3: Write minimal implementation**

In `znicz-core/src/player/state.rs` on `TrackInfo`, after `url`:

```rust
    /// Icecast `StreamUrl` when the station sent one. Not the audio stream URL.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icy_stream_url: Option<String>,
```

Add `icy_stream_url: None` to every `TrackInfo {` in the workspace (grep).

In `znicz-core/src/audio/icy.rs`:

```rust
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
```

Change `IcyStripRead` to hold `url: Arc<Mutex<IcyUrl>>`. `new` takes that mutex. In `skip_metadata`, after a complete block:

```rust
        if got == meta_len {
            if let Some(parsed) = parse_stream_title(&meta) {
                *self.title.lock().unwrap() = IcyTitle::from_parsed(&parsed);
            }
            if let Some(parsed) = parse_stream_url(&meta) {
                *self.url.lock().unwrap() = IcyUrl::from_parsed(&parsed);
            }
        }
```

Parse URL even when title is missing.

In `znicz-core/src/audio/http.rs`, `IcyStripRead::new` currently takes three arguments. Pass a throwaway `Arc::new(Mutex::new(IcyUrl::Unset))` as the fourth so this task compiles. Task 2 replaces that with a mutex stored on `HttpStreamSource`. Update other `IcyStripRead::new` call sites the same way.

- [ ] **Step 4: Run tests**

Run: `cargo test -p znicz-core --lib --offline icy:: -- --nocapture`

Expected: PASS. Then `cargo test --offline --workspace` and fix any remaining `TrackInfo` literals.

- [ ] **Step 5: Commit**

```bash
git add znicz-core/src/audio/icy.rs znicz-core/src/player/state.rs znicz-core/src/audio/source.rs znicz-core/src/audio/http.rs znicz-core/tests/stream.rs znicz-tui/tests/render.rs znicz-tui/src/views/inspector.rs znicz-tui/examples/preview.rs
git commit -m "$(cat <<'EOF'
Parse ICY StreamUrl onto TrackInfo without fetching the image.

EOF
)"
```

---

### Task 2: Wire `StreamUrl` through HTTP, the decoder, and the engine

**Files:**
- Modify: `znicz-core/src/audio/http.rs`
- Modify: `znicz-core/src/audio/source.rs`
- Modify: `znicz-core/src/player/engine.rs`

**Interfaces:**
- Consumes: `IcyUrl`, `IcyStripRead::new(..., title, url)`, `apply_icy_url_to_track`
- Produces: `AudioSource::icy_url_slot(&self) -> Option<Arc<Mutex<IcyUrl>>>` (default `None`), `HttpStreamSource` holds `icy_url`, `AudioDecoder::icy_url(&self) -> IcyUrl`, engine copies URL when it copies `StreamTitle`

- [ ] **Step 1: Write the failing tests** in `znicz-core/src/audio/http.rs` `mod tests`.

Extend `decoder_sees_stream_title_from_icy_blocks` (keep the title asserts) and add:

```rust
    #[test]
    fn decoder_sees_stream_url_from_icy_blocks() {
        use crate::audio::icy::IcyUrl;
        use crate::audio::source::AudioDecoder;
        let wav = silent_wav_bytes(44_100, 2, 256);
        let mut body = wav[..44].to_vec();
        body.extend_from_slice(&icy_block("StreamUrl='http://127.0.0.1/cover.png';"));
        body.extend_from_slice(&wav[44..]);
        let (url, _rx) = serve_once_icy(body, "audio/wav", 44);
        let source = HttpStreamSource::new("Station", url);
        let (mut decoder, info) = AudioDecoder::open(&source).unwrap();
        assert!(info.icy_stream_url.is_none());
        let _ = decoder.decode_next();
        assert_eq!(
            decoder.icy_url(),
            IcyUrl::Text("http://127.0.0.1/cover.png".into())
        );
    }
```

In `open_reader_strips_icy_metadata_from_the_body`, after the title assert:

```rust
        assert_eq!(
            source.icy_url_slot().unwrap().lock().unwrap().clone(),
            crate::audio::icy::IcyUrl::Unset
        );
```

(That fixture has no `StreamUrl`; it must stay Unset.)

Add a strip fixture that includes both fields (same audio as the title test, block `StreamTitle='Hi';StreamUrl='http://x';`) and assert both slots.

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p znicz-core --lib --offline decoder_sees_stream_url -- --nocapture`

Expected: FAIL (`icy_url` / `icy_url_slot` missing).

- [ ] **Step 3: Write minimal implementation**

`HttpStreamSource`: add `icy_url: Arc<Mutex<IcyUrl>>` initialized `Unset`. Pass it into `IcyStripRead::new`. Implement `icy_url_slot`.

`AudioSource`: default `fn icy_url_slot(&self) -> Option<Arc<Mutex<IcyUrl>>> { None }`.

`AudioDecoder`: field `icy_url: Option<Arc<Mutex<IcyUrl>>>` from `source.icy_url_slot()` in `open`. Method:

```rust
    pub fn icy_url(&self) -> IcyUrl {
        self.icy_url
            .as_ref()
            .map(|slot| slot.lock().unwrap().clone())
            .unwrap_or(IcyUrl::Unset)
    }
```

In `engine.rs` `publish_stream_title` (keep the name), after applying title, also:

```rust
        let icy_url = decoder.icy_url();
        // ... existing title apply ...
        let url_changed = apply_icy_url_to_track(track, &icy_url);
        if title_changed || url_changed {
            drop(state);
            self.emit_state_changed();
        }
```

Refactor the existing `if apply_icy_to_track(...) { emit }` so one emit covers both. If `IcyUrl::Unset`, `apply_icy_url_to_track` is false (field unchanged).

Import `IcyUrl` / `apply_icy_url_to_track` next to the title imports.

- [ ] **Step 4: Run tests**

Run: `cargo test -p znicz-core --offline --lib icy:: http::`

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add znicz-core/src/audio/http.rs znicz-core/src/audio/source.rs znicz-core/src/player/engine.rs
git commit -m "$(cat <<'EOF'
Copy ICY StreamUrl from the HTTP reader onto the current track.

EOF
)"
```

---

### Task 3: `fetch_cover`

**Files:**
- Create: `znicz-core/src/cover_fetch.rs`
- Modify: `znicz-core/src/lib.rs` (`mod cover_fetch;` and `pub use cover_fetch::fetch_cover;`)
- Modify: `znicz-core/src/metadata.rs` only if you reuse `sniff_image_mime` — prefer `pub(crate) fn sniff_image_mime` and call it from `cover_fetch.rs`

**Interfaces:**
- Consumes: `CoverArt`, `ureq` (same 8s connect timeout as `znicz-core/src/audio/http.rs` `agent()`)
- Produces: `pub fn fetch_cover(url: &str) -> Option<CoverArt>`

- [ ] **Step 1: Write the failing tests** in `znicz-core/src/cover_fetch.rs` `mod tests`.

Copy the valid 1×1 PNG bytes from `znicz-tui/src/cover.rs` (`TINY_PNG`). Copy the `serve_once` helper from `znicz-core/src/audio/http.rs` tests (bind `127.0.0.1:0`, write status + headers + body).

```rust
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
```

`serve_once_owned` is `serve_once` that takes `Vec<u8>` instead of `'static`. HTML may be `Some` with octet-stream (TUI decode fails later) — that matches the spec. Do not require `None` for HTML in core.

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p znicz-core --lib --offline cover_fetch -- --nocapture`

Expected: FAIL (module missing).

- [ ] **Step 3: Write minimal implementation**

```rust
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
    if limited.read_to_end(&mut body).is_err() {
        return None;
    }
    if body.is_empty() || body.len() as u64 > MAX_BYTES {
        tracing::debug!(url, bytes = body.len(), "cover fetch empty or oversize");
        return None;
    }
    let mime = header_mime.unwrap_or_else(|| sniff_image_mime(&body));
    Some(CoverArt { mime, bytes: body })
}
```

Make `sniff_image_mime` `pub(crate)` in `metadata.rs`.

- [ ] **Step 4: Run tests**

Run: `cargo test -p znicz-core --lib --offline cover_fetch -- --nocapture`

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add znicz-core/src/cover_fetch.rs znicz-core/src/lib.rs znicz-core/src/metadata.rs
git commit -m "$(cat <<'EOF'
Add fetch_cover for http(s) image bytes with a 2 MiB cap.

EOF
)"
```

---

### Task 4: Station `art` in `znicz-core`

**Files:**
- Modify: `znicz-core/src/station.rs`
- Modify: `znicz-core/src/lib.rs` (`pub use station::{..., set_art as set_station_art, ...}`)

**Interfaces:**
- Consumes: existing `Station { name, url }`
- Produces: `Station.art: Option<PathBuf>`, `pub fn set_art(stations: &mut [Station], name: &str, art: Option<&str>) -> Result<()>`, `copy` clones `art`

- [ ] **Step 1: Write the failing tests** in `znicz-core/src/station.rs` `mod tests`.

Update existing `Station { name, url }` literals with `art: None`. Add:

```rust
    #[test]
    fn art_round_trips_and_copy_keeps_the_path() {
        let png = tmp(); // this helper returns stations.toml path; use its parent
        let img = png.parent().unwrap().join("logo.png");
        fs::write(&img, b"not-a-real-decode-here").unwrap();
        let mut stations = vec![Station {
            name: "A".into(),
            url: "https://a".into(),
            art: None,
        }];
        set_art(&mut stations, "A", Some(img.to_str().unwrap())).unwrap();
        assert_eq!(stations[0].art.as_deref(), Some(img.canonicalize().unwrap().as_path()));
        copy(&mut stations, "A", "B").unwrap();
        assert_eq!(stations[1].art, stations[0].art);
        set_art(&mut stations, "A", None).unwrap();
        assert!(stations[0].art.is_none());
        assert!(stations[1].art.is_some());
    }

    #[test]
    fn art_rejects_http_and_missing_file() {
        let mut stations = vec![Station {
            name: "A".into(),
            url: "https://a".into(),
            art: None,
        }];
        assert!(set_art(&mut stations, "A", Some("https://x/a.png")).is_err());
        assert!(set_art(&mut stations, "A", Some("/definitely/missing/cover.png")).is_err());
        assert!(stations[0].art.is_none());
    }
```

`fs::write` of dummy bytes is enough: save only checks the path is a file, not that it decodes.

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p znicz-core --lib --offline art_round_trips -- --nocapture`

Expected: FAIL (`set_art` / `art` missing).

- [ ] **Step 3: Write minimal implementation**

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Station {
    pub name: String,
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub art: Option<PathBuf>,
}

fn expand_tilde(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Ok(home) = std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE")) {
            return PathBuf::from(home).join(rest);
        }
    }
    PathBuf::from(path)
}

pub fn set_art(stations: &mut [Station], name: &str, art: Option<&str>) -> Result<()> {
    let station = stations
        .iter_mut()
        .find(|s| s.name == name)
        .ok_or_else(|| ZniczError::Player(format!("no station named {name}")))?;
    let Some(raw) = art.map(str::trim).filter(|s| !s.is_empty()) else {
        station.art = None;
        return Ok(());
    };
    if raw.starts_with("http://") || raw.starts_with("https://") {
        return Err(ZniczError::Player(
            "station art must be a local image file".into(),
        ));
    }
    let path = expand_tilde(raw);
    if !path.is_file() {
        return Err(ZniczError::Player(format!(
            "station art not found: {}",
            path.display()
        )));
    }
    station.art = Some(path.canonicalize().map_err(|e| {
        ZniczError::Player(format!("station art: {e}"))
    })?);
    Ok(())
}
```

`add`: `Station { name, url, art: None }`.

`copy`: clone `art` from the source after `add`, or push a full clone with the new name:

```rust
pub fn copy(stations: &mut Vec<Station>, name: &str, new_name: &str) -> Result<()> {
    let src = find(stations, name)
        .ok_or_else(|| ZniczError::Player(format!("no station named {name}")))?
        .clone();
    add(stations, new_name, &src.url)?;
    if let Some(last) = stations.last_mut() {
        last.art = src.art;
    }
    Ok(())
}
```

Re-export `set_art as set_station_art` from `lib.rs`.

- [ ] **Step 4: Run tests**

Run: `cargo test -p znicz-core --lib --offline station:: -- --nocapture`

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add znicz-core/src/station.rs znicz-core/src/lib.rs
git commit -m "$(cat <<'EOF'
Store an optional local image path on each radio station.

EOF
)"
```

---

### Task 5: CLI and MCP `set_station_art`

**Files:**
- Modify: `znicz/src/main.rs`
- Modify: `znicz-mcp/src/server.rs`
- Modify: `znicz-mcp/skills/radio-streaming/SKILL.md`

**Interfaces:**
- Consumes: `znicz_core::set_station_art`
- Produces: `znicz station art NAME PATH` and `znicz station art NAME --clear`; MCP tool `set_station_art`; `list_stations` / `znicz://stations` already serialize `Station` so `art` appears when set

- [ ] **Step 1: Write the failing MCP test** in `znicz-mcp/src/server.rs` tests, next to `add_and_list_stations_round_trip`:

```rust
    #[test]
    fn set_station_art_round_trip_and_clear() {
        let (server, path) = station_server();
        let img = path.parent().unwrap().join("cover.png");
        std::fs::write(&img, b"png").unwrap();
        server
            .add_radio_station(Parameters(StationAddParams {
                name: "Example".into(),
                url: "https://example.com/stream".into(),
            }))
            .unwrap();
        server
            .set_station_art(Parameters(StationArtParams {
                name: "Example".into(),
                path: Some(img.to_string_lossy().into_owned()),
            }))
            .unwrap();
        let listed = result_text(&server.list_stations().unwrap());
        assert!(listed.contains("cover.png"), "{listed}");
        server
            .set_station_art(Parameters(StationArtParams {
                name: "Example".into(),
                path: None,
            }))
            .unwrap();
        let listed = result_text(&server.list_stations().unwrap());
        assert!(!listed.contains("cover.png"), "{listed}");
        let _ = std::fs::remove_file(path);
        let _ = std::fs::remove_file(img);
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p znicz-mcp --offline set_station_art_round_trip -- --nocapture`

Expected: FAIL (`set_station_art` / `StationArtParams` missing).

- [ ] **Step 3: Write minimal implementation**

CLI in `StationCmd`:

```rust
    /// Set or clear a station's local cover file
    Art {
        name: String,
        path: Option<String>,
        #[arg(long)]
        clear: bool,
    },
```

Dispatch:

```rust
            StationCmd::Art { name, path, clear } => {
                if clear {
                    mutate_stations(|s| znicz_core::set_station_art(s, &name, None))?
                } else {
                    let Some(path) = path else {
                        return Err(color_eyre::eyre::eyre!(
                            "pass a path or --clear"
                        ));
                    };
                    mutate_stations(|s| znicz_core::set_station_art(s, &name, Some(&path)))?
                }
            }
```

MCP:

```rust
struct StationArtParams {
    name: String,
    #[serde(default)]
    path: Option<String>,
}

    #[tool(description = "Set a station's local cover image path; omit path to clear")]
    fn set_station_art(
        &self,
        Parameters(params): Parameters<StationArtParams>,
    ) -> Result<rmcp::model::CallToolResult, McpError> {
        self.mutate_stations(|stations| {
            znicz_core::set_station_art(stations, &params.name, params.path.as_deref())
        })
    }
```

Skill: add `set_station_art` to the edit bullet.

- [ ] **Step 4: Run tests**

Run: `cargo test -p znicz-mcp --offline set_station_art_round_trip -- --nocapture`

Expected: PASS. `cargo test -p znicz --offline --lib` if the binary crate has tests; otherwise compile with `cargo build -p znicz --offline`.

- [ ] **Step 5: Commit**

```bash
git add znicz/src/main.rs znicz-mcp/src/server.rs znicz-mcp/skills/radio-streaming/SKILL.md
git commit -m "$(cat <<'EOF'
Add CLI and MCP commands to set station cover art.

EOF
)"
```

---

### Task 6: TUI cover choice (ICY, then station art, then logo)

**Files:**
- Modify: `znicz-tui/src/cover.rs`
- Modify: `znicz-tui/src/views/now_playing.rs`

**Interfaces:**
- Consumes: `znicz_core::fetch_cover`, `znicz_core::Station.art`, `TrackInfo.path` / `url` / `icy_stream_url`
- Produces: `pub enum CoverKey { File(PathBuf), ImageFile(PathBuf), Url(String) }`, `CoverCache::get(&self, key: CoverKey) -> CoverReady`, `pub fn pick_stream_cover(icy: CoverReady, station: CoverReady) -> CoverReady`, failed-URL set inside the cache

- [ ] **Step 1: Write the failing tests** in `znicz-tui/src/cover.rs` `mod tests`.

```rust
    #[test]
    fn pick_stream_cover_prefers_icy_then_station_then_logo() {
        let icy_img = Arc::new(DynamicImage::new_rgb8(2, 2));
        let station_img = Arc::new(DynamicImage::new_rgb8(3, 3));
        let icy = CoverReady::Embedded(icy_img.clone());
        let station = CoverReady::Embedded(station_img.clone());
        assert!(matches!(
            pick_stream_cover(icy.clone(), station.clone()),
            CoverReady::Embedded(img) if Arc::ptr_eq(&img, &icy_img)
        ));
        assert!(matches!(
            pick_stream_cover(CoverReady::Pending, station.clone()),
            CoverReady::Embedded(img) if Arc::ptr_eq(&img, &station_img)
        ));
        assert!(matches!(
            pick_stream_cover(CoverReady::Logo, station),
            CoverReady::Embedded(img) if Arc::ptr_eq(&img, &station_img)
        ));
        assert_eq!(
            pick_stream_cover(CoverReady::Logo, CoverReady::Logo),
            CoverReady::Logo
        );
        assert_eq!(
            pick_stream_cover(CoverReady::Pending, CoverReady::Logo),
            CoverReady::Logo
        );
    }
```

`PartialEq` for `Embedded` is pointer equality today — the `matches!` + `ptr_eq` form is required.

For failed-URL once: start a loopback server that counts GETs and returns HTML. `get(CoverKey::Url)` twice after worker settles → one GET. (Spawn `TcpListener`, serve HTML once, second accept would hang — count with `AtomicUsize` and close after first. Simpler: unit-test a `failed: HashSet` by calling an internal `record_failure` if you extract it; otherwise one integration-style test with `serve_once` HTML and two `get` after 100×10ms wait, asserting `Logo` both times. A second GET would need a second server connection — `serve_once` dies after one request, so a second `fetch_cover` would fail anyway. **Do not** rely on that. Instead test the set directly:

```rust
    #[test]
    fn a_failed_url_is_not_fetched_again() {
        // After Logo for a Url key, a second get must not send another request.
        // Use a listener that increments a counter and always returns HTML.
        let hits = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let (url, handle) = serve_counting_html(hits.clone());
        let cache = CoverCache::new();
        let key = CoverKey::Url(url);
        let mut ready = CoverReady::Pending;
        for _ in 0..100 {
            std::thread::sleep(Duration::from_millis(10));
            ready = cache.get(key.clone());
            if ready != CoverReady::Pending {
                break;
            }
        }
        assert_eq!(ready, CoverReady::Logo);
        let after_first = hits.load(std::sync::atomic::Ordering::SeqCst);
        assert!(after_first >= 1);
        let _ = cache.get(key);
        std::thread::sleep(Duration::from_millis(50));
        assert_eq!(hits.load(std::sync::atomic::Ordering::SeqCst), after_first);
        handle.join().ok();
    }
```

`serve_counting_html` loops accepting until the test process ends, or accept twice. Join with a timeout by dropping the listener (bind, spawn, return url + join handle; test drops by `let _ = TcpStream::connect` not needed if the worker only hits once).

Also: `CoverCache::get` currently takes `Option<&Path>`. Tests `no_path_is_the_logo` must switch to a dedicated `get_logo` or `get` with no key. Keep `get` requiring a `CoverKey`. Change `no_path_is_the_logo` to assert `pick_stream_cover(CoverReady::Logo, CoverReady::Logo) == Logo`. Keep `missing_file_resolves_to_logo` with `CoverKey::File`.

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p znicz-tui --lib --offline pick_stream_cover -- --nocapture`

Expected: FAIL (`pick_stream_cover` missing).

- [ ] **Step 3: Write minimal implementation**

```rust
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum CoverKey {
    File(PathBuf),
    ImageFile(PathBuf),
    Url(String),
}

pub fn pick_stream_cover(icy: CoverReady, station: CoverReady) -> CoverReady {
    if let CoverReady::Embedded(_) = &icy {
        return icy;
    }
    if let CoverReady::Embedded(_) = &station {
        return station;
    }
    CoverReady::Logo
}
```

Cache map keys become `CoverKey`. Worker:

- `File` → `read_cover` + `decode_capped` (existing)
- `ImageFile` → `std::fs::read` + `decode_capped`; missing/bad → `Logo`
- `Url` → if in `failed: HashSet<String>`, `Logo`; else `fetch_cover` then `decode_capped`; `None` or decode fail → insert URL into `failed`, `Logo`

`get(&self, key: CoverKey) -> CoverReady` same pending/insert/send pattern as today. Include `failed` in the mutex tuple: `(HashMap<CoverKey, Slot>, VecDeque<CoverKey>, HashSet<String>)`.

In `now_playing.rs` `render_cover`:

If `cover_protocol == Off` or no current track: logo (unchanged).

If `track.path` is `Some(p)`: `get(CoverKey::File(p))` as today.

Else (stream):

```rust
        let icy_ready = match track.icy_stream_url.as_deref() {
            Some(u) if u.starts_with("http://") || u.starts_with("https://") => {
                app.covers.get(CoverKey::Url(u.to_string()))
            }
            _ => CoverReady::Logo,
        };
        let station_ready = app
            .stations
            .iter()
            .find(|s| Some(s.url.as_str()) == track.url.as_deref())
            .and_then(|s| s.art.clone())
            .map(|p| app.covers.get(CoverKey::ImageFile(p)))
            .unwrap_or(CoverReady::Logo);
        let ready = crate::cover::pick_stream_cover(icy_ready, station_ready);
```

Draw `ready` (Pending treated as Logo **only after** pick — pick already prefers station Embedded). Set `cover_draw_key` from a label of `ready` (pointer for Embedded, `"logo"` for Logo, `"pending-station"` if you still have Pending after pick — should be rare). Do not key as `"logo"` while station Embedded is showing.

`App` already loads `stations`.

- [ ] **Step 4: Run tests**

Run: `cargo test -p znicz-tui --offline`

Expected: PASS (including render tests).

- [ ] **Step 5: Commit**

```bash
git add znicz-tui/src/cover.rs znicz-tui/src/views/now_playing.rs
git commit -m "$(cat <<'EOF'
Pick stream covers from ICY images, then station art, then the logo.

EOF
)"
```

---

### Task 7: Radio form, wiki, version 0.4.1

**Files:**
- Modify: `znicz-tui/src/app.rs`
- Modify: `znicz-tui/src/views/radio.rs`
- Modify: `znicz-tui/src/keys.rs`
- Modify: `Cargo.toml` workspace version `0.4.0` → `0.4.1`
- Modify: `wiki/Plans/Phase-5-Album-Art.md`
- Modify: `wiki/Architecture/TUI.md` (Radio pane)
- Modify: `wiki/Domain/Formats-and-Metadata.md` (radio section)
- Modify: `wiki/Architecture/MCP.md`
- Modify: `wiki/Plans/Roadmap.md` (one line under Phase 5)
- Modify: `README.md`

**Interfaces:**
- Consumes: `Station.art`, `set_station_art`, `RadioPrompt::Form`
- Produces: third field `art:`; Tab cycles name → url → art; empty art allowed; http art rejected by `set_art`; version 0.4.1

- [ ] **Step 1: Write the failing tests** in `znicz-tui/src/app.rs` under `#[cfg(test)]` (create the module if this file has none).

```rust
    #[test]
    fn radio_form_tab_cycles_name_url_art() {
        let mut prompt = RadioPrompt::new_station();
        assert!(matches!(
            prompt,
            RadioPrompt::Form {
                field: StationField::Name,
                ..
            }
        ));
        prompt.cycle_field(true);
        assert!(matches!(
            prompt,
            RadioPrompt::Form {
                field: StationField::Url,
                ..
            }
        ));
        prompt.cycle_field(true);
        assert!(matches!(
            prompt,
            RadioPrompt::Form {
                field: StationField::Art,
                ..
            }
        ));
        prompt.cycle_field(true);
        assert!(matches!(
            prompt,
            RadioPrompt::Form {
                field: StationField::Name,
                ..
            }
        ));
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p znicz-tui --lib --offline radio_form_tab_cycles -- --nocapture`

Expected: FAIL (`cycle_field` / `StationField::Art` missing).

- [ ] **Step 3: Write minimal implementation**

`StationField { Name, Url, Art }`.

`RadioPrompt::Form` adds `art: LineEdit`.

`new_station`: empty art. `edit_station`: `LineEdit::from_text(station.art.as_ref().map(|p| p.display().to_string()).unwrap_or_default())`.

`buffer_mut` / `cycle_field(forward: bool)`: Name ↔ Url ↔ Art.

Replace `focus_url` Tab handlers with `cycle_field`.

`confirm_radio_prompt`: after successful `add_station` / `update_station`, call `set_station_art` with `None` if art buffer empty, else `Some(art.as_str())`. If `set_art` errors, restore the form and toast (do not persist a half-written add: if add succeeded and art failed, `set_art` error should still persist the station **without** art only when art was empty; when art was non-empty and invalid, toast and keep the form — if add already happened, `original` should become `Some(name)` so retry is an update). Simplest correct behavior: validate art **before** add/update (empty OK; non-empty must be a local existing file, same rules as `set_art` on a dummy one-element slice) OR: add/update first, then `set_art`; on `set_art` error toast, set `original` to the new name, restore form including art field. Implement that second path.

`views/radio.rs`: `form_rows` is 3 for Form. Draw `art:` like url. Hint already says Tab field.

`keys.rs`: `b("e", "edit name, URL, and art")`.

Wiki / README / version:

- `Cargo.toml`: `version = "0.4.1"`
- Phase-5-Album-Art: check **Radio / stream covers**; leave disk cache and `cover://current` unchecked; mention station `art` + ICY `StreamUrl`
- TUI.md Radio: form has art; cover slot uses ICY image then station file then logo
- Formats-and-metadata radio: `art` optional local path; ICY `StreamUrl` may be a cover image
- MCP.md: `set_station_art`
- Roadmap Phase 5: station art + ICY image URL
- README radio examples: `znicz station art "Example" ~/Pictures/example.png`; transport sentence: streams use station art or ICY image, else the logo

- [ ] **Step 4: Run tests**

Run: `cargo fmt --all && cargo test --workspace --offline && cargo clippy --workspace --all-targets --offline -- -D warnings`

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add znicz-tui/src/app.rs znicz-tui/src/views/radio.rs znicz-tui/src/keys.rs Cargo.toml wiki README.md
git commit -m "$(cat <<'EOF'
Add station art to the radio form and ship stream covers as 0.4.1.

EOF
)"
```

---

## Spec coverage

| Spec item | Task |
| --- | --- |
| Parse `StreamUrl` with/without `StreamTitle` | 1 |
| Strip-read keeps audio with both fields | 1 |
| `TrackInfo.icy_stream_url` serde optional | 1 |
| Engine copies / clears URL | 1–2 |
| `fetch_cover` http only, 8s, 2 MiB, loopback PNG | 3 |
| Failed URL not fetched again | 6 |
| Station `art` save/load, reject http, missing file | 4 |
| Copy keeps art; clear removes | 4 |
| CLI + MCP | 5 |
| ICY image wins; else art; else logo; pending keeps station | 6 |
| TUI third field | 7 |
| Wiki, keys help, 0.4.1 | 7 |
| No JPEG on IPC; no player-thread decode | 1–3, 6 |
| Not disk cache / `cover://` / folder.jpg / MusicBrainz | none |
