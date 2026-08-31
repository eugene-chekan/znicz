# Radio Streams Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Play HTTP(S) Icecast-style byte streams from a `stations.toml` list, with the same motion from the TUI (`R`), CLI (`znicz station`), and MCP.

**Architecture:** Stations are a TOML file next to playlists (not SQLite). The queue becomes `Vec<QueueItem>` (`File` or `Stream`). The engine opens `HttpStreamSource` through the existing `AudioSource` / Symphonia path. Playing a station clears the queue and starts that one stream.

**Tech Stack:** Rust workspace, ureq 3 (blocking HTTP, rustls, redirects), toml 0.8, Symphonia `MediaSource` (unseekable body), clap, rmcp, ratatui overlay patterned on playlists.

**Spec:** `docs/superpowers/specs/2026-08-31-radio-streams-design.md`

## Global Constraints

- Workspace version **0.2.0 → 0.3.0** in the same development cycle (Task 8).
- Icecast **HTTP(S) byte streams only**. No HLS, no ICY `StreamTitle`, no M3U `http://` playback, no mixed-queue product behaviour.
- Stations live in **`stations.toml`**. Override **`ZNICZ_STATIONS_PATH`**. Do not change SQLite schema or `config.toml`.
- Keymap: **`R`** radio overlay, **`r`** reload the surface in front, **`e`** repeat (moved off `r`). Shuffle stays **`z`**.
- Do **not** send `Icy-MetaData: 1`. Request the body as audio.
- HTTP client: **ureq 3**, default rustls. **Connect timeout 8s**. **No global timeout** (a live stream must not die after 30s). Optional **read timeout 30s** for a stalled socket.
- Tests use a **loopback HTTP fixture**, never a public station. Keep skipping hardware playback when `CI` is set.
- Wiki, README, and `znicz-tui/src/keys.rs` stay in sync in the same change as the behaviour.
- Parked TUI issues `#5`–`#9`, Phase 5, and Phase 6 stay untouched.

---

## File map

| File | Responsibility |
| --- | --- |
| Create `znicz-core/src/station.rs` | Load/save `stations.toml`, name/URL rules, `play_station` helper |
| Modify `znicz-core/src/lib.rs` | `pub mod station` and re-exports |
| Modify `znicz-core/Cargo.toml` | `toml`, `serde` already; add `ureq` |
| Modify `Cargo.toml` | `ureq = "3"` under `[workspace.dependencies]` |
| Modify `znicz-library/src/lib.rs` | `default_stations_path()` |
| Modify `znicz-core/src/player/state.rs` | `QueueItem`; `TrackInfo.path`/`url`; `PlayerState.queue` |
| Modify `znicz-core/src/player/commands.rs` | `Play(QueueItem)`, `QueueAdd(Vec<QueueItem>)` |
| Modify `znicz-core/src/playlist.rs` | Map files to `QueueItem`; `m3u_paths` for save |
| Create `znicz-core/src/audio/http.rs` | `HttpStreamSource`, unseekable `MediaSource` |
| Modify `znicz-core/src/audio/mod.rs` | `pub mod http` |
| Modify `znicz-core/src/audio/source.rs` | `AudioSource` + `AudioDecoder::open` from `MediaSource` |
| Modify `znicz-core/src/player/engine.rs` | Play streams, seek error, skip decode while paused |
| Modify tests/examples that use `Command::Play` / `QueueAdd` / `TrackInfo` / `queue: Vec<PathBuf>` | Compile |
| Modify `znicz-tui/src/app.rs` | `Modal::Radio`, keys, prompts, play/CRUD |
| Create `znicz-tui/src/views/radio.rs` | Overlay |
| Modify `znicz-tui/src/views/mod.rs`, `help.rs`, `status.rs`, `queue.rs`, `now_playing.rs`, `inspector.rs`, `keys.rs` | Draw + keymap |
| Modify `znicz-tui/tests/keys.rs`, `tests/render.rs`, `examples/preview.rs` | Bindings and overlay |
| Modify `znicz/src/main.rs` | `station` subcommand |
| Modify `znicz-mcp/src/server.rs` | Tools, `znicz://stations`, player-state JSON |
| Modify `znicz-mcp/skills/radio-streaming/SKILL.md` | Real workflow |
| Modify wiki + README + `Cargo.toml` version | wiki-sync, 0.3.0 |

Shared play helper (TUI, CLI, MCP) in `znicz-core::station`:

```rust
pub fn play_station(player: &PlayerHandle, station: &Station) -> Result<()> {
    player.send_blocking(Command::QueueClear)?;
    player.send_blocking(Command::QueueAdd(vec![QueueItem::stream(
        station.name.clone(),
        station.url.clone(),
    )]))?;
    player.send_blocking(Command::QueuePlayIndex(0))?;
    Ok(())
}
```

---

### Task 1: Stations file

**Files:**
- Create: `znicz-core/src/station.rs`
- Modify: `znicz-core/src/lib.rs`
- Modify: `znicz-core/Cargo.toml` (add `toml.workspace = true` and `serde` is already there)
- Modify: `znicz-library/src/lib.rs`

**Interfaces:**
- Consumes: `ZniczError`, `fs`, `toml`, `serde`
- Produces: `Station { name: String, url: String }`, `load`, `save`, `validate_name`, `validate_url`, `add`, `remove`, `rename`, `set_url`, `find`

