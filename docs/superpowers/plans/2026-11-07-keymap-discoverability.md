# Keymap and Discoverability Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `h` / `l` move in the library while arrows seek; the footer shows a short hint list for the current context ending in `? more`; help opens on the current context, scrolls, and filters.

**Architecture:** `keys.rs` gains a `Context` enum, `Hint` consts listed per context, and help sections built from the existing binding tables plus a pure filter. `App::context()` works out the context once per frame for the footer and help. `LibraryPane::move_horizontal` handles `h` / `l` per layout. Help state (`help_state.rs`) lives on `App` next to the unit `Modal::Help`.

**Tech Stack:** Rust 2021, ratatui 0.30, crossterm 0.28, cargo test.

**Spec:** [`docs/superpowers/specs/2026-11-07-keymap-discoverability-design.md`](../specs/2026-11-07-keymap-discoverability-design.md)

**Commands used throughout:**

- Unit tests in the crate: `cargo test -p znicz-tui --lib <filter>`
- Integration tests: `cargo test -p znicz-tui --test keys <filter>` (also `render`, `mouse`)
- Everything: `cargo test --workspace`

---

### Task 1: Commit the plan and align the spec

Three details changed while planning; record them in the spec so spec and code agree.

**Files:**
- Create: `docs/superpowers/plans/2026-11-07-keymap-discoverability.md` (this file)
- Modify: `docs/superpowers/specs/2026-11-07-keymap-discoverability-design.md`

- [x] **Step 1: Update the spec's `keys.rs` architecture section**

In the spec, replace the block from `The binding tables stay the single source. \`Binding\` gains an optional hint:` through the end of the bullet list (the line ending `implements the filter as\n  a pure function.`) with:

````markdown
The binding tables stay the single source for help. `Binding` itself does not
change (it only gains `Debug, PartialEq, Eq` derives). Footer hints are separate
consts, listed per context, because footer order differs from table order:

```rust
pub struct Hint {
    pub label: &'static str,
    pub key: KeyEvent,
}
```

A unit test checks that every hint in a context with a table names a key that
the context's help sections document, so hints cannot drift from the tables.

New items:

- `enum Context { Library, LibraryTree, Search, Queue, Devices, Inspector,
  Playlists, Radio, Prompt, StationPrompt, Help }`.
- `fn context_hints(context: Context) -> &'static [Hint]` and
  `fn more_hint(context: Context) -> Option<Hint>`.
- `pub const INSPECTOR: &[Binding]` (`Esc / i` close), so the inspector has a
  help section.
- `fn help_sections(context: Context) -> Vec<Section>` where
  `Section { title, bindings, dim }`, in display order.
- `fn filter_sections(sections, query) -> Vec<Section>` implements the filter as
  a pure function.
````

- [x] **Step 2: Update the spec's `app.rs` architecture bullets**

Replace the bullet starting `` - `Modal::Help` becomes `Modal::Help(HelpState)` `` (two lines) with:

```markdown
- `Modal::Help` stays a unit variant so `Modal` keeps `Copy`. `App` gains
  `pub help: HelpState` (new `help_state.rs`:
  `HelpState { context, scroll, query: LineEdit, editing }`), reset each time
  help opens with the context captured at that moment.
- The mouse wheel scrolls help.
```

- [x] **Step 3: Note Ctrl-C in the help key table**

In the spec's "Keys inside help" table, replace the row `| any other key | Ignored |` with:

```markdown
| `Ctrl-C` | Quit the TUI, as everywhere else |
| any other key | Ignored |
```

- [x] **Step 4: Commit**

```bash
git add docs/superpowers/plans/2026-11-07-keymap-discoverability.md docs/superpowers/specs/2026-11-07-keymap-discoverability-design.md
git commit -m "Add keymap and discoverability implementation plan"
```

---

### Task 2: Footer hint data in `keys.rs`

**Files:**
- Modify: `znicz-tui/src/keys.rs`

- [ ] **Step 1: Write the failing tests**

Add to the `#[cfg(test)] mod tests` block in `znicz-tui/src/keys.rs` (after the existing tests):

```rust
    const ALL_CONTEXTS: [Context; 11] = [
        Context::Library,
        Context::LibraryTree,
        Context::Search,
        Context::Queue,
        Context::Devices,
        Context::Inspector,
        Context::Playlists,
        Context::Radio,
        Context::Prompt,
        Context::StationPrompt,
        Context::Help,
    ];

    /// Columns a hint row needs: labels plus ` · ` between them.
    fn row_width(hints: &[Hint]) -> usize {
        let labels: usize = hints.iter().map(|h| h.label.chars().count()).sum();
        labels + 3 * hints.len().saturating_sub(1)
    }

    #[test]
    fn every_context_fits_eighty_columns_with_more() {
        for context in ALL_CONTEXTS {
            let mut width = row_width(context_hints(context));
            if let Some(more) = more_hint(context) {
                width += 3 + more.label.chars().count();
            }
            assert!(width <= 80, "{context:?} needs {width} columns");
        }
    }

    #[test]
    fn every_context_but_help_ends_in_more() {
        for context in ALL_CONTEXTS {
            match context {
                Context::Help => assert_eq!(more_hint(context), None),
                _ => assert_eq!(more_hint(context), Some(MORE), "{context:?}"),
            }
        }
        assert_eq!(MORE.key.code, KeyCode::Char('?'));
    }

    #[test]
    fn library_hints_lead_with_pause_and_label_p_and_r() {
        let labels: Vec<&str> = context_hints(Context::Library)
            .iter()
            .map(|h| h.label)
            .collect();
        assert_eq!(
            labels,
            [
                "Space pause",
                "/ search",
                "a add",
                "] queue",
                "P playlists",
                "R radio"
            ]
        );
    }

    #[test]
    fn the_tree_offers_expand_instead_of_pause() {
        let labels: Vec<&str> = context_hints(Context::LibraryTree)
            .iter()
            .map(|h| h.label)
            .collect();
        assert!(labels.contains(&"o expand"), "{labels:?}");
        assert!(!labels.contains(&"Space pause"), "{labels:?}");
    }

    #[test]
    fn each_hint_sends_the_key_its_label_names() {
        for context in ALL_CONTEXTS {
            for hint in context_hints(context) {
                let token = hint.label.split_once(' ').map(|(k, _)| k).unwrap();
                let expected = match token {
                    "Space" => KeyCode::Char(' '),
                    "Enter" => KeyCode::Enter,
                    "Esc" => KeyCode::Esc,
                    "Tab" => KeyCode::Tab,
                    one => KeyCode::Char(one.chars().next().unwrap()),
                };
                assert_eq!(hint.key.code, expected, "{context:?}: {}", hint.label);
                assert_eq!(hint.key.modifiers, KeyModifiers::NONE);
            }
        }
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p znicz-tui --lib keys::`
Expected: compile error, `cannot find type Context` / `cannot find function context_hints`.

- [ ] **Step 3: Implement the hint data**

At the top of `znicz-tui/src/keys.rs`, under the module doc comment, add:

```rust
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
```

After `const fn b(...)`, add:

```rust
/// Where the user is: picks the footer hints and the first help section.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Context {
    Library,
    LibraryTree,
    Search,
    Queue,
    Devices,
    Inspector,
    Playlists,
    Radio,
    Prompt,
    StationPrompt,
    Help,
}

/// One clickable footer hint: what it says and the key a click sends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Hint {
    pub label: &'static str,
    pub key: KeyEvent,
}

const fn hint(label: &'static str, code: KeyCode) -> Hint {
    Hint {
        label,
        key: KeyEvent::new(code, KeyModifiers::NONE),
    }
}

const SPACE_PAUSE: Hint = hint("Space pause", KeyCode::Char(' '));
const SEARCH: Hint = hint("/ search", KeyCode::Char('/'));
const ADD: Hint = hint("a add", KeyCode::Char('a'));
const EXPAND: Hint = hint("o expand", KeyCode::Char('o'));
const QUEUE_OPEN: Hint = hint("] queue", KeyCode::Char(']'));
const QUEUE_CLOSE: Hint = hint("] close", KeyCode::Char(']'));
const PLAYLISTS_OPEN: Hint = hint("P playlists", KeyCode::Char('P'));
const RADIO_OPEN: Hint = hint("R radio", KeyCode::Char('R'));
const ENTER_PLAY: Hint = hint("Enter play", KeyCode::Enter);
const REMOVE: Hint = hint("d remove", KeyCode::Char('d'));
const JUMP: Hint = hint("o jump", KeyCode::Char('o'));
const CLEAR: Hint = hint("C clear", KeyCode::Char('C'));
const SELECT: Hint = hint("Enter select", KeyCode::Enter);
const RESCAN: Hint = hint("r rescan", KeyCode::Char('r'));
const NEW: Hint = hint("n new", KeyCode::Char('n'));
const EDIT: Hint = hint("e edit", KeyCode::Char('e'));
const DELETE: Hint = hint("d delete", KeyCode::Char('d'));
const CLOSE: Hint = hint("Esc close", KeyCode::Esc);

// Prompt and help hints: these keys belong to a prompt, not a binding table.
const ENTER_SEARCH: Hint = hint("Enter search", KeyCode::Enter);
const ENTER_SAVE: Hint = hint("Enter save", KeyCode::Enter);
const CANCEL: Hint = hint("Esc cancel", KeyCode::Esc);
const NEXT_FIELD: Hint = hint("Tab next field", KeyCode::Tab);
const FILTER: Hint = hint("/ filter", KeyCode::Char('/'));

/// Drawn at the right end of the footer in every context except help.
pub const MORE: Hint = hint("? more", KeyCode::Char('?'));

/// Footer hints for a context, most important first.
pub fn context_hints(context: Context) -> &'static [Hint] {
    match context {
        Context::Library => &[
            SPACE_PAUSE,
            SEARCH,
            ADD,
            QUEUE_OPEN,
            PLAYLISTS_OPEN,
            RADIO_OPEN,
        ],
        Context::LibraryTree => &[SEARCH, ADD, EXPAND, QUEUE_OPEN, PLAYLISTS_OPEN, RADIO_OPEN],
        Context::Search => &[ENTER_SEARCH, CANCEL],
        Context::Queue => &[ENTER_PLAY, REMOVE, JUMP, CLEAR, QUEUE_CLOSE],
        Context::Devices => &[SELECT, RESCAN, CLOSE],
        Context::Inspector => &[SPACE_PAUSE, CLOSE],
        Context::Playlists | Context::Radio => &[ENTER_PLAY, ADD, NEW, EDIT, DELETE, CLOSE],
        Context::Prompt => &[ENTER_SAVE, CANCEL],
        Context::StationPrompt => &[ENTER_SAVE, CANCEL, NEXT_FIELD],
        Context::Help => &[FILTER, CLOSE],
    }
}

/// The right-aligned `? more` hint, or `None` inside help itself.
pub fn more_hint(context: Context) -> Option<Hint> {
    (context != Context::Help).then_some(MORE)
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p znicz-tui --lib keys::`
Expected: PASS (the old `each_pane_has_its_own_hint_line` still passes; it is removed in Task 5).

