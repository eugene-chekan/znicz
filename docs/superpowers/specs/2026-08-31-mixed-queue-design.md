# Mixed queue (later radio)

**Date:** 2026-08-31
**Status:** Approved
**Crates:** `znicz-core`, `znicz-tui`, `znicz-mcp`, `znicz`

First slice of [later radio](../../../wiki/Plans/Roadmap.md#later-radio-after-phase-4).
Depends on Phase 4 (`QueueItem::File` / `QueueItem::Stream` already exist).

## Problem

Playing a station **replaces** the queue. Radio `a` only toasts that adding a
station to the queue is later. Next and previous toast when the queue is a
single stream, so you cannot line up files and stations and walk between them.
Saving an M3U from a mixed queue would drop stream rows without saying so.

## Goals

1. Files and stations may sit in **one queue**.
2. Radio **Enter** still **clears and plays**. Radio **`a`** **appends** the
   highlighted station and does **not** start or stop playback (same as
   Playlists `a`), including when the queue is empty.
3. **Next / previous** always mean the next / previous **row**, including
   leaving a live stream. A stream never ends on its own.
4. CLI and MCP match the TUI: `play_station` / `znicz station play` gain
   `append` (default false).
5. Saving an M3U **refuses** if any row is a station. No silent drop.

## This slice does not include

- ICY `StreamTitle` on the transport
- HLS (`.m3u8` segments)
- Playlist `http://` / `https://` lines as playable streams (next later-radio
  slice; M3U files still skip those lines)
- Writing `http://` lines when saving
- Changing Playlists Enter / `a`
- Phase 5 / 6, parked issues `#5`–`#9`

## Daily motion

You are playing a file. `R`, highlight a station, `a`. The station is appended.
The file keeps playing. Later `n` leaves the file (when it ends, or when you
press next) and can land on that station. While the station plays, `n` again
leaves the stream and plays the following row if there is one.

Enter on a station still throws away the file queue and starts that stream.

## Queue behaviour

`QueueItem` does not change. `Command::QueueAdd` already accepts stream rows.

| Action | Behaviour |
| --- | --- |
| Radio Enter / `play_station` / `znicz station play NAME` | `QueueClear`, add one stream, `QueuePlayIndex(0)` |
| Radio `a` / `play_station` `append: true` / `--append` | `QueueAdd` one stream only. Do not clear. Do not play. |
| Next / previous | `pick_next` / previous as today. Leaving a stream is allowed. |
| Next / previous, queue is **one** station | TUI toast (`radio has no next/previous track`). Engine stays a no-op. |
| Next / previous at the **end** of a mixed queue | Same as files: stop, or wrap if repeat all. No extra stream toast. |
| Seek | Refused only while the **current** row is a stream. |
| Repeat one | Next still moves on (existing `pick_next`). A stream never auto-ends, so it does not self-repeat. |
| Shuffle | May pick a stream row. Fine. |
| Duplicate stations | Allowed, like duplicate files. |
| Failed URL | Error, stop, do not keep the previous file playing (Phase 4). Press next again to skip the dead row. Do not auto-skip. |

CLI `znicz station play NAME --append` starts a **new** player process, so the
queue is empty: the station is enqueued and the TUI opens **stopped**. That
matches `znicz playlist play NAME --append` on a fresh process. On a running
MCP server, `append: true` appends to the live queue.

## Save playlist

`m3u_paths` (and TUI `n` save / MCP `save_playlist`):

- Queue empty: existing “queue is empty”
- **Any** `QueueItem::Stream`: error, e.g. `cannot save a queue that contains a radio station`
- All files: write paths as today

Do not filter streams out and write the rest.

## Core API

Extend `play_station` with append (default false in MCP/CLI):

```text
play_station(player, station, append: false)  → clear, add, play index 0
play_station(player, station, append: true)   → QueueAdd only
```

No new `Command` variant.

## TUI

Radio overlay `a`: `QueueAdd` the highlighted station; success toast (e.g.
`added Example`). Empty station list: existing “no stations” toast. Drop the
“adding a station to the queue is later” copy.

`keys.rs` RADIO: `a` is “add to the queue”, not “later”. Footer may include
`a add` again.

Skip-track toast stays **only** when `queue.len() == 1` and that row is a
stream.

## CLI

```bash
znicz station play Example
znicz station play Example --append
```

`--append` is the same flag shape as `znicz playlist play`.

## MCP

`play_station` params: `name`, `append` (default **false**). Update
`znicz-mcp/skills/radio-streaming/SKILL.md`. `queue_add` stays paths-only.

## Tests

Loopback-only where a stream is actually opened. No public stations.

- Core: `play_station(..., append: true)` does not clear; position/status
  unchanged; mixed next from a stream row plays the following file (engine
  `NextTrack`, no TUI)
- Core: `m3u_paths` errors when any row is a stream; still works for files only
- TUI: Radio `a` appends; current track unchanged; overlay `n`/`e` still not
  next/repeat
- TUI: queue file + station, on the station, `n` does not toast and the
  position moves
- TUI: single station, `n` still toasts
- TUI / MCP: save with a stream in the queue errors
- MCP: `play_station` default still clears; `append: true` appends

## Version and docs

Bump `[workspace.package] version` **0.3.0 → 0.3.1** (compatible addition).

Same change: README station examples, `wiki/Architecture/TUI.md` (Radio `a`),
`wiki/Domain/Formats-and-Metadata.md`, `wiki/Architecture/MCP.md`,
`wiki/Plans/Roadmap.md` (mixed queue done; other later-radio items remain),
skills as above. Overlay keys stay `n`/`e`/`c`/`d`; do not resurrect old `w` /
two-step add copy in the wiki.
