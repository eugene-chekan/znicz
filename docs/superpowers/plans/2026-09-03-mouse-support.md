# TUI Mouse Support Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let the TUI select list rows with the mouse, scroll with the wheel, dismiss overlays with click-outside, and toggle the queue drawer from the library’s right border — without activating (no Enter).

**Architecture:** Each draw writes a `HitMap` on `App`. `App::on_mouse` hit-tests that map and moves existing `Cursor`s. Crossterm `EnableMouseCapture` wraps the run loop. Tests drive `on_mouse` (and one TestBackend draw) without a tty.

**Tech Stack:** Ratatui 0.30 `Rect` / `ListState`, crossterm 0.28 `Event::Mouse` / `EnableMouseCapture`, existing `znicz-tui` `App` + `Cursor`.

**Spec:** `docs/superpowers/specs/2026-09-03-mouse-support-design.md`

## Global Constraints

- Version **0.4.3 → 0.4.4** in the same change as the feature (compatible addition).
- Select only: a click never plays, never opens an album, never applies a device.
- Mouse never becomes seek (`h`/`l`) or title pan (`Alt-←` / `Alt-→`).
- No `[tui] mouse` flag, no toggle key, no mouse lines in `keys.rs` / `?`.
- Out of scope: drag-reorder (#36), seek-bar / cover / footer / toast clicks, double-click activate, touch.
- Wiki matches the code in the same change.
- Parked TUI `#5`–`#7` / `#9` / `#22` / `#34` / `#36`, playlist `#18` / `#19`: untouched.

## File map

| File | Role |
| --- | --- |
| Create `znicz-tui/src/hit.rs` | `ListHit`, `HitMap`, `row_at` |
| Modify `znicz-tui/src/lib.rs` | `mod hit` |
| Modify `znicz-tui/src/library_pane.rs` | `set_index`; `pub` inject helpers for tests |
| Modify `znicz-tui/src/app.rs` | `hits`, `ListState`s, `on_mouse`, capture in `run` |
| Modify `znicz-tui/src/views/mod.rs` | Fill hit map; pass `&mut App` into help/inspector |
| Modify `znicz-tui/src/views/{library,queue,help,inspector,devices,playlists,radio}.rs` | Persist `ListState`; record rects |
| Create `znicz-tui/tests/mouse.rs` | Click / wheel / overlay / prompt / toggle |
| Modify `wiki/Architecture/TUI.md`, `wiki/Issues.md`, `wiki/Plans/Roadmap.md` | Mouse paragraph; #8 closed |
| Modify `Cargo.toml` + `Cargo.lock` | `0.4.4` |

---

### Task 1: Hit-test helpers

**Files:**
- Create: `znicz-tui/src/hit.rs`
- Modify: `znicz-tui/src/lib.rs`

**Interfaces:**
- Consumes: `ratatui::layout::{Position, Rect}`
- Produces: `pub struct ListHit { pub inner: Rect, pub offset: usize, pub len: usize }` with `pub fn row_at(self, column: u16, row: u16) -> Option<usize>`; `pub struct HitMap` (`Default`) with `library`, `queue`, `overlay`, `overlay_list`, `queue_toggle`, `library_pane`, `search_prompt` all `Option` as below

- [ ] **Step 1: Write the failing tests** in `znicz-tui/src/hit.rs`

```rust
use ratatui::layout::Rect;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ListHit {
    pub inner: Rect,
    pub offset: usize,
    pub len: usize,
}

impl ListHit {
    pub fn row_at(self, column: u16, row: u16) -> Option<usize> {
        todo!("task 1")
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct HitMap {
    pub library: Option<ListHit>,
    pub queue: Option<ListHit>,
    pub overlay: Option<Rect>,
    pub overlay_list: Option<ListHit>,
    pub queue_toggle: Option<Rect>,
    pub library_pane: Option<Rect>,
    pub search_prompt: Option<Rect>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::layout::Position;

    fn list() -> ListHit {
        ListHit {
            inner: Rect::new(1, 1, 20, 5),
            offset: 2,
            len: 10,
        }
    }

    #[test]
    fn a_click_on_the_first_visible_row_uses_the_offset() {
        assert_eq!(list().row_at(2, 1), Some(2));
        assert_eq!(list().row_at(2, 2), Some(3));
    }

    #[test]
    fn a_click_past_the_last_item_is_ignored() {
        let hit = ListHit {
            inner: Rect::new(1, 1, 20, 8),
            offset: 0,
            len: 2,
        };
        assert_eq!(hit.row_at(2, 3), None);
    }

    #[test]
    fn a_click_outside_the_inner_rect_is_ignored() {
        assert_eq!(list().row_at(0, 1), None);
        assert_eq!(list().row_at(2, 0), None);
        assert_eq!(list().row_at(2, 6), None);
    }

    #[test]
    fn contains_uses_ratatui_position() {
        let rect = Rect::new(10, 5, 4, 3);
        assert!(rect.contains(Position { x: 10, y: 5 }));
        assert!(!rect.contains(Position { x: 14, y: 5 }));
    }
}
```

Keep `todo!` in `row_at` so the tests compile and fail.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p znicz-tui --lib hit::tests --offline`

Expected: FAIL — `todo!("task 1")` or `not yet implemented`

- [ ] **Step 3: Implement `row_at`**

```rust
use ratatui::layout::{Position, Rect};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ListHit {
    pub inner: Rect,
    pub offset: usize,
    pub len: usize,
}

impl ListHit {
    pub fn row_at(self, column: u16, row: u16) -> Option<usize> {
        if !self.inner.contains(Position {
            x: column,
            y: row,
        }) {
            return None;
        }
        let index = self.offset + usize::from(row.saturating_sub(self.inner.y));
        (index < self.len).then_some(index)
    }
}
```

Keep the `HitMap` struct from step 1 (no `todo`).

In `znicz-tui/src/lib.rs` add `pub mod hit;` next to `pub mod cursor;`.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p znicz-tui --lib hit::tests --offline`

Expected: PASS (4 tests)

- [ ] **Step 5: Commit**

```bash
git add znicz-tui/src/hit.rs znicz-tui/src/lib.rs
git commit -m "$(cat <<'EOF'
Add list hit-testing so a click maps to a visible row index.

EOF
)"
```

---

### Task 2: Click a library row (select only)

**Files:**
- Modify: `znicz-tui/src/library_pane.rs`
- Modify: `znicz-tui/src/app.rs`
- Create: `znicz-tui/tests/mouse.rs`

**Interfaces:**
- Consumes: `HitMap`, `ListHit::row_at`, `LibraryPane::inject_albums_for_test`
- Produces: `LibraryPane::set_index(&mut self, index: usize)`; `App { pub hits: HitMap, ... }`; `App::on_mouse(&mut self, mouse: MouseEvent)`

- [ ] **Step 1: Write the failing tests** in `znicz-tui/tests/mouse.rs`

Change `inject_albums_for_test` / `inject_tracks_for_test` in `library_pane.rs` from `pub(crate)` to `pub` so integration tests can seed rows.

Add `set_index` to `LibraryPane` next to `step` (it can `todo!` until step 3):

```rust
pub fn set_index(&mut self, index: usize) {
    self.cursor.set(index, self.len());
    self.h_offset = 0;
}
```

Tests:

```rust
//! Mouse: select-only clicks, wheel, click-outside, queue toggle.

use crossterm::event::{
    KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::layout::Rect;
use znicz_core::{spawn_player, AudioConfig, PlaybackStatus, PlayerHandle};
use znicz_library::AlbumSummary;
use znicz_tui::hit::{HitMap, ListHit};
use znicz_tui::{App, Focus, Modal};

fn player() -> PlayerHandle {
    let (player, _thread) = spawn_player(AudioConfig::default());
    player
}

fn new_app() -> App {
    App::with_library(player(), None)
}

fn left_click(column: u16, row: u16) -> MouseEvent {
    MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column,
        row,
        modifiers: KeyModifiers::NONE,
    }
}

fn albums(n: usize) -> Vec<AlbumSummary> {
    (0..n)
        .map(|i| AlbumSummary {
            album: format!("Album {i}"),
            album_artist: Some("Artist".into()),
            year: None,
            track_count: 1,
            total_secs: None,
        })
        .collect()
}

fn library_hits(len: usize) -> HitMap {
    HitMap {
        library: Some(ListHit {
            inner: Rect::new(1, 1, 40, 10),
            offset: 0,
            len,
        }),
        library_pane: Some(Rect::new(0, 0, 80, 20)),
        queue_toggle: Some(Rect::new(79, 0, 1, 20)),
        ..HitMap::default()
    }
}

#[test]
fn a_library_click_moves_the_cursor_and_does_not_play() {
    let mut app = new_app();
    app.library.inject_albums_for_test(albums(5));
    app.hits = library_hits(5);
    assert_eq!(app.library.selected_index(), Some(0));

    app.on_mouse(left_click(2, 3));

    assert_eq!(app.library.selected_index(), Some(2));
    assert_eq!(app.focus, Focus::Library);
    assert_eq!(app.player.state().status, PlaybackStatus::Stopped);
    assert!(!app.library.is_empty());
    match app.library.mode() {
        znicz_tui::library_pane::Mode::Albums => {}
        other => panic!("must not open an album, got {other:?}"),
    }
}

#[test]
fn a_click_below_the_last_library_row_does_nothing() {
    let mut app = new_app();
    app.library.inject_albums_for_test(albums(2));
    app.hits = library_hits(2);
    app.on_mouse(left_click(2, 5));
    assert_eq!(app.library.selected_index(), Some(0));
}
```

`App` has no `hits` or `on_mouse` yet — tests fail to compile. That is the red.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p znicz-tui --test mouse --offline`

Expected: compile error: no `hits` / `on_mouse` on `App`

- [ ] **Step 3: Minimal implementation**

In `library_pane.rs` add `set_index` (step 1 snippet) and make both inject helpers `pub`.

In `app.rs`:

- `use crate::hit::HitMap;`
- `use crossterm::event::{..., MouseButton, MouseEvent, MouseEventKind};`
- On `App`: `pub hits: HitMap,`
- In `with_engine`: `hits: HitMap::default(),`
- Add:

```rust
pub fn on_mouse(&mut self, mouse: MouseEvent) {
    match mouse.kind {
        MouseEventKind::Down(MouseButton::Left) => self.on_left_click(mouse.column, mouse.row),
        _ => {}
    }
}

fn on_left_click(&mut self, column: u16, row: u16) {
    if self.modal != Modal::None {
        return;
    }
    if let Some(hit) = self.hits.library {
        if let Some(index) = hit.row_at(column, row) {
            self.library.set_index(index);
            self.focus = Focus::Library;
        }
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p znicz-tui --test mouse --offline`

Expected: PASS (2 tests)

- [ ] **Step 5: Commit**

```bash
git add znicz-tui/src/library_pane.rs znicz-tui/src/app.rs znicz-tui/tests/mouse.rs
git commit -m "$(cat <<'EOF'
Select a library row on left click without opening or playing it.

EOF
)"
```

---

### Task 3: Queue row click and drawer toggle

**Files:**
- Modify: `znicz-tui/src/app.rs`
- Modify: `znicz-tui/tests/mouse.rs`

**Interfaces:**
- Consumes: `HitMap.queue`, `queue_toggle`, `library_pane`; `layout::is_sheet`; `App::open_queue` / `close_queue` (already private on `App`)
- Produces: `on_left_click` handles queue select and toggle as specified

- [ ] **Step 1: Write the failing tests** — append to `znicz-tui/tests/mouse.rs`

Reuse `queue` helper from `znicz-tui/tests/keys.rs` (copy it; do not import from that file):

```rust
use znicz_core::Command;

fn queue(app: &mut App, count: usize) {
    let items: Vec<znicz_core::QueueItem> = (0..count)
        .map(|i| znicz_core::QueueItem::file(format!("/music/track-{i}.flac")))
        .collect();
    app.player
        .send_blocking(Command::QueueAdd(items))
        .expect("queue add");
}

fn contains(rect: Rect, column: u16, row: u16) -> bool {
    rect.contains(ratatui::layout::Position { x: column, y: row })
}

#[test]
fn a_queue_click_selects_the_row_and_focuses_the_queue() {
    let mut app = new_app();
    queue(&mut app, 4);
    app.queue_open = true;
    app.focus = Focus::Library;
    app.hits.queue = Some(ListHit {
        inner: Rect::new(60, 1, 38, 10),
        offset: 0,
        len: 4,
    });
    app.on_mouse(left_click(62, 3));
    assert_eq!(app.queue_cursor.selected(4), Some(2));
    assert_eq!(app.focus, Focus::Queue);
    assert_eq!(app.player.state().queue_position, 0);
}

#[test]
fn the_right_border_opens_the_queue_when_it_is_closed() {
    let mut app = new_app();
    app.list_width = 100;
    app.hits.queue_toggle = Some(Rect::new(79, 0, 1, 20));
    assert!(!app.queue_open);
    app.on_mouse(left_click(79, 4));
    assert!(app.queue_open);
    assert_eq!(app.focus, Focus::Queue);
}

#[test]
fn a_click_on_the_library_closes_an_overlay_queue() {
    let mut app = new_app();
    app.list_width = 100;
    app.queue_open = true;
    app.focus = Focus::Queue;
    app.hits.library_pane = Some(Rect::new(0, 0, 59, 20));
    app.hits.queue = Some(ListHit {
        inner: Rect::new(60, 1, 38, 18),
        offset: 0,
        len: 0,
    });
    app.on_mouse(left_click(10, 5));
    assert!(!app.queue_open);
    assert_eq!(app.focus, Focus::Library);
}

#[test]
fn the_right_border_closes_a_queue_sheet() {
    let mut app = new_app();
    app.list_width = 81;
    app.queue_open = true;
    app.focus = Focus::Queue;
    app.hits.queue_toggle = Some(Rect::new(80, 0, 1, 20));
    app.hits.queue = Some(ListHit {
        inner: Rect::new(1, 1, 79, 18),
        offset: 0,
        len: 0,
    });
    app.on_mouse(left_click(80, 2));
    assert!(!app.queue_open);
    assert_eq!(app.focus, Focus::Library);
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p znicz-tui --test mouse --offline`

Expected: FAIL — queue stays closed / cursor stays 0 / focus stays Library

- [ ] **Step 3: Extend `on_left_click`**

Replace `on_left_click` with:

```rust
fn point_in(rect: Rect, column: u16, row: u16) -> bool {
    rect.contains(ratatui::layout::Position { x: column, y: row })
}

fn on_left_click(&mut self, column: u16, row: u16) {
    if self.modal != Modal::None {
        return;
    }

    if self.queue_open {
        if let Some(hit) = self.hits.queue {
            if let Some(index) = hit.row_at(column, row) {
                let len = self.player.state().queue.len();
                self.queue_cursor.set(index, len);
                self.focus = Focus::Queue;
                return;
            }
        }
        let sheet = layout::is_sheet(self.list_width, true);
        if sheet {
            if self
                .hits
                .queue_toggle
                .is_some_and(|r| point_in(r, column, row))
            {
                self.close_queue();
            }
            return;
        }
        if self
            .hits
            .library_pane
            .is_some_and(|r| point_in(r, column, row))
            || self
                .hits
                .queue_toggle
                .is_some_and(|r| point_in(r, column, row))
        {
            self.close_queue();
        }
        return;
    }

    if self
        .hits
        .queue_toggle
        .is_some_and(|r| point_in(r, column, row))
    {
        self.open_queue();
        return;
    }

    if let Some(hit) = self.hits.library {
        if let Some(index) = hit.row_at(column, row) {
            self.library.set_index(index);
            self.focus = Focus::Library;
        }
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p znicz-tui --test mouse --offline`

Expected: PASS (all mouse tests so far)

- [ ] **Step 5: Commit**

```bash
git add znicz-tui/src/app.rs znicz-tui/tests/mouse.rs
git commit -m "$(cat <<'EOF'
Let a click select a queue row and toggle the drawer from the right border.

EOF
)"
```

---

### Task 4: Overlay click-outside and list-overlay select

**Files:**
- Modify: `znicz-tui/src/app.rs`
- Modify: `znicz-tui/tests/mouse.rs`

**Interfaces:**
- Consumes: `HitMap.overlay`, `overlay_list`; `device_cursor`, `playlist_cursor`, `station_cursor`
- Produces: overlay clicks select a row or close like `Esc` (without applying the row)

- [ ] **Step 1: Write the failing tests** — append to `znicz-tui/tests/mouse.rs`

```rust
#[test]
fn a_click_outside_help_closes_it() {
    let mut app = new_app();
    app.modal = Modal::Help;
    app.hits.overlay = Some(Rect::new(10, 4, 60, 16));
    app.on_mouse(left_click(0, 0));
    assert_eq!(app.modal, Modal::None);
}

#[test]
fn a_click_inside_help_does_not_close_it() {
    let mut app = new_app();
    app.modal = Modal::Help;
    app.hits.overlay = Some(Rect::new(10, 4, 60, 16));
    app.on_mouse(left_click(12, 6));
    assert_eq!(app.modal, Modal::Help);
}

#[test]
fn a_click_outside_inspector_closes_it() {
    let mut app = new_app();
    app.modal = Modal::Inspector;
    app.hits.overlay = Some(Rect::new(20, 4, 40, 12));
    app.on_mouse(left_click(0, 0));
    assert_eq!(app.modal, Modal::None);
}

#[test]
fn a_devices_row_click_moves_the_cursor_without_applying() {
    let mut app = new_app();
    app.modal = Modal::Devices;
    app.devices = vec![
        znicz_core::AudioDeviceInfo {
            id: "a".into(),
            name: "A".into(),
            is_default: true,
        },
        znicz_core::AudioDeviceInfo {
            id: "b".into(),
            name: "B".into(),
            is_default: false,
        },
    ];
    app.hits.overlay = Some(Rect::new(10, 4, 40, 12));
    app.hits.overlay_list = Some(ListHit {
        inner: Rect::new(11, 5, 38, 10),
        offset: 0,
        len: 2,
    });
    let before = app.player.state().device_id.clone();
    app.on_mouse(left_click(12, 6));
    assert_eq!(app.device_cursor.selected(2), Some(1));
    assert_eq!(app.modal, Modal::Devices);
    assert_eq!(app.player.state().device_id, before);
}

#[test]
fn a_click_outside_devices_closes_the_overlay() {
    let mut app = new_app();
    app.modal = Modal::Devices;
    app.hits.overlay = Some(Rect::new(10, 4, 40, 12));
    app.on_mouse(left_click(0, 0));
    assert_eq!(app.modal, Modal::None);
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p znicz-tui --test mouse --offline`

Expected: FAIL — `on_left_click` returns immediately when `modal != None`, so help stays open on outside click and devices cursor does not move

- [ ] **Step 3: Handle overlays first in `on_left_click`**

At the **top** of `on_left_click` (before queue/library), after you still ignore nothing globally:

```rust
fn on_left_click(&mut self, column: u16, row: u16) {
    if self.modal != Modal::None {
        self.on_overlay_click(column, row);
        return;
    }
    // ... existing queue / library / toggle from Task 3
}

fn on_overlay_click(&mut self, column: u16, row: u16) {
    if let Some(hit) = self.hits.overlay_list {
        if let Some(index) = hit.row_at(column, row) {
            match self.modal {
                Modal::Devices => self.device_cursor.set(index, self.devices.len()),
                Modal::Playlists => self.playlist_cursor.set(index, self.playlists.len()),
                Modal::Radio => self.station_cursor.set(index, self.stations.len()),
                _ => {}
            }
            return;
        }
    }
    if self
        .hits
        .overlay
        .is_some_and(|r| point_in(r, column, row))
    {
        return;
    }
    self.modal = Modal::None;
    self.playlist_prompt = None;
    self.radio_prompt = None;
}
```

Keep the Task 3 queue/library body in the `modal == None` path.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p znicz-tui --test mouse --offline`

Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add znicz-tui/src/app.rs znicz-tui/tests/mouse.rs
git commit -m "$(cat <<'EOF'
Close overlays on click-outside and select list-overlay rows without applying them.

EOF
)"
```

---

### Task 5: Typing prompts cancel on click-outside

**Files:**
- Modify: `znicz-tui/src/app.rs`
- Modify: `znicz-tui/tests/mouse.rs`

**Interfaces:**
- Consumes: `library.is_typing()`, `hits.search_prompt`, `playlist_prompt`, `radio_prompt`, `hits.overlay`
- Produces: search / playlist / radio prompts cancel like `Esc`; no row select while typing

- [ ] **Step 1: Write the failing tests** — append to `znicz-tui/tests/mouse.rs`

```rust
#[test]
fn a_click_outside_search_cancels_the_prompt() {
    let mut app = new_app();
    app.library.inject_albums_for_test(albums(3));
    app.library.begin_search();
    app.hits.search_prompt = Some(Rect::new(0, 0, 80, 1));
    app.hits.library = Some(ListHit {
        inner: Rect::new(1, 2, 40, 10),
        offset: 0,
        len: 3,
    });
    app.on_mouse(left_click(2, 5));
    assert!(!app.library.is_typing());
    assert_eq!(app.library.selected_index(), Some(0));
}

#[test]
fn a_click_on_the_search_line_does_not_type_or_select() {
    let mut app = new_app();
    app.library.begin_search();
    app.hits.search_prompt = Some(Rect::new(0, 0, 80, 1));
    app.on_mouse(left_click(4, 0));
    assert!(app.library.is_typing());
}

#[test]
fn a_click_outside_a_playlist_form_cancels_the_form_and_keeps_the_overlay() {
    let mut app = new_app();
    app.modal = Modal::Playlists;
    app.playlist_prompt = Some(PlaylistPrompt::Save(znicz_tui::line_edit::LineEdit::from_text(
        "x".into(),
    )));
    app.hits.overlay = Some(Rect::new(10, 4, 40, 12));
    app.on_mouse(left_click(0, 0));
    assert!(app.playlist_prompt.is_none());
    assert_eq!(app.modal, Modal::Playlists);
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p znicz-tui --test mouse --offline`

Expected: FAIL — search stays open or library cursor moves; playlist overlay closes entirely

- [ ] **Step 3: Prompt handling before overlay/list clicks**

At the very start of `on_left_click`:

```rust
fn on_left_click(&mut self, column: u16, row: u16) {
    if self.library.is_typing() {
        let on_prompt = self
            .hits
            .search_prompt
            .is_some_and(|r| point_in(r, column, row));
        if !on_prompt {
            self.library.cancel_search();
            self.toasts.info("search cancelled");
        }
        return;
    }
    if self.playlist_prompt.is_some() || self.radio_prompt.is_some() {
        let inside = self
            .hits
            .overlay
            .is_some_and(|r| point_in(r, column, row));
        if !inside {
            self.playlist_prompt = None;
            self.radio_prompt = None;
        }
        return;
    }
    if self.modal != Modal::None {
        self.on_overlay_click(column, row);
        return;
    }
    // ... queue / library from Task 3
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p znicz-tui --test mouse --offline`

Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add znicz-tui/src/app.rs znicz-tui/tests/mouse.rs
git commit -m "$(cat <<'EOF'
Cancel search and name prompts on click-outside without typing into them.

EOF
)"
```

---

### Task 6: Wheel

**Files:**
- Modify: `znicz-tui/src/app.rs`
- Modify: `znicz-tui/tests/mouse.rs`

**Interfaces:**
- Consumes: `MouseEventKind::ScrollUp` / `ScrollDown`; `Cursor::step` / `LibraryPane::step`
- Produces: `on_mouse` steps the overlay list, or the focused pane when no overlay/prompt

- [ ] **Step 1: Write the failing tests** — append to `znicz-tui/tests/mouse.rs`

```rust
fn wheel(up: bool) -> MouseEvent {
    MouseEvent {
        kind: if up {
            MouseEventKind::ScrollUp
        } else {
            MouseEventKind::ScrollDown
        },
        column: 2,
        row: 2,
        modifiers: KeyModifiers::NONE,
    }
}

#[test]
fn wheel_down_steps_the_library() {
    let mut app = new_app();
    app.library.inject_albums_for_test(albums(5));
    app.on_mouse(wheel(false));
    assert_eq!(app.library.selected_index(), Some(1));
}

#[test]
fn wheel_up_wraps_like_k() {
    let mut app = new_app();
    app.library.inject_albums_for_test(albums(5));
    app.on_mouse(wheel(true));
    assert_eq!(app.library.selected_index(), Some(4));
}

#[test]
fn wheel_steps_a_list_overlay_not_the_library() {
    let mut app = new_app();
    app.library.inject_albums_for_test(albums(5));
    app.modal = Modal::Playlists;
    app.playlists = vec!["a".into(), "b".into(), "c".into()];
    app.on_mouse(wheel(false));
    assert_eq!(app.playlist_cursor.selected(3), Some(1));
    assert_eq!(app.library.selected_index(), Some(0));
}

#[test]
fn wheel_is_ignored_while_help_is_open() {
    let mut app = new_app();
    app.library.inject_albums_for_test(albums(5));
    app.modal = Modal::Help;
    app.on_mouse(wheel(false));
    assert_eq!(app.library.selected_index(), Some(0));
}

#[test]
fn wheel_is_ignored_while_searching() {
    let mut app = new_app();
    app.library.inject_albums_for_test(albums(5));
    app.library.begin_search();
    app.on_mouse(wheel(false));
    assert_eq!(app.library.selected_index(), Some(0));
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p znicz-tui --test mouse --offline`

Expected: FAIL — `on_mouse` ignores `ScrollDown` / `ScrollUp`

- [ ] **Step 3: Handle wheel in `on_mouse`**

```rust
pub fn on_mouse(&mut self, mouse: MouseEvent) {
    match mouse.kind {
        MouseEventKind::Down(MouseButton::Left) => self.on_left_click(mouse.column, mouse.row),
        MouseEventKind::ScrollUp => self.on_wheel(-1),
        MouseEventKind::ScrollDown => self.on_wheel(1),
        _ => {}
    }
}

fn on_wheel(&mut self, delta: isize) {
    if self.library.is_typing()
        || self.playlist_prompt.is_some()
        || self.radio_prompt.is_some()
    {
        return;
    }
    match self.modal {
        Modal::Devices => self.device_cursor.step(delta, self.devices.len()),
        Modal::Playlists => self.playlist_cursor.step(delta, self.playlists.len()),
        Modal::Radio => self.station_cursor.step(delta, self.stations.len()),
        Modal::Help | Modal::Inspector => {}
        Modal::None => match self.focus {
            Focus::Library => self.library.step(delta),
            Focus::Queue if self.queue_open => {
                let len = self.player.state().queue.len();
                self.queue_cursor.step(delta, len);
            }
            Focus::Queue => {}
        },
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p znicz-tui --test mouse --offline`

Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add znicz-tui/src/app.rs znicz-tui/tests/mouse.rs
git commit -m "$(cat <<'EOF'
Step the focused list with the mouse wheel, ignoring help and prompts.

EOF
)"
```

---

### Task 7: Fill the hit map while drawing, capture the mouse

**Files:**
- Modify: `znicz-tui/src/app.rs`
- Modify: `znicz-tui/src/views/mod.rs`
- Modify: `znicz-tui/src/views/library.rs`
- Modify: `znicz-tui/src/views/queue.rs`
- Modify: `znicz-tui/src/views/help.rs`
- Modify: `znicz-tui/src/views/inspector.rs`
- Modify: `znicz-tui/src/views/devices.rs`
- Modify: `znicz-tui/src/views/playlists.rs`
- Modify: `znicz-tui/src/views/radio.rs`
- Modify: `znicz-tui/tests/mouse.rs` (one TestBackend test)

**Interfaces:**
- Consumes: `Block::inner`, `ListState::offset` after `render_stateful_widget`, `layout::drawer` / `strip_width`
- Produces: each frame `app.hits` is replaced; `library_list_state` (and queue/device/playlist/station) live on `App`; `run` enables/disables mouse capture; `run_loop` calls `on_mouse`

- [ ] **Step 1: Write the failing TestBackend test** — append to `znicz-tui/tests/mouse.rs`

```rust
use ratatui::backend::TestBackend;
use ratatui::Terminal;
use znicz_tui::views;

#[test]
fn a_drawn_library_row_is_clickable() {
    let mut app = new_app();
    app.library.inject_albums_for_test(albums(5));
    let state = app.player.state();
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).expect("backend");
    terminal
        .draw(|frame| views::render(frame, &mut app, &state))
        .expect("draw");
    let inner = app.hits.library.expect("library hit after draw");
    let col = inner.inner.x + 1;
    let row = inner.inner.y + 2;
    app.on_mouse(left_click(col, row));
    assert_eq!(app.library.selected_index(), Some(2));
}
```

This fails until render writes `hits.library`.

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p znicz-tui --test mouse a_drawn_library_row_is_clickable --offline`

Expected: FAIL — `library hit after draw` (hits still default)

- [ ] **Step 3: Persist `ListState`, record hits, capture mouse**

On `App` add (not `pub` except tests need hits, already pub):

```rust
pub(crate) library_list_state: ratatui::widgets::ListState,
pub(crate) queue_list_state: ratatui::widgets::ListState,
pub(crate) device_list_state: ratatui::widgets::ListState,
pub(crate) playlist_list_state: ratatui::widgets::ListState,
pub(crate) station_list_state: ratatui::widgets::ListState,
```

Initialize each with `ListState::default()` in `with_engine`.

At the **start** of `views::render`, `app.hits = HitMap::default();`. Then after `let list = chunks[0];`:

```rust
app.hits.queue_toggle = list.width.checked_sub(1).map(|w| Rect {
    x: list.x + w,
    y: list.y,
    width: 1,
    height: list.height,
});
let lib_w = crate::layout::strip_width(list, app.queue_open);
app.hits.library_pane = Some(Rect {
    x: list.x,
    y: list.y,
    width: lib_w,
    height: list.height,
});
```

**Library** (`views/library.rs`): take `&mut App`. After building `block` / `list_area`:

- If empty: leave `hits.library` as `None`; if typing, set `search_prompt` to the prompt rect.
- Else: `app.library_list_state.select(app.library.selected_index());` then `frame.render_stateful_widget(list, list_area, &mut app.library_list_state);`
- `let block = views::pane_block(...);` (already created). `app.hits.library = Some(ListHit { inner: block.inner(list_area), offset: app.library_list_state.offset(), len: app.library.len() });`
- If typing: `app.hits.search_prompt = prompt_area;`

You must compute `block.inner(list_area)` from the same `Block` passed to the list. Clone is unnecessary; `pane_block` returns a new `Block` each call — call it once, use `.inner(list_area)` for the hit, and `.clone()` is not available on Block with callbacks; build:

```rust
let block = views::pane_block(&title, focused, Some(summary));
let inner = block.inner(list_area);
let list = List::new(items).block(block).highlight_style(...);
app.library_list_state.select(app.library.selected_index());
frame.render_stateful_widget(list, list_area, &mut app.library_list_state);
app.hits.library = Some(ListHit {
    inner,
    offset: app.library_list_state.offset(),
    len: count,
});
```

**Queue:** same pattern with `app.queue_list_state` and `app.hits.queue`. `len` is `state.queue.len()`. Empty queue: `hits.queue = None`.

**Help:** change `help::render(frame, area)` to `help::render(frame, area, app: &mut App)` and `app.hits.overlay = Some(popup);` (`overlay_list` stays `None`).

**Inspector:** same, `app.hits.overlay = Some(popup)`.

**Devices / playlists / radio:** `app.hits.overlay = Some(popup)` in `render_modal` (the centered rect). After drawing the list, `app.hits.overlay_list = Some(ListHit { inner: list_area or block.inner(area), offset: ...offset(), len })`. Playlists/radio: `list_area` after the prompt split is the list inner (already inside the block). Devices: `block.inner(area)` like library. Use `device_list_state` / `playlist_list_state` / `station_list_state`. Empty lists: `overlay_list = None`.

`views::render` currently calls `help::render(frame, area)` — pass `app`. Inspector currently `inspector::render(frame, area, state)` — add `app`.

**Run loop** in `app.rs`:

```rust
match event::read()? {
    Event::Key(key) if key.kind == KeyEventKind::Press => self.on_key(key),
    Event::Mouse(mouse) => self.on_mouse(mouse),
    _ => {}
}
```

**`run`:**

```rust
pub fn run(&mut self) -> color_eyre::Result<()> {
    let mut terminal = ratatui::init();
    let _ = crossterm::execute!(std::io::stdout(), event::EnableMouseCapture);
    self.picker = Some(make_picker(self.tui.cover_protocol));
    tracing::info!(
        protocol = ?self.picker.as_ref().map(|p| p.protocol_type()),
        "cover renderer"
    );
    let result = self.run_loop(&mut terminal);
    let _ = crossterm::execute!(std::io::stdout(), event::DisableMouseCapture);
    ratatui::restore();
    result
}
```

Import `EnableMouseCapture` and `DisableMouseCapture` from `crossterm::event`.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p znicz-tui --test mouse --offline`

Expected: PASS including `a_drawn_library_row_is_clickable`

Run: `cargo test -p znicz-tui --offline`

Expected: PASS (existing key/render tests still pass; help/inspector signatures updated)

- [ ] **Step 5: Commit**

```bash
git add znicz-tui/src/app.rs znicz-tui/src/views znicz-tui/tests/mouse.rs
git commit -m "$(cat <<'EOF'
Record list hit rects while drawing and capture the mouse in the TUI run loop.

EOF
)"
```

---

### Task 8: Docs, version, ignored chrome

**Files:**
- Modify: `znicz-tui/tests/mouse.rs`
- Modify: `wiki/Architecture/TUI.md`
- Modify: `wiki/Issues.md`
- Modify: `wiki/Plans/Roadmap.md`
- Modify: `Cargo.toml`
- Modify: `Cargo.lock`

**Interfaces:**
- Consumes: existing `on_mouse` (transport/footer clicks already no-op if they miss hits)
- Produces: version **0.4.4**; #8 documented as done

- [ ] **Step 1: Write the failing test** for ignored chrome — append to `znicz-tui/tests/mouse.rs`

```rust
#[test]
fn a_click_on_the_transport_does_nothing() {
    let mut app = new_app();
    app.library.inject_albums_for_test(albums(3));
    app.hits = library_hits(3);
    app.on_mouse(left_click(10, 22));
    assert_eq!(app.library.selected_index(), Some(0));
    assert!(!app.queue_open);
    assert_eq!(app.modal, Modal::None);
}
```

This should already pass if Task 3 ignores unknown coordinates. Run it; if it passes immediately, keep it as a regression test (do not weaken it).

- [ ] **Step 2: Run the chrome test**

Run: `cargo test -p znicz-tui --test mouse a_click_on_the_transport_does_nothing --offline`

Expected: PASS (no-op). If it fails, do not map y≥transport into a list.

- [ ] **Step 3: Wiki and version**

In `wiki/Architecture/TUI.md`, after the paragraph that the loop waits for a key, add:

```markdown
While the TUI is up it also **captures the mouse**. A left click on a visible
library, queue, or overlay list row **selects** that row (it does not play or
open). The wheel moves the focused list one row, like `j` / `k`. Click outside
help, inspector, devices, playlists, radio, or a typing prompt closes or
cancels it, like `Esc`. Click the library pane's right-border column to open
the queue drawer; click the library (overlay) or that column (sheet) to close
it. Clicks on the transport, cover, footer, and toasts do nothing. Terminals
that never send mouse events keep working from the keyboard. `?` stays
keyboard-only.
```

In `wiki/Plans/Roadmap.md` **Later TUI**, remove the Mouse `#8` bullet (it is no longer parked).

In `wiki/Issues.md` **Open**, remove the `#8` bullet. Under **Closed**, add (newest first):

```markdown
### [#8 Mouse support in the TUI](https://github.com/eugene-chekan/znicz/issues/8)

- **Fixed:** 2026-09-03
- **Component:** `znicz-tui`
- **Status:** **Fixed** in 0.4.4

Left click selects a visible list row. Wheel steps the focused list. Click
outside closes overlays and cancels prompts. The library right-border column
toggles the queue drawer. Capture is on for the life of the TUI. Drag-to-reorder
and seek-bar clicks stay out.
```

In root `Cargo.toml` set `version = "0.4.4"`. Run `cargo check -p znicz --offline` so `Cargo.lock` picks up `0.4.4`.

Do **not** add mouse lines to `znicz-tui/src/keys.rs`.

- [ ] **Step 4: Run the suite**

Run: `cargo test -p znicz-tui --offline && cargo test -p znicz-core --offline`

Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add znicz-tui/tests/mouse.rs wiki/Architecture/TUI.md wiki/Issues.md wiki/Plans/Roadmap.md Cargo.toml Cargo.lock
git commit -m "$(cat <<'EOF'
Document TUI mouse support and ship it as 0.4.4.

EOF
)"
```

---

## Spec coverage

| Spec requirement | Task |
| --- | --- |
| Select-only library/queue click | 2, 3 |
| Click past last row ignored | 1, 2 |
| Wheel focused list / overlay / ignore help & prompt | 6 |
| Click-outside overlay | 4 |
| Prompt cancel, no typing | 5 |
| Queue toggle column, overlay close, sheet close | 3 |
| Overlay columns belong to queue | 3 (`library_pane` strip width) + 7 |
| Always capture | 7 |
| Hit map from draw / ListState offset | 7 |
| Transport/footer ignore | 8 |
| Wiki, #8 closed, keys.rs unchanged | 8 |
| Version z bump | 8 |
| No drag / seek / double-click / config | none (not built) |
