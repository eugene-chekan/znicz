# Library Tree Render Performance Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the tree library view redraw without re-querying SQLite for every row or every frame.

**Architecture:** Resolve the tree selection once per frame instead of once per row, and cache the album list of each expanded artist inside `LibraryPane` so `tree_rows` is a pure in-memory operation. Refresh those caches on `reload_browse` so external scans still show up.

**Tech Stack:** Rust 2021, ratatui 0.30, rusqlite (via `znicz-library`), cargo test.

---

### Task 1: Add a tree frame benchmark and record the baseline

**Files:**
- Create: `znicz-tui/examples/tree_bench.rs`
- Test: run the example against `/home/eugene/.local/share/znicz/library.db`

- [ ] **Step 1: Write the benchmark example**

```rust
//! Measures the per-frame cost of the tree library layout.
//!
//! Usage:
//! cargo run --release -p znicz-tui --example tree_bench -- /path/library.db

use std::path::PathBuf;
use std::time::Instant;

use znicz_library::Library;
use znicz_tui::library_pane::LibraryPane;
use znicz_tui::LibraryLayout;

fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("usage: tree_bench <library.db>");
    let library = Library::open(&PathBuf::from(path)).expect("open library");
    let mut pane = LibraryPane::new(Some(library));
    pane.set_preferred_layout(LibraryLayout::Tree);

    if pane.artists().is_empty() {
        eprintln!("library has no browse artists");
        return;
    }

    // Expand the first artist, as pressing Enter on the top row does.
    pane.toggle_expand_at_cursor();
    let rows = pane.tree_rows().len();
    println!("tree rows: {rows}");

    let start = Instant::now();
    let mut sink = 0usize;
    for index in 0..rows {
        sink += pane.offset_for(index, 100);
    }
    println!(
        "old per-row offset_for frame: {:?} (sink {sink})",
        start.elapsed()
    );

    let start = Instant::now();
    let selected = pane.selected_index(100);
    let h_offset = pane.h_offset();
    let mut sink = 0usize;
    for index in 0..rows {
        sink += if selected == Some(index) { h_offset } else { 0 };
    }
    println!(
        "single-selection frame: {:?} (sink {sink})",
        start.elapsed()
    );

    let start = Instant::now();
    for _ in 0..3 {
        sink += pane.tree_rows().len();
    }
    println!("3x tree_rows: {:?} (sink {sink})", start.elapsed());
}
```

- [ ] **Step 2: Run the benchmark to record the baseline**

Run:

```bash
cargo run --release -p znicz-tui --example tree_bench --offline -- /home/eugene/.local/share/znicz/library.db
```

Expected on the reporter's database (2,437 tracks, 62 artists, 210 albums):

```text
tree rows: 69
old per-row offset_for frame: 300ms..400ms
single-selection frame: 4ms..6ms
3x tree_rows: 14ms..16ms
```

Record the actual three timings in the commit message body.

- [ ] **Step 3: Commit**

```bash
git add znicz-tui/examples/tree_bench.rs
git commit -m "Add tree layout frame benchmark"
```

---

### Task 2: Resolve the tree selection once per frame

**Files:**
- Modify: `znicz-tui/src/library_pane.rs` (`offset_for`, tests module)
- Modify: `znicz-tui/src/views/library.rs` (`render_tree`)

- [ ] **Step 1: Write the failing unit test**

In `znicz-tui/src/library_pane.rs`, inside `mod tests`, after
`tree_layout_expands_and_remembers_session_state`, add:

```rust
    #[test]
    fn only_the_selected_row_gets_the_pan_offset_regardless_of_layout() {
        let mut pane = LibraryPane::new(None);
        pane.inject_albums_for_test(vec![long_album("a"), long_album("b")]);
        pane.pan(4, 10, strip());

        assert_eq!(pane.offset_for_row(0, Some(0)), 4);
        assert_eq!(pane.offset_for_row(1, Some(0)), 0);
        assert_eq!(pane.offset_for_row(0, None), 0);
    }
```

- [ ] **Step 2: Run it to verify it fails**

Run:

```bash
cargo test -p znicz-tui --lib --offline only_the_selected_row_gets_the_pan_offset_regardless_of_layout
```

Expected: compile error, `no method named offset_for_row found for struct LibraryPane`.

- [ ] **Step 3: Add the pure offset rule**

