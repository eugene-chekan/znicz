# TUI Layered Layout Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the library the home screen, the queue a 36-column overlay drawer, and now-playing a two-line transport, matching `docs/superpowers/specs/2026-08-23-tui-layered-layout-design.md`.

**Architecture:** Keep the immediate-mode loop. Replace `Pane` (three full-screen homes) with `Focus` (Library | Queue), `queue_open`, and `Modal` (None | Help | Devices). A small `layout` module decides overlay vs full-width sheet and the library's visible strip width. Draw the library into the full list region, then the drawer, then transport, hints, modal, toasts.

**Tech Stack:** Rust, ratatui, crossterm, existing `znicz-tui` tests (`TestBackend`, `App::on_key`).

**Spec:** `docs/superpowers/specs/2026-08-23-tui-layered-layout-design.md`

Do not add a command palette, settings screen, signal inspector, three-column browser, or mouse.

Names below match the crate as it stands on `main`: `App::with_library`, `App::on_key`, `player.send_blocking`, `Command::QueueAdd`, `format::truncate`, `views::render`, `views::pane_block`, `views::inner_width`, `show_help`, `should_quit`.

---

## File map

| File | Responsibility |
| --- | --- |
| Create `znicz-tui/src/layout.rs` | Drawer geometry: overlay vs sheet, strip width, toast box |
| Modify `znicz-tui/src/app.rs` | `Focus`, `Modal`, `queue_open`, `list_width`, dispatch |
| Modify `znicz-tui/src/lib.rs` | Export `Focus` / `Modal` instead of `Pane` |
| Modify `znicz-tui/src/library_pane.rs` | `h_offset`, `pan`, `longest_middle`, clamp |
| Modify `znicz-tui/src/format.rs` | `pan` helper (skip offset, then truncate) |
| Modify `znicz-tui/src/views/mod.rs` | New frame: no tabs, library stage, overlays |
| Modify `znicz-tui/src/views/now_playing.rs` | Two-line transport (no header block) |
| Modify `znicz-tui/src/views/status.rs` | Hints only (never a toast) |
| Modify `znicz-tui/src/views/library.rs` | Rows composed for the visible strip |
| Modify `znicz-tui/src/views/queue.rs` | Drawer rect; empty hint without "press 2" |
| Modify `znicz-tui/src/views/devices.rs` | Centered modal (Clear + existing list) |
| Modify `znicz-tui/src/views/help.rs` | Keymap copy only |
| Modify `znicz-tui/src/keys.rs` | Drawer / pan / devices-modal bindings and hints |
| Modify `znicz-tui/src/toast.rs` | `visible()` — up to 3 newest |
| Modify `znicz-tui/tests/keys.rs` | Drawer, Tab, Esc, pan, unbound `1` `2` `3` |
| Modify `znicz-tui/tests/render.rs` | Library home, overlay, sheet, toasts vs hints |
| Modify `znicz-tui/tests/library_browse.rs` | Drop "start on queue" assumptions |
| Modify `znicz-tui/examples/preview.rs` | New frames |
| Modify `wiki/Architecture/TUI.md` | Layout diagram and modules |
| Modify `wiki/Plans/Roadmap.md` | Phase 2.5 bullets |

`views::render` takes `&mut App` so it can record `list_width` for sheet/Tab logic in key tests.

---

### Task 1: Drawer geometry

**Files:**
- Create: `znicz-tui/src/layout.rs`
- Modify: `znicz-tui/src/lib.rs` (add `pub mod layout;`)

- [ ] **Step 1: Write the failing tests in `layout.rs`**