- [ ] **Step 1: Write failing tests** in `znicz-core/src/station.rs`

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT: AtomicU64 = AtomicU64::new(0);

    fn tmp() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "znicz-stations-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&dir).unwrap();
        dir.join("stations.toml")
    }

    #[test]
    fn missing_file_is_an_empty_list() {
        let path = tmp();
        let _ = fs::remove_file(&path);
        assert!(load(&path).unwrap().is_empty());
    }

    #[test]
    fn round_trip_keeps_file_order() {
        let path = tmp();
        let stations = vec![
            Station {
                name: "One".into(),
                url: "https://example.com/one".into(),
            },
            Station {
                name: "Two".into(),
                url: "http://example.com/two".into(),
            },
        ];
        save(&path, &stations).unwrap();
        assert_eq!(load(&path).unwrap(), stations);
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains("[[station]]"));
        assert!(text.contains("name = \"One\""));
    }

    #[test]
    fn add_rejects_duplicate_names() {
        let mut stations = Vec::new();
        add(&mut stations, " Example ", "https://example.com/a").unwrap();
        let err = add(&mut stations, "Example", "https://example.com/b").unwrap_err();
        assert!(err.to_string().contains("already exists"));
        assert_eq!(stations.len(), 1);
        assert_eq!(stations[0].name, "Example");
    }

    #[test]
    fn names_reject_empty_slash_dotdot() {
        for name in ["", "  ", "a/b", "a\\b", "..", "foo..bar"] {
            assert!(validate_name(name).is_err(), "{name:?}");
        }
        assert_eq!(validate_name("  BBC  ").unwrap(), "BBC");
    }

    #[test]
    fn urls_must_be_http_or_https() {
        assert!(validate_url("ftp://x").is_err());
        assert!(validate_url("example.com/stream").is_err());
        assert!(validate_url("").is_err());
        assert_eq!(
            validate_url("  https://ex.com/s  ").unwrap(),
            "https://ex.com/s"
        );
        assert!(validate_url("http://ex.com/s").is_ok());
    }

    #[test]
    fn rename_collision_is_an_error() {
        let mut stations = vec![
            Station {
                name: "A".into(),
                url: "https://a".into(),
            },
            Station {
                name: "B".into(),
                url: "https://b".into(),
            },
        ];
        assert!(rename(&mut stations, "A", "B").is_err());
        rename(&mut stations, "A", "C").unwrap();
        assert_eq!(stations[0].name, "C");
        assert_eq!(stations[0].url, "https://a");
    }

    #[test]
    fn remove_and_set_url_by_name() {
        let mut stations = vec![Station {
            name: "A".into(),
            url: "https://a".into(),
        }];
        set_url(&mut stations, "A", "https://b").unwrap();
        assert_eq!(stations[0].url, "https://b");
        remove(&mut stations, "A").unwrap();
        assert!(stations.is_empty());
        assert!(remove(&mut stations, "A").is_err());
    }
}
```

Also in `znicz-library/src/lib.rs` tests (next to the playlists-dir test):

```rust
#[test]
fn stations_file_sits_beside_the_library_database() {
    let db = default_database_path().expect("data dir");
    let expected = db.parent().unwrap().join("stations.toml");
    match std::env::var_os("ZNICZ_STATIONS_PATH") {
        Some(path) if !path.is_empty() => {
            assert_eq!(
                default_stations_path().unwrap(),
                std::path::PathBuf::from(path)
            );
        }
        _ => {
            assert_eq!(default_stations_path().expect("data dir"), expected);
        }
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p znicz-core --lib station -- --nocapture`

Expected: compile error (`station` module missing) or FAIL `missing_file_is_an_empty_list`

- [ ] **Step 3: Write minimal implementation**

`znicz-core/Cargo.toml` dependencies: add `toml.workspace = true`.

`znicz-core/src/station.rs`:

```rust
//! Saved radio stations: a TOML file of name + URL.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{Result, ZniczError};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Station {
    pub name: String,
    pub url: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct StationsFile {
    #[serde(default)]
    station: Vec<Station>,
}

pub fn load(path: &Path) -> Result<Vec<Station>> {
    if !path.is_file() {
        return Ok(Vec::new());
    }
    let text = fs::read_to_string(path)?;
    let file: StationsFile = toml::from_str(&text)
        .map_err(|e| ZniczError::Player(format!("stations.toml: {e}")))?;
    Ok(file.station)
}

pub fn save(path: &Path, stations: &[Station]) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let file = StationsFile {
        station: stations.to_vec(),
    };
    let text = toml::to_string_pretty(&file)
        .map_err(|e| ZniczError::Player(format!("stations.toml: {e}")))?;
    fs::write(path, text)?;
    Ok(())
}

pub fn validate_name(name: &str) -> Result<String> {
    let name = name.trim();
    if name.is_empty() {
        return Err(ZniczError::Player("illegal station name".into()));
    }
    if name.contains('/') || name.contains('\\') || name.contains("..") {
        return Err(ZniczError::Player("illegal station name".into()));
    }
    Ok(name.to_string())
}

pub fn validate_url(url: &str) -> Result<String> {
    let url = url.trim();
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err(ZniczError::Player(
            "station URL must be http:// or https://".into(),
        ));
    }
    Ok(url.to_string())
}

pub fn find<'a>(stations: &'a [Station], name: &str) -> Option<&'a Station> {
    stations.iter().find(|s| s.name == name)
}

pub fn add(stations: &mut Vec<Station>, name: &str, url: &str) -> Result<()> {
    let name = validate_name(name)?;
    let url = validate_url(url)?;
    if stations.iter().any(|s| s.name == name) {
        return Err(ZniczError::Player(format!(
            "station {name:?} already exists"
        )));
    }
    stations.push(Station { name, url });
    Ok(())
}

pub fn remove(stations: &mut Vec<Station>, name: &str) -> Result<()> {
    let Some(index) = stations.iter().position(|s| s.name == name) else {
        return Err(ZniczError::Player(format!("no station named {name}")));
    };
    stations.remove(index);
    Ok(())
}

pub fn rename(stations: &mut Vec<Station>, name: &str, new_name: &str) -> Result<()> {
    let new_name = validate_name(new_name)?;
    if stations.iter().any(|s| s.name == new_name && s.name != name) {
        return Err(ZniczError::Player(format!(
            "station {new_name:?} already exists"
        )));
    }
    let station = stations
        .iter_mut()
        .find(|s| s.name == name)
        .ok_or_else(|| ZniczError::Player(format!("no station named {name}")))?;
    station.name = new_name;
    Ok(())
}

