# Mouse Footer and Explicit Close Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Stop outside-click dismiss for modals and the queue drawer; add a top-right clickable X to close them; make each footer hint segment a click target that runs the matching key action.

**Architecture:** Extend `HitMap` with `close` and `footer_hints`. Draw records those rects. `on_left_click` prefers footer hits (synthesize `KeyEvent` → `on_key`), then `close`, then list select — and no longer closes overlays on outside click. Queue border column opens only.

**Tech Stack:** Ratatui 0.30 `Rect` / `Block` titles, crossterm `KeyEvent` / `MouseEvent`, existing `znicz-tui` `App::on_key` / `on_mouse`.

**Spec:** `docs/superpowers/specs/2026-09-04-mouse-footer-and-close-design.md`

## Global Constraints

- Version **0.4.4 → 0.4.5** in the same change as the feature (compatible addition).
- Select-only on list rows unchanged (click never plays / opens album / applies device).
- Outside-click **does not** close help, inspector, devices, playlists, radio, or the queue drawer.
- Typing prompts still cancel on outside click.
- Queue toggle column **opens only**; close via X, `]`, or footer `] close`.
- Footer clicks synthesize keys into `on_key` (including Alt+Right for the pan hint). No new mouse lines in `keys.rs` / `?` help tables.
- Out of scope: seek/cover/toast clicks, double-click activate, action-enum refactor, `ratatui-interact`.
- Wiki matches the code in the same change.

## File map

| File | Role |
| --- | --- |
| Modify `znicz-tui/src/hit.rs` | `FooterHit`; `HitMap.close` / `footer_hints`; drop `Copy` |
| Create `znicz-tui/src/footer_hits.rs` | Parse hint segments → `KeyEvent`; layout rects in the footer row |
| Modify `znicz-tui/src/lib.rs` | `mod footer_hits` |
| Modify `znicz-tui/src/app.rs` | Click priority: footer → close → lists; no overlay outside-close; toggle open-only |
| Modify `znicz-tui/src/views/mod.rs` | Shared `close_button_rect` / title helper for X |
| Modify `znicz-tui/src/views/{help,inspector,devices,playlists,radio,queue}.rs` | Paint X; set `hits.close` |
| Modify `znicz-tui/src/views/status.rs` | Layout footer hits while drawing |
| Modify `znicz-tui/tests/mouse.rs` | Flip outside-close tests; add close / footer / open-only cases |
| Modify `wiki/Architecture/TUI.md`, `wiki/Domain/TUI-Players.md` | Match new behaviour |
| Modify `Cargo.toml` + `Cargo.lock` | `0.4.5` |

---

### Task 1: Hit map — `close` and `footer_hints`

**Files:**
- Modify: `znicz-tui/src/hit.rs`
- Test: unit tests in `znicz-tui/src/hit.rs`

**Interfaces:**
- Consumes: `ratatui::layout::Rect`, `crossterm::event::KeyEvent`
- Produces: `pub struct FooterHit { pub rect: Rect, pub key: KeyEvent }`; `HitMap` fields `pub close: Option<Rect>`, `pub footer_hints: Vec<FooterHit>` (and existing fields). `HitMap` is `Default + Clone + PartialEq` but **not** `Copy`.

- [ ] **Step 1: Write the failing tests**

Add to `znicz-tui/src/hit.rs` `#[cfg(test)]` (keep existing `ListHit` tests):

```rust
#[test]
fn hit_map_default_has_no_close_or_footer_hints() {
    let hits = HitMap::default();
    assert!(hits.close.is_none());
    assert!(hits.footer_hints.is_empty());
}

#[test]
fn footer_hit_stores_rect_and_key() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    let hit = FooterHit {
        rect: Rect::new(0, 23, 7, 1),
        key: KeyEvent::new(KeyCode::Char('?'), KeyModifiers::NONE),
    };
    assert_eq!(hit.rect.width, 7);
    assert_eq!(hit.key.code, KeyCode::Char('?'));
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p znicz-tui hit::tests -- --nocapture`
Expected: compile error (`FooterHit` / `close` / `footer_hints` missing) or FAIL on assertions if stubs exist without fields.