```rust
//! Overlay vs full-width sheet, and the library strip the drawer leaves open.

use ratatui::layout::Rect;

/// Width of the side overlay, in columns.
pub const DRAWER_WIDTH: u16 = 36;
/// If the leftover library strip would be narrower than this, use a sheet.
pub const MIN_STRIP: u16 = 40;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Drawer {
    Closed,
    Overlay(Rect),
    Sheet(Rect),
}

pub fn drawer(list: Rect, open: bool) -> Drawer {
    unimplemented!()
}

pub fn strip_width(list: Rect, open: bool) -> u16 {
    unimplemented!()
}

/// Inner columns for library row text (strip minus the left border).
pub fn strip_inner(list: Rect, open: bool) -> usize {
    unimplemented!()
}

pub fn is_sheet(list_width: u16, open: bool) -> bool {
    unimplemented!()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn list(width: u16) -> Rect {
        Rect::new(0, 0, width, 20)
    }

    #[test]
    fn a_wide_list_gets_a_36_column_overlay_on_the_right() {
        let list = list(100);
        match drawer(list, true) {
            Drawer::Overlay(rect) => {
                assert_eq!(rect.width, 36);
                assert_eq!(rect.x, 64);
                assert_eq!(rect.height, 20);
            }
            other => panic!("expected overlay, got {other:?}"),
        }
        assert_eq!(strip_width(list, true), 64);
    }

    #[test]
    fn a_narrow_list_becomes_a_full_width_sheet() {
        // 36 + 40 = 76; at 76 or below the strip would be too thin.
        let list = list(76);
        match drawer(list, true) {
            Drawer::Sheet(rect) => assert_eq!(rect, list),
            other => panic!("expected sheet, got {other:?}"),
        }
        assert_eq!(strip_width(list, true), 76);
        assert!(is_sheet(76, true));
        assert!(!is_sheet(77, true));
    }

    #[test]
    fn a_closed_drawer_leaves_the_full_list() {
        let list = list(80);
        assert_eq!(drawer(list, false), Drawer::Closed);
        assert_eq!(strip_width(list, false), 80);
        assert!(!is_sheet(40, false));
    }

    #[test]
    fn strip_inner_counts_characters_inside_the_visible_library() {
        let wide = list(100);
        // overlay: visible 64, minus left border → 63
        assert_eq!(strip_inner(wide, true), 63);
        // closed: inner_width = 80 - 2 for a 80-wide pane; here 100 - 2
        assert_eq!(strip_inner(wide, false), 98);
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p znicz-tui layout --lib`

Expected: FAIL (`unimplemented!` or module missing)

- [ ] **Step 3: Implement `layout.rs`**

```rust
//! Overlay vs full-width sheet, and the library strip the drawer leaves open.

use ratatui::layout::Rect;

pub const DRAWER_WIDTH: u16 = 36;
pub const MIN_STRIP: u16 = 40;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Drawer {
    Closed,
    Overlay(Rect),
    Sheet(Rect),
}

pub fn drawer(list: Rect, open: bool) -> Drawer {
    if !open {
        return Drawer::Closed;
    }
    if list.width <= DRAWER_WIDTH.saturating_add(MIN_STRIP) {
        Drawer::Sheet(list)
    } else {
        Drawer::Overlay(Rect {
            x: list.x + list.width - DRAWER_WIDTH,
            y: list.y,
            width: DRAWER_WIDTH,
            height: list.height,
        })
    }
}

pub fn strip_width(list: Rect, open: bool) -> u16 {
    match drawer(list, open) {
        Drawer::Closed | Drawer::Sheet(_) => list.width,
        Drawer::Overlay(rect) => list.width.saturating_sub(rect.width),
    }
}

pub fn strip_inner(list: Rect, open: bool) -> usize {
    match drawer(list, open) {
        Drawer::Overlay(_) => strip_width(list, true).saturating_sub(1) as usize,
        _ => list.width.saturating_sub(2) as usize,
    }
}

pub fn is_sheet(list_width: u16, open: bool) -> bool {
    open && list_width <= DRAWER_WIDTH.saturating_add(MIN_STRIP)
}

/// Bottom-right boxes for up to three toast lines inside `list`.
pub fn toast_areas(list: Rect, count: u16, line_width: u16) -> Vec<Rect> {
    let count = count.min(3);
    if count == 0 || list.width == 0 || list.height == 0 {
        return Vec::new();
    }
    let width = line_width.min(list.width).max(1);
    let x = list.x + list.width.saturating_sub(width);
    (0..count)
        .map(|i| {
            let y = list.y + list.height.saturating_sub(1).saturating_sub(i);
            Rect {
                x,
                y,
                width,
                height: 1,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    // keep the tests from Step 1, plus:

    #[test]
    fn toast_areas_stack_up_from_the_bottom_right() {
        let list = Rect::new(0, 0, 80, 10);
        let areas = toast_areas(list, 3, 32);
        assert_eq!(areas.len(), 3);
        assert_eq!(areas[0], Rect::new(48, 9, 32, 1));
        assert_eq!(areas[1], Rect::new(48, 8, 32, 1));
        assert_eq!(areas[2], Rect::new(48, 7, 32, 1));
    }
}
```