pub fn set_url(stations: &mut Vec<Station>, name: &str, url: &str) -> Result<()> {
    let url = validate_url(url)?;
    let station = stations
        .iter_mut()
        .find(|s| s.name == name)
        .ok_or_else(|| ZniczError::Player(format!("no station named {name}")))?;
    station.url = url;
    Ok(())
}
```

`znicz-core/src/lib.rs`: `pub mod station;` and

```rust
pub use station::{
    add as add_station, find as find_station, load as load_stations, remove as remove_station,
    rename as rename_station, save as save_stations, set_url as set_station_url, validate_name,
    validate_url, Station,
};
```

Do **not** alias `play_station` yet (needs `QueueItem` from Task 2). Tests call `station::add` via the module.

`znicz-library/src/lib.rs`:

```rust
/// Where `stations.toml` lives: beside `library.db`, unless `ZNICZ_STATIONS_PATH` is set.
pub fn default_stations_path() -> Option<std::path::PathBuf> {
    if let Some(path) = std::env::var_os("ZNICZ_STATIONS_PATH") {
        if !path.is_empty() {
            return Some(std::path::PathBuf::from(path));
        }
    }
    dirs_data_dir().map(|dir| dir.join("znicz").join("stations.toml"))
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p znicz-core --lib station && cargo test -p znicz-library --lib stations_file`

Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add znicz-core/src/station.rs znicz-core/src/lib.rs znicz-core/Cargo.toml znicz-library/src/lib.rs Cargo.lock
git commit -m "$(cat <<'EOF'
Add stations.toml load and save with name and URL rules.

EOF
)"
```

---

### Task 2: QueueItem and TrackInfo

**Files:**
- Modify: `znicz-core/src/player/state.rs`
- Modify: `znicz-core/src/player/commands.rs`
- Modify: `znicz-core/src/lib.rs`
- Modify: `znicz-core/src/playlist.rs`
- Modify: `znicz-core/src/audio/source.rs` (`TrackInfo { path: ... }` → `path: Some(...), url: None`)
- Modify: `znicz-core/src/player/engine.rs` (compile: `Play`/`QueueAdd`/`play_path` still files-only this task; wrap paths in `QueueItem::file`)
- Modify: `znicz-core/tests/commands.rs`, `znicz-core/tests/queue.rs`, `znicz-core/tests/playback.rs`, `znicz-core/examples/timing.rs`
- Modify: `znicz-tui/src/app.rs`, `views/queue.rs`, `views/inspector.rs`, `examples/preview.rs`, `tests/keys.rs`, `tests/render.rs`
- Modify: `znicz/src/main.rs`
- Modify: `znicz-mcp/src/server.rs`

**Interfaces:**
- Consumes: Task 1 `Station` types (unused here except re-exports)
- Produces:

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum QueueItem {
    File { path: PathBuf },
    Stream { name: String, url: String },
}

impl QueueItem {
    pub fn file(path: impl Into<PathBuf>) -> Self {
        Self::File { path: path.into() }
    }
    pub fn stream(name: impl Into<String>, url: impl Into<String>) -> Self {
        Self::Stream {
            name: name.into(),
            url: url.into(),
        }
    }
    pub fn as_path(&self) -> Option<&Path> {
        match self {
            Self::File { path } => Some(path),
            Self::Stream { .. } => None,
        }
    }
    pub fn is_stream(&self) -> bool {
        matches!(self, Self::Stream { .. })
    }
}
```

```rust
pub struct TrackInfo {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    pub title: String,
    // existing fields unchanged
}
```

```rust
pub enum Command {
    Play(QueueItem),
    QueueAdd(Vec<QueueItem>),
    // others unchanged
}
```

`PlayerState.queue: Vec<QueueItem>`

- [ ] **Step 1: Write a failing unit test** in `znicz-core/src/player/state.rs`

```rust
#[test]
fn queue_item_json_is_tagged_not_a_bare_path() {
    let file = QueueItem::file("/music/a.flac");
    let stream = QueueItem::stream("Example", "https://example.com/s");
    let file_json = serde_json::to_value(&file).unwrap();
    let stream_json = serde_json::to_value(&stream).unwrap();
    assert_eq!(file_json["kind"], "file");
    assert!(file_json["path"].as_str().unwrap().ends_with("a.flac"));
    assert_eq!(stream_json["kind"], "stream");
    assert_eq!(stream_json["name"], "Example");
    assert_eq!(stream_json["url"], "https://example.com/s");
}

#[test]
fn track_info_stream_has_a_url_not_a_path() {
    let track = TrackInfo {
        path: None,
        url: Some("https://example.com/s".into()),
        title: "Example".into(),
        codec: "MP3".into(),
        sample_rate: 44_100,
        channels: 2,
        bits_per_sample: None,
        bitrate_kbps: None,
        duration: None,
        tags: TrackTags::default(),
    };
    let json = serde_json::to_value(&track).unwrap();
    assert!(json.get("path").is_none());
    assert_eq!(json["url"], "https://example.com/s");
    assert_eq!(json["title"], "Example");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p znicz-core --lib queue_item_json -- --nocapture`

Expected: FAIL (no `QueueItem`)

- [ ] **Step 3: Implement types and fix the workspace**

Put `QueueItem` in `state.rs`. Re-export from `lib.rs`: `QueueItem`.

`commands.rs`: drop `PathBuf` import if unused; `Play(QueueItem)`, `QueueAdd(Vec<QueueItem>)`.

`engine.rs` `play_path`:
- `Command::Play(item)` → only `QueueItem::File { path }` this task; `QueueItem::Stream { .. }` returns `Err(ZniczError::NotImplemented("radio stream".into()))` until Task 4.
- Queue membership: compare `QueueItem` equality, not `PathBuf`.
- `play_queue_index` clones a `QueueItem` and matches.

`playlist.rs` `apply_to_player`:

```rust
player.send_blocking(Command::QueueAdd(
    result
        .paths
        .iter()
        .cloned()
        .map(QueueItem::file)
        .collect(),
))?;
```

Add:

```rust
pub fn m3u_paths(queue: &[crate::player::state::QueueItem]) -> Result<Vec<PathBuf>> {
    let paths: Vec<PathBuf> = queue
        .iter()
        .filter_map(|item| item.as_path().map(Path::to_path_buf))
        .collect();
    if paths.is_empty() {
        return Err(ZniczError::Player(
            "cannot save a radio queue as a playlist".into(),
        ));
    }
    Ok(paths)
}
```

Export `m3u_paths` from `lib.rs`.

Test `empty_apply_leaves_the_queue_alone`: compare `vec![QueueItem::file(a)]`.

`source.rs` `track_info`: `path: Some(path.to_path_buf()), url: None`.

Every `TrackInfo { path: PathBuf::from(...),` becomes `path: Some(PathBuf::from(...)), url: None,`.

Every `Command::Play(path)` becomes `Command::Play(QueueItem::file(path))`.

Every `Command::QueueAdd(paths)` where `paths: Vec<PathBuf>` becomes `.map(QueueItem::file).collect()`.

TUI `poll_player_events` `TrackStarted`: insert meta only when `track.path` is `Some`.

TUI `queue.rs`: match `QueueItem`:

```rust
match item {
    QueueItem::Stream { name, .. } => {
        // label = name, time = format::duration_opt(None)
    }
    QueueItem::File { path } => {
        // existing meta.get(path) path
    }
}
```

TUI `queue_label_len` takes `&QueueItem`.

TUI `confirm_playlist_save`: `write_path(&path, &m3u_paths(&queue)?)` — on `Err`, toast.

MCP `play`: `Command::Play(QueueItem::file(params.path))`.

MCP `queue_add`: map strings through `QueueItem::file`.

MCP `save_playlist`: `m3u_paths(&queue)`.

MCP `queue_add_returns_the_new_queue`: JSON `queue[0]["kind"] == "file"` is enough; do not assume a bare string.

`znicz/src/main.rs` `run_tui`: wrap file args in `QueueItem::file`.

`tests/keys.rs` `queue()` helper:

```rust
fn queue(app: &mut App, count: usize) {
    let items: Vec<QueueItem> = (0..count)
        .map(|i| QueueItem::file(format!("/music/track-{i}.flac")))
        .collect();
    app.player.send_blocking(Command::QueueAdd(items)).unwrap();
}
```

`d_removes_the_selected_queue_entry`: `assert_eq!(queue[1], QueueItem::file("/music/track-2.flac"));`

Playlist tests that compared `queue[0]` to a `PathBuf`: use `QueueItem::file(other)` and `queue[0].as_path()`.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --workspace`

Expected: PASS (hardware tests may skip on CI). Clippy later in Task 8 is fine; here the workspace must compile.

- [ ] **Step 5: Commit**

```bash
git add -u
git commit -m "$(cat <<'EOF'
Represent the queue as files or streams instead of bare paths.

EOF
)"
```

Do not add `ureq` yet.

---

### Task 3: HttpStreamSource

**Files:**
- Create: `znicz-core/src/audio/http.rs`
- Modify: `znicz-core/src/audio/mod.rs`
- Modify: `znicz-core/src/audio/source.rs`
- Modify: `znicz-core/Cargo.toml`
- Modify: root `Cargo.toml` (`ureq = "3"`)

**Interfaces:**
- Consumes: `AudioSource` (updated below), `TrackInfo`
- Produces: `HttpStreamSource::new(name: String, url: String)`, `UnseekableRead`, `AudioDecoder::open(source: &dyn AudioSource)`

Change the trait (update `wiki/Rust/Traits.md` in Task 8, not here):

```rust
use std::io::{Read, Seek};
use symphonia::core::io::MediaSource;

pub trait AudioSource: Send {
    fn path(&self) -> Option<&Path>;
    fn url(&self) -> Option<&str>;
    fn title_hint(&self) -> &str;
    fn read_info(&self) -> Result<TrackInfo>;
    fn open_reader(&self) -> Result<Box<dyn MediaSource>>;
}
```

`LocalFileSource`: `path() -> Some(&self.path)`, `url() -> None`, `title_hint` = file stem, `open_reader` = `Box::new(File::open(...)?)`.

`HttpStreamSource::read_info` may return a stub (`title` = name, `url` = Some, `duration` = None, `codec` = "Audio") **without** a GET. The engine probes once via `AudioDecoder::open`.

- [ ] **Step 1: Write failing tests** in `znicz-core/src/audio/http.rs`

```rust
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
            let _ = stream.read(&mut buf);
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
        let err = crate::audio::source::AudioDecoder::open(&source).unwrap_err();
        assert!(
            err.to_string().contains("decode") || err.to_string().contains("probe"),
            "{err}"
        );
    }
}
```

`AudioDecoder::open` must exist taking `&dyn AudioSource`. Keep `open(path: &Path)` as a thin wrapper for `probe_track` / old tests **or** change `probe_track` to `LocalFileSource` and delete path-only `open`. Prefer:

```rust
impl AudioDecoder {
    pub fn open(source: &dyn AudioSource) -> Result<(Self, TrackInfo)> { ... }

    pub fn open_path(path: &Path) -> Result<(Self, TrackInfo)> {
        Self::open(&LocalFileSource::new(path.to_path_buf()))
    }
}
```

Engine Task 4 calls `AudioDecoder::open`. This task can leave engine on `open_path` for files.

Loopback GET must **not** set `Icy-MetaData`. Assert by not putting that header on `HttpStreamSource` (no test against a public server).

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p znicz-core --lib audio::http -- --nocapture`

Expected: FAIL / missing module

- [ ] **Step 3: Implement**

Root `Cargo.toml`:

```toml
ureq = "3"
```

`znicz-core/Cargo.toml`: `ureq.workspace = true`

`http.rs`:

```rust
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
        .timeout_read(Some(Duration::from_secs(30)))
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
        let mut response = agent()
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

impl MediaSource for UnseekableRead {
    fn is_seekable(&self) -> bool {
        false
    }

    fn byte_len(&self) -> Option<u64> {
        None
    }
}
```

If `into_body()` / `into_reader()` names differ on the locked ureq 3, use `body_mut().as_reader()` **only** if the `Response` is stored inside `UnseekableRead` for the lifetime of the reader. Do not load the whole body into a `Vec`.

`AudioDecoder::open`:

```rust
pub fn open(source: &dyn AudioSource) -> Result<(Self, TrackInfo)> {
    let reader = source.open_reader()?;
    let mss = MediaSourceStream::new(reader, Default::default());
    let mut hint = Hint::new();
    if let Some(path) = source.path() {
        if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            hint.with_extension(ext);
        }
    } else if let Some(url) = source.url() {
        if let Some(ext) = url.rsplit('.').next().filter(|s| s.len() <= 4) {
            hint.with_extension(ext);
        }
    }
    // existing probe / decoder setup
    let mut info = track_info_from_params(&codec_params, source);
    ...
}

fn track_info_from_params(codec_params: &CodecParameters, source: &dyn AudioSource) -> TrackInfo {
    let mut info = /* same as today's track_info, but */;
    if let Some(path) = source.path() {
        // today's file metadata / title_from_path
        info.path = Some(path.to_path_buf());
        info.url = None;
    } else {
        info.path = None;
        info.url = source.url().map(str::to_string);
        info.title = source.title_hint().to_string();
        info.duration = None;
        info.tags = TrackTags::default();
    }
    info
}
```

Keep `probe_track(path)` using `LocalFileSource` so existing probe tests stay.

`LocalFileSource::path` now returns `Option<&Path>`.

Bump `COMMAND_TIMEOUT` in `engine.rs` to **20 seconds** so connect (8s) + probe fits inside `send_blocking` (used in Task 4; do it here so Task 4 does not forget).

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p znicz-core --lib audio::http && cargo test -p znicz-core --lib audio::source`

Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml znicz-core/Cargo.toml znicz-core/src/audio Cargo.lock
git commit -m "$(cat <<'EOF'
Add a blocking HTTP audio source for Icecast-style streams.

EOF
)"
```

---

### Task 4: Engine plays streams

**Files:**
- Modify: `znicz-core/src/player/engine.rs`
- Modify: `znicz-core/src/station.rs` (add `play_station`)
- Modify: `znicz-core/src/lib.rs` (export `play_station`)
- Test: `znicz-core/tests/playback.rs` (optional ffmpeg MP3 over loopback; skip on `CI` / no device / no ffmpeg)

**Interfaces:**
- Consumes: `QueueItem`, `HttpStreamSource`, `AudioDecoder::open`, `play_station` inputs
- Produces: `Command::Play(QueueItem::Stream { .. })` actually decodes; seek returns `ZniczError::Player("radio cannot seek")`; pause does not `pump_decode`

- [ ] **Step 1: Write failing tests**

In `znicz-core/src/player/engine.rs` you cannot easily unit-test without spawning. Add `znicz-core/tests/stream.rs`:

```rust
use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread;
use std::time::Duration;

use znicz_core::{
    spawn_player, AudioConfig, Command, QueueItem, ZniczError,
};

fn serve_html() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut buf = [0u8; 2048];
        let _ = stream.read(&mut buf);
        let body = b"<html>nope</html>";
        let header = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        let _ = stream.write_all(header.as_bytes());
        let _ = stream.write_all(body);
    });
    format!("http://{addr}/x")
}