- [ ] **Step 5: Commit**

```bash
git add znicz-tui/src/keys.rs
git commit -m "Add footer hint contexts to the keymap"
```

---

### Task 3: Help sections and filter in `keys.rs`

**Files:**
- Modify: `znicz-tui/src/keys.rs`

- [ ] **Step 1: Write the failing tests**

Add to the tests module in `znicz-tui/src/keys.rs`:

```rust
    fn titles(sections: &[Section]) -> Vec<&'static str> {
        sections.iter().map(|s| s.title).collect()
    }

    #[test]
    fn help_starts_with_the_context_then_playback_and_navigation() {
        let sections = help_sections(Context::Queue);
        assert_eq!(
            titles(&sections),
            [
                "Queue",
                "Playback",
                "Navigation",
                "Library",
                "Playlists",
                "Radio",
                "Devices",
                "Inspector"
            ]
        );
        let dims: Vec<bool> = sections.iter().map(|s| s.dim).collect();
        assert_eq!(dims, [false, false, false, true, true, true, true, true]);
    }

    #[test]
    fn tree_and_search_open_help_on_library() {
        for context in [Context::Library, Context::LibraryTree, Context::Search] {
            assert_eq!(help_sections(context)[0].title, "Library", "{context:?}");
        }
    }

    #[test]
    fn prompts_and_help_have_no_context_section() {
        for context in [Context::Prompt, Context::StationPrompt, Context::Help] {
            let sections = help_sections(context);
            assert_eq!(sections[0].title, "Playback", "{context:?}");
            assert_eq!(sections.len(), 8, "every table still listed once");
            assert_eq!(context_title(context), None);
        }
    }

    #[test]
    fn the_filter_matches_actions_ignoring_case() {
        let sections = filter_sections(help_sections(Context::Library), "VOL");
        assert_eq!(titles(&sections), ["Playback"]);
        assert!(sections[0].bindings.iter().all(|b| b.action.contains("volume")));
    }

    #[test]
    fn the_filter_matches_key_text() {
        let sections = filter_sections(help_sections(Context::Library), "del");
        assert!(sections
            .iter()
            .flat_map(|s| s.bindings.iter())
            .any(|b| b.keys == "d / Del"));
    }

    #[test]
    fn a_blank_filter_keeps_everything_and_a_miss_keeps_nothing() {
        let all = help_sections(Context::Queue);
        assert_eq!(filter_sections(all.clone(), "  "), all);
        assert!(filter_sections(all, "zzzz").is_empty());
    }

    #[test]
    fn table_backed_hints_name_a_documented_key() {
        for context in [
            Context::Library,
            Context::LibraryTree,
            Context::Queue,
            Context::Devices,
            Context::Inspector,
            Context::Playlists,
            Context::Radio,
        ] {
            let sections = help_sections(context);
            let documented: Vec<&Binding> = sections
                .iter()
                .filter(|s| !s.dim)
                .flat_map(|s| s.bindings.iter().copied())
                .collect();
            for hint in context_hints(context) {
                let token = hint.label.split_once(' ').map(|(k, _)| k).unwrap();
                assert!(
                    documented
                        .iter()
                        .any(|b| b.keys.split(" / ").any(|k| k == token)),
                    "{context:?}: hint {:?} has no binding in its help sections",
                    hint.label
                );
            }
        }
    }
```

Also add `INSPECTOR` to the `tables` array in the existing `every_binding_documents_both_a_key_and_an_action` test:

```rust
        let tables = [
            GLOBAL, NAVIGATION, QUEUE, LIBRARY, DEVICES, PLAYLISTS, RADIO, INSPECTOR,
        ];
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p znicz-tui --lib keys::`
Expected: compile error, `cannot find type Section` / `cannot find value INSPECTOR`.

- [ ] **Step 3: Implement sections and the filter**

In `znicz-tui/src/keys.rs`, add a derive to `Binding`:

```rust
/// One documented binding.
#[derive(Debug, PartialEq, Eq)]
pub struct Binding {
```

After the `RADIO` table, add:

```rust
pub const INSPECTOR: &[Binding] = &[b("Esc / i", "close")];

/// A titled group of bindings in the help overlay.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Section {
    pub title: &'static str,
    pub bindings: Vec<&'static Binding>,
    /// Belongs to another context: drawn dim, after the sections that apply now.
    pub dim: bool,
}

/// Context tables, in the order they appear when they are not first.
const CONTEXT_SECTIONS: &[(&str, &[Binding])] = &[
    ("Library", LIBRARY),
    ("Queue", QUEUE),
    ("Playlists", PLAYLISTS),
    ("Radio", RADIO),
    ("Devices", DEVICES),
    ("Inspector", INSPECTOR),
];

/// Title of the section holding this context's own keys, if it has a table.
pub fn context_title(context: Context) -> Option<&'static str> {
    match context {
        Context::Library | Context::LibraryTree | Context::Search => Some("Library"),
        Context::Queue => Some("Queue"),
        Context::Devices => Some("Devices"),
        Context::Inspector => Some("Inspector"),
        Context::Playlists => Some("Playlists"),
        Context::Radio => Some("Radio"),
        Context::Prompt | Context::StationPrompt | Context::Help => None,
    }
}

fn section(title: &'static str, bindings: &'static [Binding], dim: bool) -> Section {
    Section {
        title,
        bindings: bindings.iter().collect(),
        dim,
    }
}

/// Help in display order: this context, Playback, Navigation, then the rest dim.
pub fn help_sections(context: Context) -> Vec<Section> {
    let first = context_title(context);
    let mut sections = Vec::new();
    for &(title, bindings) in CONTEXT_SECTIONS {
        if Some(title) == first {
            sections.push(section(title, bindings, false));
        }
    }
    sections.push(section("Playback", GLOBAL, false));
    sections.push(section("Navigation", NAVIGATION, false));
    for &(title, bindings) in CONTEXT_SECTIONS {
        if Some(title) != first {
            sections.push(section(title, bindings, true));
        }
    }
    sections
}

/// Keep bindings whose keys or action contain `query`, ignoring case.
///
/// Sections left empty are dropped. A blank query keeps everything.
pub fn filter_sections(sections: Vec<Section>, query: &str) -> Vec<Section> {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return sections;
    }
    sections
        .into_iter()
        .filter_map(|mut section| {
            section.bindings.retain(|b| {
                b.keys.to_lowercase().contains(&query) || b.action.to_lowercase().contains(&query)
            });
            (!section.bindings.is_empty()).then_some(section)
        })
        .collect()
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p znicz-tui --lib keys::`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add znicz-tui/src/keys.rs
git commit -m "Build help sections and a filter from the keymap"
```

---

### Task 4: `h` / `l` in the library pane

**Files:**
- Modify: `znicz-tui/src/library_pane.rs`

- [ ] **Step 1: Write the failing tests**

Add to the `#[cfg(test)] mod tests` block at the bottom of `znicz-tui/src/library_pane.rs`:

```rust
    /// Miles (one album, two tracks) sorts before Portishead (one track).
    fn two_artist_library() -> Library {
        let mut library = Library::open_in_memory().expect("open");
        for (path, title, artist, album) in [
            ("/music/a1.flac", "So What", "Miles", "Kind of Blue"),
            ("/music/a2.flac", "Freddie Freeloader", "Miles", "Kind of Blue"),
            ("/music/b1.flac", "Roads", "Portishead", "Dummy"),
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
        library
    }

    #[test]
    fn h_and_l_step_through_columns_without_wrapping() {
        let mut pane = LibraryPane::new(Some(two_artist_library()));
        assert_eq!(pane.effective_layout(wide()), EffectiveLayout::Columns);

        assert!(!pane.move_horizontal(false, wide()), "h stops at Artists");
        assert_eq!(pane.column_focus(), ColumnFocus::Artists);
        assert!(pane.move_horizontal(true, wide()));
        assert_eq!(pane.column_focus(), ColumnFocus::Albums);
        assert!(pane.move_horizontal(true, wide()));
        assert_eq!(pane.column_focus(), ColumnFocus::Tracks);
        assert!(!pane.move_horizontal(true, wide()), "l stops at Tracks");
        assert_eq!(pane.column_focus(), ColumnFocus::Tracks);
        assert!(pane.move_horizontal(false, wide()));
        assert_eq!(pane.column_focus(), ColumnFocus::Albums);
    }

    #[test]
    fn h_and_l_drill_in_and_back_out_in_paging() {
        let mut pane = LibraryPane::new(Some(two_artist_library()));
        assert_eq!(pane.effective_layout(strip()), EffectiveLayout::Paging);

        assert!(!pane.move_horizontal(false, strip()), "h at the top does nothing");
        assert!(pane.move_horizontal(true, strip()));
        assert_eq!(pane.paging_level(), ColumnFocus::Albums);
        assert!(pane.move_horizontal(true, strip()));
        assert_eq!(pane.paging_level(), ColumnFocus::Tracks);
        assert!(!pane.move_horizontal(true, strip()), "l on a track does nothing");
        assert_eq!(pane.paging_level(), ColumnFocus::Tracks);
        assert!(pane.move_horizontal(false, strip()));
        assert_eq!(pane.paging_level(), ColumnFocus::Albums);
        assert!(pane.move_horizontal(false, strip()));
        assert_eq!(pane.paging_level(), ColumnFocus::Artists);
    }

    #[test]
    fn h_and_l_expand_collapse_and_climb_the_tree() {
        let mut pane = LibraryPane::new(Some(two_artist_library()));
        pane.set_preferred_layout(LibraryLayout::Tree);
        let s = strip();
        assert_eq!(pane.tree_rows().len(), 2, "two collapsed artists");

        assert!(pane.move_horizontal(true, s), "l expands the artist");
        assert_eq!(pane.tree_rows().len(), 3);
        assert!(!pane.move_horizontal(true, s), "l on an open artist does nothing");

        pane.step(1, s); // album row
        assert!(pane.move_horizontal(true, s), "l expands the album");
        assert_eq!(pane.tree_rows().len(), 5);

        pane.step(1, s); // first track
        assert_eq!(pane.selected_index(s), Some(2));
        assert!(!pane.move_horizontal(true, s), "l on a track does nothing");

        assert!(pane.move_horizontal(false, s), "h on a track climbs to its album");
        assert_eq!(pane.selected_index(s), Some(1));
        assert!(pane.move_horizontal(false, s), "h on an open album collapses it");
        assert_eq!(pane.tree_rows().len(), 3);
        assert_eq!(pane.selected_index(s), Some(1));
        assert!(pane.move_horizontal(false, s), "h on a closed album climbs to the artist");
        assert_eq!(pane.selected_index(s), Some(0));
        assert!(pane.move_horizontal(false, s), "h on an open artist collapses it");
        assert_eq!(pane.tree_rows().len(), 2);
        assert!(
            !pane.move_horizontal(false, s),
            "h on a closed top-level artist does nothing"
        );
    }

    #[test]
    fn h_and_l_do_nothing_in_search_results() {
        let mut pane = LibraryPane::new(None);
        pane.inject_search_hits_for_test(
            "Love",
            vec![SearchHit::Artist(ArtistSummary {
                name: "Love".into(),
                track_count: 1,
            })],
        );
        assert!(!pane.move_horizontal(true, wide()));
        assert!(!pane.move_horizontal(false, wide()));
        assert!(matches!(pane.mode(), Mode::Search(_)));
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p znicz-tui --lib library_pane::tests::h_and_l`
Expected: compile error, `no method named move_horizontal`.

- [ ] **Step 3: Implement `move_horizontal`**

In `znicz-tui/src/library_pane.rs`, after the `TreeRow` enum, add:

```rust
impl TreeRow {
    /// Nesting level: artists 0, albums 1, tracks 2.
    fn depth(&self) -> usize {
        match self {
            Self::Artist { .. } => 0,
            Self::Album { depth, .. } | Self::Track { depth, .. } => *depth,
        }
    }
}
```

In `impl LibraryPane`, after `toggle_expand_at_cursor`, add:

```rust
    /// `h` / `l`: move left or right without ever playing a track.
    ///
    /// Columns step between columns, paging drills in or backs out, the tree
    /// expands, collapses, or climbs to the parent. Search results ignore it.
    /// Returns whether anything changed.
    pub fn move_horizontal(&mut self, forward: bool, strip_inner: usize) -> bool {
        if !matches!(self.mode, Mode::Browse) {
            return false;
        }
        match self.effective_layout(strip_inner) {
            EffectiveLayout::Columns => {
                if forward {
                    self.column_focus != ColumnFocus::Tracks
                        && self.enter(strip_inner) == EnterResult::Moved
                } else if self.column_focus == ColumnFocus::Artists {
                    false
                } else {
                    self.focus_column(self.column_focus.prev());
                    true
                }
            }
            EffectiveLayout::Paging => {
                if forward {
                    self.paging_level != ColumnFocus::Tracks
                        && self.enter(strip_inner) == EnterResult::Moved
                } else {
                    self.back()
                }
            }
            EffectiveLayout::Tree => {
                if forward {
                    self.tree_expand()
                } else {
                    self.tree_collapse_or_parent()
                }
            }
        }
    }

    /// Expand the closed artist or album under the cursor.
    fn tree_expand(&mut self) -> bool {
        let rows = self.tree_rows();
        let Some(index) = self.tree_cursor.selected(rows.len()) else {
            return false;
        };
        match rows.get(index) {
            Some(TreeRow::Artist {
                expanded: false, ..
            })
            | Some(TreeRow::Album {
                expanded: false, ..
            }) => self.toggle_expand_at_cursor(),
            _ => false,
        }
    }

    /// Collapse the open node under the cursor, or move to its parent row.
    fn tree_collapse_or_parent(&mut self) -> bool {
        let rows = self.tree_rows();
        let Some(index) = self.tree_cursor.selected(rows.len()) else {
            return false;
        };
        match &rows[index] {
            TreeRow::Artist { expanded: true, .. } | TreeRow::Album { expanded: true, .. } => {
                self.toggle_expand_at_cursor()
            }
            row => {
                let depth = row.depth();
                match rows[..index].iter().rposition(|r| r.depth() < depth) {
                    Some(parent) => {
                        self.tree_cursor.set(parent, rows.len());
                        self.h_offset = 0;
                        true
                    }
                    None => false,
                }
            }
        }
    }
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p znicz-tui --lib library_pane::`
Expected: PASS (new and existing library pane tests).

- [ ] **Step 5: Commit**

```bash
git add znicz-tui/src/library_pane.rs
git commit -m "Move left and right in every library layout"
```

---

### Task 5: Footer drawn from the context

**Files:**
- Modify: `znicz-tui/src/footer_hits.rs` (rewrite)
- Modify: `znicz-tui/src/views/status.rs` (rewrite)
- Modify: `znicz-tui/src/app.rs` (add `App::context`)
- Modify: `znicz-tui/src/keys.rs` (delete `hints`, replace its test)
- Test: `znicz-tui/tests/render.rs`, `znicz-tui/tests/mouse.rs`

- [ ] **Step 1: Write the failing integration tests**

Add to `znicz-tui/tests/render.rs`:

```rust
fn footer(screen: &str) -> &str {
    screen.lines().last().expect("a footer row")
}

#[test]
fn the_footer_keeps_more_and_drops_low_priority_hints_when_narrow() {
    let mut app = App::with_library(player(), None);

    let wide = draw(&mut app, 80, 24);
    let row = footer(&wide);
    assert!(row.starts_with("Space pause · / search"), "{row}");
    assert!(row.contains("R radio"), "{row}");
    assert!(row.trim_end().ends_with("? more"), "{row}");

    let narrow = draw(&mut app, 40, 12);
    let row = footer(&narrow);
    assert!(row.starts_with("Space pause"), "{row}");
    assert!(!row.contains("R radio"), "{row}");
    assert!(row.trim_end().ends_with("? more"), "{row}");
}

#[test]
fn the_tree_footer_offers_expand_instead_of_pause() {
    let mut app = App::with_library(player(), None);
    app.library
        .set_preferred_layout(znicz_tui::LibraryLayout::Tree);
    let screen = draw(&mut app, 100, 24);
    let row = footer(&screen);
    assert!(row.contains("o expand"), "{row}");
    assert!(!row.contains("Space pause"), "{row}");
}

#[test]
fn the_queue_footer_lists_queue_keys() {
    let mut app = App::with_library(player(), None);
    app.queue_open = true;
    app.focus = Focus::Queue;
    let screen = draw(&mut app, 100, 24);
    let row = footer(&screen);
    assert!(row.contains("d remove"), "{row}");
    assert!(row.contains("] close"), "{row}");
}

#[test]
fn a_typing_prompt_footer_says_save_and_cancel() {
    let mut app = App::with_library(player(), None);
    app.modal = Modal::Playlists;
    app.playlist_prompt = Some(PlaylistPrompt::Save(LineEdit::from_text("x")));
    let screen = draw(&mut app, 100, 24);
    let row = footer(&screen);
    assert!(row.contains("Enter save · Esc cancel"), "{row}");
}
```

