# Mouse support in the TUI

**Date:** 2026-09-03
**Status:** Approved
**Issue:** [#8](https://github.com/eugene-chekan/znicz/issues/8)
**Crates:** `znicz-tui` (hit map + `on_mouse` + capture), `znicz` (run loop already owns restore)
**Version:** bump `z` (compatible addition)

## Problem

The player is keyboard-only. Ratatui draws; crossterm reads input. Playtesting the layered layout wanted click-to-select lists and click-outside to dismiss overlays. Mouse was a non-goal of the layout spec and is parked as #8.

## Goals

1. **Select only.** A left click on a visible list row moves that list’s cursor, the same as `j`/`k` landing on that row. It does **not** activate (no Enter: no play, no open album).
2. **Wheel.** One notch moves the focused list one row, wrapping like `j`/`k`.
3. **Click outside closes** help, inspector, devices, playlists, radio, and a typing prompt, same as `Esc`.
4. **Queue drawer** opens and closes from a hit target: the library pane’s right-border column, plus click-on-library-to-close when the overlay is open.
5. **Always capture** the mouse while the TUI is up. No config flag and no toggle key in this slice. Keyboard still works if the terminal sends no mouse events (SSH, old clients).

## This slice does not include

- Drag to reorder rows ([#36](https://github.com/eugene-chekan/znicz/issues/36))
- Click the seek bar, cover, footer hints, or toasts
- Double-click to activate
- `[tui] mouse` or a capture-toggle key
- Touch / trackpad gestures
- Making Ratatui widgets handle mouse without our hit-testing

## Architecture

Immediate-mode stays. Each `views::render` writes a **hit map** on `App`:

| Region | What we store |
| --- | --- |
| Library list | Inner rect (inside the pane border), first visible row (`ListState` offset after draw), row count |
| Queue list | Same, when the drawer is open |
| Overlay body | Rect of help / inspector / devices / playlists / radio |
| Overlay list | Inner list rect + offset + count when the overlay has a list |
| Queue toggle | The library pane’s right-border column (`x = list.x + list.width - 1`, full list height) |

`ListState` for those lists is kept on `App` (not built and dropped in render) so the offset after `render_stateful_widget` is the one hit-testing uses.

The run loop enables crossterm `EnableMouseCapture` with the TUI and disables it on restore (same path as today for the alternate screen). `Event::Mouse` calls `App::on_mouse`. Tests call `on_mouse` with a filled-in hit map; they do not need a tty.

Keyboard bindings and `on_key` do not change. Mouse never synthesizes key events.

## Data flow

```
crossterm Event::Mouse
        │
        ▼
App::on_mouse(kind, column, row)
        │  look at hit map from last draw
        ▼
Cursor::set / Cursor::step / queue_open / modal / prompt cancel
```

Priority: a mouse event never becomes seek (`h`/`l`) or title pan (`Alt-←` / `Alt-→`). Those stay keys only.

## Click rules

Use **left button down**. Ignore right, middle, drag, move, and up. A second click is another select, not activate.

**Overlay up** (help, inspector, devices, playlists, radio):

- Click **inside** a list overlay’s list inner rect → select that visible row (devices / playlists / radio).
- Click **inside** the overlay body but not on a row (help, inspector, padding, title) → ignore.
- Click **outside** the overlay body → close, same as `Esc`.

**Typing prompt** (library search, playlist/station name fields): mouse does not insert text and does not change list cursors. A click **outside** the prompt line (library search) or **outside** the overlay body (playlist/station forms) **cancels** the prompt, same as `Esc` — the overlay stays up until a later click-outside with no prompt. Other clicks while typing are ignored.

**No overlay:**

- Click a **visible library row** (inner rect, not covered by the queue overlay) → focus library, `Cursor::set` that index.
- Click a **visible queue row** → focus queue, set that index.
- Click the **queue-toggle column** while the drawer is closed → open the drawer and focus the queue.
- Click the **library** (including the toggle column) while the drawer is an overlay → close the drawer and focus the library.
- Click the **toggle column** while the drawer is a full-width sheet → close the sheet and focus the library. (There is no library strip.)
- Empty list, title/border except the toggle column, transport, cover, footer, toasts → ignore.

Row index: `offset + (click.y - inner.y)`. If that index is `>= len`, or `y` is outside the inner rect, ignore — do not clamp onto another item.

When the queue overlay covers the right of the library, those columns belong to the queue, not the library.

## Wheel rules

`ScrollUp` → `Cursor::step(-1)` (previous row). `ScrollDown` → `Cursor::step(+1)` (next row). One notch, wrapping like `j`/`k`.

Target:

1. If a **list overlay** is open (devices, playlists, radio) and no typing prompt is open → that overlay’s list.
2. Else if **no overlay** is open → the focused pane: library, or queue when the drawer is open and focused.
3. Else ignore (help, inspector, or a typing prompt).

Wheel does not close overlays, does not toggle the queue, and does not seek. Wheel over chrome with no list target is ignored.

## Terminals without mouse

`EnableMouseCapture` is a no-op or unused stream on clients that do not speak mouse. No `Event::Mouse` arrives. Keys keep working. We do not detect SSH or disable capture automatically.

Local emulators that still allow Shift-select for copy may need Shift; that is accepted for this slice.

## Testing

Same style as `znicz-tui/tests/keys.rs`: `App` + `on_mouse`, assert cursor, `focus`, `queue_open`, `modal`.

Must cover:

- Click a library / queue row selects and focuses; does not play
- Click below the last visible row does nothing
- Wheel steps the focused list; with a list overlay up, wheel steps the overlay
- Click outside help / inspector / a list overlay closes it
- Click outside a search prompt cancels it
- Queue toggle column opens; overlay: click library closes; sheet: toggle column closes
- Click on transport / footer does nothing

Do not integration-test `EnableMouseCapture` against a real terminal.

## Docs

- [wiki/Architecture/TUI.md](../../../wiki/Architecture/TUI.md): short mouse paragraph (capture, select-only, wheel, click-outside, queue border).
- [wiki/Issues.md](../../../wiki/Issues.md) and [wiki/Plans/Roadmap.md](../../../wiki/Plans/Roadmap.md): #8 closed when this ships.
- `keys.rs` / `?` help stay keyboard-only. Do not add mouse lines to the generated help.

## Error handling

Hit map missing or zero-sized (first frame, tiny window): `on_mouse` is a no-op. Out-of-range coordinates are a no-op. None of this is a toast or a player error.
