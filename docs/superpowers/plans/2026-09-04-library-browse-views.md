# Library Browse Views Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship artist-first library browse with a shared artist → album → track model, rendered as three columns (wide), expandable tree (preferred alternate), or single-column paging (narrow columns fallback), with search Enter landing and session tree expand — closing #7 and #9.

**Architecture:** Add `browse_artists` / `albums_for_browse_artist` in `znicz-library` (compilation rule → synthetic **Various Artists**). Rebuild `LibraryPane` around browse state + layout enum; render three columns or tree or paging in `views/library.rs`. Config prefers `columns` | `tree`; width gates three-column. Tab/Shift-Tab move column focus when columns are active (queue still via `]` / Tab when not in columns). Seek `h`/`l` untouched.

**Tech Stack:** Rust, rusqlite, ratatui, existing `Library` / `LibraryPane` / `App` / `TuiConfig`.

**Spec:** `docs/superpowers/specs/2026-09-04-library-browse-views-design.md`

## Global Constraints

- Version **0.4.8 → 0.5.0** (product-phase TUI browse layouts). Bump root `Cargo.toml` `[workspace.package] version` in the same change.
- Approach **C**: one shared model; **both** columns and tree in **one PR** (no columns-only merge).
- Default home: **artist-first browse**. Untagged libraries still use `AllTracks`.
- Column focus: **Tab / Shift-Tab** among the three columns when three-column layout is painted. Global **h / l stay seek**.
- When three-column is **not** painted: Tab / Shift-Tab keep today’s library ↔ queue behaviour (`]` still toggles the drawer).
- Narrow + preferred `columns`: **single-column paging** (artists → albums → tracks), not classic albums-first home.
- Preferred `tree`: tree at any width (no forced paging).
- Search Enter on artist/album: leave Search, focus that entity in browse (remove stub toasts).
- Queue drawer behaviour unchanged (overlay/sheet math in `layout.rs`).
- Tree expand state: **session only** (`HashSet`), cleared on quit; not written to config.
- Various Artists rule: exact design wording (tagged VA COLLATE NOCASE, or untagged multi-artist album). No `"VA"` / `"V/A"` synonyms.
- Wiki sync in the same change. Simple English. No invented features.
- One coherent branch/PR; link design + plan; `Closes #7` and `Closes #9`.

## File map

| File | Role |
| --- | --- |
| Modify `znicz-library/src/store.rs` | `browse_artists`, `albums_for_browse_artist`, VA classification helpers + unit tests |
| Modify `znicz-library/src/lib.rs` | Re-export `VARIOUS_ARTISTS_NAME` if public |
| Modify `znicz-library/src/track.rs` | Optional: doc on browse artist vs tags (only if needed) |
| Modify `znicz-tui/src/library_pane.rs` | Browse state, layouts, paging, tree expand, search landing |
| Modify `znicz-tui/src/views/library.rs` | Three-column, tree, paging, search render; per-column hit rects |
| Modify `znicz-tui/src/layout.rs` | `MIN_COLUMNS_STRIP` (or similar) width gate helper |
| Modify `znicz-tui/src/app.rs` | Tab column focus when columns active; search Enter landing; Space/`o` expand |
| Modify `znicz-tui/src/keys.rs` + `README.md` | Document Tab columns, Enter/Space/`o` expand, Esc paging |
| Modify `znicz-tui/src/tui_config.rs` + `znicz/src/main.rs` | `library_layout` (`columns` \| `tree`), default `columns` |
| Modify `znicz-tui/tests/*` | keys, render, library_browse, mouse as needed |
| Modify wiki: `Architecture/TUI.md`, `Architecture/Library.md`, `Plans/Roadmap.md`, `Issues.md`, `Home.md` if needed, `Rust/Cargo-Workspace.md` version |
| Modify root `Cargo.toml` | `0.5.0` |
| Keep design + this plan under `docs/superpowers/` | Tracked with the PR |

---

### Task 1: Library browse queries + Various Artists

**Files:**
- Modify: `znicz-library/src/store.rs`
- Modify: `znicz-library/src/lib.rs` (re-export `VARIOUS_ARTISTS_NAME`)
- Test: unit tests in `store.rs` `#[cfg(test)]`

**Interfaces:**
- Produces:

```rust
pub const VARIOUS_ARTISTS_NAME: &str = "Various Artists";

impl Library {
    /// Distinct browse artists (artist-first roots), including synthetic
    /// Various Artists when ≥1 compilation album exists. Sorted COLLATE NOCASE.
    pub fn browse_artists(&self) -> Result<Vec<ArtistSummary>>;

    /// Albums attributed to a browse artist (or compilations for Various Artists).
    pub fn albums_for_browse_artist(&self, artist: &str) -> Result<Vec<AlbumSummary>>;
}
```

