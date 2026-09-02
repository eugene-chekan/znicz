# Radio and stream cover art (Phase 5.4)

**Date:** 2026-09-02
**Status:** Approved
**Crates:** `znicz-core` (ICY `StreamUrl`, `fetch_cover`, station `art`), `znicz-tui` (cover choice + radio form), `znicz-mcp` (`set_station_art`), `znicz` (CLI)
**Version:** **0.4.0 → 0.4.1** (compatible addition)
**Depends on:** [Album art in the TUI](2026-09-02-album-art-design.md) (0.4.0), [ICY now playing](2026-09-01-icy-now-playing-design.md)

## Problem

A stream has no filesystem `path`, so the cover slot always draws the bundled logo. Stations can have a local picture. Icecast can send `StreamUrl` in the same metadata block as `StreamTitle`; when that URL is an image, the slot should show the current song.

## Goals

1. Optional **station `art`**: a **local image file** on each row in `stations.toml`. That picture is the default cover while the station plays.
2. When ICY **`StreamUrl`** fetches and **decodes as an image**, that picture **replaces** station art for the current song.
3. Same cover slot as today. Files still use embedded art. No JPEG on JSON IPC. The player thread does not fetch or decode images.
4. Users can set and clear `art` from the TUI radio form, `znicz station art`, and MCP `set_station_art`.

## This slice does not include

- Disk cache of decoded bitmaps
- MCP `cover://current`
- `folder.jpg` / `cover.jpg` beside a library file
- MusicBrainz / Cover Art Archive (Phase 6)
- Station art as an `http(s)` URL (local path only)
- Guessing from the `StreamUrl` path or `HEAD` `Content-Type`
- HLS / PLS / XSPF
- Putting cover bytes on `PlayerState`

## What you see

| Playing | Cover slot |
| --- | --- |
| File with embedded art | that picture (unchanged) |
| File with no picture | Znicz logo (unchanged) |
| Stream, `StreamUrl` decoded as an image | that picture |
| Stream, fetch in flight / failed / not an image / no `StreamUrl` | station `art` file if set |
| Stream, no station `art` (playlist `http` row, or empty `art`) | Znicz logo |
| `cover_protocol = "off"` | logo in the slot (unchanged) |

While a song cover is loading, keep showing station art (or the logo). Do not flash the mark. Failed or rejected URLs never toast.

## Data flow

```
ICY block → TrackInfo.icy_stream_url (optional string, not bytes)
stations.toml art → local path on the matching station
        │
        ▼
TUI CoverCache worker (not the player thread)
  1. http(s) StreamUrl → znicz_core::fetch_cover → decode (512 px cap)
  2. else station art path → open as an image file → decode
  3. else logo
        │
        ▼
same 8-row slot (half-cell left pad, one-cell gap to chrome)
```

Match a station by `TrackInfo.url` == `Station.url` (file order, first hit). Playlist `http` rows have a URL and no `art`.

Station `art` is a **picture file** (PNG/JPEG/…). It is **not** `read_cover`: that helper opens audio files with lofty. The worker uses the `image` crate on the file bytes, same 512 px cap as embedded covers.

## ICY `StreamUrl`

Parse the first `StreamUrl='…';` in the metadata block the same way as `StreamTitle` (UTF-8, lossy), **even when `StreamTitle` is missing**. Hold three states beside the existing title slot: **Unset**, **Empty**, **Text**.

Copy onto `TrackInfo.icy_stream_url` each tick, same place the engine already applies `StreamTitle`:

| ICY | `icy_stream_url` |
| --- | --- |
| Unset / pattern missing | unchanged (`None` until the first URL) |
| `StreamUrl='https://…'` | that string |
| `StreamUrl=''` | `None` (fall back to station art / logo) |

Queue rows stay `{name, url}`. `session.toml` does not store `StreamUrl`.

`TrackInfo` already has `url` for the audio stream. `icy_stream_url` is a second optional string. Serde: skip if `None`. Old clients ignore the new field.

