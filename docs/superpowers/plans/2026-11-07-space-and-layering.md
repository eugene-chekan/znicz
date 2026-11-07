# Space and Layering Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Modals open inside the list region, messages move into a transport row with a history list (`M`) and unread marker, the cover only shows on tall windows, focus is shown by colour with selections always visible, titles carry breadcrumbs, the library marks and jumps to (`J`) the playing track, and help returns to the modal it replaced.

**Architecture:** `layout.rs` owns modal placement (`modal_rect`, `list_modal_rect`) and the new transport heights. `toast.rs` keeps live messages plus a 50-entry history with unread flags; `now_playing::message_line` draws the transport row and a new `views/messages.rs` draws the history modal. A new `playing.rs` decides which rows stand for the playing track; `LibraryPane::reveal` implements `J`. Footer labels become owned strings so `M messages (N)` can carry a count.

**Tech Stack:** Rust 2021, ratatui 0.30, crossterm 0.28, cargo test.

**Spec:** [`docs/superpowers/specs/2026-11-07-space-and-layering-design.md`](../specs/2026-11-07-space-and-layering-design.md)

**Commands used throughout:**

- Unit tests: `cargo test -p znicz-tui --lib <filter>`
- Integration tests: `cargo test -p znicz-tui --test <keys|render|mouse> <filter>`
- Crate: `cargo test -p znicz-tui`; lint: `cargo clippy -p znicz-tui --all-targets -- -D warnings`

Every commit message ends with a blank line then `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.

---

### Task 1: Commit the plan

- [x] **Step 1: Commit**

```bash
git add docs/superpowers/plans/2026-11-07-space-and-layering.md
git commit -m "Add space and layering implementation plan"
```

---

### Task 2: Layout — modal placement, cover threshold, transport heights

**Files:**
- Modify: `znicz-tui/src/layout.rs`
- Test: `znicz-tui/src/layout.rs` (tests module), `znicz-tui/tests/render.rs`

- [ ] **Step 1: Write the failing unit tests**

In `znicz-tui/src/layout.rs` tests module, delete `cover_shrinks_then_drops` and `cover_off_keeps_today_s_chrome`, and add:

```rust
    #[test]
    fn a_modal_is_centred_and_clamped_to_the_list_region() {
        let list = Rect::new(0, 0, 100, 30);
        assert_eq!(modal_rect(list, 40, 10), Rect::new(30, 10, 40, 10));
        assert_eq!(modal_rect(list, 200, 50), list, "never larger than the list");
    }

    #[test]
    fn a_short_list_region_gives_the_modal_all_of_it() {
        let list = Rect::new(0, 2, 80, 9);
        assert_eq!(modal_rect(list, 40, 5), list);
    }

    #[test]
    fn list_modals_take_seventy_percent_with_a_floor() {
        let list = Rect::new(0, 0, 100, 30);
        assert_eq!(list_modal_rect(list), Rect::new(15, 4, 70, 21));
        let small = Rect::new(0, 0, 50, 10);
        let rect = list_modal_rect(small);
        assert_eq!((rect.width, rect.height), (40, 8));
    }

    #[test]
    fn the_cover_needs_a_24_row_window() {
        assert!(cover_enabled(24, true));
        assert!(!cover_enabled(23, true));
        assert!(!cover_enabled(40, false));
    }

    #[test]
    fn transport_rows_include_the_message_row() {
        assert_eq!(transport_height(30, true), 8);
        assert_eq!(transport_height(22, true), 3, "no cover below 24 rows");
        assert_eq!(transport_height(15, true), 2);
        assert_eq!(transport_height(30, false), 3);
        assert_eq!(transport_height(15, false), 2);
    }
```

Keep `cover_uses_eight_rows_on_a_normal_window` as is.

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test -p znicz-tui --lib layout::`
Expected: compile error (`modal_rect`, `list_modal_rect` not found).

- [ ] **Step 3: Implement**

In `znicz-tui/src/layout.rs`, replace everything from `const HINTS: u16 = 1;` through the end of `fn transport_height` with:

```rust
/// Below this height the signal-path line is dropped to keep the lists usable.
pub const COMPACT_HEIGHT: u16 = 20;
/// The cover only shows on a window at least this tall; shorter ones keep the list.
pub const COVER_MIN_WINDOW: u16 = 24;
const COVER_ROWS: u16 = 8;
/// Below this list-region height a modal takes the whole list region.
pub const MODAL_FULL_BELOW: u16 = 10;

pub fn cover_enabled(window: u16, show_cover: bool) -> bool {
    show_cover && window >= COVER_MIN_WINDOW
}

/// Transport rows, including the message row.
pub fn transport_height(window: u16, show_cover: bool) -> u16 {
    if cover_enabled(window, show_cover) {
        COVER_ROWS
    } else if window < COMPACT_HEIGHT {
        2
    } else {
        3
    }
}

/// A `want_w` × `want_h` box centred in the list region and clamped to it.
///
/// A short list region gives the modal all of it.
pub fn modal_rect(list: Rect, want_w: u16, want_h: u16) -> Rect {
    if list.height < MODAL_FULL_BELOW {
        return list;
    }
    let width = want_w.min(list.width);
    let height = want_h.min(list.height);
    Rect {
        x: list.x + (list.width - width) / 2,
        y: list.y + (list.height - height) / 2,
        width,
        height,
    }
}

/// Devices, Playlists, Radio, Messages: 70 % of the list region, at least 40 × 8.
pub fn list_modal_rect(list: Rect) -> Rect {
    let width = ((u32::from(list.width) * 70 / 100) as u16).max(40);
    let height = ((u32::from(list.height) * 70 / 100) as u16).max(8);
    modal_rect(list, width, height)
}
```

`text_transport_height` is gone; it had no other users.

- [ ] **Step 4: Update the render test that pinned the old height**

In `znicz-tui/tests/render.rs`, replace the test `show_cover_false_keeps_two_transport_rows` with:

```rust
#[test]
fn show_cover_false_uses_three_text_rows() {
    let mut app = App::with_library(player(), None);
    app.tui.show_cover = false;
    let screen = draw(&mut app, 90, 24);
    assert!(screen.contains("Nothing playing"), "{screen}");
    assert_eq!(app.list_height, 20);
}

#[test]
fn a_short_window_keeps_the_list_instead_of_the_cover() {
    let mut app = App::with_library(player(), None);
    app.tui.show_cover = true;
    draw(&mut app, 48, 14);
    assert_eq!(app.list_height, 11, "14 rows - 2 transport - 1 footer");
}
```

- [ ] **Step 5: Run and verify**

Run: `cargo test -p znicz-tui`
Expected: PASS. (The transport has a spare blank row until Task 5 fills it.)

- [ ] **Step 6: Commit**

```bash
git add znicz-tui/src/layout.rs znicz-tui/tests/render.rs
git commit -m "Place modals in the list region and show the cover only on tall windows"
```

---

### Task 3: Modals open inside the list region

**Files:**
- Modify: `znicz-tui/src/views/mod.rs`, `views/devices.rs`, `views/playlists.rs`, `views/radio.rs`, `views/inspector.rs`, `views/help.rs`
- Test: `znicz-tui/tests/render.rs`

- [ ] **Step 1: Write the failing render test**

Add to `znicz-tui/tests/render.rs`:

```rust
/// Rows from the top of the transport to the bottom of the screen.
fn below_list(screen: &str, list_height: u16) -> Vec<String> {
    screen
        .lines()
        .skip(usize::from(list_height))
        .map(str::to_string)
        .collect()
}

#[test]
fn no_modal_covers_the_transport_or_footer() {
    let modals = [
        Modal::Help,
        Modal::Devices,
        Modal::Inspector,
        Modal::Playlists,
        Modal::Radio,
    ];
    for &(width, height) in &[(80u16, 24u16), (120, 40), (60, 18), (40, 12)] {
        let mut app = App::with_library(player(), None);
        let plain = draw(&mut app, width, height);
        let expected = below_list(&plain, app.list_height);
        for modal in modals {
            app.modal = modal;
            let screen = draw(&mut app, width, height);
            let rows = below_list(&screen, app.list_height);
            // The footer row legitimately changes hints per modal; compare the
            // transport rows only.
            let transport = rows.len() - 1;
            assert_eq!(
                rows[..transport],
                expected[..transport],
                "{modal:?} at {width}x{height} covered the transport:\n{screen}"
            );
        }
    }
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p znicz-tui --test render no_modal_covers`
Expected: FAIL (Devices/Playlists/Radio at 70 % of the whole window overlap the transport).

- [ ] **Step 3: Pass the list region to every modal**

In `znicz-tui/src/views/mod.rs` `render`, change the modal match to pass `list` instead of `area`:

```rust
    match app.modal {
        Modal::Help => help::render(frame, list, app),
        Modal::Devices => devices::render_modal(frame, list, app, state),
        Modal::Inspector => inspector::render(frame, list, app, state),
        Modal::Playlists => playlists::render_modal(frame, list, app),
        Modal::Radio => radio::render_modal(frame, list, app),
        Modal::None => {}
    }
```

- [ ] **Step 4: Use `list_modal_rect` in Devices, Playlists, Radio**

