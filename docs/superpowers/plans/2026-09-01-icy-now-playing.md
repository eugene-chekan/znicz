# ICY Now Playing Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Request Icecast metadata, strip it from the audio body, and show `StreamTitle` on now-playing (`TrackInfo.title` and `tags.title`) while queue rows stay the station name.

**Architecture:** `znicz-core/src/audio/icy.rs` parses `StreamTitle` and strips `icy-metaint` blocks from a `Read`. `HttpStreamSource` sends `Icy-MetaData: 1`, wraps the body when `icy-metaint` is set, and shares `Arc<Mutex<IcyTitle>>` with `AudioDecoder`. The engine copies that title on each decode pump (same place as coded bitrate) and emits `StateChanged` only when title fields change.

**Tech Stack:** Existing Rust workspace (`znicz-core`, wiki, MCP skill). ureq 3 `.header()` / `response.headers()`. No new crates.

**Spec:** `docs/superpowers/specs/2026-09-01-icy-now-playing-design.md`

## Global Constraints

- Workspace version **0.3.6 → 0.3.7** in the same development cycle (Task 4).
- Every stream GET sends `Icy-MetaData: 1`. No extra User-Agent. Do not use `icy-br` or `StreamUrl`.
- No artist/title split. Whole `StreamTitle` → `title` and `tags.title`.
- Empty `StreamTitle` restores the queue row’s station name and clears `tags.title`.
- Unreadable metadata: strip bytes, keep audio, do not change the stored title.
- Missing or `0` `icy-metaint`: audio-only body; title stays the station name.
- Queue rows and `session.toml` stay the station name, not the live song.
- Tests use **loopback only**. No public radio.
- No HLS, PLS, XSPF, Phase 5, parked TUI `#5`–`#9` / `#22`, playlist `#18` / `#19`.
- Wiki and `znicz-mcp/skills/radio-streaming/SKILL.md` update in the same change as the behaviour.

---

## File map

| File | Responsibility |
| --- | --- |
| Create `znicz-core/src/audio/icy.rs` | `IcyTitle`, `parse_stream_title`, `IcyStripRead`, `apply_icy_to_track` |
| Modify `znicz-core/src/audio/mod.rs` | `pub mod icy;` |
| Modify `znicz-core/src/audio/http.rs` | Header, `icy-metaint` wrap, shared title slot |
| Modify `znicz-core/src/audio/source.rs` | `icy_title_slot` on `AudioSource`; `AudioDecoder::icy_title()` |
| Modify `znicz-core/src/player/engine.rs` | Copy title on pump; `StateChanged` when it changes |
| Modify wiki + skill + `Cargo.toml` + `wiki/Rust/Cargo-Workspace.md` | 0.3.7, ICY in Phase 4 done |

Shared types (Task 1; later tasks use these names):

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IcyTitle {
    Unset,
    Empty,
    Text(String),
}

impl IcyTitle {
    pub fn from_parsed(title: &str) -> Self {
        if title.is_empty() {
            Self::Empty
        } else {
            Self::Text(title.to_string())
        }
    }
}

/// First `StreamTitle='…';` in the block. `None` if that pattern is missing.
pub fn parse_stream_title(block: &[u8]) -> Option<String> {
    let text = String::from_utf8_lossy(block);
    let rest = text.split("StreamTitle='").nth(1)?;
    let (title, _) = rest.split_once("';")?;
    Some(title.to_string())
}