- [ ] **Step 3: Minimal implementation**

Replace `HitMap` derives and fields:

```rust
use crossterm::event::KeyEvent;
use ratatui::layout::{Position, Rect};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FooterHit {
    pub rect: Rect,
    pub key: KeyEvent,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct HitMap {
    pub library: Option<ListHit>,
    pub queue: Option<ListHit>,
    pub overlay: Option<Rect>,
    pub overlay_list: Option<ListHit>,
    pub queue_toggle: Option<Rect>,
    pub library_pane: Option<Rect>,
    pub search_prompt: Option<Rect>,
    pub close: Option<Rect>,
    pub footer_hints: Vec<FooterHit>,
}
```

Keep `ListHit` as today. Fix any `Copy` uses of `HitMap` in the crate (clone or reborrow). `library_hits` helpers in tests that use `..HitMap::default()` keep working.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p znicz-tui hit::tests -- --nocapture`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add znicz-tui/src/hit.rs
git commit -m "Add close and footer hit fields to the TUI hit map."
```

---

### Task 2: Footer hint segment → `KeyEvent`

**Files:**
- Create: `znicz-tui/src/footer_hits.rs`
- Modify: `znicz-tui/src/lib.rs` (`mod footer_hits;`)

**Interfaces:**
- Consumes: hint segment strings (as produced by `keys::hints` / typing footer)
- Produces: `pub fn key_for_hint_segment(segment: &str) -> Option<KeyEvent>`; `pub fn layout_footer_hits(area: Rect, line: &str) -> Vec<FooterHit>`

- [ ] **Step 1: Write the failing tests** in `znicz-tui/src/footer_hits.rs`

```rust
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::Rect;

use crate::hit::FooterHit;

pub fn key_for_hint_segment(segment: &str) -> Option<KeyEvent> {
    todo!("task 2")
}

pub fn layout_footer_hits(area: Rect, line: &str) -> Vec<FooterHit> {
    todo!("task 2")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn maps_common_footer_segments() {
        assert_eq!(
            key_for_hint_segment("? help").map(|k| k.code),
            Some(KeyCode::Char('?'))
        );
        assert_eq!(
            key_for_hint_segment("Esc close").map(|k| k.code),
            Some(KeyCode::Esc)
        );
        assert_eq!(
            key_for_hint_segment("i / Esc close").map(|k| k.code),
            Some(KeyCode::Esc)
        );
        assert_eq!(
            key_for_hint_segment("Enter play").map(|k| k.code),
            Some(KeyCode::Enter)
        );
        assert_eq!(
            key_for_hint_segment("Space pause").map(|k| k.code),
            Some(KeyCode::Char(' '))
        );
        assert_eq!(
            key_for_hint_segment("a add").map(|k| k.code),
            Some(KeyCode::Char('a'))
        );
        assert_eq!(
            key_for_hint_segment("C clear").map(|k| k.code),
            Some(KeyCode::Char('C'))
        );
        assert_eq!(
            key_for_hint_segment("/ search").map(|k| k.code),
            Some(KeyCode::Char('/'))
        );
        assert_eq!(
            key_for_hint_segment("] queue").map(|k| k.code),
            Some(KeyCode::Char(']'))
        );
        assert_eq!(
            key_for_hint_segment(", devices").map(|k| k.code),
            Some(KeyCode::Char(','))
        );
        assert_eq!(
            key_for_hint_segment("P").map(|k| k.code),
            Some(KeyCode::Char('P'))
        );
        assert_eq!(
            key_for_hint_segment("R").map(|k| k.code),
            Some(KeyCode::Char('R'))
        );
        let pan = key_for_hint_segment("Alt-← / Alt-→ pan").expect("pan");
        assert_eq!(pan.code, KeyCode::Right);
        assert!(pan.modifiers.contains(KeyModifiers::ALT));
        assert!(key_for_hint_segment("type").is_none());
        assert!(key_for_hint_segment("← →").is_none());
    }

    #[test]
    fn layout_skips_unmapped_and_clips_to_width() {
        let area = Rect::new(0, 23, 20, 1);
        let hits = layout_footer_hits(area, "/ search · a add · ? help");
        assert_eq!(hits.len(), 2); // "/ search" (8) + " · " (3) + "a add" (5) = 16; "? help" needs 9 more → clip
        assert_eq!(hits[0].key.code, KeyCode::Char('/'));
        assert_eq!(hits[0].rect, Rect::new(0, 23, 8, 1));
        assert_eq!(hits[1].key.code, KeyCode::Char('a'));
        assert_eq!(hits[1].rect, Rect::new(11, 23, 5, 1));
    }
}
```

