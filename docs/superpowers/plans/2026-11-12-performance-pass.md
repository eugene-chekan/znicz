# Performance pass Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix #57 (whole queue in every IPC state reply), #55 (all rows built every frame), #56 (album attribution re-scans tracks every step).

**Architecture:** A revisioned, `Arc`-shared `Queue` in `PlayerState` lets the IPC server skip an unchanged queue; TUI lists build only the visible window; `Library` caches attributed albums keyed by SQLite `data_version` + `total_changes`.

**Tech Stack:** Rust 2024, ratatui, rusqlite, serde.

Spec: `docs/superpowers/specs/2026-11-12-performance-pass-design.md`.
Branch: `perf-pass-0.15.0`. Check with `git branch --show-current` before every task.

Verification for every task: `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`.

---

### Task 0: Baseline timing tests

**Files:**
- Modify: `znicz-core/src/player/ipc.rs` (tests module)
- Modify: `znicz-tui/src/views/queue.rs` or `znicz-tui/src/app.rs` tests (wherever a `TestBackend` app fixture exists)
- Modify: `znicz-library/src/store.rs` (tests module)

- [ ] Add three `#[test] #[ignore]` tests named `perf_ipc_state_2000_queue`, `perf_queue_render_2000`, `perf_albums_for_browse_artist_50k` as described in the spec's "Baseline and results". Each prints `PERF <name>: <mean µs> µs` with `println!` and asserts nothing about time. Reuse existing test fixtures (look at how ipc tests spawn a player with `spawn_player` and an `IpcServer`; how TUI tests build an `App` and draw to `TestBackend`; `Library::open_in_memory` + `upsert_simple` in store tests). Queue items: `QueueItem::File { path: format!("/music/{i}.flac").into() }`. Library: 50,000 tracks, artist `Artist {i % 500}`, album `Album {i % 5000}`; use one transaction if the API offers a batch upsert, otherwise upsert in a loop.
- [ ] Run `cargo test --release --workspace -- --ignored --nocapture perf_` and record the three numbers in the task report.
- [ ] Commit: `test: timing tests for queue state, queue render and album attribution`.

### Task 1: `Queue` type

**Files:**
- Modify: `znicz-core/src/player/state.rs`, `znicz-core/src/player/engine.rs`, `znicz-core/src/session.rs`, `znicz-core/src/lib.rs` (export `Queue`), plus any compile fallout in `znicz-tui`, `znicz-mcp`, `znicz`.

- [ ] Write tests in `state.rs`: `items_mut_raises_the_revision`, `replace_raises_the_revision`, `clone_shares_the_items` (`Arc::ptr_eq` via a `#[cfg(test)]` helper or compare `as_ptr()` of the slices), `serializes_as_a_plain_array` (`serde_json::to_value(&queue)` equals `json!([...])`, and round trip gives equal items), `compares_with_a_vec`.
- [ ] Implement:

```rust
#[derive(Debug, Clone, Default)]
pub struct Queue {
    items: Arc<Vec<QueueItem>>,
    revision: u64,
}

impl Queue {
    pub fn new(items: Vec<QueueItem>) -> Self {
        Self { items: Arc::new(items), revision: 0 }
    }
    pub fn revision(&self) -> u64 { self.revision }
    /// The only way to change the items; raises the revision.
    pub fn items_mut(&mut self) -> &mut Vec<QueueItem> {
        self.revision = self.revision.wrapping_add(1);
        Arc::make_mut(&mut self.items)
    }
    pub fn replace(&mut self, items: Vec<QueueItem>) {
        self.revision = self.revision.wrapping_add(1);
        self.items = Arc::new(items);
    }
    pub fn to_vec(&self) -> Vec<QueueItem> { self.items.as_ref().clone() }
    pub(crate) fn with_revision(items: Vec<QueueItem>, revision: u64) -> Self {
        Self { items: Arc::new(items), revision }
    }
}

impl std::ops::Deref for Queue {
    type Target = [QueueItem];
    fn deref(&self) -> &[QueueItem] { &self.items }
}
impl From<Vec<QueueItem>> for Queue { fn from(items: Vec<QueueItem>) -> Self { Self::new(items) } }
impl PartialEq for Queue { fn eq(&self, other: &Self) -> bool { self.items == other.items } }
impl PartialEq<Vec<QueueItem>> for Queue { fn eq(&self, other: &Vec<QueueItem>) -> bool { *self.items == *other } }
impl Serialize for Queue { fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> { self.items.serialize(s) } }
impl<'de> Deserialize<'de> for Queue { fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> { Vec::<QueueItem>::deserialize(d).map(Queue::new) } }
```

  (`QueueItem` needs `PartialEq`; it is likely derived already.) Change `PlayerState.queue` to `Queue`.
