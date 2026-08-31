# Radio streams (Phase 4)

**Date:** 2026-08-31
**Status:** Approved
**Crates:** `znicz-core`, `znicz-library` (data-dir helper only), `znicz-tui`, `znicz-mcp`, `znicz`

## Problem

Znicz only plays local files. MCP radio tools return “not implemented”. Playlist
`http://` lines are skipped. There is no station list and no way to play an
Icecast/HTTP stream from the TUI, CLI, or an agent.

## Goals

1. Play an **HTTP or HTTPS Icecast-style byte stream** (MP3 / AAC / Ogg that
   Symphonia can decode from a continuous `Read`). Same decoder, different
   source.
2. Saved stations are a **file** of name + URL, not SQLite and not `config.toml`.
3. Playing a station **clears the queue** and puts that one stream on it
   (same motion as playlist Enter).
4. TUI, CLI, and MCP can **list, add, rename, change URL, remove, and play**.
5. Keymap: **`R`** radio overlay, **`r`** reload/refresh, **`e`** repeat
   (moved off `r`).

## This slice does not include

- HLS (`.m3u8` segment playlists) — **planned later** (see roadmap)
- ICY “now playing” metadata in the TUI — **planned later** (see roadmap)
- Playlist `http://` lines — stay skipped **in this slice**; **planned later**
- Mixing a station with local files in one queue — **this slice** always
  replaces the queue; **planned later**
- PLS / XSPF — already “later” under playlists, not radio
- A settings screen — already [#6](https://github.com/eugene-chekan/znicz/issues/6)
- Changing SQLite schema — not a product feature; stations are a file
- Dummy audio in CI — engineering only; keep skipping hardware when `CI` is set

## Daily motion

You press `R`, pick a station, press Enter. The file queue is replaced by that
stream and it starts. `a` adds a station (name, then URL). `w` renames, `c`
changes the URL, `d` deletes. Stop and pause still work. Seek, next, and
previous toast that they do not apply to radio.

## Queue and commands

The queue is no longer `Vec<PathBuf>`. A row is:

```text
QueueItem::File(PathBuf)
QueueItem::Stream { name: String, url: String }
```

`Command::Play` and `QueueAdd` take `QueueItem`. Playlist load still produces **files only** and uses `QueueAdd` with file items.

Now-playing `TrackInfo` for a stream uses the station **name** as the title,
the URL as the identity (not a filesystem path), **no duration**, and the codec
Symphonia reports once the stream has probed. The signal-path line still shows
file (stream) format → device. Bit-perfect vs resampled uses the same rules as
files; a lossy radio stream will often show resampled. That is correct, not a
bug.

Seek on a stream: do not move the HTTP cursor. Toast that radio cannot seek.
Next / previous with a single stream in the queue: toast, do not error the
engine. Repeat (`e`) and shuffle (`z`) do not change an infinite stream.

Pause: stop pulling bytes. Resume: continue the body if it is still open,
otherwise open the URL again.

## HTTP source

`AudioSource` stays the player interface (`wiki/Rust/Traits.md`). Files keep
`LocalFileSource`. Radio adds `HttpStreamSource`:

- `open_reader()` — blocking HTTP GET (HTTPS included), follow redirects
- returns `Box<dyn Read + Send>` for the player thread
- connect timeout; the read may block while the stream is live
- no ICY metadata parsing this slice (request the body as audio)

If the URL is not a byte stream Symphonia can probe, play fails: error toast /
CLI error / MCP error. No silent skip.

Tests use a **local** HTTP fixture (in-process or loopback), not a public
station. CI already skips hardware output.

## Stations file

`stations.toml` next to playlists (same parent as `library.db`):

```
Linux:   ~/.local/share/znicz/stations.toml
Windows: %APPDATA%\znicz\stations.toml
```

Override with `ZNICZ_STATIONS_PATH`. Create the parent folder on first write.

```toml
[[station]]
name = "Example"
url = "https://example.com/stream"
```

- UTF-8. Missing file means an empty list, not an error.
- `name`: trim; reject empty, `/`, `\`, `..`. Unique, case-sensitive after trim.
- `url`: trim; must contain `://` (http or https).
- Duplicate name on add: error, do not overwrite.
- Rename to a name that already exists: error.
- Listing order: file order (stable). TUI cursor follows that list.

Saving the queue as M3U still writes **local paths only**. A queue that is only
a stream cannot be saved as a playlist; toast or CLI error.

## Keymap (breaking)

| Key | Was | Now |
| --- | --- | --- |
| `R` | Library reload / device rescan | **Radio overlay** (toggle, like `P`) |
| `r` | Repeat | **Reload** the surface in front: library list, device list, playlists folder when `P` is open, station file when `R` is open |
| `e` | (unbound globally) | **Repeat** (off → all → one) |

Shuffle stays `z`. While a radio **prompt** is open, characters go into the
field (including `s`, `n`, `e`, `R`), same as playlist save “To Listen”.

## TUI overlay

`Modal::Radio`, same shape as playlists. `R` opens and reloads the file; `R`
again or Esc closes.

| Key | Action |
| --- | --- |
| Enter | Clear queue, play highlighted station |
| `a` | Add: prompt **name**, Enter, then **URL**, Enter. Write the file |
| `w` | Rename highlighted (name prompt, URL unchanged) |
| `c` | Change URL highlighted (URL prompt, name unchanged) |
| `d` | Delete from the file (immediate, like queue `d`) |
| `r` | Re-read `stations.toml` |
| Esc | Close overlay, or cancel the current prompt |

Empty list: `a` still adds. Enter / `w` / `c` / `d` on an empty list do nothing
(optional info toast). Failed HTTP play: error toast, overlay may stay open.

Transport: station name; time shows as `—` (unknown duration). Seek keys toast.
Help overlay and footer hints come from `znicz-tui/src/keys.rs` (one table, no
stale copy).

## CLI

```bash
znicz station list
znicz station add "Example" https://example.com/stream
znicz station play Example
znicz station remove Example
znicz station rename Example "New name"
znicz station url Example https://example.com/other
```

`play` starts the TUI with that station (clear-and-play), like
`znicz playlist play`.

## MCP

Implement the stubs and match the TUI:

| Tool | Role |
| --- | --- |
| `add_radio_station` | name + url |
| `list_stations` | rows from the file |
| `play_station` | name; clear-and-play; `send_blocking` |
| `remove_radio_station` | name |
| `rename_radio_station` | name + new_name |
| `set_station_url` | name + url |

Resource `znicz://stations` is the current file as JSON. Update
`znicz-mcp/skills/radio-streaming/SKILL.md`. Player-state snapshots must show
stream rows (name + url), not fake file paths.

## Version and docs

Bump `[workspace.package] version` **0.2.0 → 0.3.0** (new product phase).

Same change: README keymap and usage, `wiki/Domain/Formats-and-Metadata.md`
(radio section; playlists still skip URLs), `wiki/Architecture/TUI.md`,
`wiki/Architecture/MCP.md`, `wiki/Architecture/Core-Engine.md` if play/queue
types change, `wiki/Rust/Traits.md` (`HttpStreamSource`),
`wiki/Plans/Roadmap.md` Phase 4 **Done**, `wiki/Home.md` if the run-it blurb
needs a station command.

## Out of scope for this slice

Phase 5 (album art) and Phase 6 (MusicBrainz) stay planned. Parked TUI issues
`#5`–`#9` stay parked. Later **radio** work (ICY, HLS, M3U stream lines, mixed
queue) is on the [roadmap](../../../wiki/Plans/Roadmap.md), not in this spec.