#[test]
fn playing_a_non_audio_url_returns_an_error() {
    let (player, _thread) = spawn_player(AudioConfig::default());
    let url = serve_html();
    let result = player.send_blocking(Command::Play(QueueItem::stream("Bad", url)));
    assert!(result.is_err(), "got {result:?}");
}

#[test]
fn seek_on_a_queued_stream_errors_without_changing_position() {
    let (player, _thread) = spawn_player(AudioConfig::default());
    player
        .send_blocking(Command::QueueAdd(vec![QueueItem::stream(
            "Live",
            "http://127.0.0.1:1/never",
        )]))
        .unwrap();
    // Play will fail (nothing listening). Seek with no decoder should still
    // refuse when the current queue row is a stream after a successful play.
    // After failed play, queue still holds the stream at index 0:
    player
        .send_blocking(Command::QueuePlayIndex(0))
        .ok();
    // If play failed, there is no current_track; skip the seek assertion
    // unless status is Playing (hardware + real stream). Always:
    let seek = player.send_blocking(Command::Seek(Duration::from_secs(5)));
    if player.state().current_track.as_ref().and_then(|t| t.url.as_ref()).is_some()
    {
        assert!(seek.is_err());
        assert!(seek.unwrap_err().to_string().contains("radio cannot seek"));
    }
}
```

The seek test is weak until play succeeds. Stronger: in `handle_command` for `Seek`, if `queue[queue_position].is_stream()`, return the error **even when stopped**. Then the test does not need playback:

```rust
#[test]
fn seek_is_refused_when_the_queue_row_is_a_stream() {
    let (player, _thread) = spawn_player(AudioConfig::default());
    player
        .send_blocking(Command::QueueAdd(vec![QueueItem::stream(
            "Live",
            "https://example.com/s",
        )]))
        .unwrap();
    let err = player
        .send_blocking(Command::Seek(Duration::from_secs(1)))
        .unwrap_err();
    assert!(err.to_string().contains("radio cannot seek"), "{err}");
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p znicz-core --test stream -- --nocapture`

Expected: FAIL (`NotImplemented` or seek succeeds)

- [ ] **Step 3: Implement engine behaviour**

`play_item(item: QueueItem)` replaces `play_path`:

```rust
fn play_item(&mut self, item: QueueItem) -> Result<()> {
    let source: Box<dyn AudioSource> = match &item {
        QueueItem::File { path } => Box::new(LocalFileSource::new(path.clone())),
        QueueItem::Stream { name, url } => {
            Box::new(HttpStreamSource::new(name.clone(), url.clone()))
        }
    };
    let (decoder, track_info) = AudioDecoder::open(source.as_ref())?;
    // rest is today's play_path from open_stream onwards, using `item` in the queue
    ...
    if state.queue.is_empty() {
        state.queue = vec![item.clone()];
        state.queue_position = 0;
    } else if let Some(pos) = state.queue.iter().position(|row| row == &item) {
        state.queue_position = pos;
    } else {
        state.queue.push(item.clone());
        state.queue_position = state.queue.len() - 1;
    }
    ...
}
```

`handle_command`: `Command::Play(item) => self.play_item(item)?`

`play_queue_index`: clone `QueueItem`, `play_item`.

`previous_track`: clone item, `play_item`.

`seek`:

```rust
fn seek(&mut self, position: Duration) -> Result<()> {
    if self.queue_row_is_stream() {
        return Err(ZniczError::Player("radio cannot seek".into()));
    }
    // existing decoder.seek
}

fn queue_row_is_stream(&self) -> bool {
    let state = self.state.read().unwrap();
    state
        .queue
        .get(state.queue_position)
        .map(QueueItem::is_stream)
        .unwrap_or(false)
}
```

`pump_decode`: if status is `Paused`, return immediately (stop pulling HTTP). Decoder stays; resume continues the body. If a later `decode_next` hits `IoError` on a stream, emit the error (no silent skip). Optional one-shot reconnect is **out of scope**; a dropped body fails visibly.

`NextTrack` / `PreviousTrack` with a single stream: keep returning `Ok(())` when `pick_next` is `None`. Do not treat it as an engine error. TUI toasts in Task 5.

`station.rs`:

```rust
use crate::player::commands::Command;
use crate::player::engine::PlayerHandle;
use crate::player::state::QueueItem;

pub fn play_station(player: &PlayerHandle, station: &Station) -> Result<()> {
    player.send_blocking(Command::QueueClear)?;
    player.send_blocking(Command::QueueAdd(vec![QueueItem::stream(
        station.name.clone(),
        station.url.clone(),
    )]))?;
    player.send_blocking(Command::QueuePlayIndex(0))?;
    Ok(())
}
```

Export `play_station` from `lib.rs` (if the Task 1 aliases clash, export this function only as `play_station`).

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p znicz-core --test stream && cargo test -p znicz-core`

Expected: PASS. Hardware playback tests still skip on `CI`.

- [ ] **Step 5: Commit**

```bash
git add znicz-core/src/player/engine.rs znicz-core/src/station.rs znicz-core/src/lib.rs znicz-core/tests/stream.rs
git commit -m "$(cat <<'EOF'
Play HTTP streams through the existing decoder path.

EOF
)"
```

---

### Task 5: TUI radio overlay and keymap

**Files:**
- Modify: `znicz-tui/src/app.rs`
- Create: `znicz-tui/src/views/radio.rs`
- Modify: `znicz-tui/src/views/mod.rs`
- Modify: `znicz-tui/src/views/help.rs`
- Modify: `znicz-tui/src/views/status.rs`
- Modify: `znicz-tui/src/views/queue.rs` (if not finished in Task 2)
- Modify: `znicz-tui/src/views/now_playing.rs`
- Modify: `znicz-tui/src/views/inspector.rs`
- Modify: `znicz-tui/src/keys.rs`
- Modify: `znicz-tui/src/format.rs` (`duration_opt(None)` → `"—"`)
- Modify: `znicz-tui/tests/keys.rs`
- Modify: `znicz-tui/tests/render.rs`
- Modify: `znicz-tui/examples/preview.rs`

**Interfaces:**
- Consumes: `load_stations`, `save_stations`, `add`/`remove`/`rename`/`set_url`, `play_station`, `default_stations_path`, `QueueItem`, `Modal`
- Produces: `Modal::Radio`, `RadioPrompt`, keys as in the spec

```rust
pub enum RadioPrompt {
    AddName(String),
    AddUrl { name: String, buffer: String },
    Rename(String),
    ChangeUrl(String),
}
```

`App` fields: `stations_path: PathBuf`, `stations: Vec<Station>`, `station_cursor: Cursor`, `radio_prompt: Option<RadioPrompt>`.

Default path: `znicz_library::default_stations_path().unwrap_or_else(|| std::env::temp_dir().join("znicz-stations.toml"))`.

- [ ] **Step 1: Write failing tests** in `znicz-tui/tests/keys.rs`

Replace `r_cycles_repeat_and_z_toggles_shuffle` with `e_cycles_repeat_and_z_toggles_shuffle` pressing `'e'` for repeat.

```rust
fn station_fixture() -> (App, PathBuf) {
    let path = std::env::temp_dir().join(format!(
        "znicz-tui-stations-{}-{}.toml",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let mut app = new_app();
    app.stations_path = path.clone();
    (app, path)
}

#[test]
fn capital_r_toggles_the_radio_modal() {
    let mut app = new_app();
    press_char(&mut app, 'R');
    assert_eq!(app.modal, Modal::Radio);
    press(&mut app, KeyCode::Esc);
    assert_eq!(app.modal, Modal::None);
}

#[test]
fn e_cycles_repeat_and_z_toggles_shuffle() {
    let mut app = new_app();
    press_char(&mut app, 'e');
    assert_eq!(app.state().repeat, RepeatMode::All);
    press_char(&mut app, 'e');
    assert_eq!(app.state().repeat, RepeatMode::One);
    press_char(&mut app, 'z');
    assert!(app.state().shuffle);
}

#[test]
fn r_on_the_library_reloads_instead_of_repeat() {
    let mut app = new_app();
    let before = app.state().repeat;
    press_char(&mut app, 'r');
    assert_eq!(app.state().repeat, before);
    assert_eq!(app.modal, Modal::None);
}

#[test]
fn radio_add_prompt_treats_letters_as_text() {
    let (mut app, _path) = station_fixture();
    press_char(&mut app, 'R');
    press_char(&mut app, 'a');
    press_typed(&mut app, "songs");
    assert!(matches!(
        app.radio_prompt,
        Some(RadioPrompt::AddName(ref s)) if s == "songs"
    ));
    assert!(!app.should_quit);
}

#[test]
fn radio_add_writes_the_file_after_name_and_url() {
    let (mut app, path) = station_fixture();
    press_char(&mut app, 'R');
    press_char(&mut app, 'a');
    press_typed(&mut app, "Example");
    press(&mut app, KeyCode::Enter);
    press_typed(&mut app, "https://example.com/stream");
    press(&mut app, KeyCode::Enter);
    let stations = znicz_core::load_stations(&path).unwrap();
    assert_eq!(stations.len(), 1);
    assert_eq!(stations[0].name, "Example");
    assert_eq!(stations[0].url, "https://example.com/stream");
}

#[test]
fn enter_on_a_station_replaces_the_queue() {
    let (mut app, path) = station_fixture();
    znicz_core::save_stations(
        &path,
        &[znicz_core::Station {
            name: "Example".into(),
            url: "https://example.com/stream".into(),
        }],
    )
    .unwrap();
    app.player
        .send_blocking(Command::QueueAdd(vec![QueueItem::file("/music/a.flac")]))
        .unwrap();
    press_char(&mut app, 'R');
    press(&mut app, KeyCode::Enter);
    let queue = app.state().queue;
    assert_eq!(queue.len(), 1);
    assert_eq!(
        queue[0],
        QueueItem::stream("Example", "https://example.com/stream")
    );
}

#[test]
fn d_deletes_a_station_immediately() {
    let (mut app, path) = station_fixture();
    znicz_core::save_stations(
        &path,
        &[znicz_core::Station {
            name: "Gone".into(),
            url: "https://example.com/g".into(),
        }],
    )
    .unwrap();
    press_char(&mut app, 'R');
    press_char(&mut app, 'd');
    assert!(znicz_core::load_stations(&path).unwrap().is_empty());
}

#[test]
fn n_on_a_single_stream_toasts_instead_of_sending_next() {
    let mut app = new_app();
    app.player
        .send_blocking(Command::QueueAdd(vec![QueueItem::stream(
            "Live",
            "https://example.com/s",
        )]))
        .unwrap();
    press_char(&mut app, 'n');
    let toast = app.toasts.current().expect("toast");
    assert!(toast.text.contains("radio"), "{}", toast.text);
}

#[test]
fn seek_on_a_stream_queue_row_toasts() {
    let mut app = new_app();
    app.player
        .send_blocking(Command::QueueAdd(vec![QueueItem::stream(
            "Live",
            "https://example.com/s",
        )]))
        .unwrap();
    // Fake a current track so seek_relative is not a no-op:
    // seek_relative returns early without current_track. After Task 4, Seek
    // errors from the engine when the row is a stream even without a track.
    press(&mut app, KeyCode::Right);
    // If current_track is None, this test only checks no panic.
}

#[test]
fn playlist_save_of_a_stream_queue_is_refused() {
    let mut app = new_app();
    app.player
        .send_blocking(Command::QueueAdd(vec![QueueItem::stream(
            "Live",
            "https://example.com/s",
        )]))
        .unwrap();
    press_char(&mut app, 'P');
    press_char(&mut app, 'w');
    assert!(app.playlist_input.is_none());
    let toast = app.toasts.current().unwrap();
    assert!(
        toast.text.contains("radio") || toast.text.contains("playlist"),
        "{}",
        toast.text
    );
}
```

Export `RadioPrompt` from `znicz_tui` if tests need it (`pub use app::{App, Focus, Modal, RadioPrompt}`).

`keys.rs` tests: `GLOBAL` contains `R` radio, `e` repeat, `r` reload; `LIBRARY` does not document `R` as reload; `DEVICES` documents `r` rescan; new `RADIO` table is included in `every_binding_documents_both_a_key_and_an_action`.

`format.rs` test: `duration_opt(None)` is `"—"`.

`render.rs`: with `Modal::Radio` and one station name, screen contains `"Radio"` and the station name at the usual sizes (copy the playlists render test).

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p znicz-tui --test keys capital_r_toggles -- --nocapture`

Expected: FAIL (`R` still reloads library / no `Modal::Radio`)

- [ ] **Step 3: Implement**

`Modal::Radio` in `blocks_list_focus`.

`on_key`: if `radio_prompt.is_some()`, handle like playlist input **before** global keys.

Global:

- `'R'` → `toggle_radio_modal` (load file, `Modal::Radio`; toggle closes and clears prompt)
- `'r'` → `reload_front()`: Radio → `reload_stations`; Playlists → `reload_playlists`; Devices → rescan devices; else library `reload_albums` + toast `"library reloaded"`
- `'e'` → `cycle_repeat`
- `'n'` / `'N'` / `'p'`: if `queue.len() == 1 && queue[0].is_stream()`, info toast `"radio has no next track"` / `"radio has no previous track"`; else existing commands
- seek keys: if `queue_row_is_stream()`, `apply(Seek)` still runs so the engine error toasts `"player error: radio cannot seek"` **or** toast `"radio cannot seek"` without sending when there is no `current_track`. Prefer sending `Seek` whenever the highlighted/playing row is a stream so MCP and TUI share the engine message. `seek_relative` today returns if `current_track` is none — change it to still `apply(Seek)` when the queue row is a stream.

Radio keys (modal open, no prompt):

| Key | Action |
| --- | --- |
| Enter | `play_station`; toast error if HTTP/decode fails; overlay may stay open |
| `a` | `RadioPrompt::AddName(String::new())` |
| `w` | rename prompt with current name as start, or toast if empty list |
| `c` | URL prompt, or toast if empty |
| `d` | `remove` + `save`; immediate |
| `r` | already global reload |
| Esc | close overlay |

Prompt keys: chars append (including `s`/`n`/`e`/`R`), Backspace pops, Esc cancels prompt only, Enter advances add name → add URL → `add` + `save`, or commits rename/URL.

Empty list: Enter/`w`/`c`/`d` toast `"no stations"` (info). `a` still works.

`views/radio.rs`: copy `playlists.rs` layout. Title `"Radio"`. Prompt prefixes: `name: `, `url: `, `rename: `, `url: `. Empty placeholder `"(empty)  —  a to add a station"`.

`help.rs`: `section(&mut right, "Radio", keys::RADIO);`

`status.rs`: `playlist_input` **or** `radio_prompt` → `"type a name · Enter · Esc cancel"` (or `"type · Enter · Esc cancel"`). `Modal::Radio` → pane `"Radio"`.

`keys.rs`:

```rust
pub const GLOBAL: &[Binding] = &[
    // ...
    b("P", "playlists"),
    b("R", "radio"),
    // seek, volume, mute
    b("e", "repeat: off, all, one"),
    b("r", "reload list"),
    b("z", "shuffle"),
    // ...
];

pub const LIBRARY: &[Binding] = &[
    // no R reload
];

pub const DEVICES: &[Binding] = &[
    b("Enter", "use this output device"),
    b("r", "rescan devices"),
    b("Esc", "close"),
];

pub const RADIO: &[Binding] = &[
    b("Enter", "clear the queue and play"),
    b("a", "add a station"),
    b("w", "rename"),
    b("c", "change URL"),
    b("d", "delete"),
    b("r", "reload stations.toml"),
    b("Esc", "close"),
];
```

Hints: Library drops `R` as reload; include `R` radio next to `P`. Devices `r rescan`. Radio hint line as in the table.

`now_playing.rs`: `duration_opt` already drives the total. After format change, streams show `elapsed / —`. Seek bar ratio stays 0 when duration is `None`.

`inspector.rs`: heading `"File"` can stay for files; if `track.url.is_some()`, heading `"Stream"` and show the URL. Idle copy can stay `"No file is playing."`

`preview.rs`: one frame with `Modal::Radio` and a station name.

Export `RadioPrompt` from `znicz-tui`.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p znicz-tui && cargo test -p znicz-core`

Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add znicz-tui
git commit -m "$(cat <<'EOF'
Add the radio overlay and move repeat to e.

EOF
)"
```

---

### Task 6: CLI `znicz station`

**Files:**
- Modify: `znicz/src/main.rs`

**Interfaces:**
- Consumes: `load_stations`, `save_stations`, `add`/`remove`/`rename`/`set_url`, `play_station`, `find_station`, `default_stations_path`
- Produces: clap subcommands matching the spec

```rust
/// Saved Icecast stations
Station {
    #[command(subcommand)]
    command: StationCmd,
}

