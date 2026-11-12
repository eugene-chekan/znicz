# Performance pass (#57, #55, #56)

**Date:** 2026-11-12
**Status:** Approved in chat
**Crates:** `znicz-core`, `znicz-tui`, `znicz-library`
**Version:** 0.15.0

One branch, one PR, one release. Order: #57, #55, #56.

## Baseline and results

Timing tests marked `#[ignore]`, run with
`cargo test --release -- --ignored --nocapture perf_`:

- `znicz-core`: `perf_ipc_state_2000_queue`, the mean time of `IpcClient::state()`
  against an in-process `IpcServer` with a 2,000-item queue (100 calls).
- `znicz-tui`: `perf_queue_render_2000`, the mean time to draw the queue drawer
  with 2,000 items into an 80x40 `TestBackend` (100 frames).
- `znicz-library`: `perf_albums_for_browse_artist_50k`, the mean time of
  `albums_for_browse_artist` on an in-memory library of 50,000 tracks
  (500 artists, 5,000 albums) over 100 calls.

The numbers before and after go into the PR description and the CHANGELOG
entry does not quote them.

## 1. Queue revision (#57)

### `Queue` type (`znicz-core/src/player/state.rs`)

```rust
#[derive(Debug, Clone, Default)]
pub struct Queue {
    items: Arc<Vec<QueueItem>>,
    revision: u64,
}
```

- `Deref<Target = [QueueItem]>`, so reads (`len`, `iter`, `get`, `[i]`,
  `is_empty`, `first`) keep working.
- `Queue::new(Vec<QueueItem>) -> Queue` (revision 0) and `From<Vec<QueueItem>>`.
- `revision(&self) -> u64`.
- `items_mut(&mut self) -> &mut Vec<QueueItem>`: bumps the revision by one
  (wrapping) and returns `Arc::make_mut(&mut self.items)`. This is the only way
  to change the items.
- `replace(&mut self, items: Vec<QueueItem>)`: new items, revision + 1.
- `to_vec(&self) -> Vec<QueueItem>`.
- `PartialEq` compares items only; also `PartialEq<Vec<QueueItem>>`.
- serde: serializes and deserializes as a plain JSON array of items (custom
  impls or `#[serde(from/into)]` via `Vec<QueueItem>`), so MCP output,
  `znicz://queue`, and older peers see the same JSON as before. Deserialized
  queues have revision 0.

`PlayerState.queue: Queue`. `PlayerState::clone` is then an `Arc` copy of the
items. Every engine write goes through `items_mut` or `replace`; the compiler
finds them. `Session` keeps `Vec<QueueItem>` (`state.queue.to_vec()`).

### IPC (`znicz-core/src/player/ipc.rs`)

- `IpcRequest::State` gains `#[serde(default)] queue_revision: Option<u64>`:
  the revision of the queue the client already holds.
- `IpcResponse::Ok` gains `#[serde(default)] queue_revision: Option<u64>` (the
  server's current revision, always sent for `State`) and `#[serde(default)]
  queue_unchanged: bool`.
- Server, on `State { queue_revision: Some(r) }` where `r` equals the current
  revision: replies with the state whose queue is `Queue::default()` and
  `queue_unchanged: true`. Otherwise the full queue and `queue_unchanged:
  false`.
- Client: `Live` holds `queue_cache: Option<Queue>` (items with the server's
  revision). `state()` sends `queue_revision` from the cache; on
  `queue_unchanged` it puts the cached queue into the returned state; otherwise
  it stores the received queue with the server's revision (when the response
  carries one; none means an old server: no caching). The cache lives in
  `Live`, so a reconnect (new player process, revisions start again) drops it.
- An old client sends no revision and always gets the full queue. An old
  server ignores the field and never sets `queue_unchanged`.

### One state read per frame (`znicz-tui`)

`App::selected_queue_middle(&self, state: &PlayerState)` and
`App::clamp_queue_pan(&mut self, state: &PlayerState)` take the state the frame
already read (`views::render` passes it). Key handlers keep their own reads.

## 2. Visible rows only (#55)

- `znicz-tui/src/views/window.rs`: `pub fn visible_window(len: usize, selected:
  Option<usize>, offset: usize, height: usize) -> Range<usize>`, the same rule
  as ratatui's `List` without scroll padding: start from `offset.min(len)`;
  if `selected < start`, start at `selected`; if `selected >= start + height`,
  start at `selected + 1 - height`; end is `(start + height).min(len)`.
  `height == 0` gives an empty range at `start`.
- Each list (queue; library columns; paged lists; single lists: tracks,
  albums, artists, search hits; tree) builds `ListItem`s only for that range,
  renders `List` with a local `ListState` (offset 0, selected
  `selected - start`), then stores the window start as the real offset: in
  `app.queue_list_state` / `app.library_list_state` via
  `*list_state.offset_mut() = start` and `select(selected)`, and in the
  `ListHit.offset`, so mouse hit-testing and next frame's scrolling match.
  Columns, which today use a fresh `ListState` each frame, pass `offset 0`.
- Visible output is unchanged: for each list, a test renders a long list into
  a `TestBackend` before and after scrolling and checks the visible rows and
  the highlighted row.
- Queue: `title_slot` and the duration summary come from one pass over the
  queue that does not format strings (the time label width comes from the
  duration directly, using the same rule as `format::duration_opt`).
- Not in scope: skipping idle redraws (ratatui already diffs cells; with
  windowed rows an idle frame is O(screen height)).

## 3. Album attribution cache (#56)

- `Library` gets `attributed: RefCell<Option<(CacheKey, Arc<Vec<AttributedAlbum>>)>>`
  where `CacheKey = (i64, u64)` = (`PRAGMA data_version`, `conn.total_changes()`).
  `data_version` changes when another connection commits (the player process
  hosts the writer); `total_changes` changes with this connection's own writes.
- `attributed_albums(&self) -> Result<Arc<Vec<AttributedAlbum>>>` reads the key,
  returns the cached list when it matches, otherwise runs `attribute_albums`
  and stores it. `browse_artists` and `albums_for_browse_artist` use it.
- Indexes: the schema creates `tracks_album_nocase_idx ON tracks(album COLLATE
  NOCASE)` and `tracks_artist_nocase_idx ON tracks(artist COLLATE NOCASE)` and
  drops `tracks_album_idx` / `tracks_artist_idx` (`DROP INDEX IF EXISTS`), so
  existing databases migrate on open. A test checks with `EXPLAIN QUERY PLAN`
  that `WHERE album = ? COLLATE NOCASE` uses the new index.

## Testing

- `Queue`: each `items_mut` / `replace` raises the revision; clone shares the
  allocation; serde round trip is a plain array.
- Engine: add, clear, remove, move, replace each change the revision; play and
  seek do not.
- IPC: second `state()` with an unchanged queue gets `queue_unchanged` and the
  client still returns the full queue; after a queue change the new queue
  arrives; a request without `queue_revision` gets the full queue; a response
  without the new fields still parses.
- `visible_window` edge cases: empty, `height` 0, no selection, selection above
  and below the window, offset past the end.
- Library: the cache is reused when nothing changed; invalidated by an upsert on
  the same connection and by a write from a second connection to the same file.

## Docs

`wiki/Architecture/Core-Engine.md` (queue revision, `State` skipping the queue),
`wiki/Architecture/TUI.md` (visible rows), `wiki/Architecture/Library.md`
(attribution cache and indexes), `CHANGELOG.md` 0.15.0, version 0.15.0,
`THIRD-PARTY-LICENSES.html` regenerated.
