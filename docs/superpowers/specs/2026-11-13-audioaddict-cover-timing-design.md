# AudioAddict covers follow the song

**Date:** 2026-11-13
**Status:** Approved
**Crates:** `znicz-core`, `znicz-tui`
**Fixes a regression from:** [Stream cover sources](2026-11-10-stream-cover-sources-design.md) (0.11.0)

## Problem

On RadioTunes / DI.FM streams the Stage cover is always the previous song's
(Smooth Beats: "Else Born - This Time/Remember" showed "Parc - 4U"; the next
song showed Else Born). Before 0.11.0 the TUI keyed AudioAddict covers by
channel and re-asked whenever the 15 s API cache went stale, so the cover caught
up. Since 0.11.0 covers are keyed by the full query (title included) and a found
cover is never re-asked, so the first answer sticks.

That first answer is stale because the source reads the global
`track_history` (lags the stream title by ~10–15 s) through a 15 s cache.

Measured: `currently_playing/channel/{id}` reports the new track with a
`start_time` one second *before* the ICY title changes, but Cloudflare caches it
(`cache-control: max-age=4`), so it is served 5–7 s late; a unique query
parameter bypasses the cache (`cf-cache-status: MISS`). `tracks/{id}` is not
cached and carries the track's cover (`asset_url`, else `images.default`), which
is what the RadioTunes app shows.

## Behaviour

- **New answer: try again shortly.** `Found::Soon` from a source stops the chain
  (later sources are not tried, so a search hit cannot win the race and stick)
  and resolves to `Resolution::Soon`. Not remembered.
- **TUI:** `Soon` keeps what is shown (previous cover while pending) and retries
  after **1.5 s**; `Unsure` keeps its 15 s retry.
- **AudioAddict lookup** `audioaddict_cover_lookup(network, channel_key, title)`:
  1. Channel id from the global `currently_playing` list (key → id), cached for
     an hour; refreshed when the key is unknown.
  2. `GET /v1/{slug}/currently_playing/channel/{id}?t=<unique>` (no cache).
  3. With a title: the track's `"{display_artist} - {display_title}"` must equal
     the title after `cover::title::fold` and trimming; otherwise `NotYet`.
  4. Cover: `GET /v1/{slug}/tracks/{track_id}` → `asset_url`, else
     `images.default` (template stripped, `//` → `https:`), remembered per track
     id (cap 256). No art → `NoArt`.
  5. Any request failing → `RefreshFailed`. Unknown channel → `NoArt`.
  `track_history` is no longer used.
- **AudioAddict source:** `NotYet` → `Found::Soon`, but once the same
  channel + title has been `NotYet` for **30 s**, → `Found::NotFound` (the chain
  moves on to search). `RefreshFailed` → `Unavailable`, `NoArt` → `NotFound`.

## Testing

- Chain: `Soon` stops the chain and resolves `Soon`; it is not remembered.
- TUI: `Soon` is retried after the short interval, keeping the previous cover.
- AudioAddict against a local server: lagging channel → `NotYet`; match (case /
  spacing) → `tracks/{id}` cover; `images.default` template stripped; track
  cover requested once per id; no title → current track's cover; failures →
  `RefreshFailed`; unknown channel → `NoArt`; cache-busting parameter differs
  between calls.
- Source: `NotYet` → `Soon`; after the give-up window → `NotFound`.
- Live (`#[ignore]`): Smooth Beats' current track resolves a cover.

## Delivery

Version **0.13.2**. `wiki/Architecture/Cover-Sources.md` (AudioAddict section,
`Soon`), `wiki/Architecture/TUI.md` (retry intervals),
`wiki/Domain/Formats-and-Metadata.md` if it describes `art_url`.
