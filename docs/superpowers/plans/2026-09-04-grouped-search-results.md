# Grouped Entity Search Results Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** TUI library search returns mixed Artist / Album / Track entity hits (title-only tracks), via a new `Library::search_entities` API, while MCP/CLI flat `search` stays unchanged.

**Architecture:** Add `SearchHit`, `ArtistSummary`, and `SearchLimits` in `znicz-library`. `search_entities` runs three Unicode-folded `LIKE` queries and concatenates artists → albums → tracks. The TUI stores `Vec<SearchHit>` for `Mode::Search`, renders each kind, plays tracks on Enter, stubs artist/album Enter with a toast, and queues the selected entity with `a`.

**Tech Stack:** Rust, rusqlite, ratatui, existing `fold_text` / `*_folded` columns from #44 (0.4.7).

**Spec:** `docs/superpowers/specs/2026-09-04-grouped-search-results-design.md`

## Global Constraints

- Version **0.4.7 → 0.4.8** (compatible addition). If implementing before #50 merges, stack on `fix/44-cyrillic-search-case` (0.4.7) and note Depends on #50 in the PR; do not invent a second fold scheme.
- Keep `Library::search` / MCP `search_library` / CLI `znicz search` as flat track OR-search.
- Match rules: artists on `artist_folded` OR `album_artist_folded`; albums on `album_folded`; tracks on **`title_folded` only**.
- Return order: all artists, then albums, then tracks (no interleaved relevance).
- Enter on artist/album in search = toast stub only (no browse). `a` still queues that entity. `A` queues **track hits only**.
- Not #7 / #9; no FTS5; no live-as-you-type.
- Wiki matches code in the same change (simple English; no invented browse-from-search).

## File map

| File | Role |
| --- | --- |
| Modify `znicz-library/src/track.rs` | `ArtistSummary`, `SearchHit`, `SearchLimits` |
| Modify `znicz-library/src/lib.rs` | Re-export new types |
| Modify `znicz-library/src/store.rs` | `search_entities`, `tracks_for_artist`; unit tests |
| Modify `znicz-tui/src/library_pane.rs` | `search_hits`, `Item::Artist`, submit/selection/`listed_tracks` |
| Modify `znicz-tui/src/views/library.rs` | Render mixed hits; summary `N artists · M albums · K tracks` |
| Modify `znicz-tui/src/app.rs` | Search Enter stub toast; keep album Enter open in Albums mode |
| Modify `znicz-tui/tests/library_browse.rs` | Update search expectations for entity hits |
| Modify `wiki/Architecture/Library.md`, `wiki/Architecture/TUI.md` | Document entity search vs flat search |
| Modify `wiki/Plans/Roadmap.md` / `wiki/Home.md` / `wiki/Issues.md` | Only if they claim flat-only TUI search |
| Modify root `Cargo.toml` (+ lock if needed) | `0.4.8` |
| Keep `docs/superpowers/specs/2026-09-04-grouped-search-results-design.md` | Tracked with the change |

---

### Task 1: Library types — `ArtistSummary`, `SearchHit`, `SearchLimits`

**Files:**
- Modify: `znicz-library/src/track.rs`
- Modify: `znicz-library/src/lib.rs`
- Test: compile + unit tests in Task 2 (types alone are asserted via search tests)

**Interfaces:**
- Produces:

```rust
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ArtistSummary {
    pub name: String,
    pub track_count: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum SearchHit {
    Artist(ArtistSummary),
    Album(AlbumSummary),
    Track(Track),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SearchLimits {
    pub artists: usize,
    pub albums: usize,
    pub tracks: usize,
}

impl Default for SearchLimits {
    fn default() -> Self {
        Self {
            artists: 50,
            albums: 50,
            tracks: 200,
        }
    }
}
```

- [ ] **Step 1: Add the types and re-exports**

In `track.rs`, after `AlbumSummary`, add `ArtistSummary`, `SearchHit`, and `SearchLimits` exactly as above.

In `lib.rs`, change:

```rust
pub use track::{AlbumSummary, ArtistSummary, SearchHit, SearchLimits, Track};
```

- [ ] **Step 2: Confirm the crate builds**

Run: `cargo check -p znicz-library`
Expected: PASS

- [ ] **Step 3: Commit**