Add to `znicz-tui/tests/mouse.rs`:

```rust
#[test]
fn a_drawn_footer_add_hint_covers_its_label_and_sends_a() {
    let mut app = new_app();
    let state = app.player.state();
    let mut terminal = Terminal::new(TestBackend::new(100, 24)).expect("backend");
    terminal
        .draw(|frame| views::render(frame, &mut app, &state))
        .expect("draw");
    let add = app
        .hits
        .footer_hints
        .iter()
        .find(|h| h.key.code == KeyCode::Char('a'))
        .expect("a add footer hit");
    assert_eq!(add.rect.width, 5, "the hit covers `a add`");
    assert_eq!(add.rect.y, 23);
}
```

- [ ] **Step 2: Run them to verify they fail**

Run: `cargo test -p znicz-tui --test render footer` and `cargo test -p znicz-tui --test mouse a_drawn_footer_add`
Expected: FAIL; the footer still reads `/ search · a add · ] queue · i inspect · P · R …` and the prompt footer reads `type · ← → · Enter · Esc cancel`.

- [ ] **Step 3: Rewrite `footer_hits.rs`**

Replace the whole of `znicz-tui/src/footer_hits.rs` with:

```rust
//! Where each footer hint lands, so a click can run that hint's key.

use ratatui::layout::Rect;

use crate::hit::FooterHit;
use crate::keys::Hint;

/// Drawn between neighbouring hints.
pub const SEPARATOR: &str = " · ";
const SEPARATOR_WIDTH: u16 = 3;

/// A hint and the cells it occupies on the footer row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlacedHint {
    pub rect: Rect,
    pub hint: Hint,
}

fn label_width(hint: &Hint) -> u16 {
    hint.label.chars().count() as u16
}

/// Lay hints out left to right, keeping `more` right-aligned.
///
/// The space for `more` (plus one separator) is reserved first, so it is never
/// pushed off by a long hint list. Hints that do not fit whole are dropped
/// rather than cut.
pub fn layout_footer(area: Rect, hints: &[Hint], more: Option<Hint>) -> Vec<PlacedHint> {
    if area.width == 0 || area.height == 0 {
        return Vec::new();
    }
    let more = more.filter(|m| label_width(m) <= area.width);
    let budget = match &more {
        Some(m) => area
            .width
            .saturating_sub(label_width(m))
            .saturating_sub(SEPARATOR_WIDTH),
        None => area.width,
    };

    let mut placed = Vec::new();
    let mut x: u16 = 0;
    for hint in hints {
        let gap = if placed.is_empty() { 0 } else { SEPARATOR_WIDTH };
        let width = label_width(hint);
        if u32::from(x) + u32::from(gap) + u32::from(width) > u32::from(budget) {
            break;
        }
        x += gap;
        placed.push(PlacedHint {
            rect: Rect::new(area.x + x, area.y, width, 1),
            hint: *hint,
        });
        x += width;
    }

    if let Some(more) = more {
        let width = label_width(&more);
        placed.push(PlacedHint {
            rect: Rect::new(area.x + area.width - width, area.y, width, 1),
            hint: more,
        });
    }
    placed
}

/// Click targets for placed hints.
pub fn footer_hits(placed: &[PlacedHint]) -> Vec<FooterHit> {
    placed
        .iter()
        .map(|p| FooterHit {
            rect: p.rect,
            key: p.hint.key,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn h(label: &'static str, c: char) -> Hint {
        Hint {
            label,
            key: KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE),
        }
    }

    fn labels(placed: &[PlacedHint]) -> Vec<&'static str> {
        placed.iter().map(|p| p.hint.label).collect()
    }

    #[test]
    fn hints_run_left_to_right_and_more_sits_on_the_right() {
        let area = Rect::new(0, 23, 40, 1);
        let placed = layout_footer(
            area,
            &[h("/ search", '/'), h("a add", 'a')],
            Some(h("? more", '?')),
        );
        assert_eq!(labels(&placed), ["/ search", "a add", "? more"]);
        assert_eq!(placed[0].rect, Rect::new(0, 23, 8, 1));
        assert_eq!(placed[1].rect, Rect::new(11, 23, 5, 1));
        assert_eq!(placed[2].rect, Rect::new(34, 23, 6, 1));
    }

    #[test]
    fn hints_that_would_crowd_more_are_dropped() {
        // Budget is 20 - 6 ("? more") - 3 (separator) = 11: only "/ search" fits.
        let placed = layout_footer(
            Rect::new(0, 0, 20, 1),
            &[h("/ search", '/'), h("a add", 'a')],
            Some(h("? more", '?')),
        );
        assert_eq!(labels(&placed), ["/ search", "? more"]);
    }

    #[test]
    fn without_more_hints_use_the_full_width() {
        let placed = layout_footer(
            Rect::new(0, 0, 16, 1),
            &[h("/ search", '/'), h("a add", 'a')],
            None,
        );
        assert_eq!(labels(&placed), ["/ search", "a add"]);
    }

    #[test]
    fn a_footer_too_narrow_for_any_label_places_nothing() {
        let placed = layout_footer(
            Rect::new(0, 0, 4, 1),
            &[h("/ search", '/')],
            Some(h("? more", '?')),
        );
        assert!(placed.is_empty());
        assert!(layout_footer(Rect::new(0, 0, 0, 1), &[h("a add", 'a')], None).is_empty());
    }

    #[test]
    fn hits_carry_each_hints_key() {
        let placed = layout_footer(
            Rect::new(0, 23, 40, 1),
            &[h("a add", 'a')],
            Some(h("? more", '?')),
        );
        let keys: Vec<KeyCode> = footer_hits(&placed).iter().map(|h| h.key.code).collect();
        assert_eq!(keys, [KeyCode::Char('a'), KeyCode::Char('?')]);
    }
}
```

- [ ] **Step 4: Add `App::context`**

In `znicz-tui/src/app.rs`, add to the imports:

```rust
use crate::keys::Context;
```

In `impl App`, directly above `fn library_strip(&self) -> usize`, add:

```rust
    /// What the footer and help describe right now.
    pub fn context(&self) -> Context {
        if self.library.is_typing() {
            return Context::Search;
        }
        match &self.radio_prompt {
            Some(RadioPrompt::Form { .. }) => return Context::StationPrompt,
            Some(RadioPrompt::Copy(_)) => return Context::Prompt,
            None => {}
        }
        if self.playlist_prompt.is_some() {
            return Context::Prompt;
        }
        match self.modal {
            Modal::Help => Context::Help,
            Modal::Devices => Context::Devices,
            Modal::Inspector => Context::Inspector,
            Modal::Playlists => Context::Playlists,
            Modal::Radio => Context::Radio,
            Modal::None => match self.focus {
                Focus::Queue => Context::Queue,
                Focus::Library
                    if self.library.effective_layout(self.library_strip())
                        == EffectiveLayout::Tree =>
                {
                    Context::LibraryTree
                }
                Focus::Library => Context::Library,
            },
        }
    }
```

- [ ] **Step 5: Rewrite `views/status.rs`**

Replace the whole of `znicz-tui/src/views/status.rs` with:

```rust
//! The hint line at the bottom of the frame.

use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::app::App;
use crate::footer_hits::{self, PlacedHint, SEPARATOR};
use crate::keys;
use crate::theme;

/// Key hints for the current context. Toasts never replace this line.
pub fn render_footer(frame: &mut Frame, area: Rect, app: &mut App) {
    let context = app.context();
    let placed = footer_hits::layout_footer(
        area,
        keys::context_hints(context),
        keys::more_hint(context),
    );
    app.hits.footer_hints = footer_hits::footer_hits(&placed);
    frame.render_widget(Paragraph::new(footer_line(area, &placed)), area);
}

/// Hints at their placed columns: ` · ` between neighbours, spaces before `? more`.
fn footer_line(area: Rect, placed: &[PlacedHint]) -> Line<'static> {
    let separator_width = SEPARATOR.chars().count() as u16;
    let mut spans = Vec::new();
    let mut x = area.x;
    for item in placed {
        let gap = item.rect.x.saturating_sub(x);
        if x > area.x && gap == separator_width {
            spans.push(Span::styled(SEPARATOR, theme::dim()));
        } else if gap > 0 {
            spans.push(Span::raw(" ".repeat(usize::from(gap))));
        }
        let (key, action) = item
            .hint
            .label
            .split_once(' ')
            .unwrap_or((item.hint.label, ""));
        spans.push(Span::styled(key, theme::key()));
        if !action.is_empty() {
            spans.push(Span::styled(format!(" {action}"), theme::dim()));
        }
        x = item.rect.x + item.rect.width;
    }
    Line::from(spans)
}
```

- [ ] **Step 6: Delete `keys::hints` and replace its test**

In `znicz-tui/src/keys.rs`, delete the whole `hints` function (from `/// Short hints for the footer, per pane.` through its closing `}`).