/// Returns true when `track` was written.
pub fn apply_icy_to_track(
    track: &mut crate::player::state::TrackInfo,
    icy: &IcyTitle,
    station_name: &str,
) -> bool {
    match icy {
        IcyTitle::Unset => false,
        IcyTitle::Text(song) => {
            let changed =
                track.title != *song || track.tags.title.as_deref() != Some(song.as_str());
            if changed {
                track.title = song.clone();
                track.tags.title = Some(song.clone());
            }
            changed
        }
        IcyTitle::Empty => {
            let changed = track.title != station_name || track.tags.title.is_some();
            if changed {
                track.title = station_name.to_string();
                track.tags.title = None;
            }
            changed
        }
    }
}
```

`IcyStripRead` (Task 1): `Read` wrapper. After every `metaint` audio bytes, read one length byte `L`, then `L * 16` metadata bytes. Parse with `parse_stream_title`; on `Some`, store `IcyTitle::from_parsed`. Truncated or unparseable blocks: drop bytes, leave the mutex as-is. `L == 0` is a no-op interval.

---

### Task 1: Parse and strip ICY in isolation

**Files:**
- Create: `znicz-core/src/audio/icy.rs`
- Modify: `znicz-core/src/audio/mod.rs`

**Interfaces:**
- Consumes: none
- Produces: `IcyTitle`, `parse_stream_title(&[u8]) -> Option<String>`, `IcyStripRead`, `apply_icy_to_track`

- [ ] **Step 1: Write the failing tests**

In `icy.rs` under `#[cfg(test)]`:

```rust
#[test]
fn parse_stream_title_reads_the_first_quoted_value() {
    assert_eq!(
        parse_stream_title(b"StreamTitle='Song';StreamUrl='http://x';"),
        Some("Song".into())
    );
    assert_eq!(parse_stream_title(b"StreamTitle='Artist - Track';"), Some("Artist - Track".into()));
    assert_eq!(parse_stream_title(b"StreamTitle='';"), Some("".into()));
    assert_eq!(parse_stream_title(b"StreamUrl='x';"), None);
    assert_eq!(parse_stream_title(b"StreamTitle='open"), None);
    assert_eq!(parse_stream_title(b"junk"), None);
}

fn icy_block(payload: &str) -> Vec<u8> {
    let mut bytes = payload.as_bytes().to_vec();
    let padded = bytes.len().div_ceil(16) * 16;
    bytes.resize(padded, 0);
    let mut out = vec![(padded / 16) as u8];
    out.extend(bytes);
    out
}

#[test]
fn strip_read_drops_metadata_and_keeps_audio() {
    let audio = b"ABCDEFGHIJKLMNOPQRSTUVWX".to_vec(); // 24 bytes
    let block = icy_block("StreamTitle='Song';");
    let mut body = Vec::new();
    body.extend_from_slice(&audio[..16]);
    body.extend_from_slice(&block);
    body.extend_from_slice(&audio[16..]);
    let title = std::sync::Arc::new(std::sync::Mutex::new(IcyTitle::Unset));
    let mut reader = IcyStripRead::new(std::io::Cursor::new(body), 16, title.clone());
    let mut out = Vec::new();
    reader.read_to_end(&mut out).unwrap();
    assert_eq!(out, audio);
    assert_eq!(*title.lock().unwrap(), IcyTitle::Text("Song".into()));
}

#[test]
fn empty_stream_title_is_empty_state() {
    let audio = b"0123456789abcdef".to_vec();
    let block = icy_block("StreamTitle='';");
    let mut body = audio.clone();
    body.extend_from_slice(&block);
    let title = std::sync::Arc::new(std::sync::Mutex::new(IcyTitle::Unset));
    let mut reader = IcyStripRead::new(std::io::Cursor::new(body), 16, title.clone());
    let mut out = Vec::new();
    reader.read_to_end(&mut out).unwrap();
    assert_eq!(out, audio);
    assert_eq!(*title.lock().unwrap(), IcyTitle::Empty);
}

#[test]
fn junk_metadata_does_not_change_title() {
    let audio = b"0123456789abcdef".to_vec();
    let block = icy_block("not-stream-title");
    let mut body = audio.clone();
    body.extend_from_slice(&block);
    let title = std::sync::Arc::new(std::sync::Mutex::new(IcyTitle::Text("Keep".into())));
    let mut reader = IcyStripRead::new(std::io::Cursor::new(body), 16, title.clone());
    let mut out = Vec::new();
    reader.read_to_end(&mut out).unwrap();
    assert_eq!(out, audio);
    assert_eq!(*title.lock().unwrap(), IcyTitle::Text("Keep".into()));
}

#[test]
fn apply_icy_to_track_text_empty_and_unset() {
    let mut track = crate::player::state::TrackInfo {
        path: None,
        url: Some("http://x".into()),
        title: "Station".into(),
        codec: "MP3".into(),
        sample_rate: 44100,
        channels: 2,
        bits_per_sample: None,
        bitrate_kbps: None,
        duration: None,
        tags: Default::default(),
    };
    assert!(!apply_icy_to_track(&mut track, &IcyTitle::Unset, "Station"));
    assert_eq!(track.title, "Station");
    assert!(track.tags.title.is_none());

    assert!(apply_icy_to_track(
        &mut track,
        &IcyTitle::Text("Song".into()),
        "Station"
    ));
    assert_eq!(track.title, "Song");
    assert_eq!(track.tags.title.as_deref(), Some("Song"));
    assert!(!apply_icy_to_track(
        &mut track,
        &IcyTitle::Text("Song".into()),
        "Station"
    ));

    assert!(apply_icy_to_track(&mut track, &IcyTitle::Empty, "Station"));
    assert_eq!(track.title, "Station");
    assert!(track.tags.title.is_none());
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p znicz-core --lib audio::icy -- --nocapture`