In each of `views/devices.rs`, `views/playlists.rs`, `views/radio.rs`:
- in `render_modal`, replace `let popup = centered_modal(area);` with `let popup = crate::layout::list_modal_rect(area);`
- delete the private `fn centered_modal`.
- rename the `area` parameter of `render_modal` to `list` (and its use) so the signature reads `render_modal(frame: &mut Frame, list: Rect, …)`.

- [ ] **Step 5: Inspector**

In `views/inspector.rs` `render`, rename the `area` parameter to `list` and replace the size/placement code (from `let height = …` through `let popup = centered(area, width, height);`) with:

```rust
    let height = (content.len() as u16).saturating_add(2).max(3);
    let width = ((u32::from(list.width) * 62 / 100) as u16).max(44);
    let popup = crate::layout::modal_rect(list, width, height);
```

Delete the private `fn centered`.

- [ ] **Step 6: Help**

In `views/help.rs`:
- rename `render`'s `area` parameter to `list`;
- replace `let popup = popup_rect(area);` with `let popup = crate::layout::modal_rect(list, MAX_WIDTH, list.height);`;
- delete `fn popup_rect` and its two tests `the_popup_caps_its_width_and_keeps_the_footer_row_free` and `a_tiny_screen_does_not_produce_an_oversized_box` (layout tests now cover placement).

- [ ] **Step 7: Run and verify**

Run: `cargo test -p znicz-tui`
Expected: PASS. If `the_signal_inspector_shows_the_device_sample_format` (90×24, list region 15 rows) no longer shows `f32` because the inspector content is taller than the list region, change that test's size from `90, 24` to `90, 30` (both draws) — the inspector now lives in the list region by design.

- [ ] **Step 8: Commit**

```bash
git add znicz-tui/src/views znicz-tui/tests/render.rs
git commit -m "Open every modal inside the list region"
```

---

### Task 4: Message history and unread alerts in `Toasts`

**Files:**
- Modify: `znicz-tui/src/toast.rs`

- [ ] **Step 1: Write the failing unit tests**

Add to the tests module of `znicz-tui/src/toast.rs`:

```rust
    #[test]
    fn history_keeps_fifty_messages_after_they_expire() {
        let mut toasts = Toasts::new();
        let now = Instant::now();
        for i in 0..60 {
            toasts.push(format!("m{i}"), Level::Info, now);
        }
        toasts.tick_at(now + ERROR_LIFETIME);
        assert!(toasts.is_empty(), "live messages expired");
        assert_eq!(toasts.history_len(), HISTORY);
        assert_eq!(toasts.history().next().unwrap().text, "m59", "newest first");
    }

    #[test]
    fn only_warnings_and_errors_are_unread() {
        let mut toasts = Toasts::new();
        toasts.info("added 4 tracks");
        toasts.success("queue cleared");
        assert_eq!(toasts.unread_alerts(), None);
        toasts.warn("stream dropped");
        assert_eq!(toasts.unread_alerts(), Some((1, Level::Warn)));
        toasts.error("device refused 96 kHz");
        assert_eq!(toasts.unread_alerts(), Some((2, Level::Error)), "error is worst");
    }

    #[test]
    fn marking_read_keeps_the_rows_fresh_until_cleared() {
        let mut toasts = Toasts::new();
        toasts.warn("stream dropped");
        toasts.info("added");
        toasts.mark_read();
        assert_eq!(toasts.unread_alerts(), None);
        let fresh: Vec<bool> = toasts.history().map(|e| e.fresh).collect();
        assert_eq!(fresh, [false, true], "newest first: info, then the warning");
        toasts.clear_fresh();
        assert!(toasts.history().all(|e| !e.fresh));
    }

    #[test]
    fn clearing_history_leaves_live_messages() {
        let mut toasts = Toasts::new();
        toasts.error("broke");
        toasts.clear_history();
        assert_eq!(toasts.history_len(), 0);
        assert_eq!(toasts.unread_alerts(), None);
        assert!(!toasts.is_empty());
    }

    #[test]
    fn live_extra_counts_messages_behind_the_newest() {
        let mut toasts = Toasts::new();
        assert_eq!(toasts.live_extra(), 0);
        toasts.info("a");
        assert_eq!(toasts.live_extra(), 0);
        toasts.info("b");
        toasts.info("c");
        assert_eq!(toasts.live_extra(), 2);
    }
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test -p znicz-tui --lib toast::`
Expected: compile error.

- [ ] **Step 3: Implement**

In `znicz-tui/src/toast.rs`:

Add `use std::collections::VecDeque;` to the imports.

Change the module doc's first line to `//! Messages: live ones for the transport row, and a history for the Messages list.`

After `const MAX_KEPT: usize = 8;` add:

```rust
/// How many messages the Messages list remembers.
const HISTORY: usize = 50;

/// One remembered message.
#[derive(Debug, Clone)]
pub struct Entry {
    pub text: String,
    pub level: Level,
    pub at: Instant,
    /// A warning or error the user has not seen in the Messages list yet.
    pub unread: bool,
    /// Was unread when the Messages list opened; drawn bold until it closes.
    pub fresh: bool,
}
```

Change `Toasts` to:

```rust
/// Live messages (newest first) and the history behind them.
#[derive(Debug, Default)]
pub struct Toasts {
    items: Vec<Toast>,
    history: VecDeque<Entry>,
}
```

Replace `fn push` with:

```rust
    fn push(&mut self, text: String, level: Level, now: Instant) {
        self.history.push_front(Entry {
            text: text.clone(),
            level,
            at: now,
            unread: matches!(level, Level::Warn | Level::Error),
            fresh: false,
        });
        self.history.truncate(HISTORY);
        self.items.insert(0, Toast::new(text, level, now));
        self.items.truncate(MAX_KEPT);
    }
```

Delete `pub fn visible`. After `pub fn recent`, add:

```rust
    /// Live messages behind the newest one, for the `+N` on the message row.
    pub fn live_extra(&self) -> usize {
        self.items.len().saturating_sub(1)
    }

    /// Remembered messages, newest first.
    pub fn history(&self) -> impl Iterator<Item = &Entry> {
        self.history.iter()
    }

    pub fn history_len(&self) -> usize {
        self.history.len()
    }

    /// Unread warnings and errors: how many, and the worst level.
    pub fn unread_alerts(&self) -> Option<(usize, Level)> {
        let mut count = 0;
        let mut worst = Level::Warn;
        for entry in self.history.iter().filter(|e| e.unread) {
            count += 1;
            if entry.level == Level::Error {
                worst = Level::Error;
            }
        }
        (count > 0).then_some((count, worst))
    }

    /// The Messages list opened: unread rows become fresh, nothing stays unread.
    pub fn mark_read(&mut self) {
        for entry in &mut self.history {
            entry.fresh = entry.unread;
            entry.unread = false;
        }
    }

    /// The Messages list closed.
    pub fn clear_fresh(&mut self) {
        for entry in &mut self.history {
            entry.fresh = false;
        }
    }

    pub fn clear_history(&mut self) {
        self.history.clear();
    }

    /// Expire every live message now, keeping the history.
    #[doc(hidden)]
    pub fn expire_live_for_test(&mut self) {
        self.items.clear();
    }
```

`views/mod.rs` still calls `app.toasts.visible()`; replace that call in `render_toasts` with `&app.toasts.recent()[..app.toasts.recent().len().min(3)]` for now (Task 5 deletes `render_toasts`).

- [ ] **Step 4: Run and verify**

Run: `cargo test -p znicz-tui`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add znicz-tui/src/toast.rs znicz-tui/src/views/mod.rs
git commit -m "Keep a message history with unread warnings and errors"
```

---

### Task 5: Message row in the transport; remove toast boxes

**Files:**
- Modify: `znicz-tui/src/views/now_playing.rs`, `znicz-tui/src/views/mod.rs`, `znicz-tui/src/layout.rs`
- Test: `znicz-tui/src/views/now_playing.rs` (tests), `znicz-tui/tests/render.rs`

- [ ] **Step 1: Write the failing unit tests**

Add to the tests module of `znicz-tui/src/views/now_playing.rs` (create `#[cfg(test)] mod tests { use super::*; … }` if absent; if one exists, add `use crate::toast::Toasts;` inside it):

```rust
    fn text(line: &Line) -> String {
        line.spans.iter().map(|s| s.content.as_ref()).collect()
    }

    #[test]
    fn the_message_row_shows_the_newest_live_message() {
        let mut toasts = crate::toast::Toasts::new();
        toasts.info("first");
        toasts.success("added 4 tracks");
        let row = text(&message_line(&toasts, 40));
        assert!(row.starts_with("● added 4 tracks"), "{row:?}");
        assert!(row.trim_end().ends_with("+1"), "{row:?}");
        assert_eq!(row.chars().count(), 40);
    }

    #[test]
    fn the_message_row_shows_unread_alerts_once_messages_expire() {
        let mut toasts = crate::toast::Toasts::new();
        toasts.warn("stream dropped");
        toasts.error("device refused");
        toasts.expire_live_for_test();
        let row = text(&message_line(&toasts, 40));
        assert!(row.trim_end().ends_with("x 2 unread · M"), "{row:?}");
    }

    #[test]
    fn the_message_row_is_blank_with_nothing_to_say() {
        let toasts = crate::toast::Toasts::new();
        assert_eq!(text(&message_line(&toasts, 40)).trim(), "");
    }

    #[test]
    fn a_long_message_is_cut_to_the_row() {
        let mut toasts = crate::toast::Toasts::new();
        toasts.error("player error: playlist had no playable files at all");
        let row = text(&message_line(&toasts, 20));
        assert_eq!(row.chars().count(), 20);
        assert!(row.ends_with('…'), "{row:?}");
    }
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test -p znicz-tui --lib now_playing`
Expected: compile error (`message_line` not found).

