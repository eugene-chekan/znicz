# MCP `queue_add` http(s) URLs

**Date:** 2026-09-01
**Status:** Approved
**Crates:** `znicz-mcp` (behaviour), `znicz-core` (shared `http://` / `https://` check)

Same development cycle as MCP `queue_remove` (0.3.5).

## Problem

The engine queue already holds files and streams. MCP `queue_add` maps every
string through `QueueItem::file`, so an agent cannot enqueue a stream without
saving a station and calling `play_station` with `append: true`.

## Goals

1. `queue_add` treats `http://` and `https://` strings (case-insensitive, same
   rule as M3U) as `QueueItem::stream(url, url)`.
2. Any other string stays a file path (`ftp://`, local paths, …).
3. One call may mix files and streams. Playback does not start. `stations.toml`
   is not written.

## This slice does not include

- Optional display names on `queue_add` (no `#EXTINF` equivalent)
- Tagged JSON items (`{ "path" }` / `{ "url", "name" }`)
- A new `queue_add_stream` tool
- CLI `queue` helpers
- ICY, HLS, PLS, XSPF

## Behaviour

`paths: Vec<String>` is unchanged. For each entry:

- `http://` or `https://` prefix → stream row; name is the URL
- otherwise → file row

`Command::QueueAdd` is unchanged. Out-of-range `queue_remove` stays a no-op.

## Tests and docs

MCP test: mix a path and an `https://` URL; assert `kind` file then stream.
Wiki `Architecture/MCP.md` and `mcp-control` / `radio-streaming` skills drop
“`queue_add` stays paths-only.”