Expected: compile fail (`mod icy` missing) or FAIL (types missing).

- [ ] **Step 3: Write minimal implementation**

`znicz-core/src/audio/mod.rs`: add `pub mod icy;`

`icy.rs`: types from the file map, plus:

```rust
use std::io::{self, Read};
use std::sync::{Arc, Mutex};

pub struct IcyStripRead<R> {
    inner: R,
    metaint: usize,
    audio_left: usize,
    title: Arc<Mutex<IcyTitle>>,
}

impl<R: Read> IcyStripRead<R> {
    pub fn new(inner: R, metaint: usize, title: Arc<Mutex<IcyTitle>>) -> Self {
        Self {
            inner,
            metaint,
            audio_left: metaint,
            title,
        }
    }

    fn skip_metadata(&mut self) -> io::Result<bool> {
        let mut len_buf = [0u8; 1];
        let n = self.inner.read(&mut len_buf)?;
        if n == 0 {
            return Ok(false);
        }
        let meta_len = usize::from(len_buf[0]) * 16;
        if meta_len == 0 {
            return Ok(true);
        }
        let mut meta = vec![0u8; meta_len];
        let mut got = 0;
        while got < meta_len {
            let n = self.inner.read(&mut meta[got..])?;
            if n == 0 {
                break;
            }
            got += n;
        }
        if got == meta_len {
            if let Some(parsed) = parse_stream_title(&meta) {
                *self.title.lock().unwrap() = IcyTitle::from_parsed(&parsed);
            }
        }
        Ok(true)
    }
}

impl<R: Read> Read for IcyStripRead<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        let mut written = 0;
        while written < buf.len() {
            if self.audio_left == 0 {
                if !self.skip_metadata()? {
                    break;
                }
                self.audio_left = self.metaint;
                continue;
            }
            let want = (buf.len() - written).min(self.audio_left);
            let n = self.inner.read(&mut buf[written..written + want])?;
            if n == 0 {
                break;
            }
            self.audio_left -= n;
            written += n;
        }
        Ok(written)
    }
}
```

If `div_ceil` is an issue on the test helper, use `(len + 15) / 16 * 16`.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p znicz-core --lib audio::icy`

Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add znicz-core/src/audio/icy.rs znicz-core/src/audio/mod.rs
git commit -m "$(cat <<'EOF'
Parse and strip Icecast StreamTitle from a Read.

EOF
)"
```