Replace `offset_for` in `znicz-tui/src/library_pane.rs` with:

```rust
    pub fn offset_for(&self, index: usize, strip_inner: usize) -> usize {
        self.offset_for_row(index, self.selected_index(strip_inner))
    }

    /// Pan offset for one row when the selection has already been resolved.
    ///
    /// Tree rendering resolves the selected index once per frame; calling
    /// [`Self::offset_for`] per row rebuilds the tree for every row.
    pub fn offset_for_row(&self, index: usize, selected: Option<usize>) -> usize {
        if selected == Some(index) {
            self.h_offset
        } else {
            0
        }
    }
```

- [ ] **Step 4: Run the test to verify it passes**

Run:

```bash
cargo test -p znicz-tui --lib --offline only_the_selected_row_gets_the_pan_offset_regardless_of_layout
```

Expected: `1 passed; 0 failed`.

- [ ] **Step 5: Use the new rule in `render_tree`**

In `znicz-tui/src/views/library.rs`, replace the start of `render_tree`:

```rust
fn render_tree(frame: &mut Frame, area: Rect, app: &mut App, strip: usize, focused: bool) {
    let rows = app.library.tree_rows();
    let count = rows.len();
    // Resolve the selection once: asking per row rebuilds the tree for every
    // row and re-queries the library.
    let selected = app.library.selected_index(strip);
    let items: Vec<ListItem> = rows
        .iter()
        .enumerate()
        .map(|(i, row)| tree_row(app, row, strip, app.library.offset_for_row(i, selected)))
        .collect();
```

The rest of the function is unchanged.

- [ ] **Step 6: Run the TUI tests**

Run:

```bash
cargo test -p znicz-tui --offline
```

Expected: all tests pass.

- [ ] **Step 7: Commit**

```bash
git add znicz-tui/src/library_pane.rs znicz-tui/src/views/library.rs
git commit -m "Resolve tree selection once per frame"
```

---

### Task 3: Cache albums for expanded tree artists

**Files:**
- Modify: `znicz-tui/src/library_pane.rs` (imports, struct, `new`, `load_albums_for_artist`, `toggle_expand_at_cursor`, `tree_rows`, tests module)

- [ ] **Step 1: Write the failing unit test**

In `znicz-tui/src/library_pane.rs`, inside `mod tests`, add:

```rust
    #[test]
    fn expanded_tree_rows_use_cached_albums_after_the_library_goes_away() {
        let mut library = Library::open_in_memory().expect("open");
        library
            .upsert_track_for_test(
                Path::new("/music/a.flac"),
                "So What",
                Some("Miles".into()),
                Some("Kind of Blue".into()),
                None,
            )
            .unwrap();
        let mut pane = LibraryPane::new(Some(library));
        pane.set_preferred_layout(LibraryLayout::Tree);

        assert_eq!(pane.enter(strip()), EnterResult::Moved, "expand Miles");
        let with_library = pane.tree_rows().len();
        assert!(with_library >= 2, "artist plus its album");

        pane.library = None;
        assert_eq!(
            pane.tree_rows().len(),
            with_library,
            "expanded rows must come from the cache, not the connection"
        );
    }
```

- [ ] **Step 2: Run it to verify it fails**

Run:

```bash
cargo test -p znicz-tui --lib --offline expanded_tree_rows_use_cached_albums_after_the_library_goes_away
```

Expected: FAIL with `assertion left == right failed`, left `1`, right `2`.
The artist row survives, but the album row needs the library.

- [ ] **Step 3: Add the cache field**

Change the import at the top of `znicz-tui/src/library_pane.rs`:

```rust
use std::collections::{HashMap, HashSet};
```

Add this field to `struct LibraryPane`, after `expanded`:

```rust
    /// Albums of each expanded artist, keyed by case-folded artist name.
    /// Tree rows read these so a redraw does not re-scan the library.
    tree_albums_by_artist: HashMap<String, Vec<AlbumSummary>>,
```

Initialize it in `LibraryPane::new`, after `expanded: HashSet::new(),`:

```rust
            tree_albums_by_artist: HashMap::new(),
```

- [ ] **Step 4: Populate the cache when albums load**

Replace `load_albums_for_artist` with:

```rust
    fn load_albums_for_artist(&mut self, name: &str) {
        let albums = self
            .library
            .as_ref()
            .and_then(|lib| lib.albums_for_browse_artist(name).ok())
            .unwrap_or_default();
        self.tree_albums_by_artist
            .insert(name.to_lowercase(), albums.clone());
        self.albums = albums;
    }
```

In `toggle_expand_at_cursor`, replace the artist expand branch (the `else`
after `if expanded { ... }`):

```rust
                } else {
                    self.expanded.insert(id);
                    let albums = self
                        .library
                        .as_ref()
                        .and_then(|lib| lib.albums_for_browse_artist(&artist.name).ok())
                        .unwrap_or_default();
                    for album in &albums {
                        self.tree_album_cache
                            .insert(album.album.to_lowercase(), album.clone());
                    }
                    self.tree_albums_by_artist
                        .insert(artist.name.to_lowercase(), albums.clone());
                    self.albums = albums;
                }
```

This keeps the old side effect (`self.albums` is set for this artist) and
removes the second query that `load_albums_for_artist` used to make.

- [ ] **Step 5: Read the cache in `tree_rows`**

Replace this block in `tree_rows`:

```rust
            let albums = self
                .library
                .as_ref()
                .and_then(|lib| lib.albums_for_browse_artist(&artist.name).ok())
                .unwrap_or_default();
            for album in albums {
```

with:

```rust
            let artist_key = artist.name.to_lowercase();
            let cached = self.tree_albums_by_artist.get(&artist_key);
            let fallback;
            let albums: &[AlbumSummary] = match cached {
                Some(albums) => albums,
                None => {
                    fallback = self
                        .library
                        .as_ref()
                        .and_then(|lib| lib.albums_for_browse_artist(&artist.name).ok())
                        .unwrap_or_default();
                    &fallback
                }
            };
            for album in albums {
```

- [ ] **Step 6: Run the test to verify it passes**

Run:

```bash
cargo test -p znicz-tui --lib --offline expanded_tree_rows_use_cached_albums_after_the_library_goes_away
```

Expected: `1 passed; 0 failed`.

- [ ] **Step 7: Run the whole TUI suite**

Run:

```bash
cargo test -p znicz-tui --offline
```

Expected: all tests pass.

- [ ] **Step 8: Commit**

```bash
git add znicz-tui/src/library_pane.rs
git commit -m "Cache albums for expanded tree artists"
```

---

### Task 4: Drop the cache when an artist collapses

**Files:**
- Modify: `znicz-tui/src/library_pane.rs` (`toggle_expand_at_cursor`, tests module)

- [ ] **Step 1: Write the failing unit test**

In `znicz-tui/src/library_pane.rs`, inside `mod tests`, add:

```rust
    #[test]
    fn collapsing_an_artist_drops_its_cached_albums() {
        let mut library = Library::open_in_memory().expect("open");
        library
            .upsert_track_for_test(
                Path::new("/music/a.flac"),
                "So What",
                Some("Miles".into()),
                Some("Kind of Blue".into()),
                None,
            )
            .unwrap();
        let mut pane = LibraryPane::new(Some(library));
        pane.set_preferred_layout(LibraryLayout::Tree);

        assert_eq!(pane.enter(strip()), EnterResult::Moved, "expand Miles");
        assert!(pane.tree_albums_by_artist.contains_key("miles"));

        assert_eq!(pane.enter(strip()), EnterResult::Moved, "collapse Miles");
        assert!(!pane.tree_albums_by_artist.contains_key("miles"));
        assert_eq!(pane.tree_rows().len(), 1);
    }
```

- [ ] **Step 2: Run it to verify it fails**

Run:

```bash
cargo test -p znicz-tui --lib --offline collapsing_an_artist_drops_its_cached_albums
```

Expected: FAIL on `assert!(!pane.tree_albums_by_artist.contains_key("miles"))`.

- [ ] **Step 3: Remove the key on collapse**

In `toggle_expand_at_cursor`, replace the artist collapse branch:

```rust
                if expanded {
                    self.expanded.remove(&id);
                    self.expanded.retain(|n| match n {
                        TreeNodeId::Album { artist: a, .. } => a != &artist.name,
                        TreeNodeId::Artist(_) => true,
                    });
                    // Drop cached albums/tracks for this artist.
                    let albums = match self.tree_albums_by_artist.remove(&artist.name.to_lowercase())
                    {
                        Some(albums) => albums,
                        None => self
                            .library
                            .as_ref()
                            .and_then(|lib| lib.albums_for_browse_artist(&artist.name).ok())
                            .unwrap_or_default(),
                    };
                    for album in albums {
                        let key = album.album.to_lowercase();
                        self.tree_album_cache.remove(&key);
                        self.tree_track_cache.remove(&key);
                    }
                } else {
```