- Classification (private helpers; Rust over SQL for the rule):
  1. Load each non-empty album (group `album` COLLATE NOCASE) with its tracks’ `artist` / `album_artist`.
  2. Album is **compilation** iff tagged VA on any track **or** all `album_artist` empty and ≥2 distinct non-empty `artist`s (NOCASE).
  3. Else browse artist = agreed non-empty `album_artist`, else agreed non-empty `artist`, else `COALESCE`-style display name from existing summary fields.
  4. Compilations appear **only** under Various Artists, not under track artists.

- [ ] **Step 1: Write failing tests** in `store.rs` tests:

```rust
#[test]
fn browse_artists_includes_various_artists_for_tagged_compilation() { /* ... */ }

#[test]
fn browse_artists_includes_various_artists_for_untagged_multi_artist_album() { /* ... */ }

#[test]
fn multi_artist_album_with_real_album_artist_stays_under_that_name() { /* ... */ }

#[test]
fn albums_for_browse_artist_returns_only_that_artists_albums() { /* ... */ }

#[test]
fn various_artists_root_absent_when_no_compilations() { /* ... */ }
```

Seed with `upsert_track`. Assert names and album membership.

- [ ] **Step 2: Run tests — expect FAIL** (methods missing)

Run: `cargo test -p znicz-library browse_artists -- --nocapture`

- [ ] **Step 3: Implement** `browse_artists` / `albums_for_browse_artist` + helpers. Reuse `AlbumSummary` / `ArtistSummary`. `track_count` on browse artists = sum of tracks under attributed albums.

- [ ] **Step 4: Run tests — expect PASS**

- [ ] **Step 5: Commit**

```bash
git add znicz-library/src/store.rs znicz-library/src/lib.rs
git commit -m "$(cat <<'EOF'
feat(library): add browse_artists and albums_for_browse_artist

Attribute albums with the locked Various Artists compilation rule for
artist-first library browse (#7 / #9).
EOF
)"
```

---

### Task 2: Config — preferred library layout

**Files:**
- Modify: `znicz-tui/src/tui_config.rs`
- Modify: `znicz/src/main.rs` (`TuiSection`, `tui_config`)
- Test: `tui_config.rs` unit tests

**Interfaces:**
- Produces:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LibraryLayout {
    Columns,
    Tree,
}

impl LibraryLayout {
    pub fn parse(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "tree" => Self::Tree,
            _ => Self::Columns, // default + unknown
        }
    }
}

pub struct TuiConfig {
    pub show_cover: bool,
    pub cover_protocol: CoverProtocol,
    pub library_layout: LibraryLayout,
}
```

TOML: `[tui] library_layout = "columns"` | `"tree"` (default columns).

- [ ] **Step 1: Failing parse tests** for `LibraryLayout::parse`
- [ ] **Step 2: Implement enum + wire TOML**
- [ ] **Step 3: Commit**

```bash
git commit -m "$(cat <<'EOF'
feat(tui): add library_layout config (columns|tree)

EOF
)"
```

---

### Task 3: Shared browse state in `LibraryPane`

**Files:**
- Modify: `znicz-tui/src/library_pane.rs`
- Modify: `znicz-tui/src/layout.rs` (width helper)
- Test: unit tests in `library_pane.rs`

**Interfaces:**
- Replace album-first default with browse home:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColumnFocus {
    Artists,
    Albums,
    Tracks,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mode {
    /// Artist → album → track browse (columns, tree, or paging).
    Browse,
    Search(String),
    AllTracks,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum TreeNodeId {
    Artist(String),
    Album { artist: String, album: String },
}

pub struct LibraryPane {
    // existing library, search_hits, input, notice, h_offset ...
    mode: Mode,
    artists: Vec<ArtistSummary>,
    albums: Vec<AlbumSummary>,   // for selected browse artist
    tracks: Vec<Track>,          // for selected album / AllTracks
    artist_cursor: Cursor,
    album_cursor: Cursor,
    track_cursor: Cursor,
    column_focus: ColumnFocus,
    /// Which level is shown in single-column paging.
    paging_level: ColumnFocus,
    expanded: HashSet<TreeNodeId>,
    tree_cursor: Cursor, // index into flattened visible tree rows
    preferred_layout: LibraryLayout,
}
```

- `effective_layout(strip_width) -> Columns | Tree | Paging`
  - preferred Tree → always Tree
  - preferred Columns + strip ≥ `MIN_COLUMNS_STRIP` (define **60** chars of library strip inner width) → Columns
  - else → Paging
- `reload_browse()` loads `browse_artists`; selects first artist; loads albums; selects first album; loads tracks via `browse_album`.
- `enter` / `back` / `selected` / `selected_tracks` / `listed_tracks` / `len` / `step` adapted per layout.
- `focus_artist(name)` / `focus_album(album, optional artist hint)` for search landing.
- Tree: `toggle_expand` on artist/album nodes; session `expanded`; flatten visible rows for step/select.
- Keep inject_* test helpers updated.