---

### Task 2: Request ICY on HTTP and expose it from the decoder

**Files:**
- Modify: `znicz-core/src/audio/http.rs`
- Modify: `znicz-core/src/audio/source.rs` (`AudioSource`, `AudioDecoder`, `MockStreamSource` uses the default)

**Interfaces:**
- Consumes: `IcyTitle`, `IcyStripRead` from Task 1
- Produces: GET includes `Icy-MetaData: 1`. `AudioSource::icy_title_slot() -> Option<Arc<Mutex<IcyTitle>>>` (default `None`). `HttpStreamSource` always has a slot. `AudioDecoder::icy_title() -> IcyTitle`. Wrap body when `icy-metaint` parses to `N > 0`.

ureq 3:

```rust
let response = agent()
    .get(&self.url)
    .header("Icy-MetaData", "1")
    .call()
    ...
let metaint = icy_metaint(response.headers());
let reader = response.into_body().into_reader();
```

```rust
fn icy_metaint(headers: &http::HeaderMap) -> Option<usize> {
    let value = headers.get("icy-metaint")?;
    let n: usize = value.to_str().ok()?.trim().parse().ok()?;
    (n > 0).then_some(n)
}
```

`HttpStreamSource` gains `icy_title: Arc<Mutex<IcyTitle>>` created as `Unset` in `new()`. `open_reader` clones it into `IcyStripRead` when wrapping.

```rust
fn icy_title_slot(&self) -> Option<Arc<Mutex<IcyTitle>>> {
    Some(self.icy_title.clone())
}
```

`AudioDecoder::open`: `let icy_title = source.icy_title_slot();` then open the reader. Store `icy_title` on `Self`.

```rust
pub fn icy_title(&self) -> IcyTitle {
    self.icy_title
        .as_ref()
        .map(|slot| slot.lock().unwrap().clone())
        .unwrap_or(IcyTitle::Unset)
}
```

- [ ] **Step 1: Write the failing tests**

In `http.rs` tests, change `open_reader_returns_the_http_body` and `a_non_audio_body_fails_to_decode` so the captured GET **does** contain `icy-metadata` (lowercase check, same as today).

Add a loopback helper that can send `icy-metaint` (copy `serve_once`, extra header line). Reuse `silent_wav_bytes` logic or a tiny `b"hello-stream"` body for strip-at-HTTP-layer:

```rust
#[test]
fn open_reader_strips_icy_metadata_from_the_body() {
    let audio = b"ABCDEFGHIJKLMNOPQRSTUVWX";
    let mut payload = audio[..16].to_vec();
    // 16-byte padded StreamTitle='Hi';  L=1
    let mut meta = b"StreamTitle='Hi';".to_vec();
    meta.resize(16, 0);
    payload.push(1);
    payload.extend_from_slice(&meta);
    payload.extend_from_slice(&audio[16..]);
    let (url, rx) = serve_once_icy(payload, "application/octet-stream", 16);
    let source = HttpStreamSource::new("Test", url);
    let mut reader = source.open_reader().unwrap();
    let mut buf = Vec::new();
    reader.read_to_end(&mut buf).unwrap();
    assert_eq!(buf, audio);
    assert_eq!(source.icy_title_slot().unwrap().lock().unwrap().clone(), IcyTitle::Text("Hi".into()));
    let req = rx.recv().unwrap();
    assert!(req.to_lowercase().contains("icy-metadata"));
}

#[test]
fn no_metaint_keeps_the_body_and_unset_title() {
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
```

`serve_once` currently returns `(String, Receiver<String>)` — keep that. Add `serve_once_icy(body, content_type, metaint: usize)`.

In `source.rs` tests or `http.rs`, WAV + `icy-metaint: 44` so probe still sees a header then PCM (metadata spliced after the 44-byte WAV header). After `AudioDecoder::open` + `decode_next`, `decoder.icy_title()` is `Text("Song")`.

