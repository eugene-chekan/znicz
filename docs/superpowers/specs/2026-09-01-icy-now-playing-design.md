# ICY now playing (StreamTitle on the transport)

**Date:** 2026-09-01
**Status:** Approved
**Crates:** `znicz-core` (HTTP reader + engine), TUI and MCP already read `TrackInfo`
**Version:** 0.3.6 → 0.3.7

## Problem

A radio row shows the **station name** on the transport. Icecast can splice
`StreamTitle` into the HTTP body when the client asks for it. Znicz does not
request that, so the current song never appears. Leaving metadata in the audio
bytes would glitch decode.

## Goals

1. Request Icecast metadata on every stream GET.
2. Strip metadata from the audio `Read` so Symphonia only sees coded audio.
3. When a non-empty `StreamTitle` arrives, put that whole string on now-playing
   (`TrackInfo.title` and `tags.title`).
4. Queue rows stay the station name. Session still stores that name, not the
   live song.

## This slice does not include

- HLS, PLS, XSPF
- Icecast `icy-br` (coded bitrate already measured from packets vs PCM)
- `StreamUrl` and other ICY fields
- Splitting `Artist - Track` into artist / title
- Saving the live song in `session.toml`
- Phase 5 album art
- Extra User-Agent or other HTTP headers for this feature

## What you see

While a stream plays:

| ICY | Now-playing `title` | `tags.title` | Queue row |
|-----|---------------------|--------------|-----------|
| None yet, or no `icy-metaint` | Station name | Empty | Station name |
| `StreamTitle='Song'` | `Song` | `Song` | Station name |
| `StreamTitle=''` | Station name | Cleared | Station name |
| Block present but unreadable | Unchanged | Unchanged | Station name |

Station name is the queue item’s `name` (same as today). No artist split.

TUI transport and inspector already prefer `tags.title` when set, else `title`.
MCP `get_player_state` and `znicz://now-playing` follow the same `TrackInfo`.

## HTTP and ICY bytes

Every stream GET sends `Icy-MetaData: 1`. Existing headers stay as they are.

Response header `icy-metaint` is case-insensitive.

- **Missing or `0`:** body is audio only. Title stays the station name.
- **`N > 0`:** after every `N` audio bytes: one length byte `L`, then `L × 16`
  metadata bytes. Strip those before the decoder. `L = 0` means no title this
  interval.

Parse the first `StreamTitle='…';` in the block (text between `StreamTitle='`
and the next `';`). Decode as UTF-8; invalid bytes are lossy. Ignore
`StreamUrl` and anything else.

The latest title lives on the reader (mutex). Three states:

- **Unset** — no `StreamTitle` parsed yet
- **Empty** — parsed empty string
- **Text** — parsed non-empty string

A truncated or junk block: drop the metadata bytes, keep decoding, leave the
stored title as-is (still Unset if nothing valid has arrived).

## Engine

`TrackStarted` still uses the station name and empty tags.

On each decode pump, beside coded bitrate:

1. Read the ICY state from the decoder.
2. **Unset:** do not write `TrackInfo`.
3. **Text:** set `title` and `tags.title` to that string.
4. **Empty:** set `title` to the current queue row’s station name; set
   `tags.title` to `None`.
5. Write only when those fields would change. Then send `StateChanged` so MCP
   subscribers see a new song. Bitrate updates stay quiet.

TUI already redraws from `player.state()`. Queue labels stay the item name.

## Tests

Local loopback only (no public radio).

- GET includes `Icy-MetaData: 1`.
- Body with `icy-metaint` spliced in: the decoder’s audio bytes exclude the
  metadata.
- After `StreamTitle='Song';`, `title` and `tags.title` are `Song`.
- Empty `StreamTitle` restores the station name and clears `tags.title`.
- Junk metadata: audio still decodes; title unchanged.
- No `icy-metaint`: still plays; title stays the station name.

Existing tests that forbade `icy-metadata` on the GET must expect the header.

## Wiki

Move ICY off Later radio into Phase 4 done. Update:

- [Formats and metadata](../../../wiki/Domain/Formats-and-Metadata.md) (radio)
- [Playback pipeline](../../../wiki/Domain/Playback-Pipeline.md)
- [TUI](../../../wiki/Architecture/TUI.md) (radio transport currently says
  station name only)
- [Roadmap](../../../wiki/Plans/Roadmap.md), [Issues](../../../wiki/Issues.md)
  later-radio lists
- `znicz-mcp/skills/radio-streaming/SKILL.md`