In the tests module, delete the whole `each_pane_has_its_own_hint_line` test and add:

```rust
    #[test]
    fn the_global_table_keeps_its_letters() {
        let has = |keys: &str, action: &str| {
            GLOBAL
                .iter()
                .any(|b| b.keys == keys && b.action.contains(action))
        };
        assert!(has("P", "playlists"));
        assert!(has("p", "previous"));
        assert!(has("R", "radio"));
        assert!(has("e", "repeat"));
        assert!(has("r", "reload"));
        assert!(has("i", "inspector"));
        assert!(GLOBAL.iter().any(|b| b.keys.contains("Alt-→")));
        assert!(GLOBAL.iter().all(|b| !b.keys.contains('N')));
        assert!(GLOBAL
            .iter()
            .all(|b| !b.keys.contains('<') && !b.keys.contains('>')));
        assert!(LIBRARY.iter().all(|b| b.keys != "R"));
        assert!(DEVICES
            .iter()
            .any(|b| b.keys == "r" && b.action.contains("rescan")));
    }
```

Update the module doc comment at the top of `keys.rs` to:

```rust
//! The keymap, written down once.
//!
//! Help is generated from the binding tables, and a test checks that every
//! footer hint names a key those tables document, so the two cannot drift.
```

- [ ] **Step 7: Run the tests to verify they pass**

Run: `cargo test -p znicz-tui`
Expected: PASS, including `a_drawn_footer_help_hint_is_clickable` (now the `? more` hint) and `hints_stay_when_a_toast_is_showing`.

- [ ] **Step 8: Commit**

```bash
git add znicz-tui/src/footer_hits.rs znicz-tui/src/views/status.rs znicz-tui/src/app.rs znicz-tui/src/keys.rs znicz-tui/tests/render.rs znicz-tui/tests/mouse.rs
git commit -m "Draw the footer from the current context"
```

---

### Task 6: Arrows seek, `h` / `l` move

**Files:**
- Modify: `znicz-tui/src/app.rs`
- Modify: `znicz-tui/src/keys.rs` (`GLOBAL`, `NAVIGATION`)
- Test: `znicz-tui/tests/keys.rs`

- [ ] **Step 1: Write the failing tests**

In `znicz-tui/src/app.rs`, add to the existing `#[cfg(test)] mod tests`:

```rust
    #[test]
    fn arrows_seek_five_seconds_and_shift_arrows_thirty() {
        let plain = |code| KeyEvent::new(code, KeyModifiers::NONE);
        let shifted = |code| KeyEvent::new(code, KeyModifiers::SHIFT);
        assert_eq!(seek_seconds(plain(KeyCode::Right)), Some(5));
        assert_eq!(seek_seconds(plain(KeyCode::Left)), Some(-5));
        assert_eq!(seek_seconds(shifted(KeyCode::Right)), Some(30));
        assert_eq!(seek_seconds(shifted(KeyCode::Left)), Some(-30));
        assert_eq!(seek_seconds(plain(KeyCode::Char('l'))), None);
    }
```

In `znicz-tui/src/keys.rs` tests, add:

```rust
    #[test]
    fn seek_lives_on_the_arrows_and_h_l_move() {
        assert!(GLOBAL
            .iter()
            .any(|b| b.keys == "→ / ←" && b.action.contains("5s")));
        assert!(GLOBAL
            .iter()
            .any(|b| b.keys == "Shift-→ / Shift-←" && b.action.contains("30s")));
        assert!(GLOBAL.iter().all(|b| !b
            .keys
            .split(" / ")
            .any(|k| matches!(k, "h" | "l" | "H" | "L"))));
        assert!(NAVIGATION.iter().any(|b| b.keys == "h / l"));
    }
```

In `znicz-tui/tests/keys.rs`, add to the imports:

```rust
use znicz_tui::library_pane::ColumnFocus;
```

and add the tests:

```rust
/// Miles (one album, two tracks) and Portishead (one track), no ffmpeg needed.
fn app_with_library_rows() -> App {
    let mut library = znicz_library::Library::open_in_memory().expect("open library");
    for (path, title, artist, album) in [
        ("/music/a1.flac", "So What", "Miles", "Kind of Blue"),
        ("/music/a2.flac", "Freddie Freeloader", "Miles", "Kind of Blue"),
        ("/music/b1.flac", "Roads", "Portishead", "Dummy"),
    ] {
        library
            .upsert_track_for_test(
                std::path::Path::new(path),
                title,
                Some(artist.into()),
                Some(album.into()),
                None,
            )
            .expect("insert track");
    }
    App::with_library(player(), Some(library))
}

#[test]
fn h_and_l_move_between_library_columns_and_never_play() {
    let mut app = app_with_library_rows();
    press_char(&mut app, 'l');
    assert_eq!(app.library.column_focus(), ColumnFocus::Albums);
    press_char(&mut app, 'l');
    press_char(&mut app, 'l');
    assert_eq!(app.library.column_focus(), ColumnFocus::Tracks);
    assert!(app.state().queue.is_empty(), "l on a track must not play it");
    press_char(&mut app, 'h');
    assert_eq!(app.library.column_focus(), ColumnFocus::Albums);
}

#[test]
fn h_and_l_do_nothing_while_the_queue_has_focus() {
    let mut app = app_with_library_rows();
    open_queue(&mut app);
    press_char(&mut app, 'l');
    assert_eq!(app.library.column_focus(), ColumnFocus::Artists);
    assert_eq!(app.focus, Focus::Queue);
}

#[test]
fn capital_h_and_l_are_unbound() {
    let mut app = app_with_library_rows();
    press_char(&mut app, 'L');
    press_char(&mut app, 'H');
    assert_eq!(app.library.column_focus(), ColumnFocus::Artists);
    assert!(app.toasts.is_empty());
}
```

In the existing `seeking_with_nothing_loaded_is_ignored` test, replace the `for key in [...]` loop with:

```rust
    press(&mut app, KeyCode::Right);
    press(&mut app, KeyCode::Left);
    app.on_key(KeyEvent::new(KeyCode::Right, KeyModifiers::SHIFT));
```

- [ ] **Step 2: Run them to verify they fail**

Run: `cargo test -p znicz-tui --lib seek` and `cargo test -p znicz-tui --test keys h_and_l`
Expected: compile error for `seek_seconds`; once it compiles, the `h_and_l` tests fail because `l` still seeks.

- [ ] **Step 3: Implement**

In `znicz-tui/src/app.rs`, below `const HALF_PAGE: isize = 10;`, add:

```rust
/// Seconds an arrow key seeks: 5 plain, 30 with Shift. `None` for other keys.
fn seek_seconds(key: KeyEvent) -> Option<i64> {
    let step = if key.modifiers.contains(KeyModifiers::SHIFT) {
        SEEK_LARGE
    } else {
        SEEK_SMALL
    };
    match key.code {
        KeyCode::Right => Some(step),
        KeyCode::Left => Some(-step),
        _ => None,
    }
}
```

In `on_global_key`, replace these four arms:

```rust
            KeyCode::Right | KeyCode::Char('l') => self.seek_relative(SEEK_SMALL),
            KeyCode::Left | KeyCode::Char('h') => self.seek_relative(-SEEK_SMALL),
            KeyCode::Char('L') => self.seek_relative(SEEK_LARGE),
            KeyCode::Char('H') => self.seek_relative(-SEEK_LARGE),
```

with:

```rust
            KeyCode::Right | KeyCode::Left => {
                if let Some(seconds) = seek_seconds(key) {
                    self.seek_relative(seconds);
                }
            }
            KeyCode::Char('l') => self.library_horizontal(true),
            KeyCode::Char('h') => self.library_horizontal(false),
```

(The `Alt` pan arms stay above these, so `Alt-←` / `Alt-→` still pan.)

Below `fn library_toggle_tree_expand`, add:

```rust
    /// `h` / `l`: move left or right in the library. Ignored anywhere else.
    fn library_horizontal(&mut self, forward: bool) {
        if self.focus != Focus::Library || self.modal != Modal::None {
            return;
        }
        let strip = self.library_strip();
        self.library.move_horizontal(forward, strip);
    }
```

Update the comment on the seek constants to `/// Seek step for arrows, and for Shift + arrows.`

In `znicz-tui/src/keys.rs`, in `GLOBAL`, replace:

```rust
    b("→ / l", "seek forward 5s"),
    b("← / h", "seek back 5s"),
    b("L / H", "seek 30s"),
```

with:

```rust
    b("→ / ←", "seek 5s"),
    b("Shift-→ / Shift-←", "seek 30s"),
```

and in `NAVIGATION`, after `b("k / ↑", "up"),` add:

```rust
    b("h / l", "left / right: column, tree level, back / into"),
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p znicz-tui`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add znicz-tui/src/app.rs znicz-tui/src/keys.rs znicz-tui/tests/keys.rs
git commit -m "Seek on arrows and move with h and l"
```

---

### Task 7: Help state and help keys

**Files:**
- Create: `znicz-tui/src/help_state.rs`
- Modify: `znicz-tui/src/lib.rs`, `znicz-tui/src/app.rs`
- Test: `znicz-tui/tests/keys.rs`, `znicz-tui/tests/mouse.rs`

- [ ] **Step 1: Create `help_state.rs` with its unit tests**

Create `znicz-tui/src/help_state.rs`:

```rust
//! What the help overlay remembers while it is open.