enum StationCmd {
    List,
    Add { name: String, url: String },
    Play { name: String },
    Remove { name: String },
    Rename { name: String, new_name: String },
    Url { name: String, url: String },
}
```

Helper:

```rust
fn stations_path() -> color_eyre::Result<PathBuf> {
    znicz_library::default_stations_path().ok_or_else(|| {
        color_eyre::eyre::eyre!("cannot work out where to keep stations; set ZNICZ_STATIONS_PATH")
    })
}
```

`list`: print names (and URLs, one per line `name\turl` or `name — url`). Missing file prints nothing, exit 0.

`add` / `remove` / `rename` / `url`: load, mutate, save; errors go to stderr via `color_eyre`.

`play`: find by exact name, `spawn_player`, `play_station`, then `run_tui_with_player` (same as `playlist play`). HTTP failure must surface before or as the TUI error toast; `play_station` uses `send_blocking` so a bad URL returns an error and the CLI should **not** open the TUI on `Err`.

- [ ] **Step 1: Add the subcommand and `cargo build -p znicz`**

Run: `cargo run -p znicz -- station list`

Expected: exit 0, empty output when no file (or after setting `ZNICZ_STATIONS_PATH` to a temp missing file).

- [ ] **Step 2: Manual check add/list** (optional in agent: spawn with env)

```bash
ZNICZ_STATIONS_PATH=/tmp/znicz-plan-stations.toml cargo run -p znicz -- station add Example https://example.com/stream
ZNICZ_STATIONS_PATH=/tmp/znicz-plan-stations.toml cargo run -p znicz -- station list
```

Expected: `Example` printed.

- [ ] **Step 3: Commit**

```bash
git add znicz/src/main.rs
git commit -m "$(cat <<'EOF'
Add znicz station list, add, play, remove, rename, and url.

