# AudioAddict song covers (Phase 5.5)

**Date:** 2026-09-02
**Status:** Approved
**Crates:** `znicz-core` (URL parse + JSON lookup), `znicz-tui` (cover choice)
**Version:** **0.4.1 → 0.4.2** (compatible addition)
**Depends on:** [Radio and stream cover art](2026-09-02-radio-cover-art-design.md) (0.4.1)

## Problem

RadioTunes, DI.FM, RockRadio, and the rest of the AudioAddict network put **`StreamTitle`** in Icecast metadata and **never `StreamUrl`**. The 0.4.1 ICY image path therefore never runs. They do publish the current song’s cover as JSON (`art_url` on `track_history`).

## Goals

1. While an AudioAddict stream plays, show that channel’s **current song cover** in the same slot as ICY / station art.
2. Identify the channel from **`TrackInfo.url` only** (host + path). Do not use `StreamTitle` matching.
3. Cover order: **ICY image → AudioAddict `art_url` → station `art` file → logo**.
4. JSON and image HTTP stay on the **cover worker**. No JPEG on IPC. Player thread unchanged.

## This slice does not include

- MusicBrainz / Cover Art Archive / iTunes (Phase 6)
- Filling `station.art` from AudioAddict **channel** logos
- Listen-key login or any AudioAddict authenticated API
- New CLI or MCP tools
- Treating Icecast `icy-url` (the site homepage) as a cover
- Disk cache of decoded bitmaps
- MCP `cover://current`
- Changing ICY parsing

## What you see

| Playing | Cover slot |
| --- | --- |
| File (embedded or not) | unchanged from 0.4.1 |
| Stream, ICY `StreamUrl` decodes as an image | that picture |
| AudioAddict stream, `art_url` fetches and decodes | that picture |
| JSON / JPEG in flight | previous AudioAddict image if any; else station `art`; else logo |
| AudioAddict stream, no `art_url` (after lookup) | station `art` or logo |
| Other stream, no ICY image | station `art` or logo (unchanged) |
| `cover_protocol = "off"` | logo; **no** JSON and **no** image GET |

No toast on miss. `debug` log on JSON or image failure. Failed **image** URLs stay in the existing process-lifetime set (do not GET that JPEG again this run). A JSON miss is **not** a permanent fail: retry after the 15s cache TTL.

## Data flow

```
TrackInfo.url
        │
        ▼
parse_audioaddict_channel → (network, channel_key) or skip
        │
        ▼
CoverCache worker (not the player thread)
  1. ICY StreamUrl → fetch_cover → decode          (unchanged)
  2. AudioAddict art_url → fetch_cover → decode    (this slice)
  3. else station art file → decode                (unchanged)
  4. else logo
        │
        ▼
same 8-row slot
```

`pick_stream_cover(icy, audioaddict, station)`: first `Embedded` wins in that order; otherwise `Logo`. `Pending` is not `Embedded`, so it falls through (station art stays while AudioAddict loads).

## Channel from the stream URL

`znicz_core::parse_audioaddict_channel(stream_url: &str) -> Option<(AudioAddictNetwork, String)>`

- Accept `http://` and `https://` only. Other schemes → `None`.
- Host (lowercase, port stripped):

| Host ends with | Network slug (`api.audioaddict.com/v1/{slug}/…`) |
| --- | --- |
| `radiotunes.com` | `radiotunes` |
| `di.fm` | `di` |
| `rockradio.com` | `rockradio` |
| `jazzradio.com` | `jazzradio` |
| `classicalradio.com` | `classicalradio` |
| `zenradio.com` | `zenradio` |

Anything else → `None` (no HTTP).

- Channel key = first path segment (`/datempolounge_hi?key` → `datempolounge_hi`). Then strip **one** trailing quality suffix, longest match first: `_aacplus`, `_aac`, `_premium`, `_hi`, `_med`, `_low`. Result: `datempolounge`. `/metal` stays `metal`. Empty path → `None`.
- **Ignore the query string.** Do not log it, do not put it in cache keys, do not send it to the API.

`AudioAddictNetwork` is a small enum (or a newtype over the slug string). Display / API path uses the slug in the table.

## JSON lookup

`znicz_core::audioaddict_cover_url(network, channel_key) -> Option<String>`

**Cover worker only.** May perform HTTP. Must not run on the UI thread or the player thread.

### Cache

Process-lifetime, per **network**:

- Map `channel_key → Option<https URL>` plus `fetched_at`.
- TTL **15 seconds**. Fresh cache: no HTTP, just map lookup (`None` in the map means “this channel has no art right now”).
- Stale or missing network: GET both:

  1. `GET https://api.audioaddict.com/v1/{slug}/currently_playing`
  2. `GET https://api.audioaddict.com/v1/{slug}/track_history`

- Replace the whole network map from those two bodies. Channels in `currently_playing` with no joinable `art_url` store `None`.
- Connect timeout **8s**. Follow http(s) redirects only. JSON body cap **1 MiB** each. Non-success, timeout, oversize, empty, or invalid JSON → keep the previous map if any, else empty; log `debug`; return `None` for this call.
- Tests inject the API **origin** (loopback). Production origin is `https://api.audioaddict.com`. No new crates (`ureq` already in `znicz-core`).

### Join

`currently_playing` is a JSON **array** of objects:

```json
{ "channel_id": 48, "channel_key": "datempolounge", "track": { "id": 15865 } }
```

`track_history` is a JSON **object** keyed by channel id **string** (`"48"`):

```json
{
  "48": {
    "art_url": "//cdn-images.audioaddict.com/a/f/9/a/4/7/af9a470e98f03d6a87a6e72bc0f8a204.jpg",
    "type": "track"
  }
}
```

For the requested `channel_key`, find that row in `currently_playing`, look up `track_history[channel_id]`, read `art_url`.

Normalize `art_url`:

- Trim.
- If it starts with `//`, prefix `https:`.
- Strip a trailing `{?…}` template (e.g. `{?size,height,width,quality,pad}`).
- After that it must be `http://` or `https://`. Else treat as missing.

Missing key, missing `art_url`, empty string, or `type` that has no usable `art_url` → `None` for that channel (fall through to station art / logo). Do not require `type == "track"` if `art_url` is present.

## TUI

`CoverKey::AudioAddict { network, channel }`.

`CoverCache::get` for this variant:

- If a cached **`Embedded`** image exists, return it immediately (no logo flash).
- If the JSON TTL is stale **or** there is no slot yet, send the key to the worker again.
- Worker: `audioaddict_cover_url` → if `Some`, `fetch_cover` + `decode_capped` (same as `CoverKey::Url`); if `None` or decode fail, `CoverReady::Logo` for this key (picker still falls through to station art).

`now_playing` `render_cover` (streams only): parse `track.url`; if `Some`, `get(AudioAddict { … })` as the middle argument to `pick_stream_cover`. `cover_protocol = "off"` still short-circuits before any `get`.

When the song changes, the `AudioAddict` key is unchanged (same channel). The 15s refresh (or a stale get) picks up the new `art_url`; `fetch_cover` uses the new JPEG URL. Until that decode lands, keep the previous AudioAddict image if we have one.

## Tests

Local loopback HTTP. **No** live `api.audioaddict.com` in CI.

- Parse: RadioTunes `…/datempolounge_hi?listenkey` → `(radiotunes, datempolounge)`; RockRadio `…/metal` → `(rockradio, metal)`; `di.fm` `…/lofiloungenchill_hi` → `(di, lofiloungenchill)`; `https://example.com/x` → `None`; query string does not affect the key.
- Suffix: `_aacplus` before `_aac`; unknown suffix left intact.
- Join: fixture `currently_playing` + `track_history` → `https://cdn-images.audioaddict.com/…` (protocol-relative `art_url`).
- Missing channel / missing `art_url` → `None`.
- Loopback: JSON over `http://127.0.0.1:…` with injected origin → `Some`; oversize / 404 → `None`.
- TUI `pick_stream_cover`: ICY `Embedded` wins over AudioAddict; AudioAddict wins over station; Pending AudioAddict + station `Embedded` → station.

Render tests stay at the 8-row slot. Do not pixel-diff Kitty. Do not hit the public API.

## Wiki (same change as the feature)

- [Phase-5-Album-Art.md](../../../wiki/Plans/Phase-5-Album-Art.md): 5.5 AudioAddict song covers; disk cache and `cover://current` still open
- [TUI.md](../../../wiki/Architecture/TUI.md): stream cover order includes AudioAddict `art_url`
- [Formats and metadata](../../../wiki/Domain/Formats-and-Metadata.md): RadioTunes / DI.FM / RockRadio covers from AudioAddict JSON, not ICY `StreamUrl`
- [Roadmap.md](../../../wiki/Plans/Roadmap.md): one line under Phase 5
- README: streams on those networks can show the current song cover

No new keys. `keys.rs` unchanged.

## Dependencies

Existing `ureq` + `serde_json` in `znicz-core`. Existing `image` in `znicz-tui`. No new crates.