use crate::keys::Context;
use crate::line_edit::LineEdit;

/// Lines moved by Ctrl-d / Ctrl-u and the page keys inside help.
pub const HELP_PAGE: isize = 10;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HelpState {
    /// Where help was opened from; its keys are listed first.
    pub context: Context,
    /// First visible line. Clamped to the content when drawn.
    pub scroll: usize,
    pub query: LineEdit,
    /// Typing goes into `query` instead of scrolling.
    pub editing: bool,
}

impl HelpState {
    pub fn open(context: Context) -> Self {
        Self {
            context,
            scroll: 0,
            query: LineEdit::new(),
            editing: false,
        }
    }

    pub fn scroll_by(&mut self, delta: isize) {
        self.scroll = if delta < 0 {
            self.scroll.saturating_sub(delta.unsigned_abs())
        } else {
            self.scroll.saturating_add(delta.unsigned_abs())
        };
    }

    /// Keep the last page full: never scroll past `content - visible`.
    pub fn clamp_scroll(&mut self, content: usize, visible: usize) {
        self.scroll = self.scroll.min(content.saturating_sub(visible));
    }

    /// The filter row shows while typing, and while a kept filter applies.
    pub fn shows_prompt(&self) -> bool {
        self.editing || !self.query.is_empty()
    }
}

impl Default for HelpState {
    fn default() -> Self {
        Self::open(Context::Library)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scrolling_up_stops_at_the_top() {
        let mut help = HelpState::default();
        help.scroll_by(-3);
        assert_eq!(help.scroll, 0);
        help.scroll_by(4);
        help.scroll_by(-1);
        assert_eq!(help.scroll, 3);
    }

    #[test]
    fn clamping_keeps_the_last_page_full() {
        let mut help = HelpState::default();
        help.scroll = usize::MAX;
        help.clamp_scroll(50, 20);
        assert_eq!(help.scroll, 30);
        help.clamp_scroll(10, 20);
        assert_eq!(help.scroll, 0);
    }

    #[test]
    fn the_prompt_shows_while_editing_or_filtered() {
        let mut help = HelpState::open(Context::Queue);
        assert!(!help.shows_prompt());
        help.editing = true;
        assert!(help.shows_prompt());
        help.editing = false;
        help.query = LineEdit::from_text("vol");
        assert!(help.shows_prompt());
    }
}
```

In `znicz-tui/src/lib.rs`, add `pub mod help_state;` after `pub mod format;`.

Run: `cargo test -p znicz-tui --lib help_state`
Expected: PASS.

- [ ] **Step 2: Write the failing key tests**

In `znicz-tui/tests/keys.rs`, add to the imports:

```rust
use znicz_core::PlaybackStatus;
use znicz_tui::keys::Context;
```

Delete the two old tests `question_mark_still_opens_help_and_the_next_key_only_closes_it` and `help_opens_and_any_key_closes_it`, and add:

```rust
#[test]
fn help_opens_on_the_current_context() {
    let mut app = new_app();
    open_queue(&mut app);
    press_char(&mut app, '?');
    assert_eq!(app.modal, Modal::Help);
    assert_eq!(app.help.context, Context::Queue);
}

#[test]
fn q_closes_help_without_quitting() {
    let mut app = new_app();
    press_char(&mut app, '?');
    press_char(&mut app, 'q');
    assert_eq!(app.modal, Modal::None);
    assert!(!app.should_quit, "q inside help must not quit");
}

#[test]
fn question_mark_and_esc_close_help() {
    let mut app = new_app();
    press_char(&mut app, '?');
    press_char(&mut app, '?');
    assert_eq!(app.modal, Modal::None);
    press_char(&mut app, '?');
    press(&mut app, KeyCode::Esc);
    assert_eq!(app.modal, Modal::None);
}

#[test]
fn help_ignores_playback_and_unbound_keys() {
    let mut app = new_app();
    queue(&mut app, 2);
    press_char(&mut app, '?');
    for c in [' ', 'n', 'x', 'h'] {
        press_char(&mut app, c);
    }
    assert_eq!(app.modal, Modal::Help);
    assert_eq!(app.state().status, PlaybackStatus::Stopped);
}

#[test]
fn slash_filters_help_and_esc_clears_before_closing() {
    let mut app = new_app();
    press_char(&mut app, '?');
    press_char(&mut app, '/');
    assert!(app.help.editing);
    press_typed(&mut app, "vol q");
    assert_eq!(app.help.query.as_str(), "vol q", "q is text while filtering");
    assert_eq!(app.modal, Modal::Help);

    press(&mut app, KeyCode::Esc);
    assert_eq!(app.modal, Modal::Help, "the first Esc only clears the filter");
    assert!(app.help.query.is_empty());
    assert!(!app.help.editing);

    press(&mut app, KeyCode::Esc);
    assert_eq!(app.modal, Modal::None);
}

#[test]
fn enter_keeps_the_help_filter_and_returns_to_scrolling() {
    let mut app = new_app();
    press_char(&mut app, '?');
    press_char(&mut app, '/');
    press_typed(&mut app, "seek");
    press(&mut app, KeyCode::Enter);
    assert!(!app.help.editing);
    assert_eq!(app.help.query.as_str(), "seek");

    press_char(&mut app, 'j');
    assert_eq!(app.help.scroll, 1, "j scrolls once the filter is kept");
    assert_eq!(app.help.query.as_str(), "seek");
}

#[test]
fn j_k_g_scroll_help() {
    let mut app = new_app();
    press_char(&mut app, '?');
    press_char(&mut app, 'j');
    press_char(&mut app, 'j');
    press_char(&mut app, 'k');
    assert_eq!(app.help.scroll, 1);
    press_ctrl(&mut app, 'd');
    assert_eq!(app.help.scroll, 11);
    press_char(&mut app, 'g');
    assert_eq!(app.help.scroll, 0);
}

#[test]
fn ctrl_c_still_quits_from_help() {
    let mut app = new_app();
    press_char(&mut app, '?');
    press_ctrl(&mut app, 'c');
    assert!(app.should_quit);
}
```

In `znicz-tui/tests/mouse.rs`, add:

```rust
#[test]
fn wheel_scrolls_help() {
    let mut app = new_app();
    app.modal = Modal::Help;
    app.on_mouse(wheel(false));
    app.on_mouse(wheel(false));
    app.on_mouse(wheel(true));
    assert_eq!(app.help.scroll, 1);
}
```

- [ ] **Step 3: Run them to verify they fail**

Run: `cargo test -p znicz-tui --test keys help` and `cargo test -p znicz-tui --test mouse wheel_scrolls_help`
Expected: compile error, `no field help on type App`.

- [ ] **Step 4: Wire help state into `App`**

In `znicz-tui/src/app.rs`:

Add to the imports:

```rust
use crate::help_state::{HelpState, HELP_PAGE};
```

Add a field to `pub struct App`, after `pub modal: Modal,`:

```rust
    pub help: HelpState,
```

and initialise it in `with_engine`, after `modal: Modal::None,`:

```rust
            help: HelpState::default(),
```

At the top of `on_key`, replace:

```rust
        if self.modal == Modal::Help {
            self.modal = Modal::None;
            return;
        }
```

with:

```rust
        if self.modal == Modal::Help {
            self.on_help_key(key);
            return;
        }
```

In `on_global_key`, replace `KeyCode::Char('?') => self.modal = Modal::Help,` with:

```rust
            KeyCode::Char('?') => self.open_help(),
```

Below `fn on_esc`, add:

```rust
    /// Open help on whatever the user is looking at now.
    fn open_help(&mut self) {
        self.help = HelpState::open(self.context());
        self.modal = Modal::Help;
    }

