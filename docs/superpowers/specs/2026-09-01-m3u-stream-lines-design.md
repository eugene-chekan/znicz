# M3U stream lines (later radio)

**Date:** 2026-09-01
**Status:** Approved
**Crates:** `znicz-core`, `znicz-tui`, `znicz-mcp`, `znicz`

Second slice of [later radio](../../../wiki/Plans/Roadmap.md#later-radio-after-phase-4).
Depends on mixed queue (`QueueItem::File` / `QueueItem::Stream` in one list).

Same cycle also drops **`N`** as previous track. Previous is **`p` only**.

## Problem

Playlist parse skips any line that contains `://` and counts it in `skipped`.
A file of `http://` / `https://` rows therefore loads as empty
(`playlist had no playable files`). Save still refuses any stream row
(`cannot save a queue that contains a radio station`), so a mixed queue cannot
round-trip.

Global previous is documented and bound as `N / p`. The extra `N` is unused
elsewhere and collides with the usual “shift-n = previous” muscle memory
people do not want.

## Goals

1. `http://` and `https://` lines in an M3U / M3U8 file enqueue as
   `QueueItem::Stream` instead of being skipped.
2. Save writes file paths **and** those URLs so a mixed queue round-trips.
   Named stream rows keep their name via `#EXTINF`.
3. TUI, CLI (`znicz playlist …`), and MCP (`import_playlist`, `play_playlist`,
   `save_playlist`) share that load and save.
4. Previous in the TUI is **`p` only**. Help and README drop **`N`**.

## This slice does not include

- ICY `StreamTitle` on the transport
- HLS playback (`.m3u8` **segment** playlists as a second source type)
- MCP `queue_remove` ([#15](https://github.com/eugene-chekan/znicz/issues/15))
- Changing Playlists / Radio overlay keys (`n` / `e` / `c` / `d`)
- Resurrecting radio `w`
- `queue_add` growing URL support (still paths-only)
- PLS / XSPF
- Phase 5 / 6, parked issues `#5`–`#9`

A playlist **file** named `.m3u8` stays an M3U list of paths and URLs (Phase 3).
That is not HLS. An `http(s)` line that happens to end in `.m3u8` is still
enqueued as a stream; **play** fails until the HLS slice, same as typing that
URL as a station today. Do not skip those lines, and do not auto-skip a failed
open.

## Daily motion

You open `P`, pick a playlist that lists two files and a station URL, press
Enter. The queue is those three rows. The first file starts. Next can land on
the stream. With Playlists still open, `n` saves the mix. Open the same file
again: files, URL, and station name come back.

You press `p` for previous. `N` does nothing (unbound). Overlay `P` is still
playlists.

## Load

`playlist::parse` returns queue items, not only `PathBuf`s.

```text
LoadResult {
    items: Vec<QueueItem>,  // in playlist order
    skipped: usize          // missing local files and non-http(s) URLs
}
```

Drop `LoadResult.paths`. Callers that counted `paths.len()` use `items.len()`.

Strip a leading UTF-8 BOM. Trim each line.

| Line (after trim) | Meaning |
| --- | --- |
| Empty | Ignore. Does not count as skipped. Does not clear a pending `#EXTINF` title. |
| Starts with `#EXTINF:` | Pending title for the **next** stream line. Title is everything after the **first comma**, trimmed. No comma, or an empty title, clears pending (the next stream uses the URL as its name). Another `#EXTINF:` replaces pending. |
| Other `#` (`#EXTM3U`, `# comment`, `#EXT-X-…`) | Ignore. Does not count as skipped. Does not clear pending. |
| Starts with `http://` or `https://` (ASCII case-insensitive scheme) | Stream row. Consume pending title, or use the **full trimmed line** as the name. Not skipped. |
| Contains `://` but is not http(s) (`ftp://`, `file://`, `rtsp://`, …) | Skip and count. Discard pending without using it. |
| Anything else | Local path. Relative paths resolve against the playlist file’s directory. If that path is a file, `QueueItem::file`. If not, skip and count. Discard pending without using it (files still use the file name in the queue, not `#EXTINF`). |

A playlist that is **only** http(s) lines is valid. `apply_to_player` still
errors with `playlist had no playable files` when `items` is empty (comments
only, or only skipped rows). Do not clear the queue in that case.

`skipped_notice` stays `{n} tracks, {m} skipped` with `n = items.len()`.
Comments never count toward `skipped`.

`apply_to_player` sends `QueueAdd` with `result.items` (files and streams).
Clear-and-play and append are unchanged: `QueueClear` then add then
`QueuePlayIndex(0)`, or add only. No new `Command` variant.

Do not open the stream during parse. Play happens only when the engine is
asked to play that row (clear-and-play of a URL-first list, or next onto it).

## Save

Empty queue: existing `queue is empty` (TUI toast / MCP error). Do **not**
refuse because a row is a stream. Remove
`cannot save a queue that contains a radio station`.

Replace `m3u_paths` → `Vec<PathBuf>` with writing the queue directly:

- `write_text(&[QueueItem]) -> String`
- `write_path(path, &[QueueItem])`

UTF-8, no BOM, no `#EXTM3U` header (same as today’s file-only saves).

| Row | Written |
| --- | --- |
| File | One absolute path per line (`canonicalize` as today, same Windows `\\?\` behaviour) |
| Stream whose `name` equals `url` (the unnamed case) | That URL only |
| Stream whose `name` is not `url` | `#EXTINF:-1,{name}` then a newline then the URL |

Reload of that file must restore files, URLs, and station names.

TUI Playlists `n`: open the name prompt whenever the queue is non-empty,
including a station-only or mixed queue. MCP `save_playlist` writes the same
body.

## Keys

| Binding | After this slice |
| --- | --- |
| `n` | Next track (global). Overlay `n` still new/save. |
| `p` | Previous track (global only) |
| `N` | Unbound. Not previous. |
| `P` | Playlists (unchanged) |

Update `znicz-tui/src/keys.rs` `GLOBAL`, the README essentials table, and any
test that requires `N` in the previous-track help string. Keep
`lowercase_p_is_still_previous_track`.

## TUI, CLI, MCP

No new play keys. Playlists Enter / `a` and CLI `--append` / MCP `append`
already go through `apply_to_player`.

MCP `loaded` is `items.len()`. `queue_add` stays paths-only.

Update `znicz-mcp/skills/playlist-curation/SKILL.md`: stream URL lines play;
save may contain `#EXTINF` and URLs.

## Tests

Loopback-only where a stream is actually opened. No public stations.

Core (`playlist.rs`):

- `http://` / `https://` become stream items; mixed file + URL order preserved
- `#EXTINF:-1,Live` then a URL → `QueueItem::stream("Live", url)`
- Bare URL → name is the URL
- `#EXTINF` then a local file → file row, title ignored, `skipped` unchanged
- `ftp://` (or other non-http `://`) still skipped
- Missing local file still skipped; URL-only playlist is not empty
- `write_text` then `parse` round-trips a file, a named station, and a bare URL
- `write_text` of a named station contains `#EXTINF:-1,` and the URL
- Files-only write still has no `#EXTINF` and still round-trips absolute paths
- `apply_to_player` with empty `items` still errors and leaves the queue
- `apply_to_player(..., append: true)` enqueues a stream row without needing
  to open it

TUI:

- Playlists `n` on a station-only or mixed queue **opens** the save prompt
- Help / `keys.rs` document `p` as previous and do **not** list `N` as previous
- `N` does not move queue position (two-file queue, press `N`, stay put)
- Overlay `n` / `e` / `c` / `d` unchanged

MCP:

- `save_playlist` succeeds with a station in the queue and the file contains
  the URL (and `#EXTINF` when named)
- `import_playlist` / `play_playlist` of a fixture with an `http://` line
  reports `loaded` including that stream

Replace tests that currently require save to fail on streams
(`m3u_paths_refuses_*`, `playlist_save_of_a_stream_queue_is_refused`,
`playlist_save_of_a_mixed_queue_is_refused`,
`save_playlist_errors_when_the_queue_has_a_station`).

## Version and docs

Bump `[workspace.package] version` **0.3.2 → 0.3.3** (compatible addition).

Same change:

- README essentials: `p` previous (no `N`)
- `wiki/Domain/Formats-and-Metadata.md` — load/save table, `#EXTINF` for names,
  http(s) lines play
- `wiki/Architecture/TUI.md` — previous is `p`; playlists can hold streams
- `wiki/Architecture/MCP.md` — playlists are mixed files + URLs
- `wiki/Plans/Roadmap.md` — M3U stream lines done; ICY and HLS remain
- `wiki/Issues.md` — later radio leftover is ICY and HLS, not M3U stream lines
- `znicz-mcp/skills/playlist-curation/SKILL.md`

Overlay keys stay `n` / `e` / `c` / `d`. Do not invent ICY or HLS in the wiki.