- [ ] **Step 4: Run the test to verify it passes**

Run:

```bash
cargo test -p znicz-tui --lib --offline collapsing_an_artist_drops_its_cached_albums
```

Expected: `1 passed; 0 failed`.

- [ ] **Step 5: Commit**

```bash
git add znicz-tui/src/library_pane.rs
git commit -m "Drop cached albums when collapsing an artist"
```

---

### Task 5: Refresh expanded artists on reload

**Files:**
- Modify: `znicz-tui/src/library_pane.rs` (`reload_browse`, new helper, tests module)

- [ ] **Step 1: Write the failing unit test**

In `znicz-tui/src/library_pane.rs`, inside `mod tests`, add:

```rust
    #[test]
    fn reload_refreshes_albums_for_every_expanded_artist() {
        let mut library = Library::open_in_memory().expect("open");
        for (path, title, artist, album) in [
            ("/music/1.flac", "Giant Steps", "Coltrane", "Giant Steps"),
            ("/music/2.flac", "So What", "Miles", "Kind of Blue"),
        ] {
            library
                .upsert_track_for_test(
                    Path::new(path),
                    title,
                    Some(artist.into()),
                    Some(album.into()),
                    None,
                )
                .unwrap();
        }
        let mut pane = LibraryPane::new(Some(library));
        pane.set_preferred_layout(LibraryLayout::Tree);

        assert_eq!(pane.enter(strip()), EnterResult::Moved, "expand Coltrane");
        pane.step(1, strip()); // Giant Steps
        pane.step(1, strip()); // Miles
        assert_eq!(pane.enter(strip()), EnterResult::Moved, "expand Miles");

        // A scan adds a second Miles album after his list was cached.
        pane.library
            .as_mut()
            .unwrap()
            .upsert_track_for_test(
                Path::new("/music/3.flac"),
                "Bitches Brew",
                Some("Miles".into()),
                Some("Bitches Brew".into()),
                None,
            )
            .unwrap();

        pane.reload_browse();

        assert!(
            pane.tree_rows().iter().any(
                |row| matches!(row, TreeRow::Album { album, .. } if album.album == "Bitches Brew")
            ),
            "reload must refresh every expanded artist, not only the selected one"
        );
    }
```

- [ ] **Step 2: Run it to verify it fails**

Run:

```bash
cargo test -p znicz-tui --lib --offline reload_refreshes_albums_for_every_expanded_artist
```

Expected: FAIL with the message `reload must refresh every expanded artist, not
only the selected one`. `reload_browse` refreshes only the selected artist
(Coltrane); Miles keeps the stale cached list.

- [ ] **Step 3: Add the refresh helper**

Add this method to `impl LibraryPane`, next to `refresh_albums_for_selection`:

```rust
    /// Refresh cached album lists for every expanded artist after a reload, so
    /// an external scan shows up without waiting for a collapse and re-expand.
    fn refresh_expanded_artists(&mut self) {
        let names: Vec<String> = self
            .expanded
            .iter()
            .filter_map(|id| match id {
                TreeNodeId::Artist(name) => Some(name.clone()),
                TreeNodeId::Album { .. } => None,
            })
            .collect();
        for name in names {
            let albums = match self.library.as_ref() {
                Some(library) => library
                    .albums_for_browse_artist(&name)
                    .unwrap_or_default(),
                None => continue,
            };
            self.tree_albums_by_artist
                .insert(name.to_lowercase(), albums);
        }
    }
```

- [ ] **Step 4: Call it from `reload_browse`**

In the non-empty-artists branch of `reload_browse`, after
`self.refresh_albums_for_selection();`, add:

```rust
            self.refresh_expanded_artists();
```

In the fallback branch (no artists), after `self.search_hits.clear();`, add:

```rust
        self.tree_albums_by_artist.clear();
```

- [ ] **Step 5: Run the test to verify it passes**

Run:

```bash
cargo test -p znicz-tui --lib --offline reload_refreshes_albums_for_every_expanded_artist
```