    /// Keys while help is open: scroll, filter, close. Everything else is inert.
    fn on_help_key(&mut self, key: KeyEvent) {
        if self.help.editing {
            match key.code {
                KeyCode::Esc => {
                    self.help.query = LineEdit::new();
                    self.help.editing = false;
                    self.help.scroll = 0;
                }
                KeyCode::Enter => self.help.editing = false,
                _ => {
                    if self.help.query.on_key(key) {
                        self.help.scroll = 0;
                    }
                }
            }
            return;
        }

        if key.modifiers.contains(KeyModifiers::CONTROL) {
            match key.code {
                KeyCode::Char('c') => self.should_quit = true,
                KeyCode::Char('d') => self.help.scroll_by(HELP_PAGE),
                KeyCode::Char('u') => self.help.scroll_by(-HELP_PAGE),
                _ => {}
            }
            return;
        }

        match key.code {
            KeyCode::Esc | KeyCode::Char('?') | KeyCode::Char('q') => self.modal = Modal::None,
            KeyCode::Char('/') => self.help.editing = true,
            KeyCode::Char('j') | KeyCode::Down => self.help.scroll_by(1),
            KeyCode::Char('k') | KeyCode::Up => self.help.scroll_by(-1),
            KeyCode::PageDown => self.help.scroll_by(HELP_PAGE),
            KeyCode::PageUp => self.help.scroll_by(-HELP_PAGE),
            KeyCode::Char('g') | KeyCode::Home => self.help.scroll = 0,
            // Clamped to the last page when help is drawn.
            KeyCode::Char('G') | KeyCode::End => self.help.scroll = usize::MAX,
            _ => {}
        }
    }
```

In `on_wheel`, replace the arm `Modal::Help | Modal::Inspector => {}` with:

```rust
            Modal::Help => self.help.scroll_by(delta),
            Modal::Inspector => {}
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test -p znicz-tui`
Expected: PASS. `wheel_is_ignored_while_help_is_open` still passes (the library does not move).

- [ ] **Step 6: Commit**

```bash
git add znicz-tui/src/help_state.rs znicz-tui/src/lib.rs znicz-tui/src/app.rs znicz-tui/tests/keys.rs znicz-tui/tests/mouse.rs
git commit -m "Scroll and filter help instead of closing on any key"
```

---

### Task 8: Help overlay view

**Files:**
- Modify: `znicz-tui/src/views/help.rs` (rewrite)
- Test: `znicz-tui/tests/render.rs`

- [ ] **Step 1: Write the failing render tests**

In `znicz-tui/tests/render.rs`, add to the imports:

```rust
use znicz_tui::help_state::HelpState;
use znicz_tui::keys::Context;
```

Replace the two existing tests `the_help_overlay_draws_at_every_size` and `the_help_overlay_shows_the_app_version_and_close_hint` with:

```rust
#[test]
fn the_help_overlay_draws_at_every_size() {
    let mut app = App::with_library(player(), None);
    app.modal = Modal::Help;

    for &(width, height) in SIZES {
        let screen = draw(&mut app, width, height);
        assert_eq!(screen.lines().count(), height as usize);
    }

    let screen = draw(&mut app, 100, 40);
    assert!(screen.contains("Keys · Library"), "{screen}");
    assert!(screen.contains("search the library"), "{screen}");
    assert!(screen.contains("play / pause"), "{screen}");
    assert!(screen.contains("seek 30s"), "{screen}");
}

#[test]
fn the_help_overlay_shows_the_app_version_and_close_hint() {
    let mut app = App::with_library(player(), None);
    app.modal = Modal::Help;
    let screen = draw(&mut app, 120, 40);
    assert!(
        screen.contains(&format!("znicz {}", env!("CARGO_PKG_VERSION"))),
        "version missing from help:\n{screen}"
    );
    assert!(screen.contains("/ filter · Esc close"), "close hint missing");
    assert!(!screen.contains("any key closes"), "{screen}");
}

#[test]
fn help_opened_from_the_queue_lists_queue_keys_first() {
    let mut app = App::with_library(player(), None);
    app.help = HelpState::open(Context::Queue);
    app.modal = Modal::Help;
    let screen = draw(&mut app, 100, 40);
    assert!(screen.contains("Keys · Queue"), "{screen}");
    let queue_row = screen.find("play the selected track").expect("queue keys");
    let playback_row = screen.find("play / pause").expect("playback keys");
    assert!(queue_row < playback_row, "queue keys come first:\n{screen}");
}

#[test]
fn a_help_filter_shows_only_matching_rows() {
    let mut app = App::with_library(player(), None);
    app.modal = Modal::Help;
    app.help.query = LineEdit::from_text("station");
    let screen = draw(&mut app, 100, 40);
    assert!(screen.contains("new station"), "{screen}");
    assert!(screen.contains("/ station"), "the kept filter shows:\n{screen}");
    assert!(!screen.contains("play / pause"), "{screen}");
}

#[test]
fn a_help_filter_with_no_match_says_so() {
    let mut app = App::with_library(player(), None);
    app.modal = Modal::Help;
    app.help.query = LineEdit::from_text("zzzz");
    let screen = draw(&mut app, 100, 40);
    assert!(screen.contains("no keys match"), "{screen}");
}

#[test]
fn the_footer_inside_help_offers_filter_and_close_only() {
    let mut app = App::with_library(player(), None);
    app.modal = Modal::Help;
    let screen = draw(&mut app, 100, 24);
    let row = screen.lines().last().expect("footer row");
    assert!(row.contains("/ filter · Esc close"), "{row}");
    assert!(!row.contains("? more"), "{row}");
}
```

- [ ] **Step 2: Run them to verify they fail**

Run: `cargo test -p znicz-tui --test render help`
Expected: FAIL; the title is ` Keys `, the close hint is ` any key closes `, the filter is not applied, and the old popup covers the footer row.

- [ ] **Step 3: Rewrite `views/help.rs`**

Replace the whole of `znicz-tui/src/views/help.rs` with:

```rust
//! The help overlay: this context's keys first, then the rest, with a filter.

use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;

use crate::app::App;
use crate::help_state::HelpState;
use crate::keys::{self, Section};
use crate::theme;

const VERSION: &str = env!("CARGO_PKG_VERSION");
const VERSION_PREFIX: &str = " znicz ";
const CLOSE_HINT: &str = " / filter · Esc close ";
const FILTER_PREFIX: &str = "/ ";
/// One column of keys and actions reads best kept narrow.
const MAX_WIDTH: u16 = 64;
/// Fits the longest key label (`Shift-→ / Shift-←`) plus a gap.
const KEY_COLUMN: usize = 19;

pub fn render(frame: &mut Frame, area: Rect, app: &mut App) {
    let popup = popup_rect(area);
    app.hits.overlay = Some(popup);
    app.hits.close = crate::views::close_button_rect(popup);

    let mut block = Block::default()
        .borders(Borders::ALL)
        .border_style(theme::border_active())
        .title(Span::styled(title(&app.help), theme::title()))
        .title(crate::views::close_title())
        .title_bottom(Line::from(Span::styled(CLOSE_HINT, theme::dim())).right_aligned());
    if let Some(version) = version_title(popup.width) {
        block = block.title_bottom(version);
    }
    let inner = block.inner(popup);

    // Clear first, or the pane underneath shows through the overlay.
    frame.render_widget(Clear, popup);
    frame.render_widget(block, popup);

    let sections = keys::filter_sections(keys::help_sections(app.help.context), &app.help.query);
    let lines = help_lines(&sections);

    let (list_area, prompt_area) = split_prompt(inner, app.help.shows_prompt());
    app.help
        .clamp_scroll(lines.len(), usize::from(list_area.height));
    let scroll = u16::try_from(app.help.scroll).unwrap_or(u16::MAX);
    frame.render_widget(Paragraph::new(lines).scroll((scroll, 0)), list_area);

    if let Some(prompt_area) = prompt_area {
        let line = if app.help.editing {
            crate::views::prompt_line(FILTER_PREFIX, &app.help.query)
        } else {
            crate::views::prompt_line_idle(FILTER_PREFIX, app.help.query.as_str())
        };
        frame.render_widget(Paragraph::new(line), prompt_area);
    }
}

fn title(help: &HelpState) -> String {
    match keys::context_title(help.context) {
        Some(name) => format!(" Keys · {name} "),
        None => " Keys ".to_string(),
    }
}

/// Section titles and binding rows. Sections for other contexts draw dim.
fn help_lines(sections: &[Section]) -> Vec<Line<'static>> {
    if sections.is_empty() {
        return vec![Line::from(Span::styled(" no keys match", theme::dim()))];
    }
    let mut lines = Vec::new();
    for section in sections {
        let (title_style, key_style, action_style) = if section.dim {
            (theme::dim(), theme::dim(), theme::dim())
        } else {
            (theme::strong(), theme::key(), theme::text())
        };
        lines.push(Line::from(Span::styled(section.title, title_style)));
        for binding in &section.bindings {
            lines.push(Line::from(vec![
                Span::raw(" "),
                Span::styled(
                    format!("{:<width$}", binding.keys, width = KEY_COLUMN),
                    key_style,
                ),
                Span::styled(binding.action, action_style),
            ]));
        }
    }
    lines
}

/// The list area, and the filter row under it when the filter is showing.
fn split_prompt(inner: Rect, show_prompt: bool) -> (Rect, Option<Rect>) {
    if !show_prompt || inner.height < 2 {
        return (inner, None);
    }
    let list = Rect {
        height: inner.height - 1,
        ..inner
    };
    let prompt = Rect {
        y: inner.y + inner.height - 1,
        height: 1,
        ..inner
    };
    (list, Some(prompt))
}

/// Centred, at most `MAX_WIDTH` wide, inset one row from the top and bottom
/// so the footer stays visible.
fn popup_rect(area: Rect) -> Rect {
    let width = area
        .width
        .saturating_sub(4)
        .max(area.width.min(20))
        .min(MAX_WIDTH);
    let height = area.height.saturating_sub(2).max(area.height.min(5));
    Rect {
        x: area.x + (area.width - width) / 2,
        y: area.y + (area.height - height) / 2,
        width,
        height,
    }
}