- [ ] Fix every engine write: `state.queue.extend(items)` → `state.queue.items_mut().extend(items)`; `.clear()`, `.push`, `.remove`, `.insert` likewise; `state.queue = vec![..]` / `= items` → `state.queue.replace(..)`. A `move` must call `items_mut()` once and do both remove and insert on the returned vec. Do not call `items_mut()` for reads.
- [ ] Engine tests: add, clear, remove, move, replace each change `state().queue.revision()`; `Play`/`Pause`/`Seek`/`SetVolume` do not.
- [ ] `Session::from_state`: `queue: state.queue.to_vec()`. Fix remaining compile errors across the workspace (`.queue` used as a `Vec`: `.to_vec()`, or `&state.queue[..]`, or deref). Keep JSON shapes unchanged.
- [ ] Verify, commit: `refactor(core): revisioned, shared Queue in PlayerState`.

### Task 2: IPC skips an unchanged queue

**Files:** Modify `znicz-core/src/player/ipc.rs`.

- [ ] Tests (in the ipc tests module, using the existing in-process server fixtures):
  - `state_skips_an_unchanged_queue`: add 3 items, call `state()` twice; use `rpc_raw` with `IpcRequest::State { token, queue_revision: Some(rev) }` to check the server replies `queue_unchanged: true` with an empty queue; and the client's second `state()` still returns 3 items.
  - `state_sends_the_queue_after_a_change`: `state()`, add an item, `state()` returns 4 items.
  - `state_without_a_revision_gets_the_full_queue`: `rpc_raw` with `queue_revision: None` → `queue_unchanged: false`, full queue.
  - `old_response_without_queue_fields_parses`: deserialize a JSON `{"kind":"ok","state":{...}}` without the new fields.
- [ ] Implement: `IpcRequest::State { token, #[serde(default)] queue_revision: Option<u64> }`; `IpcResponse::Ok { state, library, #[serde(default)] queue_revision: Option<u64>, #[serde(default)] queue_unchanged: bool }`. Every existing `IpcResponse::Ok { .. }` constructor sets `queue_revision: None, queue_unchanged: false` except the `State` reply. Server `State` handling:

```rust
IpcRequest::State { queue_revision, .. } => {
    let mut state = shared.player.state();
    let current = state.queue.revision();
    let unchanged = queue_revision == Some(current);
    if unchanged {
        state.queue = Queue::default();
    }
    let response = IpcResponse::Ok {
        state: Box::new(state),
        library: shared.library.as_ref().map(|host| host.status()),
        queue_revision: Some(current),
        queue_unchanged: unchanged,
    };
    let _ = write_json_line(&mut writer, &response);
}
```

  Client: add `queue_cache: Option<Queue>` to `Live` (initialised `None` in `open_live`). `IpcClient::state()` locks `live`, builds the request with `queue_revision: live.queue_cache.as_ref().map(Queue::revision)`, and after the reply: if `queue_unchanged` and the cache is `Some`, set `state.queue = cache.clone()`; else if `queue_revision` is `Some(r)`, set `state.queue = Queue::with_revision(state.queue.to_vec(), r)` (or keep the deserialized queue and set its revision via a crate-private setter) and store a clone in the cache; else clear the cache. Keep the reconnect-and-retry behaviour of `rpc_detailed`: a reconnect creates a new `Live` with an empty cache, and the retried request must be built from the new `Live` (so `queue_revision: None`). Make `make` read the cache from the `Live` it is given, e.g. change `make: impl Fn(&str) -> IpcRequest` into `impl Fn(&Live) -> IpcRequest` or add a dedicated path for `State`. The `queue_unchanged`-without-cache case cannot happen (the request carried no revision); treat it as an error-free empty queue only if it does.
  `LibraryClient::try_status` (also sends `State`) passes `queue_revision: None`.