- [ ] **Step 3: Add `message_line` and share the level mark**

In `znicz-tui/src/views/mod.rs`, rename `fn toast_mark` to `pub(crate) fn level_mark` (update its one caller).

In `znicz-tui/src/views/now_playing.rs`, add:

```rust
/// The transport's message row: the newest live message (with `+N` for the
/// rest), else the unread-alerts marker, else blank. Always `width` wide.
pub(crate) fn message_line(toasts: &crate::toast::Toasts, width: usize) -> Line<'static> {
    if width == 0 {
        return Line::from("");
    }
    if let Some(toast) = toasts.current() {
        let (mark, style) = crate::views::level_mark(toast.level);
        let extra = toasts.live_extra();
        let suffix = if extra > 0 {
            format!(" +{extra}")
        } else {
            String::new()
        };
        let suffix_w = suffix.chars().count();
        let text = format::truncate(&toast.text, width.saturating_sub(2 + suffix_w));
        let pad = width.saturating_sub(2 + text.chars().count() + suffix_w);
        return Line::from(vec![
            Span::styled(format!("{mark} "), style),
            Span::styled(text, theme::strong()),
            Span::raw(" ".repeat(pad)),
            Span::styled(suffix, theme::dim()),
        ]);
    }
    if let Some((count, level)) = toasts.unread_alerts() {
        let (mark, style) = crate::views::level_mark(level);
        let label = format::truncate(&format!("{mark} {count} unread · M"), width);
        let pad = width.saturating_sub(label.chars().count());
        return Line::from(vec![Span::raw(" ".repeat(pad)), Span::styled(label, style)]);
    }
    Line::from(" ".repeat(width))
}
```

(`format`, `theme`, `Line`, `Span` are already imported in this module; add any that are missing.)

- [ ] **Step 4: Draw it in both transports**

In `render_transport`, replace the `if !with_cover { … return; }` block with:

```rust
    if !with_cover {
        let width = area.width as usize;
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints(vec![Constraint::Length(1); area.height as usize])
            .split(area);
        let last = rows.len().saturating_sub(1);
        for (i, row) in rows.iter().enumerate() {
            let line = if i == 0 {
                chrome_line(state, width)
            } else if i == last {
                message_line(&app.toasts, width)
            } else if show_signal {
                signal_line(state, width)
            } else {
                Line::from("")
            };
            frame.render_widget(Paragraph::new(line), *row);
        }
        return;
    }
```

Change `render_stacked_chrome` to take the toasts and draw row 4:

```rust
fn render_stacked_chrome(
    frame: &mut Frame,
    area: Rect,
    state: &PlayerState,
    toasts: &crate::toast::Toasts,
    show_signal: bool,
) {
```

with the match:

```rust
        let line = match i {
            0 => title_row(state, width),
            1 => artist_album_row(state, width),
            2 => seek_row(state, width),
            3 if show_signal => signal_line(state, width),
            4 => message_line(toasts, width),
            _ => Line::from(""),
        };
```

and its call: `render_stacked_chrome(frame, chrome, state, &app.toasts, show_signal);`.

- [ ] **Step 5: Delete the toast boxes**

- `views/mod.rs`: delete `fn render_toasts` and the `render_toasts(frame, list, app);` call.
- `layout.rs`: delete `TOAST_BOX_HEIGHT`, `TOAST_INSET`, `TOAST_MARK_AND_SPACE`, `TOAST_BORDERS`, `TOAST_BOX_MIN_WIDTH`, `toast_boxed`, `toast_width_for`, `toast_areas`, and the tests `toast_areas_stack_up_from_the_bottom_right`, `a_tiny_list_falls_back_to_a_single_line`, `toast_width_covers_the_playlist_empty_error`.
- Remove any now-unused imports the compiler reports.

- [ ] **Step 6: Update render tests**

In `znicz-tui/tests/render.rs`, replace `hints_stay_when_a_toast_is_showing` with:

```rust
#[test]
fn a_message_sits_in_the_transport_not_over_the_list() {
    let mut app = App::with_library(player(), None);
    app.toasts.error("could not open device");
    let screen = draw(&mut app, 90, 24);
    let lines: Vec<&str> = screen.lines().collect();
    let row = lines
        .iter()
        .position(|line| line.contains("x could not open device"))
        .expect("message row");
    assert!(
        row >= usize::from(app.list_height) && row < lines.len() - 1,
        "message must be in the transport, row {row}:\n{screen}"
    );
    assert!(lines[lines.len() - 1].contains("search"), "hints remain");
}

#[test]
fn unread_alerts_show_once_the_message_expires() {
    let mut app = App::with_library(player(), None);
    app.toasts.warn("stream dropped");
    app.toasts.expire_live_for_test();
    let screen = draw(&mut app, 90, 24);
    assert!(screen.contains("▲ 1 unread · M"), "{screen}");
}
```

`a_success_toast_has_its_own_box_and_mark` and `a_playlist_error_toast_shows_the_whole_message` stay; rename the first to `a_success_message_has_its_mark`.

- [ ] **Step 7: Run and verify**

Run: `cargo test -p znicz-tui` and `cargo clippy -p znicz-tui --all-targets -- -D warnings`
Expected: PASS, clean.

- [ ] **Step 8: Commit**

```bash
git add znicz-tui/src znicz-tui/tests/render.rs
git commit -m "Show messages on a transport row instead of floating boxes"
```

---

### Task 6: Messages list (`M`)

**Files:**
- Create: `znicz-tui/src/views/messages.rs`
- Modify: `znicz-tui/src/views/mod.rs`, `znicz-tui/src/app.rs`, `znicz-tui/src/keys.rs`
- Test: `znicz-tui/src/views/messages.rs`, `znicz-tui/src/keys.rs`, `znicz-tui/tests/keys.rs`, `znicz-tui/tests/render.rs`, `znicz-tui/tests/mouse.rs`

- [ ] **Step 1: Write the failing integration tests**

In `znicz-tui/tests/keys.rs` add:

```rust
#[test]
fn capital_m_opens_and_closes_messages() {
    let mut app = new_app();
    press_char(&mut app, 'M');
    assert_eq!(app.modal, Modal::Messages);
    press_char(&mut app, 'M');
    assert_eq!(app.modal, Modal::None);
    press_char(&mut app, 'M');
    press(&mut app, KeyCode::Esc);
    assert_eq!(app.modal, Modal::None);
}

#[test]
fn opening_messages_marks_alerts_read() {
    let mut app = new_app();
    app.toasts.warn("stream dropped");
    assert!(app.toasts.unread_alerts().is_some());
    press_char(&mut app, 'M');
    assert_eq!(app.toasts.unread_alerts(), None);
}

#[test]
fn capital_c_clears_the_message_history() {
    let mut app = new_app();
    app.toasts.info("one");
    app.toasts.info("two");
    press_char(&mut app, 'M');
    press_char(&mut app, 'C');
    assert_eq!(app.toasts.history_len(), 0);
    assert_eq!(app.modal, Modal::Messages);
}

#[test]
fn j_and_k_move_through_messages() {
    let mut app = new_app();
    for i in 0..3 {
        app.toasts.info(format!("m{i}"));
    }
    press_char(&mut app, 'M');
    press_char(&mut app, 'j');
    press_char(&mut app, 'j');
    assert_eq!(app.message_cursor.selected(3), Some(2));
}
```

In `znicz-tui/tests/render.rs` add:

```rust
#[test]
fn the_messages_list_shows_history_newest_first() {
    let mut app = App::with_library(player(), None);
    app.toasts.info("added 4 tracks");
    app.toasts.error("device refused 96 kHz");
    app.modal = Modal::Messages;
    let screen = draw(&mut app, 100, 30);
    assert!(screen.contains("Messages"), "{screen}");
    let error = screen.find("device refused 96 kHz").expect("error row");
    let info = screen.find("added 4 tracks").expect("info row");
    assert!(error < info, "newest first:\n{screen}");
    assert!(screen.contains(" 0s"), "age column:\n{screen}");
}

#[test]
fn an_empty_messages_list_says_so() {
    let mut app = App::with_library(player(), None);
    app.modal = Modal::Messages;
    let screen = draw(&mut app, 100, 30);
    assert!(screen.contains("No messages yet"), "{screen}");
}
```

Also add `Modal::Messages` to the `modals` array in `no_modal_covers_the_transport_or_footer`.

In `znicz-tui/tests/mouse.rs` add:

```rust
#[test]
fn a_click_selects_a_message_row_and_the_wheel_moves_it() {
    let mut app = new_app();
    for i in 0..4 {
        app.toasts.info(format!("m{i}"));
    }
    app.modal = Modal::Messages;
    app.hits.overlay = Some(Rect::new(10, 2, 60, 10));
    app.hits.overlay_list = Some(ListHit {
        inner: Rect::new(11, 3, 58, 8),
        offset: 0,
        len: 4,
    });
    app.on_mouse(left_click(20, 5));
    assert_eq!(app.message_cursor.selected(4), Some(2));
    app.on_mouse(wheel(false));
    assert_eq!(app.message_cursor.selected(4), Some(3));
}
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test -p znicz-tui --test keys messages` (and `render messages`, `mouse message`)
Expected: compile errors (`Modal::Messages`, `message_cursor`).

- [ ] **Step 3: Keymap data**

In `znicz-tui/src/keys.rs`:

- Add `Messages,` to `enum Context` (after `Radio`).
- Add after `CLOSE`: `/// Opens the Messages list; the footer appends the unread count.\npub const MESSAGES_OPEN: Hint = hint("M messages", KeyCode::Char('M'));`
- `context_hints`: append `MESSAGES_OPEN` to the end of both the `Context::Library` and `Context::LibraryTree` lists, and add the arm `Context::Messages => &[CLEAR, CLOSE],`.
- `GLOBAL`: add `b("M", "messages"),` after `b(",", "devices"),`.
- Add the table after `INSPECTOR`:

```rust
pub const MESSAGES: &[Binding] = &[b("C", "clear the history"), b("Esc / M", "close")];
```

- `CONTEXT_SECTIONS`: add `("Messages", MESSAGES),` at the end.
- `context_title`: add `Context::Messages => Some("Messages"),`.
- Tests: add `Context::Messages` to `ALL_CONTEXTS` (size 13) and to the context list in `table_backed_hints_name_a_documented_key`; add `MESSAGES` to the `tables` array in `every_binding_documents_both_a_key_and_an_action`; append `"M messages"` to the expected labels in `library_hints_lead_with_pause_and_label_p_and_r`; in `every_context_fits_eighty_columns_with_more` change the bound to 90 and rename it `every_context_fits_ninety_columns_with_more`, with the comment `// M messages is last in the library lists, so narrower footers drop it first.`; in `help_starts_with_the_context_then_playback_and_navigation` and `prompts_and_help_have_no_context_section` the section count/titles grow by `Messages` at the end (expected titles end `…, "Devices", "Inspector", "Messages"`, dims gain one `true`, `sections.len()` becomes 9).

- [ ] **Step 4: The Messages view**

Create `znicz-tui/src/views/messages.rs`:

```rust
//! The Messages list: every recent message, newest first, with its age.

use std::time::{Duration, Instant};

use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, List, ListItem, Paragraph};
use ratatui::Frame;

use crate::app::App;
use crate::format;
use crate::hit::ListHit;
use crate::theme;
use crate::toast::Entry;
use crate::views;

/// Mark, space, three-column age, two spaces.
const PREFIX: usize = 7;

pub fn render_modal(frame: &mut Frame, list: Rect, app: &mut App) {
    let popup = crate::layout::list_modal_rect(list);
    app.hits.overlay = Some(popup);
    app.hits.close = views::close_button_rect(popup);
    frame.render_widget(Clear, popup);

    let len = app.toasts.history_len();
    let block = views::pane_block("Messages", true, Some("C clear · Esc close".into()))
        .title(views::close_title());
    if len == 0 {
        let empty = views::placeholder("No messages yet");
        frame.render_widget(Paragraph::new(empty).block(block), popup);
        return;
    }

    let inner = block.inner(popup);
    let text_w = usize::from(inner.width).saturating_sub(PREFIX);
    let selected = app.message_cursor.selected(len);
    let longest = selected
        .and_then(|i| app.toasts.history().nth(i))
        .map_or(0, |e| e.text.chars().count());
    app.message_h_offset = app.message_h_offset.min(longest.saturating_sub(text_w));

    let now = Instant::now();
    let items: Vec<ListItem> = app
        .toasts
        .history()
        .enumerate()
        .map(|(i, entry)| {
            let offset = if Some(i) == selected {
                app.message_h_offset
            } else {
                0
            };
            message_row(entry, now, text_w, offset)
        })
        .collect();

    let list_widget = List::new(items)
        .block(block)
        .highlight_style(views::highlight(true));
    app.messages_list_state.select(selected);
    frame.render_stateful_widget(list_widget, popup, &mut app.messages_list_state);
    app.hits.overlay_list = Some(ListHit {
        inner,
        offset: app.messages_list_state.offset(),
        len,
    });
}

fn message_row(entry: &Entry, now: Instant, text_w: usize, offset: usize) -> ListItem<'static> {
    let (mark, style) = views::level_mark(entry.level);
    let age = short_age(now.saturating_duration_since(entry.at));
    let text_style = if entry.fresh {
        theme::strong()
    } else {
        theme::text()
    };
    ListItem::new(Line::from(vec![
        Span::styled(format!("{mark} "), style),
        Span::styled(format!("{age:>3}  "), theme::dim()),
        Span::styled(format::pan(&entry.text, offset, text_w), text_style),
    ]))
}

/// `12s`, `3m`, `2h`: seconds under a minute, minutes under an hour, then hours.
pub(crate) fn short_age(age: Duration) -> String {
    let secs = age.as_secs();
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        format!("{}m", secs / 60)
    } else {
        format!("{}h", secs / 3600)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ages_use_the_largest_whole_unit() {
        assert_eq!(short_age(Duration::from_secs(5)), "5s");
        assert_eq!(short_age(Duration::from_secs(59)), "59s");
        assert_eq!(short_age(Duration::from_secs(180)), "3m");
        assert_eq!(short_age(Duration::from_secs(7300)), "2h");
    }
}
```

`views::highlight` is added in Task 9; until then add this to `views/mod.rs` now (Task 9 changes its body):

```rust
/// Highlight for the selected row: strong when the list has focus.
pub(crate) fn highlight(focused: bool) -> Style {
    if focused {
        theme::selected()
    } else {
        no_style()
    }
}
```

In `views/mod.rs`, add `mod messages;` and the modal arm `Modal::Messages => messages::render_modal(frame, list, app),`.

- [ ] **Step 5: App wiring**

In `znicz-tui/src/app.rs`:

- `enum Modal`: add `Messages,`. In `Modal::blocks_list_focus`, add `| Self::Messages`.
- `App` fields (after `station_cursor`): `pub message_cursor: Cursor,` and `pub message_h_offset: usize,`; after `station_list_state`: `pub(crate) messages_list_state: ratatui::widgets::ListState,`. Initialise in `with_engine`: `message_cursor: Cursor::new(), message_h_offset: 0, messages_list_state: ratatui::widgets::ListState::default(),`.
- `context()`: add `Modal::Messages => Context::Messages,`.
- Add, below `toggle_radio_modal`:

```rust
    fn toggle_messages_modal(&mut self) {
        if self.modal == Modal::Messages {
            self.close_modal();
        } else {
            self.toasts.mark_read();
            self.message_cursor.first();
            self.message_h_offset = 0;
            self.modal = Modal::Messages;
        }
    }

    /// Close whatever modal is open, tidying what it leaves behind.
    fn close_modal(&mut self) {
        if self.modal == Modal::Messages {
            self.toasts.clear_fresh();
        }
        self.modal = Modal::None;
        self.playlist_prompt = None;
        self.radio_prompt = None;
    }

    /// Keys the Messages list owns. Returns false for anything else.
    fn on_messages_key(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Char('C') => {
                // No message here: it would land in the history just cleared.
                self.toasts.clear_history();
                self.message_cursor.first();
                self.message_h_offset = 0;
                true
            }
            _ => false,
        }
    }
```

- `on_key`: after the `Modal::Radio` line add `if self.modal == Modal::Messages && self.on_messages_key(key) { return; }`.
- `on_global_key`: add `KeyCode::Char('M') => self.toggle_messages_modal(),` next to `'R'`.
- `on_esc`: replace the first branch's body and matches with:

```rust
        if matches!(
            self.modal,
            Modal::Devices | Modal::Inspector | Modal::Playlists | Modal::Radio | Modal::Messages
        ) {
            self.close_modal();
        } else if …
```

- `on_left_click` X branch: replace the three lines under `if self.modal != Modal::None {` with `self.close_modal();`.
- `on_wheel`: add `Modal::Messages => self.message_cursor.step(delta, self.toasts.history_len()),`.
- `on_overlay_click`: add `Modal::Messages => self.message_cursor.set(index, self.toasts.history_len()),`.
- `reload_front`: add `Modal::Messages => {}` before `_`.
- `list_len`: add first branch `if self.modal == Modal::Messages { self.toasts.history_len() } else …`.
- `step`, `page`, `go_first`, `go_last`: add a first branch for `Modal::Messages` mirroring the Devices one but on `self.message_cursor`, and reset `self.message_h_offset = 0` after moving.
- `pan_titles`: add a first branch:

```rust
        if self.modal == Modal::Messages {
            let next = self.message_h_offset as isize + dir;
            // Clamped to the text when drawn.
            self.message_h_offset = next.max(0) as usize;
            return;
        }
```

- [ ] **Step 6: Run and verify**

Run: `cargo test -p znicz-tui` and clippy.
Expected: PASS, clean.

- [ ] **Step 7: Commit**

```bash
git add znicz-tui/src znicz-tui/tests
git commit -m "Add the Messages list on M"
```

---

### Task 7: Unread count in the footer

**Files:**
- Modify: `znicz-tui/src/footer_hits.rs`, `znicz-tui/src/views/status.rs`
- Test: `footer_hits.rs` tests, `znicz-tui/tests/render.rs`

- [ ] **Step 1: Write the failing render test**

Add to `znicz-tui/tests/render.rs`:

```rust
#[test]
fn the_footer_counts_unread_alerts() {
    let mut app = App::with_library(player(), None);
    app.toasts.warn("stream dropped");
    app.toasts.error("device refused");
    let screen = draw(&mut app, 120, 24);
    let row = screen.lines().last().unwrap();
    assert!(row.contains("M messages (2)"), "{row}");
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p znicz-tui --test render the_footer_counts`
Expected: FAIL (footer shows `M messages`).

- [ ] **Step 3: Owned footer labels**

In `znicz-tui/src/footer_hits.rs`:

- Add `use crossterm::event::KeyEvent;`.
- Add:

```rust
/// What a footer hint says on screen, and the key a click sends.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FooterLabel {
    pub text: String,
    pub key: KeyEvent,
}

impl From<Hint> for FooterLabel {
    fn from(hint: Hint) -> Self {
        Self {
            text: hint.label.to_string(),
            key: hint.key,
        }
    }
}
```

- Change `PlacedHint` to `#[derive(Debug, Clone, PartialEq, Eq)] pub struct PlacedHint { pub rect: Rect, pub label: FooterLabel }`.
- `label_width` takes `&FooterLabel` and counts `label.text.chars()`.
- `layout_footer(area: Rect, labels: &[FooterLabel], more: Option<FooterLabel>) -> Vec<PlacedHint>`: same algorithm; push `PlacedHint { rect, label: label.clone() }` (and `label: more` for more).
- `footer_hits` maps `key: p.label.key`.
- Tests: change helper `h` to return `FooterLabel { text: label.to_string(), key: … }`, `labels()` to return `Vec<String>` from `p.label.text.clone()`, and compare against `["/ search", "a add", "? more"]` etc. via `assert_eq!(labels(&placed), vec!["/ search", "a add", "? more"])`. Pass `&[h(..), h(..)]` and `Some(h(..))` as before.

In `znicz-tui/src/views/status.rs`:

```rust
pub fn render_footer(frame: &mut Frame, area: Rect, app: &mut App) {
    let context = app.context();
    let unread = app.toasts.unread_alerts().map(|(count, _)| count);
    let labels: Vec<FooterLabel> = keys::context_hints(context)
        .iter()
        .map(|hint| display_label(*hint, unread))
        .collect();
    let more = keys::more_hint(context).map(FooterLabel::from);
    let placed = footer_hits::layout_footer(area, &labels, more);
    app.hits.footer_hints = footer_hits::footer_hits(&placed);
    frame.render_widget(Paragraph::new(footer_line(area, &placed)), area);
}

/// The hint as drawn: `M messages` carries the unread-alerts count.
fn display_label(hint: Hint, unread: Option<usize>) -> FooterLabel {
    let mut label = FooterLabel::from(hint);
    if hint == keys::MESSAGES_OPEN {
        if let Some(count) = unread {
            label.text = format!("{} ({count})", hint.label);
        }
    }
    label
}
```

Import `FooterLabel` and `keys::Hint`. In `footer_line`, read the text from `item.label.text` (`split_once(' ')` on it) and produce owned spans (`Span::styled(key.to_string(), …)`).

- [ ] **Step 4: Run and verify**

Run: `cargo test -p znicz-tui` and clippy.
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add znicz-tui/src/footer_hits.rs znicz-tui/src/views/status.rs znicz-tui/tests/render.rs
git commit -m "Count unread alerts on the footer's messages hint"
```

---

### Task 8: Help returns to the modal it replaced; border follows the filter

**Files:**
- Modify: `znicz-tui/src/help_state.rs`, `znicz-tui/src/app.rs`, `znicz-tui/src/views/help.rs`
- Test: `znicz-tui/tests/keys.rs`, `znicz-tui/tests/render.rs`

- [ ] **Step 1: Write the failing tests**

`znicz-tui/tests/keys.rs`:

```rust
#[test]
fn closing_help_returns_to_the_modal_it_replaced() {
    let mut app = new_app();
    press_char(&mut app, ',');
    press_char(&mut app, '?');
    assert_eq!(app.modal, Modal::Help);
    press(&mut app, KeyCode::Esc);
    assert_eq!(app.modal, Modal::Devices);

    press_char(&mut app, '?');
    press_char(&mut app, 'q');
    assert_eq!(app.modal, Modal::Devices);
    assert!(!app.should_quit);
}

#[test]
fn closing_help_opened_from_the_library_returns_to_nothing() {
    let mut app = new_app();
    press_char(&mut app, '?');
    press(&mut app, KeyCode::Esc);
    assert_eq!(app.modal, Modal::None);
}
```

`znicz-tui/tests/mouse.rs`:

```rust
#[test]
fn the_help_x_returns_to_the_modal_it_replaced() {
    let mut app = new_app();
    app.on_key(KeyEvent::new(KeyCode::Char(','), KeyModifiers::NONE));
    app.on_key(KeyEvent::new(KeyCode::Char('?'), KeyModifiers::NONE));
    app.hits.close = Some(Rect::new(68, 4, 1, 1));
    app.on_mouse(left_click(68, 4));
    assert_eq!(app.modal, Modal::Devices);
}
```

`znicz-tui/tests/render.rs`:

```rust
#[test]
fn the_help_border_says_keep_and_clear_while_filtering() {
    let mut app = App::with_library(player(), None);
    app.modal = Modal::Help;
    app.help.editing = true;
    let screen = draw(&mut app, 100, 40);
    let border = screen
        .lines()
        .find(|line| line.contains("znicz "))
        .expect("help bottom border");
    assert!(border.contains("Enter keep · Esc clear"), "{border}");
}
```

- [ ] **Step 2: Run to verify they fail**

Run the three tests. Expected: FAIL (help closes to `None`; border reads `/ filter · Esc close`).

- [ ] **Step 3: Implement**

`help_state.rs`: add `use crate::app::Modal;`, a field

```rust
    /// The modal help replaced; closing help goes back to it.
    pub return_to: Modal,
```

and `return_to: Modal::None,` in `open`.

`app.rs`:

- `open_help`:

```rust
    fn open_help(&mut self) {
        let return_to = self.modal;
        self.help = HelpState::open(self.context());
        self.help.return_to = return_to;
        self.modal = Modal::Help;
    }
```

- `close_modal`: add as its first lines

```rust
        if self.modal == Modal::Help {
            self.modal = self.help.return_to;
            return;
        }
```

- `on_help_key`: the close arm becomes `KeyCode::Esc | KeyCode::Char('?') | KeyCode::Char('q') => self.close_modal(),`.

`views/help.rs`:

- Replace `const CLOSE_HINT: &str = …;` with:

```rust
const CLOSE_HINT: &str = " / filter · Esc close ";
const FILTER_HINT: &str = " Enter keep · Esc clear ";

/// Bottom-right border text: what Enter and Esc do right now.
fn border_hint(editing: bool) -> &'static str {
    if editing {
        FILTER_HINT
    } else {
        CLOSE_HINT
    }
}
```

- In `render`, compute `let hint = border_hint(app.help.editing);`, use `Span::styled(hint, theme::dim())` for `title_bottom`, and `version_title(popup.width, hint)`.
- `version_title(popup_width: u16, hint: &str)`: use `hint.chars().count()` instead of `CLOSE_HINT`.
- Update its tests to pass `CLOSE_HINT` as the second argument.

- [ ] **Step 4: Run and verify**

Run: `cargo test -p znicz-tui` and clippy. Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add znicz-tui/src znicz-tui/tests
git commit -m "Return from help to the modal it replaced"
```

---

### Task 9: Focus styling

**Files:**
- Modify: `znicz-tui/src/theme.rs`, `znicz-tui/src/views/mod.rs`, `views/library.rs`, `views/queue.rs`, `views/devices.rs`, `views/playlists.rs`, `views/radio.rs`
- Test: `znicz-tui/tests/render.rs`

- [ ] **Step 1: Write the failing render test**

Add to `znicz-tui/tests/render.rs`:

```rust
fn library_app() -> App {
    let mut library = znicz_library::Library::open_in_memory().expect("open library");
    for (path, title, artist, album) in [
        ("/music/a1.flac", "So What", "Miles", "Kind of Blue"),
        ("/music/a2.flac", "Freddie Freeloader", "Miles", "Kind of Blue"),
        ("/music/sour-times.flac", "Sour Times", "Portishead", "Dummy"),
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

fn buffer(app: &mut App, width: u16, height: u16) -> ratatui::buffer::Buffer {
    let state = app.state();
    let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("terminal");
    terminal
        .draw(|frame| views::render(frame, app, &state))
        .expect("draw");
    terminal.backend().buffer().clone()
}

#[test]
fn the_focused_selection_is_accent_and_others_stay_grey() {
    use ratatui::style::Color;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    let mut app = library_app();
    // First artist row: inside the Artists column border, row 1.
    let artists = buffer(&mut app, 120, 30);
    assert_eq!(artists[(2, 1)].bg, Color::Cyan, "focused column");

    app.on_key(KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE));
    let albums = buffer(&mut app, 120, 30);
    assert_eq!(albums[(2, 1)].bg, Color::DarkGray, "artist stays visible, grey");
}
```

(Add `use crossterm::...` at the test's top only if `crossterm` is not already imported in render.rs; it is a dependency of `znicz-tui`, so it is available to its tests.)

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p znicz-tui --test render the_focused_selection`
Expected: FAIL (focused bg is DarkGray; unfocused has no bg).

- [ ] **Step 3: Implement**

`theme.rs`: replace `selected()` and add `selected_idle()`:

```rust
/// The row under the cursor in the list that has focus.
pub fn selected() -> Style {
    Style::default()
        .bg(ACCENT)
        .fg(Color::Black)
        .add_modifier(Modifier::BOLD)
}

/// The selected row in a list without focus: still visible, quieter.
pub fn selected_idle() -> Style {
    Style::default().bg(Color::DarkGray).fg(TEXT)
}
```

`views/mod.rs`: change `highlight`'s body to `if focused { theme::selected() } else { theme::selected_idle() }` and delete `no_style` once nothing uses it.

In each of `views/library.rs` (three places: `render_column_list`, `render_tree`, `finish_list`), `views/queue.rs`, `views/devices.rs`, `views/playlists.rs`, `views/radio.rs`, replace

```rust
.highlight_style(if focused {
        theme::selected()
    } else {
        views::no_style()
    })
```

with `.highlight_style(views::highlight(focused))`.

- [ ] **Step 4: Run and verify**

Run: `cargo test -p znicz-tui` and clippy. Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add znicz-tui/src znicz-tui/tests/render.rs
git commit -m "Keep selections visible and show focus by colour"
```

---

### Task 10: Breadcrumbs

**Files:**
- Modify: `znicz-tui/src/views/mod.rs`, `znicz-tui/src/views/library.rs`
- Test: `views/mod.rs` tests, `znicz-tui/tests/render.rs`

- [ ] **Step 1: Write the failing tests**

Unit tests in `znicz-tui/src/views/mod.rs` (add a `#[cfg(test)] mod tests { use super::*; … }` if there is none):

```rust
    #[test]
    fn a_short_breadcrumb_is_joined_whole() {
        assert_eq!(
            breadcrumb(&["Library", "Miles", "Kind of Blue"], 40),
            "Library › Miles › Kind of Blue"
        );
    }

    #[test]
    fn middle_segments_shrink_before_the_last() {
        let crumb = breadcrumb(&["Library", "Miles Davis Quintet", "Kind of Blue"], 30);
        assert_eq!(crumb.chars().count(), 30);
        assert!(crumb.starts_with("Library › "), "{crumb}");
        assert!(crumb.ends_with(" › Kind of Blue"), "{crumb}");
    }

    #[test]
    fn library_is_never_shortened() {
        let crumb = breadcrumb(&["Library", "Miles", "Kind of Blue"], 16);
        assert!(crumb.starts_with("Library"), "{crumb}");
        assert!(crumb.chars().count() <= 16, "{crumb}");
    }
```

Render tests in `znicz-tui/tests/render.rs` (uses `library_app` from Task 9):

```rust
#[test]
fn column_titles_name_their_parent() {
    let mut app = library_app();
    let screen = draw(&mut app, 120, 30);
    assert!(screen.contains("Albums · Miles"), "{screen}");
    assert!(screen.contains("Tracks · Kind of Blue"), "{screen}");
}

#[test]
fn the_tree_title_follows_the_cursor() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    let mut app = library_app();
    app.tui.library_layout = znicz_tui::LibraryLayout::Tree;
    app.on_key(KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE)); // expand Miles
    app.on_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE)); // album row
    let screen = draw(&mut app, 120, 30);
    assert!(screen.contains("Library › Miles › Kind of Blue"), "{screen}");
}

#[test]
fn paging_titles_show_the_path() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    let mut app = library_app();
    draw(&mut app, 50, 30); // narrow: single-column paging
    app.on_key(KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE));
    let screen = draw(&mut app, 50, 30);
    assert!(screen.contains("Library › Miles"), "{screen}");
}
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test -p znicz-tui --lib breadcrumb` and `cargo test -p znicz-tui --test render title`
Expected: compile error, then FAIL.

- [ ] **Step 3: `breadcrumb`**

In `znicz-tui/src/views/mod.rs`:

```rust
const CRUMB_SEPARATOR: &str = " › ";

/// `Library › Artist › Album`, fitted to `width` characters.
///
/// Middle segments shrink first, then the last; the first is never shortened.
pub(crate) fn breadcrumb(parts: &[&str], width: usize) -> String {
    let mut parts: Vec<String> = parts.iter().map(|p| p.to_string()).collect();
    let joined = |parts: &[String]| parts.join(CRUMB_SEPARATOR);
    let len = parts.len();
    if len > 1 {
        for i in (1..len - 1).chain(std::iter::once(len - 1)) {
            let over = joined(&parts).chars().count().saturating_sub(width);
            if over == 0 {
                break;
            }
            let keep = parts[i].chars().count().saturating_sub(over).max(1);
            parts[i] = format::truncate(&parts[i], keep);
        }
    }
    format::truncate(&joined(&parts), width)
}

/// Room for a pane title: the width minus corners and the padding spaces.
pub(crate) fn title_width(area: Rect) -> usize {
    usize::from(area.width).saturating_sub(4)
}
```

- [ ] **Step 4: Use it in the library titles**

In `znicz-tui/src/views/library.rs`:

- `render_columns`: before the three calls, compute

```rust
    let artist = app
        .library
        .selected_artist_index()
        .and_then(|i| app.library.artists().get(i))
        .map(|a| a.name.clone());
    let album = app
        .library
        .selected_album_index()
        .and_then(|i| app.library.albums().get(i))
        .map(|a| a.album.clone());
    let albums_title = column_title("Albums", artist.as_deref(), views::title_width(parts[1]));
    let tracks_title = column_title("Tracks", album.as_deref(), views::title_width(parts[2]));
```

and pass `&albums_title` / `&tracks_title` instead of `"Albums"` / `"Tracks"`. Add:

```rust
/// `Albums · Miles`: a column title naming its parent selection.
fn column_title(name: &str, parent: Option<&str>, width: usize) -> String {
    match parent {
        Some(parent) => format::truncate(&format!("{name} · {parent}"), width),
        None => name.to_string(),
    }
}
```

- `render_tree`: replace `let title = "Library".to_string();` with

```rust
    let crumbs = tree_crumbs(&rows, selected);
    let parts: Vec<&str> = std::iter::once("Library")
        .chain(crumbs.iter().map(String::as_str))
        .collect();
    let title = views::breadcrumb(&parts, views::title_width(area));
```

and add:

```rust
/// Artist (and album) names above the tree cursor, for the title.
fn tree_crumbs(rows: &[TreeRow], selected: Option<usize>) -> Vec<String> {
    let Some(index) = selected else {
        return Vec::new();
    };
    // A track row belongs to the nearest album row above it.
    let index = match rows.get(index) {
        Some(TreeRow::Track { .. }) => rows[..index]
            .iter()
            .rposition(|r| matches!(r, TreeRow::Album { .. }))
            .unwrap_or(index),
        _ => index,
    };
    match rows.get(index) {
        Some(TreeRow::Artist { artist, .. }) => vec![artist.name.clone()],
        Some(TreeRow::Album { artist, album, .. }) => vec![artist.clone(), album.album.clone()],
        _ => Vec::new(),
    }
}
```

- `render_paging`: replace the `let title = match … ;` block with

```rust
    let artist = app
        .library
        .selected_artist_index()
        .and_then(|i| app.library.artists().get(i))
        .map(|a| a.name.clone())
        .unwrap_or_default();
    let album = app
        .library
        .selected_album_index()
        .and_then(|i| app.library.albums().get(i))
        .map(|a| a.album.clone())
        .unwrap_or_default();
    let parts: Vec<&str> = match app.library.paging_level() {
        ColumnFocus::Artists => vec!["Library"],
        ColumnFocus::Albums => vec!["Library", &artist],
        ColumnFocus::Tracks => vec!["Library", &artist, &album],
    };
    let title = views::breadcrumb(&parts, views::title_width(area));
```

- `render_single_list`: the title becomes

```rust
    let search;
    let parts: Vec<&str> = match app.library.mode() {
        Mode::AllTracks => vec!["Library", "all tracks"],
        Mode::Search(query) => {
            search = format!("search \"{query}\"");
            vec!["Library", &search]
        }
        Mode::Browse => vec!["Library"],
    };
    let title = views::breadcrumb(&parts, views::title_width(area));
```

- [ ] **Step 5: Run and verify**

Run: `cargo test -p znicz-tui` and clippy. Expected: PASS. Fix any older render test that asserted the old `Library / …` text (grep `"Library /"` in `znicz-tui/tests`) to the new ` › ` form.

- [ ] **Step 6: Commit**

```bash
git add znicz-tui/src znicz-tui/tests
git commit -m "Show breadcrumbs in library titles"
```

---

### Task 11: Playing marker

**Files:**
- Create: `znicz-tui/src/playing.rs`
- Modify: `znicz-tui/src/lib.rs`, `znicz-tui/src/library_pane.rs`, `znicz-tui/src/app.rs`, `znicz-tui/src/views/mod.rs`, `znicz-tui/src/views/library.rs`
- Test: `playing.rs`, `znicz-tui/tests/render.rs`

- [ ] **Step 1: Create `playing.rs` with its tests**

```rust
//! Which library rows stand for what is playing.

use std::path::PathBuf;

use znicz_core::TrackInfo;
use znicz_library::{AlbumSummary, ArtistSummary, Track};

/// The playing file, and the browse artist and album it belongs to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlayingRef {
    pub path: PathBuf,
    pub artist: Option<String>,
    pub album: Option<String>,
}

/// Case-insensitive for any script, not just ASCII.
fn same(a: &str, b: &str) -> bool {
    a.to_lowercase() == b.to_lowercase()
}

impl PlayingRef {
    /// From the library's own row: browse artist is the album artist, else the artist.
    pub fn from_track(track: &Track) -> Self {
        Self {
            path: track.path.clone(),
            artist: track.album_artist.clone().or_else(|| track.artist.clone()),
            album: track.album.clone(),
        }
    }

    /// From the file's tags, when the library does not know the file. Streams give `None`.
    pub fn from_info(info: &TrackInfo) -> Option<Self> {
        Some(Self {
            path: info.path.clone()?,
            artist: info
                .tags
                .album_artist
                .clone()
                .or_else(|| info.tags.artist.clone()),
            album: info.tags.album.clone(),
        })
    }

    pub fn is_track(&self, track: &Track) -> bool {
        track.path == self.path
    }

    pub fn is_artist(&self, artist: &ArtistSummary) -> bool {
        self.artist.as_deref().is_some_and(|a| same(a, &artist.name))
    }

    pub fn is_album(&self, album: &AlbumSummary) -> bool {
        let Some(name) = self.album.as_deref() else {
            return false;
        };
        let artist_matches = match (album.album_artist.as_deref(), self.artist.as_deref()) {
            (Some(album_artist), Some(artist)) => same(album_artist, artist),
            _ => true,
        };
        same(name, &album.album) && artist_matches
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn summary(album: &str, artist: Option<&str>) -> AlbumSummary {
        AlbumSummary {
            album: album.into(),
            album_artist: artist.map(Into::into),
            year: None,
            track_count: 1,
            total_secs: None,
        }
    }

    fn playing() -> PlayingRef {
        PlayingRef {
            path: PathBuf::from("/music/a.flac"),
            artist: Some("Кино".into()),
            album: Some("Группа крови".into()),
        }
    }

    #[test]
    fn artist_and_album_match_ignoring_case_in_any_script() {
        let p = playing();
        assert!(p.is_artist(&ArtistSummary {
            name: "КИНО".into(),
            track_count: 1
        }));
        assert!(p.is_album(&summary("группа крови", Some("кино"))));
        assert!(p.is_album(&summary("Группа крови", None)), "no album artist");
        assert!(!p.is_album(&summary("Группа крови", Some("Other"))));
    }

    #[test]
    fn a_stream_has_no_playing_ref() {
        let info = TrackInfo {
            path: None,
            url: Some("http://radio".into()),
            ..TrackInfo::default()
        };
        assert_eq!(PlayingRef::from_info(&info), None);
    }
}
```

If `TrackInfo` does not implement `Default`, build it with the full field list as in `tests/render.rs` `playing_state()` (path `None`, `url: Some(..)`, other fields zero/empty).

In `lib.rs`, add `pub mod playing;`.

Run: `cargo test -p znicz-tui --lib playing::` — Expected: PASS.

- [ ] **Step 2: Write the failing render test**

```rust
#[test]
fn the_library_marks_the_playing_track_album_and_artist() {
    let mut app = library_app();
    let screen = draw_with(&mut app, &playing_state(), 140, 30);
    assert!(screen.contains("Portishead ♪"), "{screen}");
    // Select Portishead so its albums and tracks show.
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    app.on_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE));
    let screen = draw_with(&mut app, &playing_state(), 140, 30);
    assert!(screen.contains("Dummy ♪"), "{screen}");
    assert!(
        screen
            .lines()
            .any(|l| l.contains("Sour Times — Portishead") && l.contains('▶')),
        "{screen}"
    );
    assert!(!screen.contains("Miles ♪"), "{screen}");
}
```

Run it — Expected: FAIL.

- [ ] **Step 3: Resolve the playing track once per change**

`library_pane.rs`, in `impl LibraryPane`:

```rust
    /// The library's row for a file, if it has one.
    pub fn locate(&self, path: &std::path::Path) -> Option<Track> {
        self.library.as_ref()?.get_by_path(path).ok().flatten()
    }
```

`app.rs`: add the field `pub(crate) playing: Option<crate::playing::PlayingRef>,` (init `None`) and:

```rust
    /// Re-resolve what is playing when the current file changes.
    pub fn refresh_playing(&mut self, state: &PlayerState) {
        let info = state.current_track.as_ref();
        let path = info.and_then(|t| t.path.as_ref());
        if self.playing.as_ref().map(|p| &p.path) == path {
            return;
        }
        self.playing = info.and_then(|info| {
            let path = info.path.as_ref()?;
            self.library
                .locate(path)
                .map(|track| crate::playing::PlayingRef::from_track(&track))
                .or_else(|| crate::playing::PlayingRef::from_info(info))
        });
    }
```

`views/mod.rs` `render`: call `app.refresh_playing(state);` right after `app.hits = HitMap::default();`.

- [ ] **Step 4: Draw the markers**

In `views/library.rs`:

- Change `list_item` to take `playing: bool` and style the middle and right with `theme::playing()` when true:

```rust
fn list_item(middle: &str, right: &str, strip: usize, offset: usize, playing: bool) -> ListItem<'static> {
    let right_w = right.chars().count();
    let middle_w = strip.saturating_sub(right_w.saturating_add(1));
    let shown = format::pan(middle, offset, middle_w);
    let pad = middle_w.saturating_sub(shown.chars().count());
    let (middle_style, right_style) = if playing {
        (theme::playing(), theme::playing())
    } else {
        (Style::default(), theme::dim())
    };
    ListItem::new(Line::from(vec![
        Span::styled(shown, middle_style),
        Span::raw(" ".repeat(pad)),
        Span::styled(format!(" {right}"), right_style),
    ]))
}
```

(import `ratatui::style::Style`).

- Give `artist_row`, `album_row`, `track_row`, `hit_row`, and `tree_row` a `playing: Option<&PlayingRef>` parameter:
  - `artist_row`: `let mark = playing.is_some_and(|p| p.is_artist(artist));` middle = name, plus `" ♪"` when `mark`; pass `mark` to `list_item`.
  - `album_row`: same with `is_album`.
  - `track_row`: `let mark = playing.is_some_and(|p| p.is_track(track));` and when `mark` the number part is `" ▶ "` instead of `track_number(track)`; pass `mark`.
  - `hit_row`: forward `playing` to the three cases (artist/album hits use `is_artist` / `is_album` with the same ` ♪`).
  - `tree_row`: Artist and Album rows append ` ♪` after the name when matched; Track rows use `▶` in place of nothing — prefix the middle with `▶ ` after the indent when matched — and pass `mark` to `list_item`.
- At the top of each render function that builds rows (`render_columns`, `render_tree`, `render_paging`, `render_single_list`), take `let playing = app.playing.clone();` and pass `playing.as_ref()` into the row builders.

Widths: the ` ♪` is part of the middle text, so pan and truncation already account for it.

- [ ] **Step 5: Run and verify**

Run: `cargo test -p znicz-tui` and clippy. Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add znicz-tui/src znicz-tui/tests/render.rs
git commit -m "Mark the playing track, album, and artist in the library"
```

---

### Task 12: `J` — jump to the playing track

**Files:**
- Modify: `znicz-tui/src/library_pane.rs`, `znicz-tui/src/app.rs`, `znicz-tui/src/keys.rs`
- Test: `library_pane.rs` tests, `znicz-tui/tests/keys.rs`

- [ ] **Step 1: Write the failing unit tests**

In `library_pane.rs` tests (uses `two_artist_library()` from part 1):

```rust
    fn freddie(pane: &LibraryPane) -> Track {
        pane.locate(Path::new("/music/a2.flac")).expect("in library")
    }

    fn selected_path(pane: &LibraryPane, strip: usize) -> Option<std::path::PathBuf> {
        match pane.selected(strip) {
            Some(Item::Track(track)) => Some(track.path.clone()),
            _ => None,
        }
    }

    #[test]
    fn reveal_selects_artist_album_and_track_in_columns() {
        let mut pane = LibraryPane::new(Some(two_artist_library()));
        let track = freddie(&pane);
        assert!(pane.reveal(&track, wide()));
        assert_eq!(pane.column_focus(), ColumnFocus::Tracks);
        assert_eq!(selected_path(&pane, wide()), Some("/music/a2.flac".into()));
    }

    #[test]
    fn reveal_drills_paging_to_the_track() {
        let mut pane = LibraryPane::new(Some(two_artist_library()));
        let track = freddie(&pane);
        assert!(pane.reveal(&track, strip()));
        assert_eq!(pane.paging_level(), ColumnFocus::Tracks);
        assert_eq!(selected_path(&pane, strip()), Some("/music/a2.flac".into()));
    }

    #[test]
    fn reveal_expands_the_tree_to_the_track() {
        let mut pane = LibraryPane::new(Some(two_artist_library()));
        pane.set_preferred_layout(LibraryLayout::Tree);
        let track = freddie(&pane);
        assert!(pane.reveal(&track, strip()));
        assert_eq!(selected_path(&pane, strip()), Some("/music/a2.flac".into()));
    }

    #[test]
    fn reveal_leaves_search_first() {
        let mut pane = LibraryPane::new(Some(two_artist_library()));
        pane.begin_search();
        for c in "Roads".chars() {
            pane.push_char(c);
        }
        pane.submit_search();
        assert!(matches!(pane.mode(), Mode::Search(_)));
        let track = freddie(&pane);
        assert!(pane.reveal(&track, wide()));
        assert!(matches!(pane.mode(), Mode::Browse));
    }

    #[test]
    fn reveal_without_an_album_fails() {
        let mut pane = LibraryPane::new(Some(two_artist_library()));
        let mut track = freddie(&pane);
        track.album = None;
        assert!(!pane.reveal(&track, wide()));
    }
```

Run: `cargo test -p znicz-tui --lib library_pane::tests::reveal` — Expected: compile error.

- [ ] **Step 2: Implement `reveal`**

In `impl LibraryPane`, after `focus_album`:

```rust
    /// `J`: leave search and select `track` in browse, in every layout.
    pub fn reveal(&mut self, track: &Track, strip_inner: usize) -> bool {
        let Some(album) = track.album.as_deref() else {
            return false;
        };
        let album_artist = track.album_artist.as_deref().or(track.artist.as_deref());
        if !self.focus_album(album, album_artist) {
            return false;
        }
        let Some(index) = self.tracks.iter().position(|t| t.path == track.path) else {
            return false;
        };
        self.track_cursor.set(index, self.tracks.len());
        if self.effective_layout(strip_inner) == EffectiveLayout::Tree {
            self.reveal_in_tree(index);
        }
        self.h_offset = 0;
        true
    }

    /// Load the album's tracks into the tree and put the cursor on one of them.
    fn reveal_in_tree(&mut self, track_index: usize) {
        let Some(album) = self
            .selected_album_index()
            .and_then(|i| self.albums.get(i))
            .cloned()
        else {
            return;
        };
        let key = album.album.to_lowercase();
        self.tree_track_cache.insert(key.clone(), self.tracks.clone());
        self.tree_album_cache.insert(key.clone(), album);
        let rows = self.tree_rows();
        if let Some(row) = rows.iter().position(|r| {
            matches!(r, TreeRow::Track { album_key, index, .. }
                if *album_key == key && *index == track_index)
        }) {
            self.tree_cursor.set(row, rows.len());
        }
    }
```

Run the unit tests — Expected: PASS.

- [ ] **Step 3: Write the failing key tests**

In `znicz-tui/tests/keys.rs` (uses `app_with_library_rows()` from part 1):

```rust
#[test]
fn capital_j_with_nothing_playing_says_so() {
    let mut app = app_with_library_rows();
    press_char(&mut app, 'J');
    assert_eq!(
        app.toasts.current().unwrap().text,
        "nothing playing in the library"
    );
}

#[test]
fn jumping_to_a_library_file_selects_it() {
    let mut app = app_with_library_rows();
    app.jump_to_path(Some(std::path::Path::new("/music/a2.flac")));
    assert_eq!(app.library.column_focus(), ColumnFocus::Tracks);
    assert!(app.toasts.is_empty());
}

#[test]
fn jumping_to_a_file_outside_the_library_says_so() {
    let mut app = app_with_library_rows();
    app.jump_to_path(Some(std::path::Path::new("/elsewhere/x.flac")));
    assert_eq!(
        app.toasts.current().unwrap().text,
        "playing track is not in the library"
    );
}
```

Run — Expected: compile error (`jump_to_path`).

- [ ] **Step 4: Wire `J`**

`app.rs`: add `use std::path::Path;` and, below `library_horizontal`:

```rust
    /// `J`: select the playing track in the library.
    fn jump_to_playing(&mut self) {
        if self.modal != Modal::None {
            return;
        }
        let path = self.player.state().current_track.and_then(|t| t.path);
        self.jump_to_path(path.as_deref());
    }

    /// Select `path` in the library, or say why not. `None`: nothing (or a stream) plays.
    pub fn jump_to_path(&mut self, path: Option<&Path>) {
        let Some(path) = path else {
            self.toasts.info("nothing playing in the library");
            return;
        };
        let strip = self.library_strip();
        let found = match self.library.locate(path) {
            Some(track) => self.library.reveal(&track, strip),
            None => false,
        };
        if !found {
            self.toasts.info("playing track is not in the library");
        }
    }
```

In `on_library_key` add `KeyCode::Char('J') => self.jump_to_playing(),`.

`keys.rs`: in `LIBRARY`, add `b("J", "jump to the playing track"),` after `b("A", "add everything listed"),`.

- [ ] **Step 5: Run and verify**

Run: `cargo test -p znicz-tui` and clippy. Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add znicz-tui/src znicz-tui/tests/keys.rs
git commit -m "Jump to the playing track with J"
```

---

### Task 13: Version, docs, preview

**Files:**
- Modify: `Cargo.toml`, `Cargo.lock`, `README.md`, `wiki/Architecture/TUI.md`, `wiki/Plans/Roadmap.md`, `wiki/Rust/Cargo-Workspace.md`

- [ ] **Step 1: Version**

Root `Cargo.toml` `[workspace.package] version` → `0.7.0`; run `cargo build -p znicz`; update the "currently 0.6.0" mention in `wiki/Rust/Cargo-Workspace.md` to 0.7.0.

- [ ] **Step 2: README**

In the key table add rows after the `]` row:

```markdown
| M | Messages: recent history; unread warnings and errors are counted |
| J | Jump the library to the playing track |
```

In the paragraph about the transport (above "Press `?`"), add a sentence: `Messages appear on the transport's last row rather than over the list.`

- [ ] **Step 3: `wiki/Architecture/TUI.md`**

- Layout diagram: add a message row under the signal line (`         ● added 4 tracks`) and update the prose under it: modals open inside the list region, the transport has a message row, toasts no longer float.
- Replace the "Responsive behaviour" table with the spec's table (cover only at ≥ 24 rows with `show_cover`; text transport 3 rows at ≥ 20, 2 below; list region `Min(3)`), and state that a modal takes the whole list region when it is shorter than 10 rows.
- Add a `## Messages` section (message row contents and order of preference; history of 50; unread = warnings and errors; `M` list with ages, bold fresh rows, `C` clear, `Esc`/`M` close; footer `M messages (N)`), replacing the toast-box description in "Messages instead of silence".
- In "Panes → Library": focus styling (accent vs grey selection), breadcrumb titles per layout, the `▶` / `♪` markers and how the album/artist is resolved, and `J`.
- In "Keys, footer, and help": help returns to the modal it replaced; border reads `Enter keep · Esc clear` while filtering.
- Modules table: add `views/messages.rs` (Messages list) and `playing.rs` (which rows stand for what is playing); `toast.rs` becomes "live messages and history".

- [ ] **Step 4: Roadmap**

In `wiki/Plans/Roadmap.md` "TUI reimagining", part 2 row status → `**Done** in 0.7.0 ([spec](../../docs/superpowers/specs/2026-11-07-space-and-layering-design.md))`.

- [ ] **Step 5: Preview**

Run `cargo run -q -p znicz-tui --example preview -- 110 32 2>/dev/null` and `-- 48 14`. Check: modals sit inside the list region with the transport visible; messages on the transport's last row; no cover at 48×14. If the example references removed APIs (`toasts.visible`, toast layout functions), update it to compile.

- [ ] **Step 6: Commit**

```bash
git add Cargo.toml Cargo.lock README.md wiki znicz-tui/examples
git commit -m "Bump to 0.7.0 and document space and layering"
```

---

### Task 14: Verify and open the PR

- [ ] **Step 1:** `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace` — all clean.
- [ ] **Step 2:** Tick every checkbox in this plan; commit `Mark space and layering plan complete`.
- [ ] **Step 3:** Push `feat/space-and-layering` and open a PR titled "Space and layering (TUI rework part 2)" summarising the spec's goals, ending with `🤖 Generated with [Claude Code](https://claude.com/claude-code)`.