```bash
git add znicz-library/src/track.rs znicz-library/src/lib.rs
git commit -m "Add SearchHit types for entity library search."
```

---

### Task 2: `Library::search_entities` and `tracks_for_artist`

**Files:**
- Modify: `znicz-library/src/store.rs`
- Test: `znicz-library/src/store.rs` `#[cfg(test)]`

**Interfaces:**
- Consumes: `SearchLimits`, `SearchHit`, `ArtistSummary`, `AlbumSummary`, `fold_text`, `escape_like`, `COLUMNS`, `row_to_track`
- Produces:

```rust
impl Library {
    pub fn search_entities(
        &self,
        query: &str,
        limits: SearchLimits,
    ) -> Result<Vec<SearchHit>>;

    /// Tracks where `artist` or `album_artist` equals `name` (case-insensitive).
    pub fn tracks_for_artist(&self, name: &str) -> Result<Vec<Track>>;
}
```

Empty / whitespace-only query → empty `Vec` (no error).

Artist SQL (pattern = `%{fold_text(query)}%` with `ESCAPE '\\'`):

```sql
SELECT MAX(COALESCE(NULLIF(artist, ''), album_artist)) AS name,
       COUNT(*)
FROM tracks
WHERE (
        (artist_folded IS NOT NULL AND artist_folded LIKE ?1 ESCAPE '\\')
     OR (album_artist_folded IS NOT NULL AND album_artist_folded LIKE ?1 ESCAPE '\\')
      )
  AND COALESCE(NULLIF(artist, ''), album_artist) IS NOT NULL
  AND COALESCE(NULLIF(artist, ''), album_artist) <> ''
GROUP BY lower(COALESCE(NULLIF(artist, ''), album_artist))
ORDER BY name COLLATE NOCASE
LIMIT ?2
```

Prefer a stable display spelling via `MAX(...)` of the non-empty artist / album_artist string. Grouping key uses `lower(...)` so ASCII case collapses; folded `LIKE` already handles Cyrillic match. Omit empty names.

Album SQL: same aggregation as `albums()` plus `album_folded LIKE ?1 ESCAPE '\\'`, `ORDER BY album COLLATE NOCASE`, `LIMIT ?2`.

Track SQL: `title_folded LIKE ?1` only (no artist/album OR), same `ORDER BY` as `search`, `LIMIT ?2`.

Concatenate: map artists → `SearchHit::Artist`, albums → `Album`, tracks → `Track`.

`tracks_for_artist`:

```sql
SELECT {COLUMNS} FROM tracks
WHERE artist = ?1 COLLATE NOCASE
   OR album_artist = ?1 COLLATE NOCASE
ORDER BY artist, album, disc_number, track_number, title
```

- [ ] **Step 1: Write the failing tests**

Append to `store.rs` tests (reuse `upsert_track` helper pattern from existing tests):

