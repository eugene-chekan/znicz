# Library Tree Render Performance Design

## Problem

With `library_layout = "tree"` in `config.toml`, the TUI becomes sluggish once
an artist is expanded. On the reporter's library (2,437 tracks, 62 browse
artists, 210 albums), a release build needs about **368 ms** to lay out one
frame of 69 tree rows. The event tick is 200 ms, so the UI cannot keep up and
every keypress waits for a redraw.

## Evidence

Measured with a temporary example against the real
`~/.local/share/znicz/library.db`:

| operation | time |
|---|---|
| `browse_artists()` | 5.14 ms |
| `tree_rows()` (69 rows, one artist expanded) | 5.17 ms |
| per-row `offset_for()` loop over 69 rows (one frame) | 368.16 ms |

The arithmetic matches: 69 rows x 5.17 ms = 357 ms, plus the frame's other
`tree_rows()` calls.

## Root cause

`views::library::render_tree` asks for each row's pan offset with
`LibraryPane::offset_for(i, strip)`, and that is called inside a loop over every
row:

```text
render_tree
  -> offset_for(i)              per row
       -> selected_index(strip)
            -> tree_rows()      builds every row again
                 -> albums_for_browse_artist()   per expanded artist
                      -> attribute_albums()
                           -> full scan + regroup of the `tracks` table
```

So one frame costs `rows x expanded_artists x tracks`. The album list is also
re-queried on every redraw even though `LibraryPane` already keeps a
`tree_album_cache` populated at expansion time.

## Goals

- A tree frame does no SQLite work after an artist has been expanded.
- Laying out rows no longer rebuilds the tree once per row.
- Sorting, expansion state, panning, hit targets, and key handling stay the
  same.

## Non-goals

- Caching `attribute_albums` inside `znicz-library` for column/paging selection
  changes. That remains a follow-up.
- Rendering only the visible row window.
- Trimming the queue from the IPC `State` response.
- Audio-path allocations (`SampleBuffer`, per-sample ring writes).

## Approach

1. **Resolve the selection once.** Add
   `LibraryPane::offset_for_row(index, Option<usize>)` as the pure pan rule.
   `offset_for` delegates to it. `render_tree` computes `selected_index` once
   per frame and passes it to every row.
2. **Cache albums per expanded artist.** Add
   `tree_albums_by_artist: HashMap<String, Vec<AlbumSummary>>` keyed by the
   lowercased artist name. Populate it when expanding an artist and when
   loading the selected artist's albums. `tree_rows` reads the cache and keeps
   the library query only as a fallback for a missing key. Collapsing an artist
   removes the key.
3. **Refresh on reload.** `reload_browse` refreshes the cached album list for
   every expanded artist, so an external `znicz scan` shows up on `r` instead
   of waiting for collapse and re-expand.

## Risks

| Risk | Mitigation |
|---|---|
| Stale albums after a library change | The TUI never writes the library; `reload_browse` refreshes all expanded artists. |
| Cache key mismatch (`Miles` vs `miles`) | Keys use `str::to_lowercase()`, matching the existing album-key convention. |
| Tree rows become empty if a key is missing | `tree_rows` falls back to `albums_for_browse_artist`. |

## Acceptance criteria

- New unit tests cover: pan offset from a pre-resolved selection, cached rows
  surviving a missing library connection, collapse invalidating the cache, and
  reload refreshing every expanded artist.
- Benchmark on the reporter's database: single-selection frame **< 10 ms**
  (down from 368 ms); three `tree_rows()` calls **< 2 ms** (down from ~15 ms).
- `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`, and `cargo test --workspace` pass.
- Workspace version bumped to 0.5.1 and wiki updated in the same PR.