- [ ] **Step 1: Write failing unit tests** for reload browse home, paging enter/back, tree expand session, focus_artist/focus_album
- [ ] **Step 2: Implement state machine** (render can still be stubbed/single-list temporarily if needed, but prefer Task 4 next)
- [ ] **Step 3: Commit**

```bash
git commit -m "$(cat <<'EOF'
feat(tui): artist-first browse state for columns, tree, and paging

EOF
)"
```

---

### Task 4: Render — three columns, tree, paging

**Files:**
- Modify: `znicz-tui/src/views/library.rs`
- Modify: `znicz-tui/src/hit.rs` if hit maps need multiple rects
- Modify: mouse handling in `app.rs` for per-column clicks
- Test: `znicz-tui/tests/render.rs`, `library_browse.rs`, `mouse.rs`

**Behaviour:**
- **Columns:** three side-by-side lists (Artists | Albums | Tracks); highlight active column; queue overlay still on top via existing drawer.
- **Tree:** indented roots/children; expanded artists show albums; expanded albums show tracks.
- **Paging / Search / AllTracks:** one list (today’s style) with titles reflecting level (`Library / Artists`, `Library / Artist / Albums`, etc.).
- Mouse: click selects row under pointer in that column/list; wheel steps focused list.

- [ ] **Step 1: Failing render fixtures** (wide three-col headers; narrow paging title; tree indent)
- [ ] **Step 2: Implement render + hit rects**
- [ ] **Step 3: Commit**

---

### Task 5: Keys — Tab columns, expand, search Enter, seek unchanged

**Files:**
- Modify: `znicz-tui/src/app.rs`
- Modify: `znicz-tui/src/keys.rs`
- Modify: `README.md` key table
- Test: `znicz-tui/tests/keys.rs`, `library_browse.rs`

**Behaviour:**
- When `effective_layout == Columns` and `Focus::Library` and not typing/modal: **Tab** → next column, **Shift-Tab** → previous; do **not** open/focus queue.
- Otherwise: keep existing Tab / BackTab library ↔ queue.
- Enter: track → play; artist/album in columns → focus child column + refresh; paging → drill; tree parent → toggle expand (and/or move into children); search artist/album → `focus_*` then leave Search.
- Space and `o` on tree parent: toggle expand (document both).
- Esc: paging back one level; tree does not need Esc to collapse (optional: Esc collapses focused node only if easy — **not required**; Esc from Search clears search and returns to browse home).
- `a` / `A`: selection / listed adapted to focused column / visible tree scope / paging level.
- Assert `h`/`l` still seek in keys tests.

- [ ] **Step 1: Failing key tests**
- [ ] **Step 2: Wire `app.rs` + keys tables**
- [ ] **Step 3: Commit**

---

### Task 6: Wiki, version, design status

**Files:**
- `Cargo.toml` → `0.5.0`
- `wiki/Architecture/TUI.md`, `Library.md`, `Plans/Roadmap.md`, `Issues.md`, `Rust/Cargo-Workspace.md`, `Home.md` if it claims albums-first
- Design doc status → approved / implemented
- README `[tui]` sample for `library_layout`

Wiki content (simple English):
- Library home is artist-first browse; layouts columns / tree / narrow paging.
- Tab cycles columns when three-column is shown; `]` queue; h/l seek.
- Search Enter lands in browse.
- Move #7 / #9 from Open parked to Closed (0.5.0); remove from Roadmap “Later TUI (parked)”.

- [ ] **Step 1: Update wiki + version + README**
- [ ] **Step 2: Commit**

```bash
git commit -m "$(cat <<'EOF'
docs: library browse views wiki sync and 0.5.0

EOF
)"
```

---

### Task 7: Verification + PR

- [ ] Run: `cargo test --workspace` (fix expected)
- [ ] Run focused: `cargo test -p znicz-library browse_`; `cargo test -p znicz-tui`
- [ ] Push branch `feat/library-browse-views`, `gh pr create` with design + plan links, `Closes #7`, `Closes #9`

---

## Spec coverage checklist

| Spec item | Task |
| --- | --- |
| Shared artist→album→track model | 3 |
| `browse_artists` / albums-by-artist + VA rule | 1 |
| Three-column layout | 4–5 |
| Expandable tree + session expand | 3–5 |
| Narrow single-column paging | 3–4 |
| Config preferred layout | 2 |
| Width gates columns | 3–4 |
| Search Enter landing | 5 |
| Tab/Shift-Tab columns; h/l seek | 5 |
| Queue unchanged | 4–5 |
| Both layouts one PR | all |
| Wiki + version | 6 |

## Placeholder scan

No TBD steps. Width gate fixed at **60** strip-inner characters (adjust only if render tests prove unusable; document in commit if changed).

## Type consistency

- `LibraryLayout` in `tui_config` matches pane `preferred_layout`.
- `ColumnFocus` used for column focus **and** paging level.
- `TreeNodeId` keys the expand set.
- `VARIOUS_ARTISTS_NAME` shared string for synthetic root.