Adjust the clip assertion widths if your exact character counts differ — count Unicode width of each segment and of `" · "` (3). Only emit a hit when the **entire** segment (not the trailing separator) fits.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p znicz-tui footer_hits -- --nocapture`
Expected: FAIL (`todo!` panic) or compile error until the module is wired.

- [ ] **Step 3: Minimal implementation**

```rust
//! Footer hint segments → key events and hit rects.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::Rect;

use crate::hit::FooterHit;

pub fn key_for_hint_segment(segment: &str) -> Option<KeyEvent> {
    let segment = segment.trim();
    if segment.is_empty() {
        return None;
    }
    // Close-preferring compound labels first.
    if segment.contains("Esc") && segment.contains("close") {
        return Some(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    }
    if segment.starts_with("Alt-") && segment.contains("pan") {
        return Some(KeyEvent::new(KeyCode::Right, KeyModifiers::ALT));
    }
    let key_part = segment.split_whitespace().next()?;
    match key_part {
        "Enter" => Some(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        "Esc" => Some(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
        "Space" => Some(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE)),
        "type" | "←" | "→" => None,
        s if s.chars().count() == 1 => {
            let c = s.chars().next()?;
            Some(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE))
        }
        _ => None,
    }
}

pub fn layout_footer_hits(area: Rect, line: &str) -> Vec<FooterHit> {
    if area.width == 0 || area.height == 0 {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut x = 0u16;
    let parts: Vec<&str> = line.split(" · ").collect();
    for (i, part) in parts.iter().enumerate() {
        let sep = if i + 1 < parts.len() { " · " } else { "" };
        let part_width = part.chars().count() as u16;
        if x.saturating_add(part_width) > area.width {
            break;
        }
        if let Some(key) = key_for_hint_segment(part) {
            out.push(FooterHit {
                rect: Rect {
                    x: area.x + x,
                    y: area.y,
                    width: part_width,
                    height: 1,
                },
                key,
            });
        }
        x = x.saturating_add(part_width);
        let sep_width = sep.chars().count() as u16;
        if x.saturating_add(sep_width) > area.width {
            break;
        }
        x = x.saturating_add(sep_width);
    }
    out
}
```

Wire `mod footer_hits;` in `lib.rs` (no need to re-export).

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p znicz-tui footer_hits -- --nocapture`
Expected: PASS. Fix clip widths in the test if the implementation’s counting differs from the assertion — prefer fixing the test to match the documented “full segment must fit” rule.

- [ ] **Step 5: Commit**

```bash
git add znicz-tui/src/footer_hits.rs znicz-tui/src/lib.rs
git commit -m "Map footer hint segments to keys and hit rects."
```

---

### Task 3: Close-on-X and stop outside-close; queue toggle open-only

**Files:**
- Modify: `znicz-tui/src/app.rs` (`on_left_click`, `on_overlay_click`)
- Modify: `znicz-tui/tests/mouse.rs`

**Interfaces:**
- Consumes: `hits.close`, existing overlay/list/toggle hits
- Produces: updated click behaviour matching the spec priority steps 3–5 (footer comes in Task 4)

- [ ] **Step 1: Write failing tests** — flip and add in `znicz-tui/tests/mouse.rs`

Rename/replace the outside-close tests:

```rust
#[test]
fn a_click_outside_help_does_not_close_it() {
    let mut app = new_app();
    app.modal = Modal::Help;
    app.hits.overlay = Some(Rect::new(10, 4, 60, 16));
    app.on_mouse(left_click(0, 0));
    assert_eq!(app.modal, Modal::Help);
}

#[test]
fn a_click_outside_inspector_does_not_close_it() {
    let mut app = new_app();
    app.modal = Modal::Inspector;
    app.hits.overlay = Some(Rect::new(20, 4, 40, 12));
    app.on_mouse(left_click(0, 0));
    assert_eq!(app.modal, Modal::Inspector);
}

#[test]
fn a_click_outside_devices_does_not_close_the_overlay() {
    let mut app = new_app();
    app.modal = Modal::Devices;
    app.hits.overlay = Some(Rect::new(10, 4, 40, 12));
    app.on_mouse(left_click(0, 0));
    assert_eq!(app.modal, Modal::Devices);
}

#[test]
fn a_click_on_close_dismisses_help() {
    let mut app = new_app();
    app.modal = Modal::Help;
    app.hits.overlay = Some(Rect::new(10, 4, 60, 16));
    app.hits.close = Some(Rect::new(68, 4, 1, 1));
    app.on_mouse(left_click(68, 4));
    assert_eq!(app.modal, Modal::None);
}

#[test]
fn a_click_on_close_closes_the_queue_drawer() {
    let mut app = new_app();
    app.queue_open = true;
    app.focus = Focus::Queue;
    app.hits.close = Some(Rect::new(78, 0, 1, 1));
    app.on_mouse(left_click(78, 0));
    assert!(!app.queue_open);
    assert_eq!(app.focus, Focus::Library);
}

#[test]
fn a_library_click_under_an_open_queue_does_not_close_it() {
    let mut app = new_app();
    queue(&mut app, 2);
    app.queue_open = true;
    app.focus = Focus::Queue;
    app.hits = HitMap {
        library: Some(ListHit {
            inner: Rect::new(1, 1, 40, 10),
            offset: 0,
            len: 0,
        }),
        library_pane: Some(Rect::new(0, 0, 50, 20)),
        queue: Some(ListHit {
            inner: Rect::new(51, 1, 28, 10),
            offset: 0,
            len: 2,
        }),
        queue_toggle: Some(Rect::new(49, 0, 1, 20)),
        ..HitMap::default()
    };
    app.on_mouse(left_click(2, 5));
    assert!(app.queue_open);
}

#[test]
fn a_toggle_column_click_does_not_close_an_open_queue() {
    let mut app = new_app();
    app.queue_open = true;
    app.focus = Focus::Queue;
    app.hits.queue_toggle = Some(Rect::new(79, 0, 1, 20));
    app.hits.library_pane = Some(Rect::new(0, 0, 80, 20));
    app.on_mouse(left_click(79, 4));
    assert!(app.queue_open);
}
```

Keep `a_click_outside_search_cancels_the_prompt` and playlist-form outside cancel as they are. Keep `a_click_outside_a_playlist_form_cancels_the_form_and_keeps_the_overlay`. Remove or rewrite any test that expected library-click or toggle-click to close the drawer (`a_click_on_the_library_closes_the_queue_overlay`, sheet toggle-close) so open still works and close only via `close` / keys.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p znicz-tui --test mouse -- --nocapture`
Expected: FAIL on the new “does not close” / close-button / open-only assertions (old outside-close behaviour still active).

- [ ] **Step 3: Minimal implementation** in `app.rs`

In `on_left_click`, after typing-prompt handling and **before** overlay/list logic:

```rust
if self.hits.close.is_some_and(|r| point_in(r, column, row)) {
    if self.modal != Modal::None {
        self.modal = Modal::None;
        self.playlist_prompt = None;
        self.radio_prompt = None;
    } else if self.queue_open {
        self.close_queue();
    }
    return;
}
```

Change `on_overlay_click` so outside the overlay body is a no-op (delete `self.modal = Modal::None` on outside). Keep list-row select and inside-chrome ignore.

When `queue_open`, remove the branches that call `close_queue()` from library-pane or toggle clicks. Keep row select on the queue list. When closed, keep toggle → `open_queue()`.

Playlist/radio prompt outside cancel stays as today (before close/overlay).

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p znicz-tui --test mouse -- --nocapture`
Expected: PASS for Task 3 tests; fix any leftover tests that still expected outside-close or library-close.

- [ ] **Step 5: Commit**

```bash
git add znicz-tui/src/app.rs znicz-tui/tests/mouse.rs
git commit -m "Close overlays with an X hit and stop outside-click dismiss."
```

---

### Task 4: Footer hint clicks call `on_key`

**Files:**
- Modify: `znicz-tui/src/app.rs` (`on_left_click`)
- Modify: `znicz-tui/tests/mouse.rs`

**Interfaces:**
- Consumes: `hits.footer_hints: Vec<FooterHit>`
- Produces: footer hit handled **after** typing-prompt cancel check, **before** `close` / lists

- [ ] **Step 1: Write failing tests**

```rust
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use znicz_tui::hit::FooterHit;

#[test]
fn a_footer_help_hit_opens_help() {
    let mut app = new_app();
    app.hits.footer_hints = vec![FooterHit {
        rect: Rect::new(70, 23, 7, 1),
        key: KeyEvent::new(KeyCode::Char('?'), KeyModifiers::NONE),
    }];
    app.on_mouse(left_click(72, 23));
    assert_eq!(app.modal, Modal::Help);
}

#[test]
fn a_footer_esc_hit_closes_devices() {
    let mut app = new_app();
    app.modal = Modal::Devices;
    app.hits.overlay = Some(Rect::new(10, 4, 40, 12));
    app.hits.footer_hints = vec![FooterHit {
        rect: Rect::new(10, 23, 9, 1),
        key: KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
    }];
    app.on_mouse(left_click(12, 23));
    assert_eq!(app.modal, Modal::None);
}

#[test]
fn a_footer_hit_runs_while_an_overlay_is_open() {
    let mut app = new_app();
    app.modal = Modal::Inspector;
    app.hits.overlay = Some(Rect::new(20, 4, 40, 12));
    app.hits.footer_hints = vec![FooterHit {
        rect: Rect::new(0, 23, 11, 1),
        key: KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE),
    }];
    // Space is a global transport key; inspector stays open for non-close keys.
    app.on_mouse(left_click(2, 23));
    assert_eq!(app.modal, Modal::Inspector);
}
```

For the third test, assert a side effect that `on_key(Space)` would produce if the player can pause from Stopped, **or** change the key to `?` and assert Help replaces Inspector — simpler:

```rust
app.hits.footer_hints = vec![FooterHit {
    rect: Rect::new(0, 23, 7, 1),
    key: KeyEvent::new(KeyCode::Char('?'), KeyModifiers::NONE),
}];
app.on_mouse(left_click(2, 23));
assert_eq!(app.modal, Modal::Help);
```

(Note: with Help’s “any key closes”, opening Help via `?` while Inspector is open goes through `on_global_key` only if Inspector does not swallow `?`. Today `on_key` runs global after playlists/radio; Inspector returns early only for Inspector-specific path after global — verify `?` opens Help from Inspector via keyboard in `keys.rs` tests; mirror that.)

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p znicz-tui --test mouse a_footer -- --nocapture`
Expected: FAIL (footer clicks ignored).

- [ ] **Step 3: Minimal implementation**

In `on_left_click`, after typing-prompt handling:

```rust
if let Some(hit) = self
    .hits
    .footer_hints
    .iter()
    .find(|h| point_in(h.rect, column, row))
{
    let key = hit.key;
    self.on_key(key);
    return;
}
```

Then the existing `close` / overlay / list logic from Task 3.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p znicz-tui --test mouse -- --nocapture`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add znicz-tui/src/app.rs znicz-tui/tests/mouse.rs
git commit -m "Run footer hint clicks through the key handler."
```

---

### Task 5: Paint X and record `hits.close` while drawing

**Files:**
- Modify: `znicz-tui/src/views/mod.rs` (helper)
- Modify: `znicz-tui/src/views/help.rs`
- Modify: `znicz-tui/src/views/inspector.rs`
- Modify: `znicz-tui/src/views/devices.rs`
- Modify: `znicz-tui/src/views/playlists.rs`
- Modify: `znicz-tui/src/views/radio.rs`
- Modify: `znicz-tui/src/views/queue.rs`
- Modify: `znicz-tui/tests/mouse.rs`

**Interfaces:**
- Consumes: overlay/drawer `area: Rect`
- Produces: `pub(crate) fn close_button_rect(area: Rect) -> Option<Rect>`; title decoration that shows `X`; `app.hits.close = close_button_rect(...)`

- [ ] **Step 1: Write the failing draw test**

```rust
#[test]
fn a_drawn_overlay_exposes_a_close_hit() {
    let mut app = new_app();
    app.modal = Modal::Help;
    let state = app.player.state();
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).expect("backend");
    terminal
        .draw(|frame| views::render(frame, &mut app, &state))
        .expect("draw");
    let close = app.hits.close.expect("close hit after draw");
    app.on_mouse(left_click(close.x, close.y));
    assert_eq!(app.modal, Modal::None);
}

#[test]
fn a_drawn_queue_exposes_a_close_hit() {
    let mut app = new_app();
    queue(&mut app, 1);
    app.queue_open = true;
    let state = app.player.state();
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).expect("backend");
    terminal
        .draw(|frame| views::render(frame, &mut app, &state))
        .expect("draw");
    let close = app.hits.close.expect("queue close hit");
    app.on_mouse(left_click(close.x, close.y));
    assert!(!app.queue_open);
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p znicz-tui --test mouse a_drawn_overlay_exposes -- --nocapture`
Expected: FAIL (`close` is `None` after draw).

- [ ] **Step 3: Minimal implementation**

In `views/mod.rs`:

```rust
/// Top-right cell for the close control (`X`), inside the border row.
pub(crate) fn close_button_rect(area: Rect) -> Option<Rect> {
    if area.width < 3 || area.height < 1 {
        return None;
    }
    Some(Rect {
        x: area.x + area.width - 2,
        y: area.y,
        width: 1,
        height: 1,
    })
}

pub(crate) fn close_title() -> Line<'static> {
    Line::from(Span::styled(" X ", theme::key())).right_aligned()
}
```

For each modal and the queue drawer, after computing the popup/area rect:

```rust
app.hits.close = views::close_button_rect(popup);
```

Add `.title(views::close_title())` on the `Block` (help/inspector already use `.title(...)`; chain a second right-aligned title). For `pane_block` users (devices/playlists/radio/queue), either:

- extend `pane_block` with `with_close: bool` that adds `close_title()`, or
- add `.title(views::close_title())` on the block returned before render.

When a modal is open, it must overwrite `hits.close` after the queue draws (render order in `views::render` already draws queue then modals — modal wins). When only the queue is open, queue sets `close`.

Empty queue still gets a bordered pane — set `close` there too.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p znicz-tui --test mouse -- --nocapture`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add znicz-tui/src/views znicz-tui/tests/mouse.rs
git commit -m "Draw a top-right X close control on overlays and the queue."
```

---

### Task 6: Footer draw fills `footer_hints`

**Files:**
- Modify: `znicz-tui/src/views/status.rs`
- Modify: `znicz-tui/tests/mouse.rs`

**Interfaces:**
- Consumes: `footer_hits::layout_footer_hits`, `hints_for(app)`
- Produces: `app.hits.footer_hints` set during `render_footer`

- [ ] **Step 1: Write the failing draw test**

```rust
#[test]
fn a_drawn_footer_help_hint_is_clickable() {
    let mut app = new_app();
    let state = app.player.state();
    let mut terminal = Terminal::new(TestBackend::new(100, 24)).expect("backend");
    terminal
        .draw(|frame| views::render(frame, &mut app, &state))
        .expect("draw");
    let help = app
        .hits
        .footer_hints
        .iter()
        .find(|h| h.key.code == KeyCode::Char('?'))
        .expect("? help footer hit")
        .rect;
    app.on_mouse(left_click(help.x, help.y));
    assert_eq!(app.modal, Modal::Help);
}
```

Add `use crossterm::event::KeyCode;` at the top of `mouse.rs` if missing.

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p znicz-tui --test mouse a_drawn_footer_help -- --nocapture`
Expected: FAIL (no footer hints after draw).

