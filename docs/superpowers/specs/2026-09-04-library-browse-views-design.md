# Library browse views: three-column + expandable tree

**Date:** 2026-09-04  
**Status:** Approved and implemented (0.5.0)  
**Issues:** [#7](https://github.com/eugene-chekan/znicz/issues/7), [#9](https://github.com/eugene-chekan/znicz/issues/9)  
**Related:** Entity search (#51 / [grouped-search design](2026-09-04-grouped-search-results-design.md)) — Enter on artist/album lands in browse  
**Crates (expected):** `znicz-tui` primarily; `znicz-library` for artist listing / albums-by-artist queries if missing  
**Version:** **0.5.0** (product-phase browse layouts)

## Decisions (locked)

| # | Topic | Decision |
| --- | --- | --- |
| 1 | Approach | **C** — one shared artist → album → track model; two layouts (columns when usable, tree as the alternate preferred layout) |
| 2 | Default home | **Artist-first** browse; classic album-list-as-home is not the default. Home root preference is **configurable in settings** (see Config) |
| 3 | Tree / column roots | **Artists**, plus a synthetic **`Various Artists`** root for compilation albums (rule below) |
| 4 | Column focus keys | **`Tab`** / **`Shift-Tab`**. Global **`h` / `l` stay seek** — do not reuse them for column movement |
| 5 | Narrow terminal | **Single-column paging** through the same artist → album → track model (v1 choice; see note) |
| 6 | Ship shape | **Both layouts in one PR** (shared model + columns + tree + search landing + narrow paging) |
| 7 | View selection | **Config + width** — config chooses the preferred layout (`columns` or `tree`); width gates whether three-column is usable. When preferred is `columns` but the library region is too narrow, fall back to single-column paging |

Also locked from earlier design direction:

- Search **Enter** on artist/album **leaves Search** and focuses that entity in browse (no stub toast).
- **Queue pane unchanged** (still overlays on the right; columns/tree shrink under it as today’s library strip does).
- **Do not steal seek keys.**
- Tree **session-remembers expand state** (#9): which artist/album nodes were open until the TUI exits (not persisted to disk in v1).

## Problem

Today the library is a **one-column drill-down**:

| Mode | What you see |
| --- | --- |
| `Albums` | Album list (default) |
| `Album(name)` | That album’s tracks (replaces the list) |
| `Search(query)` | Mixed entity hits (artists → albums → title tracks) |
| `AllTracks` | Flat tracks when nothing has album tags |

Enter opens an album; Esc replaces that view with the previous list. You never see albums and their tracks on screen at once, and there is no artist-first browser.

Two parked ideas address that differently:

- **#7 Three-column** — desktop-style `artist | album | tracks` side by side; queue drawer still overlays on the right; narrow terminals need a fallback.
- **#9 Expandable tree** — roots expand in place with indented children; session remembers which nodes were open. Explicitly **not** a filesystem tree.

Entity search (#51) already surfaces artist and album **rows**, but Enter only toasts “browse later”. This browse surface is where those stubs land.

## What exists today (grounding)

| Surface | Today |
| --- | --- |
| **TUI** | One list region; modes above; `/` entity search; Enter on album (non-search) → tracks; Enter on search artist/album → stub |
| **Library API** | `albums()`, `browse_album`, `search`, `search_entities`; artist queue uses `tracks_for_artist` — **no** `artists()` / `albums_by_artist` browse API yet |
| **Track fields** | `artist`, `album`, `album_artist` (`Option<String>` each); album summaries today use `MAX(COALESCE(album_artist, artist))` |
| **Keys** | Global `h`/`l` are seek |
| **Mouse** | Click selects a row in the focused list; wheel steps that list; queue toggle on library right-border column |
| **Roadmap** | Both #7 and #9 under “Later TUI (parked)” |

## Goal

One browse hierarchy (artist → album → track) with two presentations, artist-first by default, search Enter wired in, seek keys untouched, queue behaviour unchanged.

## Non-goals (v1)

- Filesystem / folder tree (#9 out of scope)
- Changing the playback engine or queue drawer behaviour
- Persisting tree expand state across process restarts
- FTS5 / MusicBrainz
- Replacing MCP/CLI flat `search`
- In-tree search filtering (#9 “later”)
- Drag-to-reorder (#36)
- Keeping classic **Albums-first** as the default home (it may remain only as untagged `AllTracks` / escape hatch, not the normal entry)
- Inferring compilations from fuzzy synonyms beyond the rule below (e.g. `"VA"`, `"V/A"` are **not** treated as tagged Various Artists in v1)

## Approach (chosen): C — one hierarchy, two layouts

Shared **browse model**: selected artist, selected album, track list, focus depth, and (for tree) an expand-set. Render as:

- **Preferred `columns` + wide enough:** three columns (#7)
- **Preferred `tree`:** expandable tree (#9) at any width
- **Preferred `columns` + too narrow:** single-column paging of the same three levels

Same queries, same search landing (`focus artist X` / `focus album Y`), different paint + focus movement.

Approaches A (three independent modes beside classic) and B (columns only, tree later) were considered and **rejected**.

## Design

### Shared browse state

Illustrative fields (names not normative):

- `artists: Vec<ArtistSummary>` — includes synthetic **Various Artists** when any compilation album exists
- `albums_for_artist: Vec<AlbumSummary>`
- `tracks_for_album: Vec<Track>`
- Focus: which column / which tree depth has the cursor
- Tree-only: `expanded: HashSet<NodeId>` for the **session** (cleared on quit; not written to config)

Untagged libraries still use **`AllTracks`** (nothing to group by album). That path stays outside the artist hierarchy.

### Library queries to add

Likely in `znicz-library` (exact SQL at plan time):

- List distinct browse artists (plus synthetic Various Artists when needed)
- List albums for a browse artist (including the compilation bucket)
- Reuse `browse_album` for tracks

No schema change unless a real query needs it (#7).

### Various Artists / compilations (concrete rule)

Grounded in existing `Track` fields `album_artist` and `artist` only.

An album (grouped by `album` COLLATE NOCASE, non-empty) is a **compilation** and is listed **only** under the synthetic artist root **`Various Artists`** when **either**:

1. **Tagged VA:** At least one track of that album has `album_artist` equal to `"Various Artists"` (**COLLATE NOCASE**). No other spellings in v1.
2. **Untagged multi-artist album:** Every track of that album has empty / `NULL` `album_artist`, **and** there are **two or more** distinct non-empty `artist` values on that album (**COLLATE NOCASE**).

Otherwise the album is attributed to a single browse artist:

- If tracks agree on a non-empty `album_artist` (COLLATE NOCASE), use that name.
- Else if they agree on a non-empty `artist`, use that name.
- Else fall back to today’s summary style `COALESCE(album_artist, artist)` for display; do not invent a second synthetic bucket in v1.

Consequences:

- A properly tagged compilation (`album_artist = "Various Artists"`, per-track `artist`s) sits under **Various Artists**, not under each track artist.
- A multi-artist album with **no** `album_artist` also sits under **Various Artists**.
- A multi-artist album with a real `album_artist` (band, “Original Soundtrack”, label, etc.) sits under that `album_artist`, even when track `artist`s differ.
- The **Various Artists** root appears in the artist list only when ≥1 compilation album exists; sort it with other names (COLLATE NOCASE), not forced to the top.
- Track rows still show each track’s own `artist` tag in the tracks column / tree leaves.

### Three-column layout (#7)

```
┌ Artists ───┬ Albums ────────┬ Tracks ────────────────────┐
│ Miles Davis│ Kind of Blue   │ 1 So What              9:22│
│ John Coltr.│ Bitches Brew   │ 2 Freddie Freeloader   9:46│
│ Various Art│ …              │ …                          │
└────────────┴────────────────┴────────────────────────────┘
```

- Queue drawer still overlays on the right; columns shrink under it.
- Focus moves with **`Tab`** / **`Shift-Tab`** only among the three columns when that layout is active. **`h` / `l` remain seek.**
- Enter on a track plays (today). Enter on artist/album in columns: move focus right / refresh the child list (do not replace the whole pane).
- `a` / `A` keep “selection / everything listed” semantics adapted to the focused column.
- Width gate: if the library region is below the three-column minimum (exact columns threshold at plan/implementation time), do **not** draw three columns; use the narrow fallback.

### Expandable tree (#9)

- **Roots: artists** (same list as the left column), including **Various Artists** when applicable.
- Expand path: artist → albums → tracks (indented children).
- Expand/collapse: Enter and/or Space/`o` on a parent (exact binding set in the plan; document in `keys.rs` + README).
- **Session** remembers which nodes were open; quitting clears expand state.
- Search stays a flat entity list; not an in-tree filter in v1.

### Narrow fallback: single-column paging

**Preferred v1 behaviour** when three-column is not usable (and preferred layout is `columns`):

Show **one** of the three levels at a time in the library list:

1. Artists → Enter → albums for that artist  
2. Albums → Enter → tracks for that album  
3. Esc steps back one level  

Same model and selection state as columns; only one list is painted. This is **not** “return to today’s Albums-first classic” — paging is still **artist-first**.

**Note:** Exact UX details (whether the header shows the parent name, whether selecting an artist auto-loads albums without Enter, etc.) can be refined in the implementation plan. Treat single-column paging as the locked narrow strategy unless something in implementation blocks it; if blocked, document the blocker and revisit rather than silently switching to classic album drill-down.

When preferred layout is **`tree`**, use the tree at narrow widths (no forced paging).

### Config

Extend TUI settings (exact TOML keys at plan time; illustrative):

| Setting | Role | Default |
| --- | --- | --- |
| Preferred library layout | `columns` or `tree` | `columns` |
| Default browse home | Artist-first browse (this feature) vs any retained escape hatch | **Artist-first** |

Config chooses preference; **width only gates columns**. There is no “automatic layout with no preference” mode in v1. A cycle key is **not** required for v1 (config is enough); add one later only if wanted.

### Search Enter (#51 follow-up)

| Hit | Behaviour |
| --- | --- |
| Track | Play (unchanged) |
| Album | Leave Search; open browse focused on that album (columns: select its browse artist + album, focus tracks; tree: expand path, cursor on album or first track; paging: land on that album’s track list) |
| Artist | Leave Search; open browse focused on that artist (columns: select artist, focus albums; tree: expand artist; paging: land on that artist’s album list) |

Esc from browse returns toward the previous top-level artist list (or prior stack in paging), **not** back into the old search query unless a later change adds that.

### Keys and mouse

- Document new bindings in `keys.rs` + README; wiki points at those (no stale duplicate key tables).
- Mouse: click selects the row **in the column / tree / list under the pointer**; wheel scrolls the focused list (per-column hit rects as needed).
- Do not invent drag-to-reorder here.

### Testing / wiki

- Render fixtures: three-col wide, narrow paging, tree expand/collapse, Various Artists bucket.
- Key tests: Tab column focus, seek `h`/`l` unchanged, expand/collapse session state, search Enter landing.
- Same change: `wiki/Architecture/TUI.md`, `wiki/Issues.md` / Roadmap when issues unpark or close, version bump.

## Sequencing

1. **User approval** of this locked design (this doc).
2. **Implementation plan** (writing-plans) — one PR covering library browse queries, shared state, columns, tree, narrow paging, search Enter, config, tests, wiki.
3. No separate “columns-only then tree” PR under this design.

## Spec self-review

- Open questions from the draft are **resolved** in **Decisions**; no lingering TBD on approach, home, roots, keys, narrow fallback, ship shape, or view toggle.
- Various Artists has one concrete detection rule tied to `album_artist` / `artist`.
- #7 and #9 stay two layouts on one model; ship together.
- Scope is TUI browse + needed library queries; not engine/MCP flat search.
- Narrow paging is preferred v1 with a light uncertainty note, not left as an open question.