```rust
fn seed_entity_fixture(library: &mut Library) {
    // Artist "Love" with two tracks on album "Forever"; title does not contain "Love"
    library
        .upsert_track(
            Path::new("/music/love1.flac"),
            "Alone",
            Some("Love".into()),
            Some("Forever".into()),
            None, None, None, None, None, None, None, None, None, None, Some(0),
        )
        .unwrap();
    library
        .upsert_track(
            Path::new("/music/love2.flac"),
            "Together",
            Some("Love".into()),
            Some("Forever".into()),
            None, None, None, None, None, None, None, None, None, None, Some(0),
        )
        .unwrap();
    // Album named "Love" by another artist; titles do not match "Love"
    library
        .upsert_track(
            Path::new("/music/album-love.flac"),
            "Opening",
            Some("Other".into()),
            Some("Love".into()),
            None, None, None, None, None, None, None, None, None, None, Some(0),
        )
        .unwrap();
    // Track title contains "Love"
    library
        .upsert_track(
            Path::new("/music/title-love.flac"),
            "Love Song",
            Some("Singer".into()),
            Some("Hits".into()),
            None, None, None, None, None, None, None, None, None, None, Some(0),
        )
        .unwrap();
}

#[test]
fn search_entities_artist_query_returns_artist_not_every_track() {
    let mut library = Library::open_in_memory().unwrap();
    seed_entity_fixture(&mut library);
    let hits = library
        .search_entities("Love", SearchLimits::default())
        .unwrap();

    let artists: Vec<_> = hits
        .iter()
        .filter_map(|h| match h {
            SearchHit::Artist(a) => Some(a.name.as_str()),
            _ => None,
        })
        .collect();
    assert!(artists.iter().any(|n| n.eq_ignore_ascii_case("Love")));

    let albums: Vec<_> = hits
        .iter()
        .filter_map(|h| match h {
            SearchHit::Album(a) => Some(a.album.as_str()),
            _ => None,
        })
        .collect();
    assert!(albums.iter().any(|n| n.eq_ignore_ascii_case("Love")));

    let track_titles: Vec<_> = hits
        .iter()
        .filter_map(|h| match h {
            SearchHit::Track(t) => Some(t.title.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(track_titles, vec!["Love Song"]);
    assert!(!track_titles.iter().any(|t| *t == "Alone" || *t == "Together"));
}

#[test]
fn search_entities_orders_artists_then_albums_then_tracks() {
    let mut library = Library::open_in_memory().unwrap();
    seed_entity_fixture(&mut library);
    let hits = library
        .search_entities("Love", SearchLimits::default())
        .unwrap();
    let mut seen_album = false;
    let mut seen_track = false;
    for hit in &hits {
        match hit {
            SearchHit::Artist(_) => assert!(!seen_album && !seen_track),
            SearchHit::Album(_) => {
                seen_album = true;
                assert!(!seen_track);
            }
            SearchHit::Track(_) => seen_track = true,
        }
    }
    assert!(seen_album && seen_track);
}

#[test]
fn search_entities_cyrillic_fold_matches_artist() {
    let mut library = Library::open_in_memory().unwrap();
    library
        .upsert_track(
            Path::new("/music/lyapis.flac"),
            "Ау",
            Some("Ляпис Трубецкой".into()),
            Some("Веселые Картинки".into()),
            None, None, None, None, None, None, None, None, None, None, Some(0),
        )
        .unwrap();
    let hits = library
        .search_entities("ляпис", SearchLimits::default())
        .unwrap();
    assert!(matches!(
        &hits[..],
        [SearchHit::Artist(a), ..] if a.name.contains("Ляпис")
    ));
    assert!(hits.iter().all(|h| !matches!(h, SearchHit::Track(_))));
}

#[test]
fn tracks_for_artist_matches_artist_or_album_artist() {
    let mut library = Library::open_in_memory().unwrap();
    library
        .upsert_track(
            Path::new("/music/a.flac"),
            "One",
            Some("Band".into()),
            Some("LP".into()),
            Some("Various".into()),
            None, None, None, None, None, None, None, None, None, Some(0),
        )
        .unwrap();
    library
        .upsert_track(
            Path::new("/music/b.flac"),
            "Two",
            Some("Other".into()),
            Some("Comp".into()),
            Some("Band".into()),
            None, None, None, None, None, None, None, None, None, Some(0),
        )
        .unwrap();
    let tracks = library.tracks_for_artist("band").unwrap();
    assert_eq!(tracks.len(), 2);
}

#[test]
fn empty_entity_query_returns_no_hits() {
    let library = Library::open_in_memory().unwrap();
    assert!(library
        .search_entities("  ", SearchLimits::default())
        .unwrap()
        .is_empty());
}
```