Add `pub mod layout;` to `znicz-tui/src/lib.rs`.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p znicz-tui layout --lib`

Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add znicz-tui/src/layout.rs znicz-tui/src/lib.rs
git commit -m "Add overlay vs sheet geometry for the queue drawer."
```

---

### Task 2: `Focus`, `Modal`, and library home

**Files:**
- Modify: `znicz-tui/src/app.rs`
- Modify: `znicz-tui/src/lib.rs`
- Modify: every file that names `Pane` (views, tests, example) just enough to compile

- [ ] **Step 1: Write failing unit tests at the bottom of `app.rs`**

Replace the `Pane::next` tests. The real failing tests belong in `tests/keys.rs` (next step). Do not add placeholder tests in `app.rs`.

```rust
#[test]
fn the_player_opens_on_the_library() {
    let app = new_app();
    assert_eq!(app.focus, Focus::Library);
    assert!(!app.queue_open);
    assert_eq!(app.modal, Modal::None);
}

#[test]
fn bracket_opens_the_queue_drawer_and_closes_it() {
    let mut app = new_app();
    press_char(&mut app, ']');
    assert!(app.queue_open);
    assert_eq!(app.focus, Focus::Queue);
    press_char(&mut app, ']');
    assert!(!app.queue_open);
    assert_eq!(app.focus, Focus::Library);
}

#[test]
fn tab_opens_the_drawer_then_swaps_focus_without_closing_it() {
    let mut app = new_app();
    press(&mut app, KeyCode::Tab);
    assert!(app.queue_open);
    assert_eq!(app.focus, Focus::Queue);
    press(&mut app, KeyCode::Tab);
    assert!(app.queue_open);
    assert_eq!(app.focus, Focus::Library);
    press(&mut app, KeyCode::BackTab);
    assert_eq!(app.focus, Focus::Queue);
}

#[test]
fn backtab_does_nothing_while_the_drawer_is_closed() {
    let mut app = new_app();
    press(&mut app, KeyCode::BackTab);
    assert!(!app.queue_open);
    assert_eq!(app.focus, Focus::Library);
}

#[test]
fn numbers_no_longer_switch_homes() {
    let mut app = new_app();
    press_char(&mut app, '1');
    press_char(&mut app, '2');
    press_char(&mut app, '3');
    assert_eq!(app.focus, Focus::Library);
    assert!(!app.queue_open);
    assert_eq!(app.modal, Modal::None);
}

#[test]
fn comma_toggles_the_devices_modal() {
    let mut app = new_app();
    press_char(&mut app, ',');
    assert_eq!(app.modal, Modal::Devices);
    press(&mut app, KeyCode::Esc);
    assert_eq!(app.modal, Modal::None);
}

#[test]
fn question_mark_still_opens_help_and_the_next_key_only_closes_it() {
    let mut app = new_app();
    press_char(&mut app, '?');
    assert_eq!(app.modal, Modal::Help);
    press_char(&mut app, 'q');
    assert_eq!(app.modal, Modal::None);
    assert!(!app.should_quit);
}

#[test]
fn esc_closes_search_then_devices_then_the_drawer() {
    let mut app = new_app();
    // search
    press_char(&mut app, '/');
    assert!(app.library.is_typing());
    press(&mut app, KeyCode::Esc);
    assert!(!app.library.is_typing());
    assert_eq!(app.focus, Focus::Library);

    // devices
    press_char(&mut app, ',');
    press(&mut app, KeyCode::Esc);
    assert_eq!(app.modal, Modal::None);

    // drawer
    press_char(&mut app, ']');
    press(&mut app, KeyCode::Esc);
    assert!(!app.queue_open);
    assert_eq!(app.focus, Focus::Library);
}
```

Delete `numbers_and_tab_both_switch_panes`. Change the unbound-keys assertion from `app.pane == Pane::Queue` to `app.focus == Focus::Library`. Change help tests from `show_help` to `modal == Modal::Help`.

- [ ] **Step 2: Run the key tests to verify they fail**

Run: `cargo test -p znicz-tui --test keys`

