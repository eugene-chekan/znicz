# Stream cover sources

**Date:** 2026-11-10
**Status:** Approved
**Crates:** `znicz-core`, `znicz-tui`, `znicz`

## Problem

Stream covers come from three hard-wired places: the ICY `StreamUrl`, the
AudioAddict API (RadioTunes, DI.FM, RockRadio), and the station's local `art`
file. Each is its own `CoverKey` variant, the Stage view asks for all three, and
`pick_stream_cover` chooses. Adding a source means touching the key enum, the
cover worker's `match`, the picker, and `views/stage.rs`.

Most stations send no cover at all. Two examples from `stations.toml`:

| Station | Server | ICY title | ICY `StreamUrl` |
| --- | --- | --- | --- |
| FluxFM – Jazzradio Schwarzenstein | radiosphere.io | `Ben Webster - Willow Weep For Me` | none |
| HEAVY-MUSIC.NET | Icecast 2.4.4 | `Serotonal - Memory Leaves` | none |

Both send `Artist - Title`. Deezer finds the first (1000 px cover); neither
Deezer nor iTunes knows the second, but MusicBrainz finds the release and the
Cover Art Archive has its front cover.

## Goals

1. A chain of cover sources behind one trait in `znicz-core`; a new source is
   one file plus one line in the registry.
2. An artist / title search (Deezer, then MusicBrainz + Cover Art Archive) so
   ordinary Icecast / Shoutcast stations get per-track covers.
3. The TUI asks one question per stream and ICY title; it no longer knows
   which sources exist.

## Non-goals

- Covers for local files (they keep embedded art only). Folder images and
  online lookups for files are later (Phase 6 overlaps).
- A disk cache of covers or lookups.
- Station-specific APIs beyond AudioAddict (the chain makes them easy later).
- Showing covers over MCP or IPC.

## Behaviour

- For a stream, the cover is the first that loads from, in order:
  1. **`icy_url`** — the ICY `StreamUrl`, when it is an `http(s)` URL.
  2. **`audioaddict`** — the AudioAddict channel cover (unchanged logic).
  3. **`deezer`** — artist / title search.
  4. **`musicbrainz`** — recording search, then the Cover Art Archive.
  5. **`station_art`** — the station's local `art` file.
  6. The logo.
- Search (3, 4) runs only when the ICY title parses as `Artist - Title`
  (split on the first ` - ` or ` – `, both sides trimmed and non-empty) and the
  title is not the station name. It is off when `[covers] search = false` in
  `config.toml`, or for one station with `cover_search = false` in
  `stations.toml`.
- When the ICY title changes, the previous cover stays on screen until the new
  one is resolved (no flash of the logo between tracks). A new station starts
  from the logo.
- A stream with no ICY title is resolved again every 15 s (the AudioAddict
  cache lifetime), so AudioAddict covers keep following the playing track.
- A lookup answer, including "nothing found", is remembered for the session, so
  each title is looked up once. Network failures are not remembered and are
  retried on the next resolve.

## Design

### Module `znicz-core/src/cover/`

```
cover/
  mod.rs          StreamQuery, Found, CoverSource, StreamCovers, default_sources()
  title.rs        split_icy_title, artist_matches
  icy_url.rs      IcyUrlSource
  audioaddict.rs  AudioAddictSource (wraps crate::audioaddict)
  deezer.rs       DeezerSource
  musicbrainz.rs  MusicBrainzSource
  station_art.rs  StationArtSource
```

`cover_fetch.rs` (`fetch_cover`) stays where it is and is used by the resolver.
`audioaddict.rs` keeps its channel map, parsing, and cache; `cover/audioaddict.rs`
only adapts it to the trait.