Also import `SearchHit` / `SearchLimits` in the test module (`use crate::track::{...}` or via `super::*` after re-exports in store — prefer `use crate::{SearchHit, SearchLimits};` if store tests already use `super::*` and types live in track; add `use crate::track::{ArtistSummary, SearchHit, SearchLimits};` or export through store's crate root).

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p znicz-library search_entities -- --nocapture`
Expected: compile error (`search_entities` missing) or FAIL

- [ ] **Step 3: Implement `search_entities` and `tracks_for_artist`**

Add methods on `Library` after `search` (before `all_tracks`). Use three prepares + `Vec` concat. Respect per-kind limits (`0` means skip that kind).

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p znicz-library -- --nocapture`
Expected: PASS (including existing Cyrillic flat-search tests)

- [ ] **Step 5: Commit**

```bash
git add znicz-library/src/store.rs
git commit -m "Add search_entities with artist, album, and title hits."
```

---

### Task 3: Library pane stores and selects entity hits

**Files:**
- Modify: `znicz-tui/src/library_pane.rs`
- Test: unit tests in `library_pane.rs`; update `znicz-tui/tests/library_browse.rs` search assertions

**Interfaces:**
- Consumes: `Library::search_entities`, `SearchLimits::default()`, `SearchHit`, `ArtistSummary`, `tracks_for_artist`, `browse_album`
- Produces:
  - Field `search_hits: Vec<SearchHit>`
  - `Item::Artist(&'a ArtistSummary)`
  - `submit_search` fills `search_hits`, clears or ignores `tracks` for Search mode
  - `len` / `selected` / `selected_tracks` / `listed_tracks` / pan helpers aware of Search
  - `hits()` accessor (or `search_hits()`) for the view
  - Toast message strings returned from submit still describe match counts; prefer summary like `"3 matches for \"q\""` counting total hits
  - `inject_search_hits_for_test(hits: Vec<SearchHit>)` for view/unit tests

Behaviour details:

| Method | Search mode |
| --- | --- |
| `len` | `search_hits.len()` |
| `selected` | map hit → `Item::Artist` / `Album` / `Track` |
| `selected_tracks` | Track → one; Album → `browse_album`; Artist → `tracks_for_artist` |
| `listed_tracks` | **only** `SearchHit::Track` entries (design: `A` = track hits) |
| `enter` | still only opens Album when **not** in Search (Albums mode). In Search, `enter` returns `false` so the app can toast |
| `back` | clear `search_hits`, reload albums |
| `selected_middle_len` / `longest_middle` | artist middle = name; album/track reuse helpers |

Replace `SEARCH_LIMIT` usage in `submit_search` with `SearchLimits::default()` (or keep a named const equal to defaults).

- [ ] **Step 1: Write failing pane tests**

```rust
#[test]
fn search_selected_artist_queues_artist_tracks() {
    // open_in_memory library, seed one artist with 2 tracks, submit_search that artist name,
    // select artist hit, assert selected_tracks().len() == 2
}

#[test]
fn search_listed_tracks_are_title_hits_only() {
    // seed fixture like Task 2, search "Love", assert listed_tracks titles == ["Love Song"]
}

#[test]
fn enter_does_not_open_album_from_search_hits() {
    // inject or search so cursor is on album hit in Mode::Search; assert !pane.enter()
}
```

Wire a real `Library::open_in_memory` into the pane for these tests (construct `LibraryPane` with `Some(library)` after seeding).

- [ ] **Step 2: Run to verify fail**

Run: `cargo test -p znicz-tui library_pane::tests::search_ -- --nocapture`
Expected: FAIL / compile errors on missing API

- [ ] **Step 3: Implement pane changes**

Update imports, fields, methods as above. Update `submit_search` message for 0 / 1 / n **hits**.

- [ ] **Step 4: Fix `library_browse` integration test**

`a_search_narrows_the_list_to_matches` searches `"miles"`. With entity search, Miles Davis may appear as an **artist** row and/or track title rows. Assert the screen still shows `So What` **or** the artist name and that `Mysterons` is absent. Prefer seeding/query that hits a **title** if the fixture titles allow; if `"miles"` only matches artist, assert artist row visible and track rows for that artist are **not** all listed (entity behaviour).

Read the fixture builder in `library_browse.rs` and adjust the assertion to match entity semantics without weakening the “non-matches gone” check.

- [ ] **Step 5: Pass tests and commit**

```bash
cargo test -p znicz-tui library_pane -- --nocapture
cargo test -p znicz-tui --test library_browse -- --nocapture
git add znicz-tui/src/library_pane.rs znicz-tui/tests/library_browse.rs
git commit -m "Store entity search hits in the library pane."
```

---

### Task 4: Render mixed hits and handle Enter / `a` in the app

**Files:**
- Modify: `znicz-tui/src/views/library.rs`
- Modify: `znicz-tui/src/app.rs`
- Test: unit tests in `views/library.rs`; optional keys smoke if easy

**Interfaces:**
- Consumes: `pane.search_hits()` / mode Search; `Item::Artist`
- Produces: rows for artist (name + dim track count + dim `artist` cue), album (reuse `album_row` + dim `album` cue), track (`track_row`); summary `N artists · M albums · K tracks`; `library_enter` stubs Search artist/album

Artist row sketch:

```rust
fn artist_row(artist: &ArtistSummary, strip: usize, offset: usize) -> ListItem<'static> {
    let right = format!("{} · artist", tracks_label(artist.track_count));
    // pan artist.name as middle, same pad pattern as album_row
}
```

For album rows in Search mode, append ` · album` to the right column (or a dim cue) so Artist("X") vs Album("X") stay distinct.

`library_enter`:

```rust
fn library_enter(&mut self) {
    match self.library.selected() {
        Some(Item::Album(_)) if matches!(self.library.mode(), Mode::Search(_)) => {
            self.toasts.info("album browse from search comes later");
        }
        Some(Item::Artist(_)) => {
            self.toasts.info("artist browse from search comes later");
        }
        Some(Item::Album(_)) => {
            self.library.enter();
        }
        Some(Item::Track(track)) => { /* existing play */ }
        None => {}
    }
}
```

`on_library_key` for `a` / `A` already uses `selected_tracks` / `listed_tracks` — no change once pane is correct.

Update `title_slot` for Search mode (max fixed width across hit kinds).

- [ ] **Step 1: Failing test for summary / artist cue (optional render test)**

If hard to unit-test ratatui without a full draw, cover behaviour via pane + `library_enter` unit test in `app.rs` tests module (toast text). Prefer a small `app` test that selects an injected album search hit and calls `library_enter`, then asserts a toast contains `"comes later"`.

- [ ] **Step 2: Implement view + app**

- [ ] **Step 3: Run**

Run: `cargo test -p znicz-tui -- --nocapture`
Expected: PASS

- [ ] **Step 4: Commit**

```bash
git add znicz-tui/src/views/library.rs znicz-tui/src/app.rs
git commit -m "Render entity search hits and stub artist/album Enter."
```

---

### Task 5: Wiki, version, design doc, PR

**Files:**
- Modify: `wiki/Architecture/Library.md` (Searching section)
- Modify: `wiki/Architecture/TUI.md` (Library pane paragraph)
- Modify: root `Cargo.toml` version `0.4.7` → `0.4.8`
- Add: `docs/superpowers/specs/2026-09-04-grouped-search-results-design.md` (if not tracked)
- Add: this plan under `docs/superpowers/plans/`
- Touch Roadmap/Home/Issues only if they still say TUI search is flat tracks for any field

**Wiki copy (Library.md):** Explain two APIs: flat `search` (MCP/CLI) vs `search_entities` (TUI): artists / albums / title-only tracks; Unicode fold unchanged; Enter browse from search not yet.

**Wiki copy (TUI.md):** Search results are entity rows (artists, albums, tracks). Enter plays a track; Enter on artist/album shows a short status toast for now. `a` queues the entity; `A` queues title-matched tracks only.

- [ ] **Step 1: Edit wiki + bump version**

- [ ] **Step 2: Full verification**

Run:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Expected: all PASS

- [ ] **Step 3: Commit**

```bash
git add wiki Cargo.toml Cargo.lock docs/superpowers/
git commit -m "Document entity search and bump to 0.4.8."
```

- [ ] **Step 4: Push and open PR**

```bash
git push -u origin HEAD
gh pr create --base main --title "…" --body "…"
```

PR body must link the design + plan, note Depends on #50 if that PR is still open, list test plan.

---

## Spec coverage checklist

| Spec requirement | Task |
| --- | --- |
| Mixed Artist / Album / Track hits | 2, 3, 4 |
| Title-only track matches | 2 |
| Artists → Albums → Tracks order | 2, 4 |
| `search_entities` + keep flat `search` | 2 |
| Enter track plays; artist/album stub | 4 |
| `a` queues entity; `A` track hits only | 3, 4 |
| No browse / not #7/#9 | 4 + wiki |
| Unicode fold | 2 (depends on #44 columns) |
| Wiki sync + version bump | 5 |

## Placeholder / consistency review

- Types named `SearchHit`, `ArtistSummary`, `SearchLimits` throughout.
- No TBD steps; SQL and test bodies are concrete.
- TUI defaults match `SearchLimits::default()` (50 / 50 / 200).