Expected: `1 passed; 0 failed`.

- [ ] **Step 6: Run the whole TUI suite**

Run:

```bash
cargo test -p znicz-tui --offline
```

Expected: all tests pass.

- [ ] **Step 7: Commit**

```bash
git add znicz-tui/src/library_pane.rs
git commit -m "Refresh expanded artist albums on reload"
```

---

### Task 6: Verify performance and the full suite

**Files:**
- Test: `znicz-tui/examples/tree_bench.rs`
- Test: workspace

- [ ] **Step 1: Re-run the benchmark**

Run:

```bash
cargo run --release -p znicz-tui --example tree_bench --offline -- /home/eugene/.local/share/znicz/library.db
```

Expected:

```text
tree rows: 69
old per-row offset_for frame: 3ms..10ms
single-selection frame: < 1ms
3x tree_rows: < 1ms
```

The rendered path now uses the single-selection figure. Record the exact
numbers in the pull request body.

- [ ] **Step 2: Run formatting**

Run:

```bash
cargo fmt --all -- --check
```

Expected: exit code 0, no output.

- [ ] **Step 3: Run Clippy**

Run:

```bash
cargo clippy --workspace --all-targets --offline -- -D warnings
```

Expected: exit code 0, no warnings.

- [ ] **Step 4: Run the full test suite**

Run:

```bash
cargo test --workspace --offline
```

Expected: every suite reports `test result: ok` with 0 failed.

- [ ] **Step 5: Commit any formatting fixes**

If `cargo fmt` changed files:

```bash
git add -A
git commit -m "Format tree render changes"
```

---

### Task 7: Bump the version and update the wiki

**Files:**
- Modify: `Cargo.toml` (`[workspace.package] version`)
- Modify: `Cargo.lock` (regenerated by cargo)
- Modify: `wiki/Rust/Cargo-Workspace.md`
- Modify: `wiki/Architecture/TUI.md`

- [ ] **Step 1: Bump the workspace version**

In `Cargo.toml`, change:

```toml
version = "0.5.0"
```

to:

```toml
version = "0.5.1"
```

- [ ] **Step 2: Refresh the lock file**

Run:

```bash
cargo check --workspace --offline
```

Then verify every workspace member moved:

```bash
grep -A1 'name = "znicz' Cargo.lock
```

Expected: `znicz`, `znicz-core`, `znicz-library`, `znicz-mcp`, and `znicz-tui`
all show `version = "0.5.1"`.

- [ ] **Step 3: Update the wiki version**

In `wiki/Rust/Cargo-Workspace.md`, change `currently **0.5.0**` to
`currently **0.5.1**`.

- [ ] **Step 4: Document the tree cache**

In `wiki/Architecture/TUI.md`, in the `library_layout = "tree"` bullet, after
the sentence about Enter/Space expanding a node, add:

```text
Album lists are cached per expanded artist, so a redraw does not re-scan the
library; **r** refreshes those lists.
```

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml Cargo.lock wiki/Rust/Cargo-Workspace.md wiki/Architecture/TUI.md
git commit -m "Bump workspace version to 0.5.1"
```

---

### Task 8: Push and open the pull request

**Files:** none.

- [ ] **Step 1: Confirm the branch state**

Run:

```bash
git --no-pager status --short
git --no-pager log --oneline main..HEAD
```

Expected: a clean tree and the planned commits on
`fix/tree-render-performance`.

- [ ] **Step 2: Push**

```bash
git push -u origin HEAD
```

- [ ] **Step 3: Open the pull request**

```bash
gh pr create --title "Fix tree library redraw performance" --body "$(cat <<'PR'
## Summary
- Resolve the tree selection once per frame instead of once per row.
- Cache albums per expanded artist so redraws stop re-scanning SQLite.
- Refresh those caches on reload; bump to 0.5.1 and update the wiki.

## Test Plan
- [ ] `cargo test --workspace --offline`
- [ ] `cargo clippy --workspace --all-targets --offline -- -D warnings`
- [ ] `cargo fmt --all -- --check`
- [ ] `cargo run --release -p znicz-tui --example tree_bench --offline -- /home/eugene/.local/share/znicz/library.db`

Baseline frame: 368 ms over 69 rows. Post-fix single-selection frame: see PR.
PR
)"
```

Expected: the PR URL is printed. Do not merge it.