## `fetch_cover`

`znicz_core::fetch_cover(url: &str) -> Option<CoverArt>` — TUI worker only.

- `http://` / `https://` only. `file://` and anything else → `None`, no GET.
- Connect timeout **8s** (same as radio GET). Body cap **2 MiB**. Follow http(s) redirects only.
- Return `CoverArt { mime, bytes }` (`Content-Type` when it is `image/*`, else sniff PNG/JPEG magic, else `application/octet-stream`).
- HTTP error, timeout, oversize, empty body → `None`. Log `debug`.

The TUI worker then `decode_capped` as for files. If decode fails, treat as `None`.

Remember failed URLs for the **process lifetime**. Do not GET that string again this run. Successful fetches sit in the existing in-memory cover cache (cap 16).

## Station `art`

```toml
[[station]]
name = "Example"
url = "https://example.com/stream"
art = "/home/you/Pictures/example.png"
```

`art` is optional. Omit or empty = no station picture. Unknown extra TOML keys still ignore as today.

Rules when the value is non-empty:

- Trim. Expand `~` the same way the CLI expands `~/` paths.
- Reject `http://` / `https://` (station art is a file).
- On **save** (TUI form, CLI, MCP): the path must exist as a file. Otherwise error to that surface (TUI toast, CLI/MCP error), do not write. Store the canonical absolute path so later play does not depend on cwd.
- While **playing**: if the file is gone or will not decode, skip to ICY or the logo. Debug log, no toast.

Copy station copies the `art` path. Rename and `set_station_url` leave `art` as-is.

### TUI

Radio add/edit form: third field `art:`. Tab cycles **name → url → art**. Empty `art` is allowed. Copy still copies name (and the path). Footer/help: `e` is “edit name, URL, and art”. No new key.

### CLI

```bash
znicz station art "Example" /path/to.png
znicz station art "Example" --clear
```

### MCP

`set_station_art`: `name` (string), `path` (optional string). Omitted/`null`/empty clears. `znicz://stations` includes `art` when set. Radio skill lists the tool.

`add_radio_station` stays name+url (no art on add). Set art afterwards.

## Tests

Local loopback HTTP. No public station.

- Parse `StreamUrl` with and without `StreamTitle` in the same block; missing pattern is `None`.
- Strip-read still drops metadata from the audio body when the block has both fields.
- Engine: non-empty URL appears on `TrackInfo`; empty URL clears `icy_stream_url`.
- `fetch_cover`: tiny PNG over loopback → `Some`; HTML body / oversize / `file://` → `None`.
- A failed URL is not requested a second time in the same cache.
- Station save/load round-trips `art`; http(s) in `art` is rejected; missing file on save errors.
- Copy keeps `art`. Clear removes it from the file.
- Cover choice (TUI worker tests): ICY image wins; else `art` file; else logo. Pending ICY keeps the previous station/logo image.

Render tests keep the 8-row slot. Do not pixel-diff Kitty.

## Wiki (same change as the feature)

- [Phase-5-Album-Art.md](../../../wiki/Plans/Phase-5-Album-Art.md): 5.4 radio/stream covers done; disk cache and `cover://current` still open
- [TUI.md](../../../wiki/Architecture/TUI.md): stream cover choice; radio form third field
- [Formats and metadata](../../../wiki/Domain/Formats-and-Metadata.md): `art` on stations; ICY `StreamUrl` as cover when it is an image
- [MCP.md](../../../wiki/Architecture/MCP.md): `set_station_art`
- [Roadmap.md](../../../wiki/Plans/Roadmap.md): one line under Phase 5 / later radio art
- README: `znicz station art`; radio feature line
- `znicz-tui/src/keys.rs`: `e` help text; README key table unchanged (no new key)

## Dependencies

Existing `ureq` in `znicz-core`. Existing `image` in `znicz-tui`. No new crates.