- [ ] Verify, commit: `perf(ipc): skip the queue in state replies when it has not changed (#57)`.

### Task 3: One state read per frame

**Files:** Modify `znicz-tui/src/app.rs` (`selected_queue_middle`, `clamp_queue_pan`, callers), `znicz-tui/src/views/mod.rs:111`.

- [ ] Change signatures to `fn selected_queue_middle(&self, state: &PlayerState) -> usize` and `pub(crate) fn clamp_queue_pan(&mut self, state: &PlayerState)`; drop the internal `self.player.state()`. `views::render` passes its `state`. Other callers (key handlers around `app.rs:2458`) read `let state = self.player.state();` once and pass `&state`.
- [ ] Existing pan tests must still pass. Verify, commit: `perf(tui): read player state once per frame (#57)`.

### Task 4: `visible_window` and the queue drawer

**Files:**
- Create: `znicz-tui/src/views/window.rs` (register in `views/mod.rs`)
- Modify: `znicz-tui/src/views/queue.rs`, `znicz-tui/src/app.rs` (callers of `title_slot`)

- [ ] Tests in `window.rs`: empty list → `0..0`; `height == 0` → empty; no selection keeps offset (clamped to len); selection above the window moves start to it; selection below moves start to `selected + 1 - height`; offset past end clamps; selection inside keeps offset.
- [ ] Implement:

```rust
use std::ops::Range;

/// Rows a list shows: ratatui's `List` scrolling rule (no scroll padding).
pub fn visible_window(len: usize, selected: Option<usize>, offset: usize, height: usize) -> Range<usize> {
    let mut start = offset.min(len.saturating_sub(1));
    if let Some(selected) = selected.map(|s| s.min(len.saturating_sub(1))) {
        if selected < start {
            start = selected;
        } else if height > 0 && selected >= start + height {
            start = selected + 1 - height;
        }
    }
    let end = (start + height).min(len);
    start.min(end)..end
}
```

  (Adjust after the tests if ratatui's real behaviour differs; add a test that compares with ratatui's `List` by rendering 100 items with a selection near the end into a `TestBackend` with a plain `ListState`, and the windowed version, and asserting identical buffers.)
- [ ] Queue view: compute `height = inner.height as usize` from `block.inner(area)` before building items; `let window = visible_window(len, selected, app.queue_list_state.offset(), height)`; build items only for `window` (keep `i` as the absolute index for numbers, marker, `queue_offset_for(i, len)`); render with a local `ListState` with `select(selected.map(|s| s - window.start))`; then `*app.queue_list_state.offset_mut() = window.start; app.queue_list_state.select(selected);` and `ListHit.offset = window.start`.
- [ ] One pass for `title_slot` and the summary: a helper `fn queue_stats(app: &App, queue: &[QueueItem]) -> QueueStats { max_time_width: usize, total: Option<Duration> }` that reads each item's duration once (no string formatting: the width of `format::duration_opt(d)` computed by a new `format::duration_width(Option<Duration>) -> usize` with tests asserting it equals `duration_opt(d).chars().count()` for None, 0 s, 59 s, 1 min, 59:59, 1 h, 10 h). `title_slot` and the summary both use it; `now_playing::total_duration` semantics must be kept (check what it returns when some durations are unknown and replicate exactly).
- [ ] Test: a 200-item queue rendered into `TestBackend` with the selection at 150 shows rows 1xx with the right numbers and the highlighted row is 151.
- [ ] Verify, commit: `perf(tui): build only visible queue rows (#55)`.

### Task 5: Visible rows in the library views

**Files:** Modify `znicz-tui/src/views/library.rs`.