Expected: FAIL (no `Focus`, Tab still cycles `Pane`)

- [ ] **Step 3: Replace `Pane` with the new state**

In `app.rs`:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Library,
    Queue,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Modal {
    None,
    Help,
    Devices,
}

pub struct App {
    pub player: PlayerHandle,
    pub focus: Focus,
    pub queue_open: bool,
    pub modal: Modal,
    pub list_width: u16,
    pub queue_cursor: Cursor,
    pub library: LibraryPane,
    pub devices: Vec<AudioDeviceInfo>,
    pub device_cursor: Cursor,
    pub meta: MetaCache,
    pub toasts: Toasts,
    pub should_quit: bool,
}
```

`with_library` sets `focus: Focus::Library`, `queue_open: false`, `modal: Modal::None`, `list_width: 80`.

Remove `show_help` and `pane`.

Dispatch:

```rust
pub fn on_key(&mut self, key: KeyEvent) {
    if self.modal == Modal::Help {
        self.modal = Modal::None;
        return;
    }
    if self.library.is_typing() {
        self.on_search_key(key);
        return;
    }
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        self.on_control_key(key.code);
        return;
    }
    if self.on_escape(key) {
        return;
    }
    if self.on_global_key(key) {
        return;
    }
    self.on_focus_key(key);
}

fn on_escape(&mut self, key: KeyEvent) -> bool {
    if key.code != KeyCode::Esc {
        return false;
    }
    if self.modal == Modal::Devices {
        self.modal = Modal::None;
        return true;
    }
    if self.focus == Focus::Queue {
        self.close_queue();
        return true;
    }
    self.library.back();
    true
}

fn open_queue(&mut self) {
    self.queue_open = true;
    self.focus = Focus::Queue;
}

fn close_queue(&mut self) {
    self.queue_open = false;
    self.focus = Focus::Library;
}

fn tab_forward(&mut self) {
    if !self.queue_open {
        self.open_queue();
        return;
    }
    if crate::layout::is_sheet(self.list_width, true) {
        return;
    }
    self.focus = match self.focus {
        Focus::Library => Focus::Queue,
        Focus::Queue => Focus::Library,
    };
}

fn tab_back(&mut self) {
    if !self.queue_open {
        return;
    }
    if crate::layout::is_sheet(self.list_width, true) {
        return;
    }
    self.focus = match self.focus {
        Focus::Library => Focus::Queue,
        Focus::Queue => Focus::Library,
    };
}
```

In `on_global_key`:

- `?` → `self.modal = Modal::Help`
- `,` → toggle `Modal::Devices` / `Modal::None` (opening Devices replaces Help only via the Help early-return)
- `]` → if `queue_open` { `close_queue()` } else { `open_queue()` }
- `Tab` / `BackTab` → `tab_forward` / `tab_back`
- `<` → `self.library.pan(-1, self.title_slot())` (stub pan in Task 3; for now a no-op method)
- `>` → `self.library.pan(1, self.title_slot())`
- Drop `1` `2` `3`
- `j`/`k`/page/`g`/`G`: if `modal == Devices` move `device_cursor`, else move focused list

```rust
fn title_slot(&self) -> usize {
    let list = ratatui::layout::Rect::new(0, 0, self.list_width.max(1), 10);
    crate::layout::strip_inner(list, self.queue_open).saturating_sub(8)
}