EOF
)"
```

---

### Task 7: MCP tools and resource

**Files:**
- Modify: `znicz-mcp/src/server.rs`
- Modify: `znicz-mcp/skills/radio-streaming/SKILL.md`

**Interfaces:**
- Consumes: station helpers, `play_station`, `PlayerHandle::send_blocking`
- Produces: tools below; resource `znicz://stations`

| Tool | Params | Behaviour |
| --- | --- | --- |
| `add_radio_station` | `name`, `url` | add + save; JSON `{ "stations": [...] }` |
| `list_stations` | none | JSON `{ "stations": [{ "name", "url" }] }` |
| `play_station` | `name` | `play_station` + `ok_state()` |
| `remove_radio_station` | `name` | remove + save |
| `rename_radio_station` | `name`, `new_name` | rename + save |
| `set_station_url` | `name`, `url` | set_url + save |

`ZniczMcpServer` stores `stations_path: PathBuf` like `playlists_dir`.

`list_resources` includes `znicz://stations`. `read_resource` returns JSON array of `{name,url}` from `load`.

`play_station` must use `send_blocking` (already true if it calls `znicz_core::play_station`).

- [ ] **Step 1: Write failing tests** in `znicz-mcp/src/server.rs` `mod tests`

Set `ZNICZ_STATIONS_PATH` **before** constructing the server (the path is captured at build). Use a unique temp file; do not touch `$HOME`.