```rust
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct StreamQuery {
    pub stream_url: String,
    pub station_name: String,
    /// The ICY StreamTitle, when the station sent one.
    pub icy_title: Option<String>,
    /// The ICY StreamUrl, when the station sent one.
    pub icy_url: Option<String>,
    pub station_art: Option<PathBuf>,
    /// Artist / title search allowed (global and per-station switches).
    pub search: bool,
}

pub enum Found {
    /// An image URL to download.
    Url(String),
    /// Image bytes already in hand (local file).
    Art(CoverArt),
    /// This source has nothing for this query. Remembered.
    NotFound,
    /// The source could not answer (network, rate limit). Not remembered.
    Unavailable,
}

pub trait CoverSource: Send + Sync {
    /// Short stable name, used in cache keys and logs.
    fn name(&self) -> &'static str;
    /// What this source's answer depends on, or `None` when it does not apply
    /// to this query. Two queries with the same key share one lookup.
    fn key(&self, query: &StreamQuery) -> Option<String>;
    /// Look the cover up. Runs on the cover worker thread; may block.
    fn find(&self, query: &StreamQuery) -> Found;
}

pub struct StreamCovers { /* sources, lookup cache */ }

impl StreamCovers {
    pub fn new(sources: Vec<Box<dyn CoverSource>>) -> Self;
    /// Try each applicable source in order; the first image that downloads wins.
    pub fn resolve(&self, query: &StreamQuery) -> Option<CoverArt>;
}

/// The built-in chain, in the order listed under Behaviour.
pub fn default_sources() -> Vec<Box<dyn CoverSource>>;
```

Resolver rules:

- For each source whose `key` is `Some`, look up `(name, key)` in the cache;
  on a miss call `find` and store the answer unless it is `Unavailable`.
- `Url` → `fetch_cover`. A URL that fails to download or decode counts as
  `NotFound` for that `(name, key)` and is remembered.
- The lookup cache holds at most 256 entries, oldest dropped first.
- The AudioAddict source's `key` includes the channel and the ICY title, so a
  new track looks up again; with no ICY title it relies on the 15 s re-resolve
  and on `crate::audioaddict`'s own freshness (its answer is `Unavailable`
  when the channel map is stale and the refresh fails, which keeps the cover
  on screen).

### Title helpers (`cover/title.rs`)

- `split_icy_title(title, station_name) -> Option<(artist, title)>`: split on
  the first ` - ` or ` – `; trim; `None` if either side is empty, if there is
  no separator, or if the whole title equals the station name (case-folded).
- `artist_matches(wanted, found) -> bool`: case-insensitive, accents removed
  (Unicode NFKD via a new `unicode-normalization` dependency in
  `znicz-core`, combining marks dropped), whitespace collapsed; true when
  either contains the other. Guards Deezer and MusicBrainz against wrong hits.

### Deezer (`cover/deezer.rs`)

- `GET https://api.deezer.com/search?q=artist:"A" track:"T"&limit=5`.
- First result whose `artist.name` passes `artist_matches` → `Url(album.cover_xl)`
  (falling back to `cover_big`). No match → `NotFound`. HTTP or JSON error →
  `Unavailable`.
- `key`: `artist|title`, case-folded. Applies only when `query.search` and the
  title splits.
- Origin is a constructor argument so tests point it at a local server.

### MusicBrainz (`cover/musicbrainz.rs`)

- `GET https://musicbrainz.org/ws/2/recording/?query=artist:"A" AND recording:"T"&fmt=json&limit=5`
  with `User-Agent: znicz/<version> ( https://github.com/eugene-chekan/znicz )`.
- Recordings with `score >= 90` whose credited artist passes `artist_matches`;
  their releases, in order, up to three. For each release
  `GET https://coverartarchive.org/release/<mbid>/front-500`: the first that
  returns 200 → `Url` of the final (redirected) location. None → `NotFound`.
- Requests to `musicbrainz.org` are at least 1 s apart (a process-wide
  `Mutex<Instant>`), per the MusicBrainz rate limit. HTTP 503 → `Unavailable`.
- `key`: as Deezer. Origins (MusicBrainz, Cover Art Archive) are constructor
  arguments for tests.

### Other sources

- `IcyUrlSource`: applies when `icy_url` is `http(s)`; `key` = the URL;
  `find` → `Url(icy_url)`.