fn on_focus_key(&mut self, key: KeyEvent) {
    if self.modal == Modal::Devices {
        self.on_devices_key(key);
        return;
    }
    match self.focus {
        Focus::Queue => self.on_queue_key(key),
        Focus::Library => self.on_library_key(key),
    }
}
```

Remove `Esc` from `on_library_key` (handled in `on_escape`). Keep `/`, `R`, Enter, `a`, `A`.

`list_len` / `step` / `page` / `go_first` / `go_last`: Devices when `modal == Devices`, else Library vs Queue by `focus`.

Export `Focus` and `Modal` from `lib.rs`. Remove `Pane`.

Sweep compile errors: `app.pane == Pane::Library` → `app.focus == Focus::Library`; queue focused → `app.focus == Focus::Queue && app.queue_open`; devices focused → `app.modal == Modal::Devices`; `show_help` → `modal == Modal::Help`.

In `tests/library_browse.rs` delete `app.pane = Pane::Library` (already home).

In `tests/render.rs` stop looping `Pane::ALL`. Draw the default app; also `app.queue_open = true; app.focus = Focus::Queue;` and `app.modal = Modal::Devices`.

Add a temporary `LibraryPane::pan(&mut self, _delta: isize, _slot: usize) {}` so dispatch compiles.

- [ ] **Step 4: Run tests**

Run:

```bash
cargo test -p znicz-tui --test keys
cargo test -p znicz-tui --lib
cargo test -p znicz-tui --test library_browse
```

Expected: key tests PASS. Render tests may still expect a tab bar / "Queue is empty" on a fresh app — if they fail, update assertions in this task:

- Fresh app at 90×24 contains `Library`, does **not** contain a tab bar like `1 Queue`
- Empty library still mentions `scan`
- `Queue is empty` only when the drawer is open
- Help overlay still contains `Keys`

- [ ] **Step 5: Commit**

```bash
git add znicz-tui
git commit -m "Make the library the home screen and the queue a toggleable drawer."
```

---

### Task 3: Horizontal pan

**Files:**
- Modify: `znicz-tui/src/format.rs`
- Modify: `znicz-tui/src/library_pane.rs`
- Modify: `znicz-tui/src/views/library.rs`
- Modify: `znicz-tui/src/app.rs` (`title_slot` used by `<` `>`)
- Test: `znicz-tui/tests/keys.rs`

- [ ] **Step 1: Write failing tests**

In `format.rs`:

```rust
#[test]
fn pan_skips_then_truncates() {
    assert_eq!(pan("abcdefghij", 0, 5), "abcd…");
    assert_eq!(pan("abcdefghij", 2, 5), "cdef…");
    assert_eq!(pan("abcdefghij", 8, 5), "ij");
    assert_eq!(pan("short", 0, 10), "short");
}
```

In `library_pane.rs`:

```rust
#[test]
fn pan_clamps_to_the_longest_middle() {
    let mut pane = LibraryPane::new(None);
    pane.pan(5, 4);
    assert_eq!(pane.h_offset(), 0, "empty list has nothing to pan");
}
```

In `tests/keys.rs`:

```rust
#[test]
fn angle_brackets_pan_the_library_and_h_still_seeks() {
    let mut app = new_app();
    press_char(&mut app, '>');
    press_char(&mut app, '>');
    // no crash; offset is clamped on an empty library
    assert_eq!(app.library.h_offset(), 0);

    press_char(&mut app, 'h');
    assert_eq!(app.state().position.as_secs(), 0);
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p znicz-tui --lib format::tests::pan_skips`

Expected: FAIL (`pan` missing)

- [ ] **Step 3: Implement pan**

`format.rs`:

```rust
pub fn pan(text: &str, offset: usize, width: usize) -> String {
    let sliced: String = text.chars().skip(offset).collect();
    truncate(&sliced, width)
}
```

`library_pane.rs` — add `h_offset: usize` to the struct (init `0`). Add:

```rust
pub fn h_offset(&self) -> usize {
    self.h_offset
}

pub fn clamp_pan(&mut self, slot: usize) {
    let max = self.longest_middle().saturating_sub(slot);
    self.h_offset = self.h_offset.min(max);
}

pub fn pan(&mut self, delta: isize, slot: usize) {
    let max = self.longest_middle().saturating_sub(slot) as isize;
    let next = self.h_offset as isize + delta;
    self.h_offset = next.clamp(0, max.max(0)) as usize;
}

pub fn longest_middle(&self) -> usize {
    match self.mode {
        Mode::Albums => self
            .albums
            .iter()
            .map(album_middle)
            .map(|s| s.chars().count())
            .max()
            .unwrap_or(0),
        _ => self
            .tracks
            .iter()
            .map(track_middle)
            .map(|s| s.chars().count())
            .max()
            .unwrap_or(0),
    }
}

pub fn album_middle(album: &AlbumSummary) -> String {
    let year = album.year.map(|y| format!(" ({y})")).unwrap_or_default();
    let artist = album.album_artist.as_deref().unwrap_or("Unknown artist");
    format!("{}{year} — {artist}", album.album)
}

pub fn track_middle(track: &Track) -> String {
    match track.artist.as_deref() {
        Some(artist) => format!("{} — {artist}", track.title),
        None => track.title.clone(),
    }
}
```

Call `clamp_pan(self.title_slot())` from `close_queue` and at the end of `views::render` (after `list_width` is stored).

`views/library.rs` — compose rows for `strip_inner`, not the full pane:

```rust
let strip = crate::layout::strip_inner(list_area, app.queue_open);
let offset = app.library.h_offset();
```

Track row: pin `number` on the left, `time` on the right, middle = `format::pan(&track_middle(track), offset, strip - fixed)`. Same for albums (`album_middle`).

Do not auto-pan when the cursor moves.

`App::title_slot`: `strip_inner` minus a conservative 8 for number + duration so `<` `>` clamp using the same slot as drawing. After library render exists, prefer storing `title_slot` on `App` during render (same pattern as `list_width`).

```rust
pub title_slot: usize, // set in views::render from the library list_area
```

- [ ] **Step 4: Run tests**

Run:

```bash
cargo test -p znicz-tui --lib
cargo test -p znicz-tui --test keys angle
```

Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add znicz-tui/src/format.rs znicz-tui/src/library_pane.rs znicz-tui/src/views/library.rs znicz-tui/src/app.rs znicz-tui/tests/keys.rs
git commit -m "Pan long library titles with < and > without moving seek."
```

---

### Task 4: Frame, transport, drawer draw order

**Files:**
- Modify: `znicz-tui/src/views/mod.rs`
- Modify: `znicz-tui/src/views/now_playing.rs`
- Modify: `znicz-tui/src/views/status.rs`
- Modify: `znicz-tui/src/views/queue.rs`
- Modify: `znicz-tui/tests/render.rs`

- [ ] **Step 1: Write failing render tests**

```rust
#[test]
fn a_fresh_screen_is_the_library_with_no_tab_bar() {
    let app = App::with_library(player(), None);
    let screen = draw(&app, 90, 24);
    assert!(screen.contains("Library"), "{screen}");
    assert!(
        !screen.contains("1 Queue"),
        "tab bar must be gone:\n{screen}"
    );
}

#[test]
fn the_queue_drawer_covers_the_right_on_a_wide_screen() {
    let mut app = App::with_library(player(), None);
    app.queue_open = true;
    app.focus = Focus::Queue;
    let screen = draw(&app, 100, 24);
    assert!(screen.contains("Queue"), "{screen}");
    assert!(screen.contains("Library"), "library stays underneath:\n{screen}");
}

#[test]
fn a_narrow_screen_opens_the_queue_as_a_sheet() {
    let mut app = App::with_library(player(), None);
    app.queue_open = true;
    app.focus = Focus::Queue;
    let screen = draw(&app, 60, 20);
    assert!(screen.contains("Queue"), "{screen}");
}

#[test]
fn transport_sits_at_the_bottom_and_drops_the_signal_line_when_short() {
    let app = App::with_library(player(), None);
    let tall = draw(&app, 90, 24);
    assert!(tall.contains("stopped") || tall.contains("Nothing playing"), "{tall}");
    let short = draw(&app, 90, 16);
    assert_eq!(short.lines().count(), 16);
}

#[test]
fn hints_stay_when_a_toast_is_showing() {
    let mut app = App::with_library(player(), None);
    app.toasts.error("could not open device");
    let screen = draw(&app, 90, 24);
    assert!(screen.contains("could not open device"), "{screen}");
    assert!(
        screen.contains("? help") || screen.contains("search"),
        "hints must remain:\n{screen}"
    );
}
```

Change `draw` to take `&mut App` (or keep `&App` and set `list_width` on the app before drawing). Prefer `&mut App` so render can store `list_width`.

Remove `every_pane_draws_at_every_size` looping `Pane::ALL`. Replace with: for each size, draw library home, drawer open, devices modal, help — none panic.

Remove `messages_replace_the_hint_line_when_present` (replaced by `hints_stay_when_a_toast_is_showing`).

- [ ] **Step 2: Run render tests to verify they fail**

Run: `cargo test -p znicz-tui --test render`

Expected: FAIL (tabs still drawn, toasts still steal the footer)

- [ ] **Step 3: Implement the frame**

`views/mod.rs` `render`:

```rust
pub fn render(frame: &mut Frame, app: &mut App, state: &PlayerState) {
    let area = frame.area();
    let compact = area.height < COMPACT_HEIGHT;
    let transport = if compact { 1 } else { 2 };

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(3),
            Constraint::Length(transport),
            Constraint::Length(1),
        ])
        .split(area);

    let list = chunks[0];
    app.list_width = list.width;
    app.title_slot = crate::layout::strip_inner(list, app.queue_open).saturating_sub(8);
    app.library.clamp_pan(app.title_slot);

    library::render(frame, list, app);

    match crate::layout::drawer(list, app.queue_open) {
        crate::layout::Drawer::Overlay(rect) | crate::layout::Drawer::Sheet(rect) => {
            frame.render_widget(ratatui::widgets::Clear, rect);
            queue::render(frame, rect, app, state);
        }
        crate::layout::Drawer::Closed => {}
    }

    now_playing::render_transport(frame, chunks[1], state, !compact);
    status::render_footer(frame, chunks[2], app);

    match app.modal {
        Modal::Help => help::render(frame, area),
        Modal::Devices => devices::render_modal(frame, area, app, state),
        Modal::None => {}
    }

    render_toasts(frame, list, app);
}
```

Remove `render_tabs`, `TINY_HEIGHT`, and `status::render_bar`.

`now_playing.rs` — add `render_transport(frame, area, state, show_signal)`:

- If `area.height >= 2 && show_signal`: line 0 = chrome, line 1 = existing `signal_line` (no block).
- Else: one chrome line.

Chrome line (never drop play symbol or elapsed/total):

1. `status_symbol` + space + truncated title
2. `artist — album` if it still fits
3. seek bar in remaining space
4. times
5. volume bar + percent, or `muted`
6. repeat label and `shuffle` if they fit; drop these first when tight

Reuse `seek_line`'s bar math. Do not draw a `Now Playing` block.

`status.rs` — `render_footer` always uses `keys::hints(...)`. Map focus/modal:

```rust
let pane = match app.modal {
    Modal::Devices => "Devices",
    _ => match app.focus {
        Focus::Queue => "Queue",
        Focus::Library => "Library",
    },
};
```

`queue.rs` empty placeholder: `Queue is empty. Add tracks from the library with a.` (no "press 2"). Focused when `app.focus == Focus::Queue`.

`devices.rs` — keep list rendering; add `render_modal` that `Clear`s a centered rect (~70% of the frame, min 40×8) then calls `render`.

`render_toasts`:

```rust
fn render_toasts(frame: &mut Frame, list: Rect, app: &App) {
    let shown: Vec<_> = app.toasts.recent().iter().take(3).collect();
    let max_width = ((list.width as usize) * 40 / 100).clamp(8, 40) as u16;
    let areas = crate::layout::toast_areas(list, shown.len() as u16, max_width);
    for (toast, area) in shown.iter().zip(areas) {
        frame.render_widget(ratatui::widgets::Clear, area);
        let style = match toast.level { /* info/warn/error as today */ };
        let text = format::truncate(&toast.text, area.width as usize);
        frame.render_widget(Paragraph::new(Span::styled(text, style)), area);
    }
}
```

Add `Toasts::visible(&self) -> &[Toast] { let n = self.recent().len().min(3); &self.recent()[..n] }` if you prefer that over `take(3)`.

Update every `views::render(frame, app, &state)` call site to pass `&mut app`.

- [ ] **Step 4: Run tests**

Run:

```bash
cargo test -p znicz-tui --test render
cargo test -p znicz-tui --test keys
cargo test -p znicz-tui --lib
```

Expected: PASS. No panic at 10×3.

- [ ] **Step 5: Commit**

```bash
git add znicz-tui
git commit -m "Draw the library full width with a queue overlay and a two-line transport."
```

---

### Task 5: Key tables, preview, wiki

**Files:**
- Modify: `znicz-tui/src/keys.rs`
- Modify: `znicz-tui/src/views/help.rs` (section titles only if needed)
- Modify: `znicz-tui/examples/preview.rs`
- Modify: `wiki/Architecture/TUI.md`
- Modify: `wiki/Plans/Roadmap.md`

- [ ] **Step 1: Update `keys.rs` (tests already require hint `?`)**

```rust
pub const GLOBAL: &[Binding] = &[
    b("Space", "play / pause"),
    b("s", "stop"),
    b("n / N", "next / previous track"),
    b("→ / l", "seek forward 5s"),
    b("← / h", "seek back 5s"),
    b("L / H", "seek 30s"),
    b("+ / -", "volume up / down"),
    b("m", "mute"),
    b("r", "repeat: off, all, one"),
    b("z", "shuffle"),
    b("]", "open / close queue"),
    b("Tab", "library ↔ queue"),
    b("< / >", "pan library titles"),
    b(",", "devices"),
    b("?", "this help"),
    b("q", "quit"),
];

pub fn hints(pane: &str) -> &'static str {
    match pane {
        "Queue" => "Enter play · d remove · C clear · o jump · ] close · ? help",
        "Library" => "/ search · a add · ] queue · < > pan · , devices · ? help",
        "Devices" => "Enter select · R rescan · Esc close · ? help",
        _ => "] queue · ? help · q quit",
    }
}
```

Keep `NAVIGATION`, `QUEUE`, `LIBRARY`, `DEVICES` tables; add `,` / Esc to DEVICES text.

- [ ] **Step 2: Run key-table unit tests**

Run: `cargo test -p znicz-tui --lib keys`

Expected: PASS

- [ ] **Step 3: Preview frames**

`examples/preview.rs`:

1. Library home (default)
2. Drawer open (`queue_open = true`, `focus = Queue`)
3. Devices modal
4. Help
5. Small window 48×14

Remove tab-bar captions. Keep the bit-perfect vs resampled transport demos.

Run: `cargo run -p znicz-tui --example preview -- 96 28`

Expected: four readable frames, library first, queue on the right when open, no `1 Queue │ 2 Library`.

- [ ] **Step 4: Wiki**

`wiki/Architecture/TUI.md` layout diagram: library stage, overlay queue, two-line transport, hints, toasts in the list corner. Drop the tab bar and six-row Now Playing box. Height table: 20+ both transport lines; 12–19 drop signal; under 12 still library + one transport line + hints.

Modules table: add `layout.rs`; `now_playing.rs` is the transport; `status.rs` is hints only.

`wiki/Plans/Roadmap.md` Phase 2.5: library home, queue drawer, overlay devices, floating toasts — not three tabs.

- [ ] **Step 5: Full crate tests, then commit**

Run:

```bash
cargo test -p znicz-tui
cargo fmt --all
```

Expected: PASS

```bash
git add znicz-tui/src/keys.rs znicz-tui/examples/preview.rs wiki/Architecture/TUI.md wiki/Plans/Roadmap.md
git commit -m "Document the layered TUI and refresh help, hints, and preview."
```

---

## Spec coverage

| Spec section | Task |
| --- | --- |
| Library home, no tab bar | 2, 4 |
| Overlay drawer 36 cols | 1, 4 |
| Full-width sheet when strip < 40 | 1, 2 (Tab no-op), 4 |
| `]` / Tab / BackTab | 2 |
| `1` `2` `3` unbound | 2 |
| Queue keys only when queue focused | 2 |
| Library keys only when library focused | 2 |
| Horizontal pan, pinned number/time | 3 |
| `<` `>` pan even with queue focused; `h`/`l` seek | 3 |
| No auto-pan on cursor move | 3 |
| Modal Help / Devices, one at a time | 2, 4 |
| Help: any key closes, not executed | 2 |
| Esc stack | 2 |
| Two-line transport, device name off the bar | 4 |
| Signal line dropped under height 20 | 4 |
| Toasts ≤3, hints never stolen | 4 |
| Draw order | 4 |
| `keys.rs` tables | 5 |
| Tests + preview | 2–5 |
| Wiki | 5 |
| Non-goals (palette, settings, inspector, 3-col, mouse) | not scheduled |

## Type names (keep stable)

- `Focus::{Library, Queue}`
- `Modal::{None, Help, Devices}`
- `layout::Drawer::{Closed, Overlay, Sheet}`
- `layout::DRAWER_WIDTH = 36`, `MIN_STRIP = 40`
- `App.queue_open: bool`, `App.list_width: u16`, `App.title_slot: usize`
- `LibraryPane.h_offset()`, `pan(delta, slot)`, `clamp_pan(slot)`
- `format::pan(text, offset, width)`