/// The bottom-left version title, or `None` when both titles cannot fit.
///
/// The close hint is functional, so it keeps the row when a tiny popup has no
/// room for the version.
fn version_title(popup_width: u16) -> Option<Line<'static>> {
    let label = format!("{VERSION_PREFIX}{VERSION} ");
    let budget = popup_width.saturating_sub(2) as usize;
    if label.chars().count() + CLOSE_HINT.chars().count() > budget {
        return None;
    }
    Some(Line::from(Span::styled(label, theme::dim())).left_aligned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keys::Context;

    #[test]
    fn the_popup_caps_its_width_and_keeps_the_footer_row_free() {
        let screen = Rect::new(0, 0, 200, 60);
        let popup = popup_rect(screen);
        assert_eq!(popup.width, MAX_WIDTH);
        assert_eq!(popup.height, 58);
        assert!(popup.right() <= screen.right());
        assert!(popup.bottom() < screen.bottom(), "footer row stays visible");
    }

    #[test]
    fn a_tiny_screen_does_not_produce_an_oversized_box() {
        for (w, h) in [(4, 3), (10, 3), (20, 5), (30, 8)] {
            let popup = popup_rect(Rect::new(0, 0, w, h));
            assert!(popup.width <= w && popup.height <= h, "{w}x{h}: {popup:?}");
        }
    }

    #[test]
    fn the_title_names_the_context() {
        assert_eq!(title(&HelpState::open(Context::Queue)), " Keys · Queue ");
        assert_eq!(title(&HelpState::open(Context::Prompt)), " Keys ");
    }

    #[test]
    fn every_section_and_binding_gets_a_row() {
        let sections = keys::help_sections(Context::Queue);
        let rows = sections.len() + sections.iter().map(|s| s.bindings.len()).sum::<usize>();
        assert_eq!(help_lines(&sections).len(), rows);
    }

    #[test]
    fn no_sections_draw_one_explaining_row() {
        let lines = help_lines(&[]);
        assert_eq!(lines.len(), 1);
    }

    #[test]
    fn the_filter_row_takes_the_last_inner_row() {
        let inner = Rect::new(1, 1, 40, 10);
        let (list, prompt) = split_prompt(inner, true);
        assert_eq!(list.height, 9);
        assert_eq!(prompt, Some(Rect::new(1, 10, 40, 1)));
        assert_eq!(split_prompt(inner, false), (inner, None));
    }

    #[test]
    fn the_version_title_names_the_package_version() {
        let title = version_title(80).expect("a normal popup has room");
        let text: String = title
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect();
        assert_eq!(text, format!(" znicz {} ", env!("CARGO_PKG_VERSION")));
    }

    #[test]
    fn a_narrow_popup_drops_the_version_and_keeps_the_close_hint() {
        assert!(version_title(30).is_none());
        assert!(version_title(80).is_some());
    }
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p znicz-tui`
Expected: PASS, including the mouse tests `the_close_hit_covers_the_drawn_x_glyph` and `a_drawn_overlay_exposes_a_close_hit`.

- [ ] **Step 5: Commit**

```bash
git add znicz-tui/src/views/help.rs znicz-tui/tests/render.rs
git commit -m "Draw help context first with a filter row"
```

---

### Task 9: Version, docs, and preview

**Files:**
- Modify: `Cargo.toml`, `Cargo.lock`
- Modify: `README.md`
- Modify: `wiki/Architecture/TUI.md`
- Modify: `wiki/Plans/Roadmap.md`

- [ ] **Step 1: Bump the version**

In the root `Cargo.toml`, change `version = "0.5.2"` under `[workspace.package]` to `version = "0.6.0"`. Run `cargo build -p znicz` so `Cargo.lock` picks up the new member versions.

- [ ] **Step 2: Update the README key table**

In `README.md`, replace:

```markdown
Press `?` in the player for the full keymap; the overlay footer shows the
running version. The essentials:
```

with:

```markdown
Press `?` in the player for the keymap: the keys for where you are come first,
and `/` filters by key or action. The overlay footer shows the running version.
The essentials:
```

Replace the row `| → ← or l h | Seek ±5s (`L` / `H` for ±30s) |` with:

```markdown
| → ← | Seek ±5s (Shift-→ / Shift-← for ±30s) |
```

After the row `| j k, g G, Ctrl-d Ctrl-u | Move in the list |`, add:

```markdown
| h l | Left / right: column, tree level, back / into (never plays) |
```

- [ ] **Step 3: Update `wiki/Architecture/TUI.md`**

Insert a new section directly above `## The signal path`:

```markdown
## Keys, footer, and help

Arrows seek (`→` / `←` 5 s, `Shift-→` / `Shift-←` 30 s). `h` / `l` move left
and right in the library and never play anything:

| Layout | `h` | `l` |
| --- | --- | --- |
| Columns | previous column (stops at Artists) | next column (stops at Tracks) |
| Tree | collapse an open node, else jump to the parent | expand a closed artist or album |
| Single column | back one level | into the artist or album |

They do nothing in search results, the queue, or modals. `Tab` still cycles
columns and wraps.

The footer shows a short hint list for the current **context** (library,
library tree, search prompt, queue, devices, inspector, playlists, radio, a
name prompt, the station form, or help), most important first, as many as fit.
` ? more ` is right-aligned and always kept. Each hint carries the key it sends,
so clicking one runs that key.

`?` opens help on the context it was opened from: that context's keys first,
then Playback and Navigation, then every other context dimmed. `j` / `k`,
`Ctrl-d` / `Ctrl-u`, `g` / `G`, and the wheel scroll. `/` filters by key or
action (case-insensitive); `Enter` keeps the filter, `Esc` clears it, and a
second `Esc`, `?`, or `q` closes help (`q` does not quit from here). Other keys
are ignored while help is open.
```

Replace:

```markdown
Right move the caret; Home and End jump to the ends. The footer switches to
type/cancel so the global keymap is not what you type into.
```

with:

```markdown
Right move the caret; Home and End jump to the ends. The footer switches to
`Enter save · Esc cancel` so the global keymap is not what you type into.
```

In the Modules table, replace the row `| \`views/status.rs\` | key hints only |` with these rows:

```markdown
| `views/status.rs` | footer hint line for the current context |
| `views/help.rs` | help overlay: context first, scroll, filter row |
| `help_state.rs` | help context, scroll, and filter text |
| `footer_hits.rs` | footer hint placement and click targets |
```

Replace the paragraph:

```markdown
`keys.rs` holds the bindings as data, and both the help overlay and the footer
hints are generated from it. The old help text had drifted from the code and
documented keys that did nothing; generating it removes that possibility.
```

with:

```markdown
`keys.rs` holds the bindings as data. Help is generated from those tables, and
the footer hints for each context are listed next to them; a unit test checks
that every hint names a key the context's help documents. The old help text had
drifted from the code and documented keys that did nothing; generating it
removes that possibility.
```

- [ ] **Step 4: Update `wiki/Plans/Roadmap.md`**

Insert directly above `## Later TUI (parked)`:

```markdown
## TUI reimagining (0.6+)

A five-part rework of the player experience. Each part has its own spec and PR.

| # | Part | Status |
| --- | --- | --- |
| 1 | Keymap and discoverability: `h` / `l` move, arrows seek, context footer, context-first help with a filter | **Done** in 0.6.0 ([spec](../../docs/superpowers/specs/2026-11-07-keymap-discoverability-design.md)) |
| 2 | Space and layering: modals inside the list region, toasts in a transport slot with history, focus styling, breadcrumb, playing marker | Planned |
| 3 | Empty-library onboarding: scan a folder from the TUI | Planned |
| 4 | Stage mode ([#46](https://github.com/eugene-chekan/znicz/issues/46)): full-screen now playing; the inspector folds in; marquee ([#34](https://github.com/eugene-chekan/znicz/issues/34)) | Planned |
| 5 | Reorder rows ([#36](https://github.com/eugene-chekan/znicz/issues/36)) and finer volume ([#47](https://github.com/eugene-chekan/znicz/issues/47)) | Planned |

Once Stage exists, Browse drops the cover and gives the library the full height.
```

- [ ] **Step 5: Check the preview frames**

Run: `cargo run -q -p znicz-tui --example preview -- 110 32 2>/dev/null`
Expected: the library footer reads `Space pause · / search · a add · ] queue · P playlists · R radio` with `? more` at the right; the help frame is titled `Keys · …`, single column, with `/ filter · Esc close` in its bottom border and the footer row still visible beneath it.

- [ ] **Step 6: Commit**

```bash
git add Cargo.toml Cargo.lock README.md wiki/Architecture/TUI.md wiki/Plans/Roadmap.md
git commit -m "Bump to 0.6.0 and document the new keymap and help"
```

---

### Task 10: Verify and open the PR

- [ ] **Step 1: Format, lint, test**

Run each and confirm it is clean:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Expected: no diff, no warnings, all tests pass. If `fmt` reports a diff, run `cargo fmt --all` and commit with `git commit -am "Format keymap changes"`.

- [ ] **Step 2: Mark the plan done**

Tick every checkbox in this file and commit:

```bash
git add docs/superpowers/plans/2026-11-07-keymap-discoverability.md
git commit -m "Mark keymap plan complete"
```

- [ ] **Step 3: Push and open the PR**

```bash
git push -u origin feat/keymap-discoverability
gh pr create --title "Keymap and discoverability (TUI rework part 1)" --body "$(cat <<'EOF'
Part 1 of the TUI rework ([spec](docs/superpowers/specs/2026-11-07-keymap-discoverability-design.md)).

- `h` / `l` move left and right in every library layout and never play; arrows seek (Shift for 30 s); `H` / `L` are unbound
- The footer shows prioritised hints for the current context and always keeps `? more`; clicks carry real key events
- Help opens on the current context, scrolls, and filters with `/`
- Bumps to 0.6.0; wiki and README updated

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```