- `AudioAddictSource`: applies when `parse_audioaddict_channel(stream_url)` is
  `Some`; maps `AudioAddictLookup::{Found, NoArt, RefreshFailed}` to
  `Url`, `NotFound`, `Unavailable`.
- `StationArtSource`: applies when `station_art` is set; `key` = the path;
  reads the file → `Art`, unreadable → `NotFound`.

### TUI (`znicz-tui/src/cover.rs`, `views/stage.rs`)

- `CoverKey` becomes `File(PathBuf)` | `Stream(StreamQuery)`; `Url`,
  `ImageFile`, and `AudioAddict` go.
- The cover worker owns one `StreamCovers::new(default_sources())` and calls
  `resolve` for `Stream` keys; bytes are decoded with `decode_capped` as today.
  A `None` → `CoverReady::Logo`.
- `CoverCache` drops the failed-URL set and the AudioAddict stale-refresh
  special case. `Stream` entries whose query has no ICY title are re-requested
  when older than 15 s (stale entries keep their image while refreshing).
- `stage.rs` builds one `StreamQuery` from the current track and the matching
  station: `stream_url = track.url`, `station_name` from the queue item,
  `icy_title = Some(track.title)` when it differs from the station name,
  `icy_url = track.icy_stream_url`, `station_art`, and `search =
  app.covers_search && station.cover_search`.
- "Keep the previous cover": `App` remembers the last `Embedded` stream cover
  and its `stream_url`; while the new key is `Pending` and the `stream_url` is
  the same, Stage draws the remembered image.
- `pick_stream_cover` and its test go; their ordering is now the source order,
  tested in core.

### Config

- `config.toml`: `[covers] search = true` (default). Wired through
  `znicz/src/main.rs` into `TuiConfig { cover_search: bool }`.
- `stations.toml`: `Station { cover_search: bool }`, default `true`, written
  only when `false` (`skip_serializing_if`).

## Testing

Unit (core):

- `split_icy_title`: `Ben Webster - Willow Weep For Me` → (`Ben Webster`,
  `Willow Weep For Me`); en dash; first separator wins
  (`A - B - C` → `A`, `B - C`); no separator, empty side, or the station name →
  `None`.
- `artist_matches`: case, accents (`Björk` / `bjork`), containment
  (`Ben Webster Quartet` / `Ben Webster`), mismatch (`Serotonal` / `Stereo Total`).
- Resolver with fake sources: order (first image wins); a source whose `key`
  is `None` is skipped; `NotFound` is remembered (second resolve does not call
  `find`); `Unavailable` is not; a `Url` that fails to download falls through
  to the next source and is remembered; the cache cap.
- Deezer against a local server: matching artist → `cover_xl`; first result
  with the wrong artist skipped; empty `data` → `NotFound`; 500 → `Unavailable`;
  not applicable without `search` or without a split title.
- MusicBrainz against local servers: score and artist filters; first release
  with art wins; no art → `NotFound`; 503 → `Unavailable`; User-Agent sent.
- Station art: readable file → `Art`; missing → `NotFound`.
- `Station` round-trips `cover_search = false`; omitted when `true`.

TUI:

- `CoverKey::Stream` resolves through the worker (fake chain via a test
  constructor) and lands as `Embedded`.
- A `Stream` key without an ICY title is re-requested after 15 s; one with a
  title is not.
- Stage keeps the previous stream cover while the next title is `Pending`,
  and shows the logo for a new station.
- The query built for a stream carries the station's `cover_search` and the
  global switch.

## Delivery

- Version **0.11.0**.
- README: stream covers (the source order), `[covers] search`, per-station
  `cover_search`, and that ICY titles are sent to Deezer and MusicBrainz.
- Wiki: new `Architecture/Cover-Sources.md` (the chain, the trait, how to add a
  source); `Architecture/TUI.md` cover section; `Domain/Formats-and-Metadata.md`
  radio covers; roadmap note under Later radio.