- [ ] **Step 3: Minimal implementation** in `status.rs`

```rust
use crate::footer_hits;

pub fn render_footer(frame: &mut Frame, area: Rect, app: &mut App) {
    let text = hints_for(app);
    app.hits.footer_hints = footer_hits::layout_footer_hits(area, text);
    let line = Line::from(Span::styled(text, theme::dim()));
    frame.render_widget(Paragraph::new(line), area);
}
```

Change `render_footer` / `hints_for` to take `&mut App` / keep `&App` for hints — `views::render` already passes `&mut App`. Update the signature to `&mut App`.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p znicz-tui --test mouse -- --nocapture`
Expected: PASS. Also run `cargo test -p znicz-tui --offline` (or without network as the project usually does).

- [ ] **Step 5: Commit**

```bash
git add znicz-tui/src/views/status.rs znicz-tui/src/views/mod.rs znicz-tui/tests/mouse.rs
git commit -m "Record clickable footer hint hits while drawing."
```

---

### Task 7: Wiki and version 0.4.5

**Files:**
- Modify: `wiki/Architecture/TUI.md` (mouse paragraph)
- Modify: `wiki/Domain/TUI-Players.md` (pointer bullet)
- Modify: `Cargo.toml` (`[workspace.package] version`)
- Modify: `Cargo.lock` (via `cargo build` / `cargo test`)

**Interfaces:** none (docs + version only)

- [ ] **Step 1: Update wiki mouse copy**

In `wiki/Architecture/TUI.md`, replace the mouse paragraph so it says:

- Capture still on while the TUI is up
- List click = select only; wheel = focused list
- Overlays and the queue drawer close via the top-right **X** (or Esc / `]` / footer), **not** outside click
- Footer hint segments are clickable and run that key action
- Queue border column opens the drawer only
- Typing prompts still cancel on outside click
- Transport / cover still ignore clicks; toasts still pass through as today if already documented

In `wiki/Domain/TUI-Players.md`, update **Limited pointer** so it mentions X close and clickable footer hints (not “footer does nothing”).

- [ ] **Step 2: Bump version**

Set `[workspace.package] version = "0.4.5"` in root `Cargo.toml`. Run `cargo test -p znicz-tui --offline` (or full workspace offline if that is the project norm) so `Cargo.lock` picks up `0.4.5`.

- [ ] **Step 3: Commit**

```bash
git add wiki/Architecture/TUI.md wiki/Domain/TUI-Players.md Cargo.toml Cargo.lock
git commit -m "Document clickable footer and X close; ship as 0.4.5."
```

---

## Spec coverage checklist

| Spec requirement | Task |
| --- | --- |
| No outside-click close for modals / queue | 3 |
| Top-right X closes modals + queue | 3, 5 |
| Footer segments clickable via `on_key` | 2, 4, 6 |
| Queue toggle opens only | 3 |
| Typing prompts still outside-cancel | 3 (keep existing) |
| Token → key table | 2 |
| Hit map `close` / `footer_hints` | 1 |
| Wiki + 0.4.5 | 7 |
| No mouse lines in `?` help | 7 (leave `keys.rs` tables alone) |

## Self-review notes

- `HitMap` loses `Copy` in Task 1 — any `*hits` / struct-update sites must compile before Task 1 commit.
- Footer priority is **before** close so a footer `Esc close` still works when both could apply; X remains available when the footer has no Esc token (Help).
- Modal draw after queue ensures modal `close` wins when both are visible.
