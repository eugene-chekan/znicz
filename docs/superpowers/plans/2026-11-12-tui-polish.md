# TUI polish (0.13.0) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix #70 and #71, and ship #72 (Browse labels) and #74 (volume icon) as one 0.13.0 release.

**Architecture:** Each issue is local. #70 is the run loop's repaint rule (`app.rs`). #71 is the Tab arm (`app.rs`). #72 is a row-context-aware label API in `library_pane.rs`, used by `views/library.rs`, plus the album sort in `znicz-library/src/store.rs`. #74 is `format::volume_icon` and a `[tui] icons` setting threaded through `TuiConfig`.

**Tech Stack:** Rust, ratatui 0.30, crossterm, rusqlite, unicode-width 0.2.

Spec: `docs/superpowers/specs/2026-11-12-tui-polish-design.md`.
Branch: `feat/tui-polish-0.13`. Every task must start by running
`git branch --show-current` and stopping if it is not `feat/tui-polish-0.13`.

Checks before each commit: `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`.

---

### Task 1: #70 Stage repaints when a modal closes with the drawer open

**Files:** Modify `znicz-tui/src/app.rs` (run loop around lines 594–610; `needs_full_redraw` around 2195; its test around 2245).

- [ ] **Step 1: Replace the test** `only_an_overlay_closing_over_the_stage_forces_a_repaint` with:

```rust
#[test]
fn closing_a_modal_or_the_drawer_over_the_stage_forces_a_repaint() {
    let none = Overlays { modal: false, drawer: false };
    let modal = Overlays { modal: true, drawer: false };
    let drawer = Overlays { modal: false, drawer: true };
    let both = Overlays { modal: true, drawer: true };
    assert!(needs_full_redraw(View::Stage, modal, none));
    assert!(needs_full_redraw(View::Stage, drawer, none));
    assert!(needs_full_redraw(View::Stage, both, drawer), "modal closes, drawer stays");
    assert!(needs_full_redraw(View::Stage, both, modal), "drawer closes, modal stays");
    assert!(!needs_full_redraw(View::Browse, both, none));
    assert!(!needs_full_redraw(View::Stage, none, both));
    assert!(!needs_full_redraw(View::Stage, both, both));
    assert!(!needs_full_redraw(View::Stage, drawer, both));
}
```

- [ ] **Step 2:** `cargo test -p znicz-tui closing_a_modal` — fails to compile (no `Overlays`).

- [ ] **Step 3: Implement.**

```rust
/// Which overlays sit over the main view this frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Overlays {
    pub(crate) modal: bool,
    pub(crate) drawer: bool,
}

/// Kitty and Sixel images bypass ratatui's cell diff, so on Stage any overlay
/// that goes away leaves its cells painted over the cover until a full repaint.
pub(crate) fn needs_full_redraw(view: View, before: Overlays, after: Overlays) -> bool {
    let modal_closed = before.modal && !after.modal;
    let drawer_closed = before.drawer && !after.drawer;
    view == View::Stage && (modal_closed || drawer_closed)
}
```

Add `fn overlays(&self) -> Overlays { Overlays { modal: self.modal != Modal::None, drawer: self.queue_open } }` on `App`. In the run loop use `let overlays_before = self.overlays();` / `let overlays_after = self.overlays();` and on a repaint also reset the cover so it is retransmitted:

```rust
if needs_full_redraw(self.view, overlays_before, overlays_after) {
    self.cover_draw_key = None;
    terminal.clear()?;
}
```

- [ ] **Step 4:** run the test and the full checks; all pass.
- [ ] **Step 5: Commit** `git commit -am "Repaint the Stage when a modal closes over the open drawer (#70)"`.

---

### Task 2: #71 Tab never opens the queue drawer

**Files:** Modify `znicz-tui/src/app.rs` (`KeyCode::Tab` arm around line 1020), `znicz-tui/src/keys.rs:158`, `znicz-tui/tests/keys.rs` (test `tab_opens_the_drawer_then_swaps_focus_without_closing_it` around line 93).