```rust
fn station_server() -> (ZniczMcpServer, PathBuf) {
    let path = std::env::temp_dir().join(format!(
        "znicz-mcp-stations-{}-{}.toml",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::env::set_var("ZNICZ_STATIONS_PATH", &path);
    let (player, _thread) = spawn_player(AudioConfig::default());
    let server = ZniczMcpServer::new(player, Vec::new());
    (server, path)
}

#[test]
fn add_and_list_stations_round_trip() {
    let (server, path) = station_server();
    server
        .add_radio_station(Parameters(StationAddParams {
            name: "Example".into(),
            url: "https://example.com/stream".into(),
        }))
        .unwrap();
    let listed = server.list_stations().unwrap();
    let text = result_text(&listed);
    assert!(text.contains("Example"));
    assert!(text.contains("https://example.com/stream"));
    let _ = std::fs::remove_file(path);
}

#[test]
fn play_station_clears_the_queue_then_errors_on_a_dead_url() {
    let (server, path) = station_server();
    server
        .add_radio_station(Parameters(StationAddParams {
            name: "Dead".into(),
            url: "http://127.0.0.1:1/nope".into(),
        }))
        .unwrap();
    let err = server
        .play_station(Parameters(StationNameParams {
            name: "Dead".into(),
        }))
        .unwrap_err();
    assert!(!err.to_string().contains("not implemented"));
    let queue = server.player.state().queue;
    assert_eq!(queue.len(), 1);
    assert!(queue[0].is_stream());
    let _ = std::fs::remove_file(path);
}

#[test]
fn duplicate_station_name_is_an_error() {
    let (server, path) = station_server();
    let params = StationAddParams {
        name: "Example".into(),
        url: "https://example.com/a".into(),
    };
    server
        .add_radio_station(Parameters(params.clone()))
        .unwrap();
    assert!(server
        .add_radio_station(Parameters(StationAddParams {
            name: "Example".into(),
            url: "https://example.com/b".into(),
        }))
        .is_err());
    let _ = std::fs::remove_file(path);
}
```