- [ ] For each list builder (`render_column_list` callers, `render_tree`, paged lists → `finish_list`, single lists → `finish_list`, search hits): change the builders so they receive the window and build items only for it. Pattern: compute `inner` from the block first, `height = inner.height as usize`; `window = visible_window(count, selected, offset, height)` where `offset` is `app.library_list_state.offset()` for tree/single/paged lists and `0` for columns (they use a fresh `ListState` today); build `items` from `rows[window.clone()]` with absolute indices; render with a local `ListState` selected at `selected - window.start`; store `window.start` as the offset in `app.library_list_state` (not for columns) and in the `ListHit`.
- [ ] Tree: `tree_row_contexts(&rows)` may stay computed for all rows if its result is needed by index; zip and then skip/take the window. Keep `selected_index(strip)` resolved once.
- [ ] Tests: for the tree and for one column, a long list (≥100 rows) with the selection near the end renders the same visible rows and highlight as before (compare with expected row text).
- [ ] Verify, commit: `perf(tui): build only visible library rows (#55)`.

### Task 6: Album attribution cache and NOCASE indexes

**Files:** Modify `znicz-library/src/store.rs`.

- [ ] Tests:
  - `attribution_is_cached_until_the_library_changes`: two `browse_artists()` calls with no write return equal results and the second does not re-run the query (expose a `#[cfg(test)] attribution_builds: Cell<u32>` counter incremented in `attribute_albums`).
  - `attribution_cache_sees_own_writes`: upsert a track for a new artist → `browse_artists` includes it.
  - `attribution_cache_sees_other_connections`: two `Library::open` on the same temp file (`tempfile::TempDir`); write through one, read through the other → new artist present.
  - `album_lookup_uses_the_nocase_index`: `EXPLAIN QUERY PLAN SELECT ... FROM tracks WHERE album = ?1 COLLATE NOCASE` mentions `tracks_album_nocase_idx`; same for artist.
  - `old_indexes_are_dropped_on_open`: a db with `tracks_album_idx` → after open, `sqlite_master` has no `tracks_album_idx`.
- [ ] Implement: field `attributed: RefCell<Option<((i64, u64), Arc<Vec<AttributedAlbum>>)>>` (initialise `RefCell::new(None)` in every constructor); `fn cache_key(&self) -> Result<(i64, u64)>` = `(self.conn.query_row("PRAGMA data_version", [], |r| r.get(0))?, self.conn.total_changes())`; `fn attributed_albums(&self) -> Result<Arc<Vec<AttributedAlbum>>>`; `browse_artists` / `albums_for_browse_artist` iterate `attributed.iter()` and clone what they keep. `AttributedAlbum` may need `Clone`.
- [ ] Schema: replace the two index lines with
  `DROP INDEX IF EXISTS tracks_album_idx; DROP INDEX IF EXISTS tracks_artist_idx;`
  `CREATE INDEX IF NOT EXISTS tracks_album_nocase_idx ON tracks(album COLLATE NOCASE);`
  `CREATE INDEX IF NOT EXISTS tracks_artist_nocase_idx ON tracks(artist COLLATE NOCASE);`
  Check every query that filters on `album`/`artist` without `COLLATE NOCASE`; if any exists, report it (do not change query semantics).
- [ ] Verify, commit: `perf(library): cache album attribution, NOCASE indexes (#56)`.

### Task 7: Results, docs, version

- [ ] Re-run `cargo test --release --workspace -- --ignored --nocapture perf_`; record numbers next to Task 0's.
- [ ] `wiki/Architecture/Core-Engine.md`: the `Queue` type and how `State` replies skip an unchanged queue. `wiki/Architecture/TUI.md`: lists build only visible rows. `wiki/Architecture/Library.md`: the attribution cache and NOCASE indexes.
- [ ] `Cargo.toml` `[workspace.package] version = "0.15.0"`; `cargo update -w`; `CHANGELOG.md` `## 0.15.0 - 2026-11-12` with three bullets; `scripts/third-party-licenses.sh`.
- [ ] Verify, commit: `chore: release 0.15.0`.