- [ ] **Step 1: Tests.** Replace `tab_opens_the_drawer_then_swaps_focus_without_closing_it` with two tests (use the file's existing helpers `new_app`, `press`, `open_queue`, `press_char`; read the old test first and keep any setup it needs):

```rust
#[test]
fn tab_does_not_open_the_drawer() {
    let mut app = new_app();
    press(&mut app, KeyCode::Tab);
    assert!(!app.queue_open);
    assert_eq!(app.focus, Focus::Library);

    press_char(&mut app, 'v'); // Stage
    press(&mut app, KeyCode::Tab);
    assert!(!app.queue_open);
    assert_eq!(app.focus, Focus::Library);
}

#[test]
fn tab_swaps_focus_while_the_drawer_is_open() {
    let mut app = new_app();
    open_queue(&mut app);
    // Same assertions the old test made after the drawer was open:
    // Tab moves focus to the queue and back, the drawer stays open.
}
```

Fill in the second test from the old test's swap assertions (it needs a wide enough `list_width` that the layout is the overlay, not the sheet; copy whatever the old test did). Also add a search case if a helper exists for entering search (`/`, type, Enter): Tab leaves the drawer closed.

- [ ] **Step 2:** `cargo test -p znicz-tui --test keys tab_` — `tab_does_not_open_the_drawer` fails.
- [ ] **Step 3: Implement.** In the Tab arm delete the branch

```rust
} else if !self.queue_open {
    self.open_queue();
    self.focus = Focus::Queue;
```

and replace it with `} else if !self.queue_open { // ] opens the drawer; Tab only moves focus`, an empty body. Keep the column branch, the sheet / Stage no-op and `swap_focus`. In `keys.rs` change the help line to `b("Tab", "next column, or library ↔ queue")`. Grep `znicz-tui/src` for any context footer that lists Tab as opening the queue and fix it.
- [ ] **Step 4:** full checks pass (fix any other test that relied on Tab opening the drawer by using `]` instead).
- [ ] **Step 5: Commit** `"Tab no longer opens the queue drawer (#71)"`.

---

### Task 3: #72 Albums sort by year

**Files:** Modify `znicz-library/src/store.rs` (`albums_for_browse_artist`, around line 433) and its tests in the same file (or `znicz-library/tests/`, wherever existing `albums_for_browse_artist` tests live — `grep -rn albums_for_browse_artist znicz-library`).

- [ ] **Step 1: Test.** Next to the existing tests for this function, using the same fixture style, insert for one artist: `Tutu` (1986), `Kind of Blue` (1959), `Bootlegs` (no year), `Agharta` (1975), `aura` (1985). Expect `["Kind of Blue", "Agharta", "aura", "Tutu", "Bootlegs"]`.
- [ ] **Step 2:** run it; fails (name order).
- [ ] **Step 3: Implement.**

```rust
albums.sort_by(|a, b| {
    // Year ascending, albums without a year last; then name.
    let year = |album: &AlbumSummary| album.year.unwrap_or(u32::MAX);
    year(a)
        .cmp(&year(b))
        .then_with(|| a.album.to_lowercase().cmp(&b.album.to_lowercase()))
        .then_with(|| a.album.cmp(&b.album))
});
```

Update the doc comment: `/// Albums attributed to a browse artist (compilations for Various Artists), oldest first; albums without a year last.`
- [ ] **Step 4:** full checks; fix any test in the workspace that assumed name order (update expectations, do not change behaviour).
- [ ] **Step 5: Commit** `"Sort an artist's albums by year (#72)"`.

---

### Task 4: #72 Grouped labels: no repeated artist, numbered tracks

**Files:** Modify `znicz-tui/src/library_pane.rs` (`album_middle`, `track_middle`, `middle_len`, `longest_middle`, `selected_middle_len`, `marker_width` around lines 900–1022), `znicz-tui/src/views/library.rs` (`tree_row` ~255–290, columns ~100–145, paging list ~340–370, `hit_row` ~520, `album_row` ~551, `track_row` ~574, `track_number` / `track_fixed` ~650). Tests: unit tests in `library_pane.rs`, render tests in `znicz-tui/tests/library_browse.rs` or `render.rs` (follow the existing style there).

**Label API** (in `library_pane.rs`, `pub`):

```rust
/// How the rows of one album's track list are numbered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Numbering {
    /// Display width of the widest label (`10.` is 3, `2-10.` is 5). 0: no numbers.
    width: usize,
    multi_disc: bool,
}

impl Numbering {
    pub fn for_tracks(tracks: &[Track]) -> Self {
        let multi_disc = tracks.iter().any(|t| t.disc_number.unwrap_or(1) > 1);
        let width = tracks
            .iter()
            .filter_map(|t| number_label(t, multi_disc))
            .map(|label| label.chars().count())
            .max()
            .unwrap_or(0);
        Self { width, multi_disc }
    }

    /// `" 9. "`, `"2-3. "`, blanks for a missing number, `""` when the list has none.
    pub fn prefix(&self, track: &Track) -> String {
        if self.width == 0 {
            return String::new();
        }
        let label = number_label(track, self.multi_disc).unwrap_or_default();
        format!("{label:>width$} ", width = self.width)
    }
}

fn number_label(track: &Track, multi_disc: bool) -> Option<String> {
    let n = track.track_number?;
    Some(if multi_disc {
        format!("{}-{n}.", track.disc_number.unwrap_or(1))
    } else {
        format!("{n}.")
    })
}

/// Where a row sits, which decides how much of its label repeats context.
#[derive(Debug, Clone, Copy)]
pub enum RowContext<'a> {
    /// Search hits and track lists with no artist above them: full labels.
    Flat,
    /// Under a browse artist (and, for tracks, an album).
    Grouped { artist: &'a str, numbering: Numbering },
}
```

Labels replace the old two functions:

```rust
pub fn album_middle(album: &AlbumSummary, context: RowContext) -> String {
    let year = album.year.map(|y| format!(" ({y})")).unwrap_or_default();
    match context {
        RowContext::Grouped { .. } => format!("{}{year}", album.album),
        RowContext::Flat => {
            let artist = album.album_artist.as_deref().unwrap_or("Unknown artist");
            format!("{}{year} — {artist}", album.album)
        }
    }
}

pub fn track_middle(track: &Track, context: RowContext) -> String {
    match context {
        RowContext::Flat => match track.artist.as_deref() {
            Some(artist) => format!("{} — {artist}", track.title),
            None => track.title.clone(),
        },
        RowContext::Grouped { artist: above, numbering } => {
            let number = numbering.prefix(track);
            match track.artist.as_deref() {
                Some(artist) if !same_artist(artist, above) => {
                    format!("{number}{} — {artist}", track.title)
                }
                _ => format!("{number}{}", track.title),
            }
        }
    }
}

fn same_artist(a: &str, b: &str) -> bool {
    a.trim().to_lowercase() == b.trim().to_lowercase()
}
```

**Where each context applies:**

| Place | Context |
|-------|---------|
| Tree `TreeRow::Album { artist, .. }` | `Grouped { artist, numbering: Numbering::default() }` |
| Tree `TreeRow::Track { album_key, .. }` | `Grouped { artist: <artist of the nearest TreeRow::Album above it>, numbering: Numbering::for_tracks(<tree_track_cache[album_key]>) }` |
| Columns: albums column, paging list `ColumnFocus::Albums` | Grouped with the selected artist's name |
| Columns: tracks column, paging list `ColumnFocus::Tracks` | Grouped with the selected artist and `Numbering::for_tracks(self.tracks())` |
| Search hits, `Mode::AllTracks`, single-list `Mode::Browse` tracks | `Flat` |

Add `pub fn album_context(&self) -> RowContext<'_>` and `pub fn track_context(&self) -> RowContext<'_>` on `LibraryPane` for the column / paging cases (selected artist name from the artists list and selected artist index — find the existing accessor; Flat when no artist is selected). For the tree, compute the track context once per album while building / iterating rows (do not call `Numbering::for_tracks` per row in a hot loop: compute per `album_key` once per frame, e.g. a small `HashMap<&str, Numbering>` or by tracking the current album while iterating rows in order). `TreeRow::Album` carries `artist`; a track row's artist is the `artist` of the most recent `TreeRow::Album` before it in the row list.

**Rows:**
- `album_row(album, context, strip, offset, playing)` — drop the `short: bool` parameter; the old `short` text is now `album_middle(album, Grouped)`.
- `track_row(track, context, strip, offset, playing)` — middle `track_middle(track, context)`; right column `" ▶ "` when playing, else `"   "`, then the time. Remove `track_number`; `track_fixed` uses 3 for the marker column. Tree track rows keep their `▶ ` lead marker.
- `middle_len(item, context, playing, tree)` uses the same `album_middle` / `track_middle` call as the row; `longest_middle` and `selected_middle_len` pass the same contexts as the table above. Keep `marker_width` as is.

- [ ] **Step 1: Unit tests** in `library_pane.rs` `#[cfg(test)]` (build `Track` / `AlbumSummary` with a small helper; check for an existing one first):
  - grouped album: `"Kind of Blue (1959)"`; flat album: `"Kind of Blue (1959) — Miles Davis"`.
  - grouped track, same artist (case / whitespace differ): `"1. So What"`.
  - grouped track, other artist under `Various Artists`: `"3. Song — Someone"`.
  - flat track: `"So What — Miles Davis"`.
  - numbering: tracks 1..=10 → prefix of 9 is `" 9. "`, of 10 is `"10. "`; disc 2 track 3 in a multi-disc list → `"2-3. "` and disc-1 tracks in that list get `"1-N. "`; a track with no number in a numbered list → `"   "` blanks of width 2 + space (`"   "` for width 2); no numbers anywhere → `""`.
- [ ] **Step 2:** tests fail to compile.
- [ ] **Step 3:** implement the API, then update every call site (compiler-driven).
- [ ] **Step 4: Render tests** (follow how existing tests in `znicz-tui/tests/library_browse.rs` / `render.rs` build a library and read the buffer): an expanded artist in the tree shows `Kind of Blue (1959)` without `— Miles Davis`, and `1. So What`; a search for the track still shows `So What — Miles Davis`. A `middle_len`/pan test: the pan clamps to the grouped label's length (if an existing pan test covers tree rows, update it).
- [ ] **Step 5:** full checks pass; update existing tests whose expected text had the old suffix or right-hand numbers.
- [ ] **Step 6: Commit** `"Browse: drop the repeated artist and number tracks (#72)"`.

---

### Task 5: #74 Volume icon and `[tui] icons`

**Files:** `Cargo.toml` (workspace deps), `znicz-tui/Cargo.toml`, `znicz-tui/src/tui_config.rs`, `znicz-tui/src/lib.rs` (re-export), `znicz-tui/src/format.rs`, `znicz-tui/src/views/stage.rs` (`volume_line` ~237), `znicz-tui/src/views/now_playing.rs` (`chrome_line` ~102–200), `znicz/src/main.rs` (`TuiSection`, `tui_config` ~179–198), `znicz-tui/src/theme.rs` (check for a dim style; reuse an existing one such as `theme::dim()` / `theme::muted()` if present).

- [ ] **Step 1: Config type** in `tui_config.rs`:

```rust
/// Which symbols the TUI draws for icons (the volume marker).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum IconSet {
    #[default]
    Unicode,
    Emoji,
    Nerd,
}

impl IconSet {
    pub fn parse(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "emoji" => Self::Emoji,
            "nerd" => Self::Nerd,
            _ => Self::Unicode,
        }
    }
}
```

Add `pub icons: IconSet` to `TuiConfig` (default `IconSet::Unicode`), re-export `IconSet` from `lib.rs`, add `icons: Option<String>` to `TuiSection` and `icons: znicz_tui::IconSet::parse(config.tui.icons.as_deref().unwrap_or("unicode"))` in `tui_config()`. Test `IconSet::parse` (each value, case, unknown → Unicode).

- [ ] **Step 2: Marker tests** in `format.rs`:

```rust
#[test]
fn the_volume_icon_follows_the_level_and_mute() {
    use crate::tui_config::IconSet::*;
    assert_eq!(volume_icon(Unicode, 0.1, false), "◢");
    assert_eq!(volume_icon(Unicode, 0.9, false), "◢");
    assert_eq!(volume_icon(Unicode, 0.9, true), "×");
    assert_eq!(volume_icon(Emoji, 0.33, false), "🔈");
    assert_eq!(volume_icon(Emoji, 0.34, false), "🔉");
    assert_eq!(volume_icon(Emoji, 0.66, false), "🔉");
    assert_eq!(volume_icon(Emoji, 0.67, false), "🔊");
    assert_eq!(volume_icon(Emoji, 0.5, true), "🔇");
    assert_eq!(volume_icon(Nerd, 0.0, false), "\u{f057f}");
    assert_eq!(volume_icon(Nerd, 0.5, false), "\u{f0580}");
    assert_eq!(volume_icon(Nerd, 1.0, false), "\u{f057e}");
    assert_eq!(volume_icon(Nerd, 1.0, true), "\u{f075f}");
}

#[test]
fn display_width_counts_emoji_as_two_cells() {
    assert_eq!(display_width("◢ ███"), 5);
    assert_eq!(display_width("🔊 ███"), 6);
}
```

- [ ] **Step 3: Implement** in `format.rs`. Add `unicode-width = "0.2"` to `[workspace.dependencies]` in the root `Cargo.toml` and `unicode-width = { workspace = true }` to `znicz-tui/Cargo.toml` (check how other workspace deps are declared there and match):

```rust
/// The marker before the volume bar: level-dependent, or a mute sign.
pub fn volume_icon(icons: IconSet, volume: f32, muted: bool) -> &'static str {
    let percent = (volume.clamp(0.0, 1.0) * 100.0).round() as u32;
    let level = match percent {
        0..=33 => 0,
        34..=66 => 1,
        _ => 2,
    };
    match (icons, muted) {
        (IconSet::Unicode, true) => "×",
        (IconSet::Unicode, false) => "◢",
        (IconSet::Emoji, true) => "🔇",
        (IconSet::Emoji, false) => ["🔈", "🔉", "🔊"][level],
        (IconSet::Nerd, true) => "\u{f075f}",
        (IconSet::Nerd, false) => ["\u{f057f}", "\u{f0580}", "\u{f057e}"][level],
    }
}

/// Terminal cells a string takes (emoji are two).
pub fn display_width(text: &str) -> usize {
    unicode_width::UnicodeWidthStr::width(text)
}
```

- [ ] **Step 4: Stage** `volume_line(state, icons)` (pass `app.tui.icons` from `render`). Not muted: one span `"{icon} {bar} {pct:>3.0}%"` in `theme::text()`. Muted: spans `"{icon} "` in `theme::warn()`, the 10-cell bar of `state.volume` in a dim style, then `" muted"` in `theme::warn()`. Update the doc comment example. Add a Stage render test (follow `znicz-tui/tests/stage.rs`) that the volume line contains `◢ ` before the bar, and `× ` and `muted` when muted (if setting mute in a test is impractical, unit-test `volume_line` in `stage.rs` instead).

- [ ] **Step 5: Transport.** `chrome_line` gets `icons: IconSet` (from `app.tui.icons` in `render_transport`). The volume segment becomes `" {icon} {bar} {pct:>3.0}%"` or, muted, `" {icon} {bar} muted"` (the bar can stay unstyled-dim here; one string is fine as long as the existing span styling still applies). Replace `.chars().count()` with `format::display_width` for the volume, toggles and subtitle lengths in the drop-order loop (`volume_len`, `toggles_len`, `subtitle_len`) so a two-cell emoji is counted correctly. Test (unit test in `now_playing.rs`, next to any existing `chrome_line` tests): with `IconSet::Emoji`, at a width where the line fits only without the volume segment, the output contains no `🔊` and its display width is ≤ width; at a wide width it contains `🔊`.

- [ ] **Step 6:** full checks pass.
- [ ] **Step 7: Commit** `"Volume marker and the [tui] icons setting (#74)"`.

---

### Task 6: Docs and version

**Files:** `Cargo.toml` (`[workspace.package] version` → `0.13.0`), `Cargo.lock` (via `cargo build`), `README.md`, `wiki/Architecture/TUI.md`, any wiki page documenting `config.toml` `[tui]` keys (`grep -rn "library_layout\|marquee" wiki README.md`).

- [ ] **Step 1:** bump the version; `cargo build` to refresh `Cargo.lock`.
- [ ] **Step 2:** README / wiki: Tab (moves columns or library ↔ queue focus; `]` alone opens the drawer); Browse labels (no repeated artist under an artist, `N.` / `D-N.` numbering, albums oldest first, year-less last; search keeps `— Artist`); the volume marker and `[tui] icons = "unicode" | "emoji" | "nerd"` with the table from the spec, next to the other `[tui]` keys.
- [ ] **Step 3:** full checks; commit `"Document the TUI polish; bump to 0.13.0"`.