WAV splice: take `silent_wav_bytes(44_100, 2, 256)`, insert one ICY block after byte 44, serve with `icy-metaint: 44`. Stripper restores a valid WAV.

```rust
#[test]
fn decoder_sees_stream_title_from_icy_blocks() {
    let wav = silent_wav_bytes_local(); // duplicate the 44_100 helper used in source tests, or put a shared helper
    let block = /* StreamTitle='Song'; padded */;
    let mut body = wav[..44].to_vec();
    body.extend_from_slice(&block);
    body.extend_from_slice(&wav[44..]);
    let url = serve_wav_with_metaint(body, 44);
    let source = HttpStreamSource::new("Station", url);
    let (mut decoder, info) = AudioDecoder::open(&source).unwrap();
    assert_eq!(info.title, "Station");
    let _ = decoder.decode_next();
    assert_eq!(decoder.icy_title(), IcyTitle::Text("Song".into()));
}
```

If probe already consumes past byte 44, `icy_title()` may be set at `open` before `decode_next` — that still passes.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p znicz-core --lib audio::http audio::source -- --nocapture`

Expected: FAIL (header still absent / no strip / `icy_title` missing).

- [ ] **Step 3: Write minimal implementation**

Wire header, `icy_metaint`, wrap, trait method, decoder field.

Do not change the engine yet.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p znicz-core --lib audio::http audio::icy`
Run: `cargo test -p znicz-core --test stream`

Expected: PASS (stream tests still decode; they now send `Icy-MetaData: 1` to loopback).

- [ ] **Step 5: Commit**

```bash
git add znicz-core/src/audio/http.rs znicz-core/src/audio/source.rs
git commit -m "$(cat <<'EOF'
Request Icecast metadata and expose StreamTitle from the decoder.

EOF
)"
```

---

### Task 3: Copy ICY title onto `TrackInfo` in the engine

**Files:**
- Modify: `znicz-core/src/player/engine.rs`

**Interfaces:**
- Consumes: `AudioDecoder::icy_title()`, `apply_icy_to_track`, queue row `QueueItem::Stream { name, .. }`
- Produces: On each decode pump (next to bitrate), apply ICY to `current_track`. `StateChanged` only when `apply_icy_to_track` returns true. Bitrate stays quiet.

```rust
fn publish_stream_title(&self, decoder: &AudioDecoder) {
    let icy = decoder.icy_title();
    if matches!(icy, crate::audio::icy::IcyTitle::Unset) {
        return;
    }
    let station_name = {
        let state = self.state.read().unwrap();
        match state.queue.get(state.queue_position) {
            Some(QueueItem::Stream { name, .. }) => name.clone(),
            _ => return,
        }
    };
    let mut state = self.state.write().unwrap();
    let Some(track) = state.current_track.as_mut() else {
        return;
    };
    if crate::audio::icy::apply_icy_to_track(track, &icy, &station_name) {
        drop(state);
        self.emit_state_changed();
    }
}
```

Call from both `PumpOutcome::SinkFull` and `PumpOutcome::Finished` after `publish_stream_bitrate`.

- [ ] **Step 1: Write the failing test**

`apply_icy_to_track` already covers the table. Add an engine-facing unit test in `engine.rs` `#[cfg(test)]` **or** extend `znicz-core/tests/stream.rs` only if a decoder+state helper is easy without cpal.

Prefer: keep Task 1’s `apply_icy_to_track` tests as the behaviour lock. Add a small `#[cfg(test)]` in `engine.rs` that constructs a `TrackInfo` + fake apply through `publish` if that needs `Engine` internals.

If `Engine` is not unit-testable without spawn, do **not** add a hardware test. After wiring, add this decoder-level assertion in `http.rs` / stream test that documents the mapping the engine uses (already in Task 1). Task 3 verification is: `cargo test -p znicz-core` plus a grep that `publish_stream_title` is called from both pump arms.