`play_station_clears_the_queue_then_errors_on_a_dead_url`: if connect to `:1` is slow, the 8s connect + 20s command timeout still fails the tool. That is the spec (no silent skip).

If `ZniczMcpServer` captures the path at `new()`, setting the env in the test **before** `new()` is required. Document that in the test helper. `set_var` is process-wide: use a mutex around station MCP tests if other tests also read the env, or pass the path into `build` as `stations_path: znicz_library::default_stations_path()...` which reads env at construction only. Serialise these tests with a `static Mutex<()>` lock in the helper.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p znicz-mcp --lib add_and_list_stations -- --nocapture`

Expected: FAIL (`not implemented`)

- [ ] **Step 3: Implement tools**

Replace the three stubs; add the three extra tools. Descriptions without “Phase 4” once they work, e.g. `"Add a radio station (name + HTTP URL)"`.

Keep `enrich_metadata` stubbed.

Skill file:

```markdown
---
name: radio-streaming
description: Add and play HTTP/Icecast radio stations in znicz.
---

# Radio Streaming

Stations are stored in `stations.toml` (override `ZNICZ_STATIONS_PATH`).

1. `add_radio_station` with `name` and `url` (`http://` or `https://`)
2. `list_stations` or resource `znicz://stations`
3. `play_station` with the exact name — this **clears the queue** and starts the stream
4. `rename_radio_station`, `set_station_url`, `remove_radio_station` to edit

This slice does not parse ICY titles or play HLS. Playlist `http://` lines are still skipped.
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p znicz-mcp --lib`

Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add znicz-mcp/src/server.rs znicz-mcp/skills/radio-streaming/SKILL.md
git commit -m "$(cat <<'EOF'
Implement MCP radio station tools and the stations resource.

EOF
)"
```

---

### Task 8: Version 0.3.0 and docs

**Files:**
- Modify: root `Cargo.toml` (`version = "0.3.0"`)
- Modify: `README.md` (keymap + `znicz station` usage)
- Modify: `wiki/Domain/Formats-and-Metadata.md`
- Modify: `wiki/Architecture/TUI.md`
- Modify: `wiki/Architecture/MCP.md`
- Modify: `wiki/Architecture/Core-Engine.md`
- Modify: `wiki/Rust/Traits.md`
- Modify: `wiki/Plans/Roadmap.md` (Phase 4 **Done**)
- Modify: `wiki/Home.md` (station command)
- Modify: `wiki/Rust/Cargo-Workspace.md` (version **0.3.0**)
- Modify: `wiki/Domain/Playback-Pipeline.md` (HTTP as another source, one short paragraph)
- Modify: `wiki/Architecture/Overview.md` only if the command table should mention `Play(QueueItem)`

**Do not** invent ICY/HLS as shipped. Keep **Later radio** on the roadmap.

- [ ] **Step 1: Bump version and rewrite the docs to match the code**

README essentials table:

| Key | Action |
| --- | --- |
| P | Playlists |
| R | Radio |
| e / z | Repeat (off, all, one) / shuffle |
| r | Reload the list in front |

Usage block:

```bash
znicz station list
znicz station add "Example" https://example.com/stream
znicz station play Example
```

Formats radio section: `stations.toml` path, `ZNICZ_STATIONS_PATH`, TUI `R`, playlists still skip URLs **in this version**, later ICY/HLS/M3U URLs.

TUI wiki: Radio overlay keys; `R`/`r`/`e`; transport `—` for unknown duration; queue can show a station name.

MCP wiki: radio tools are real; `znicz://stations`; player-state `queue` entries have `kind`.

Traits wiki: `HttpStreamSource`; `open_reader` returns `MediaSource`; `path`/`url` options.

Core-engine: `Play(QueueItem)`; HTTP `Read` on the player thread; seek refused on radio.

Roadmap: Phase 4 status **Done**; later radio list unchanged.

- [ ] **Step 2: Full verification**

Run:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Expected: all green. Hardware tests skip on `CI`.

- [ ] **Step 3: Commit**

```bash
git add Cargo.toml README.md wiki docs/superpowers/plans/2026-08-31-radio-streams.md
git commit -m "$(cat <<'EOF'
Ship Phase 4 radio streams as 0.3.0.

EOF
)"
```

Include any `Cargo.lock` version bump from `0.2.0` → `0.3.0`.

---

## Spec coverage

| Spec | Task |
| --- | --- |
| `stations.toml` rules, env path, unique names | 1 |
| `QueueItem` / `TrackInfo` URL identity / M3U save files only | 2 |
| `HttpStreamSource`, blocking GET, local fixture, no ICY header | 3 |
| Clear-and-play, seek error, pause stops pulling | 4 |
| Overlay `R`, keys `r`/`e`, CRUD prompts, transport `—`, toasts | 5 |
| CLI `znicz station …` | 6 |
| MCP tools, `znicz://stations`, skill, `send_blocking` | 7 |
| 0.3.0, wiki-sync, README keymap | 8 |
| HLS, ICY UI, M3U URL play, mixed queue | **not this plan** (roadmap later radio) |

## Self-review

- Every Phase 4 goal in the spec has a task. Later radio is excluded on purpose.
- No TBD / “handle edge cases” steps. ureq 3 method names (`into_body` / `into_reader`) must be adjusted to the crate that `cargo add` locks; keep the body streaming.
- `QueueItem::file` / `stream` names are the same in Tasks 2–7. `play_station` is defined in Task 4 and used in 5–7. `m3u_paths` is Task 2 and used in TUI/MCP save.
- `COMMAND_TIMEOUT` is 20s from Task 3 so stream `Play` acks fit.
