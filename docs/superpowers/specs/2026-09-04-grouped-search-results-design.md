# Grouped search: entity hits (artists, albums, tracks)

**Date:** 2026-09-04
**Status:** Approved
**Depends on:** Unicode-folded search (#44 / 0.4.7)
**Crates (v1):** `znicz-library` (new query API), `znicz-tui` (Mode::Search presentation + selection)
**Version:** bump `z` when implemented (compatible addition)

## Problem

Library search today returns a **flat list of tracks** whenever the query matches
title, artist, album, or album artist (`Library::search`). Searching an artist
name therefore lists every track by that artist. Searching an album name lists
every track on that album.

That is useful for “find something to play”, but it is a poor discovery surface:
the interesting answer is often the **artist** or **album** itself, not dozens of
track rows.

## Goal (v1)

Search results are a **mixed list of entity hits**:

| Match | Result row |
| --- | --- |
| Query matches a **track title** | That track, once |
| Query matches an **album name** | One **album** row (not every track on it) |
| Query matches an **artist name** | One **artist** row (not every track by them) |

Display these three kinds correctly in the TUI. Navigation from artist/album
rows into a library browse view is **explicitly later** (stub Enter in v1).

This supersedes the earlier A/B/C brainstorm options (section headers over track
hits; separate buckets that still expand tracks; drill-down hierarchy).

## Non-goals (v1)

- **No drill-down navigation** from artist/album search rows into browse (deferred;
  rows exist so later work can wire them).
- **Not** [#7](https://github.com/eugene-chekan/znicz/issues/7) three-column
  library or [#9](https://github.com/eugene-chekan/znicz/issues/9) expandable
  tree — those stay parked.
- **No** change to MCP `search_library` or CLI `znicz search` shape in v1 (they
  stay flat track lists). Agents still need paths to play; entity hits without
  browse are less useful there until navigation exists.
- **No** FTS5 / relevance ranking; keep Unicode-folded `LIKE` (#44).
- **No** live-as-you-type search; keep submit-on-Enter prompt behaviour.

## Current behaviour (grounding)

| Surface | Today |
| --- | --- |
| **Library API** | `Library::search(query, limit) -> Vec<Track>` — `OR` over `title_folded`, `artist_folded`, `album_folded`, `album_artist_folded`; order artist / album / disc / track / title |
| **TUI** | `/` → prompt → `Mode::Search(query)` stores `tracks: Vec<Track>`; same track-row render as album tracks; Enter plays a track; `a` / `A` queue selection / all listed |
| **MCP** | `search_library` → `{ query, count, tracks: [...] }` |
| **CLI** | `znicz search` prints title + artist/album + path |

Album browse already has entity rows (`AlbumSummary` + Enter → tracks). Search
does not reuse that model.

## Approaches considered

### A — TUI-only regroup of existing `search` tracks

Group or dedupe the flat track list in the UI.

**Reject:** Cannot produce a true artist/album entity hit without either
expanding every matched track (what we are removing) or inventing entities from
track fields while still having pulled every track into memory. Wrong API shape.

### B — Three separate library queries, TUI merges

`search_artists`, `search_albums`, `search_tracks_by_title`; TUI concatenates.

**Works**, but callers invent ordering and limits three times. Fine later if MCP
wants one kind only; heavier for the common “one search box” case.

### C — One typed search API returning entity hits (recommended)

`Library::search_entities(query, limits) -> Vec<SearchHit>` (or a small struct
with three vecs that the TUI flattens). Matching rules are explicit per kind.
Flat `Library::search` remains for MCP/CLI.

**Choose C:** one place for match semantics; TUI is the only v1 consumer; flat
search stays stable for agents.

## Surfaces in scope

**v1: TUI only** for presentation, plus the shared **library** query it needs.

| Surface | v1 |
| --- | --- |
| `znicz-library` | Add entity search; keep `search` unchanged |
| `znicz-tui` | Render and select entity hits in `Mode::Search` |
| MCP / CLI | **Leave flat** (`search` / `search_library`) |

Revisit MCP/CLI when artist/album browse from search ships, or if agents need
distinct entity discovery sooner.

## Data model / query changes

### New types (in `znicz-library`)

```rust
pub enum SearchHit {
    Artist(ArtistSummary),
    Album(AlbumSummary),
    Track(Track),
}

pub struct ArtistSummary {
    pub name: String,          // display spelling
    pub track_count: u32,      // optional but cheap; helps the row
}
```

`AlbumSummary` already exists. Reuse it for album hits so browse and search share
one album row shape.

### Match rules (per kind)

All matching uses the same **Unicode fold** as #44 (`fold_text` / `*_folded`
columns, `LIKE` with escaped `%` / `_`).

1. **Artists** — distinct non-empty names where `artist_folded` **or**
   `album_artist_folded` matches the pattern. One row per distinct name
   (case-insensitive group, same spirit as `albums()`’s `GROUP BY album COLLATE NOCASE`).
   Prefer a stable display spelling (e.g. `MAX(name)` / first non-empty).
2. **Albums** — distinct non-empty albums where `album_folded` matches. Reuse
   the same aggregation fields as `albums()` where practical (artist, year,
   track count, total duration).
3. **Tracks** — rows where **`title_folded` matches only**. Do **not** include a
   track merely because its artist or album matched (that is what entity rows
   are for).

A query may contribute hits of more than one kind. Example: query `Love` can
yield artist “Love”, album “Love”, and tracks titled “Love …” independently.

### API shape

Prefer one call:

```rust
pub struct SearchLimits {
    pub artists: usize,
    pub albums: usize,
    pub tracks: usize,
}

impl Default for SearchLimits {
    fn default() -> Self {
        Self { artists: 50, albums: 50, tracks: 200 }
    }
}

impl Library {
    pub fn search_entities(
        &self,
        query: &str,
        limits: SearchLimits,
    ) -> Result<Vec<SearchHit>>;
}
```

Return order: **all artist hits, then album hits, then track hits**, each kind
internally sorted (artists by name; albums by album name; tracks by the existing
artist / album / disc / track / title order). The TUI can render that order
directly.

Keep `Library::search` as today’s flat track OR-search for MCP/CLI.

### Empty query

Unchanged: TUI cancels empty submit; library API may treat empty as no hits.

## TUI presentation

### Storage

`Mode::Search(String)` stays. Replace the search result store from `tracks:
Vec<Track>` alone with `hits: Vec<SearchHit>` (or keep `tracks` only for
`Mode::Album` / `AllTracks` and a dedicated `search_hits` field for Search).

Extend `Item` (or a search-specific selection enum) so the cursor can sit on
artist, album, or track.

### List layout

One scrollable list (not three panes). **Section order, not interleaved
relevance** (no rank score yet):

1. Artists  
2. Albums  
3. Tracks  

Every result row is selectable (no non-selectable section headers — those would
fight cursor / mouse hit math and contradict “entity hits, not sectioned track
lists”).

- **Artist** rows: name + dim track count (and a dim `artist` cue if needed).
- **Album** rows: reuse existing album-row chrome (name / artist, track count ·
  duration), with a dim `album` cue if the list would otherwise be ambiguous.
- **Track** rows: keep today’s title — artist / number / duration look.

Header title stays `Library / search "…"`. Summary text:
`N artists · M albums · K tracks`.

### Selection behaviour (v1)

| Row | Enter | `a` | `A` |
| --- | --- | --- | --- |
| **Track** | Play that track (today) | Queue that track | Queue **all track hits** in the result list (not every track under matched artists/albums) |
| **Album** | Toast / status: browse later (no navigation) | Queue all tracks of that album (`browse_album`) | Same as track row’s `A` rule |
| **Artist** | Toast / status: browse later | Queue all tracks attributed to that artist (see below) | Same as track row’s `A` rule |

**Artist queue (`a`):** tracks where `artist` or `album_artist` equals the hit
name (case-insensitive), capped sensibly if needed. This matches “add this
entity” without implementing browse.

Esc from search still returns to the album list (`back` / `reload_albums`).

Mouse: row select only, same as other lists; no new hit-map behaviour.

## Edge cases

| Case | Behaviour |
| --- | --- |
| Same string is both artist and album | **Two rows** — `Artist("X")` and `Album("X")` |
| Empty / missing artist or album tags | Omit empty names from artist/album hits; tracks still appear on title match (title already falls back at scan time) |
| `album_artist` differs from `artist` | Both feed **artist** distinct names; album aggregation keeps today’s `COALESCE(album_artist, artist)` style where reused |
| Unicode / Cyrillic case | Same fold as #44; no ASCII-only `LIKE` |
| Query matches title and that track’s artist | Both the **track** hit and the **artist** hit may appear |
| Many hits | Per-kind limits (`SearchLimits`); TUI defaults above; do not return unbounded groups |
| `%` / `_` in query | Escape for `LIKE`, same as today |
| No matches | Notice / toast: nothing matched (today’s empty search UX) |

## Testing (when implementing)

- Library unit tests: artist-only query returns artist hit and **zero** track hits
  unless a title also matches; album-only likewise; title match returns tracks;
  Cyrillic fold still works on entity queries; duplicate artist/album string →
  two kinds.
- TUI: search fixture shows mixed kinds; Enter on track plays; Enter on
  artist/album does not change mode; `a` on album queues album tracks.

## Wiki (same change as implementation)

Update [Architecture/Library.md](../../../wiki/Architecture/Library.md) search
section and [Architecture/TUI.md](../../../wiki/Architecture/TUI.md) library
search behaviour so they describe entity hits, not “flat tracks for any field
match”. Do not pretend artist/album Enter already browses.

## Open for approval

1. TUI + `search_entities` in v1; MCP/CLI stay flat — OK?
2. Section order Artists → Albums → Tracks — OK?
3. Enter on artist/album = stub message; `a` still queues that entity — OK?