Optional loopback without device: not required.

- [ ] **Step 2: Run existing tests (baseline)**

Run: `cargo test -p znicz-core --lib audio::icy`

Expected: PASS (unchanged).

- [ ] **Step 3: Write minimal implementation**

Add `publish_stream_title` and call it from the two pump arms.

- [ ] **Step 4: Run tests**

Run: `cargo test -p znicz-core`

Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add znicz-core/src/player/engine.rs
git commit -m "$(cat <<'EOF'
Copy Icecast StreamTitle onto now-playing during decode.

EOF
)"
```

---

### Task 4: Wiki, skill, version 0.3.7

**Files:**
- Modify: `Cargo.toml` (`[workspace.package] version` → `0.3.7`)
- Modify: `wiki/Rust/Cargo-Workspace.md` (currently **0.3.6**)
- Modify: `wiki/Domain/Formats-and-Metadata.md` (radio: ICY now playing; Later = HLS, PLS, XSPF only)
- Modify: `wiki/Domain/Playback-Pipeline.md` (Radio HTTP: request metadata, strip, title)
- Modify: `wiki/Architecture/TUI.md` (transport: song title when ICY present; queue stays station name)
- Modify: `wiki/Plans/Roadmap.md` (ICY bullet under Phase 4 done; delete from Later radio)
- Modify: `wiki/Issues.md` (later radio list without ICY)
- Modify: `znicz-mcp/skills/radio-streaming/SKILL.md`

**Interfaces:**
- Consumes: shipped behaviour from Tasks 1–3
- Produces: docs that match the code. Version **0.3.7**. No invented HLS/PLS/XSPF.

Wiki copy (simple English):

Formats radio, replace the Later sentence with: Icecast `StreamTitle` replaces the now-playing title (and `tags.title`) when the station sends it. Empty title falls back to the station name. Queue rows stay the station name. **Later:** HLS, PLS, XSPF.

Playback-Pipeline Radio: Every stream GET sends `Icy-MetaData: 1`. If the reply has `icy-metaint`, metadata is stripped so decode only sees audio. A non-empty `StreamTitle` updates now-playing; empty restores the station name. `icy-br` is still unused.

TUI radio: Transport shows `StreamTitle` when present, otherwise the station name. The queue row stays the station name.

Roadmap Phase 4 done: add a bullet for ICY now playing. Later radio: delete the ICY bullet; keep HLS, PLS, XSPF.

Issues: `later radio (HLS, PLS, XSPF)`.

Skill: drop “does not parse ICY titles”. Say `get_player_state` / now-playing `title` and `tags.title` follow `StreamTitle` when the station sends it.

- [ ] **Step 1: Edit wiki, skill, version**

- [ ] **Step 2: Run tests**

Run: `cargo test --workspace`

Expected: PASS

- [ ] **Step 3: Commit**

```bash
git add Cargo.toml wiki znicz-mcp/skills/radio-streaming/SKILL.md
git commit -m "$(cat <<'EOF'
Document Icecast now-playing and bump to 0.3.7.

EOF
)"
```

---

## Self-review

| Spec requirement | Task |
| --- | --- |
| GET `Icy-MetaData: 1` | 2 |
| Strip `icy-metaint` blocks | 1, 2 |
| `StreamTitle` → `title` + `tags.title` | 1, 3 |
| Empty → station name, clear `tags.title` | 1, 3 |
| Unreadable: strip, no title change | 1 |
| No metaint: audio only, station name | 2 |
| Queue / session unchanged | 3 (no session writes of live title); wiki |
| `StateChanged` on title change only | 3 |
| Loopback tests | 1, 2 |
| Wiki + 0.3.7 | 4 |
| No HLS / `icy-br` / `StreamUrl` / artist split | constraints |
