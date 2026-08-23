# TUI layered layout

**Date:** 2026-08-23
**Status:** Approved
**Crate:** `znicz-tui`

## Problem

The player is a pager: a tab bar swaps one full-width list (Queue, Library, Devices), a six-row Now Playing box sits above it, and toasts steal the hint line. That fights how the player is actually used. The library is where you browse and add. The queue is a setlist you peek at from anywhere, not a second home screen. Devices are a rare switch, not a peer of the library.

## Goals

1. Open on the library. No tab bar. Queue, Library, and Devices are no longer three equal homes.
2. The queue is an overlay drawer, reachable from any screen, that does not shrink the library layout.
3. Long titles (and titles hidden by the drawer) pan horizontally. Track number and duration stay pinned in the visible strip.
4. Now Playing and the status line merge into a two-line transport at the bottom. The signal-path line and bit-perfect / resampled badge stay visible on a tall enough terminal.
5. Devices and help are centered modals. Toasts float; they do not replace hints.
6. Existing list behaviour (browse, search, add, queue edit, vim keys, seek, volume) stays, routed by focus.

## Non-goals

- Command palette
- Settings screen
- Signal inspector (`i`, device sample format such as `f32`)
- Three-column artist / album / tracks browser
- Push layout (library shrinking to sit beside the queue)
- Mouse, click-to-focus, or drag-to-reorder
- Changing the playback engine, library crate, or MCP

## Daily motion

You land in the library, browse or search, and add. `]` opens the queue over the right of the screen. You check the setlist, or keep library focus and pan with `<` `>` if a title no longer fits. Close the drawer and you are still on the same album.

## Frame

```
┌ Library / Dummy ─────────────────────────────────────────┐
│  1  Mysterons — Portishead                          5:02 │
│  2  Sour Times — Portishead                         4:11 │
│  3  Wandering Star — Portishead  ┌ Queue ────────────────┤
│  4  It Could Be Sweet — Portis.  │  1 ▶ Sour Times  4:11 │
│  5  Numb — Portishead            │  2   Strangers   3:58 │
└ 12 tracks · < > pan ─────────────┴ 3 tracks · ] close ───┘
▶ Sour Times  Portishead — Dummy  ━━━━━── 1:02/4:11  70%
  MP3 44.1 kHz 192 kbps stereo → 44.1 kHz stereo  ● bit perfect
  Space pause  a add  ] queue  < > pan  , devices  ? help
```

Vertical split, top to bottom:

| Region | Height | Role |
| --- | --- | --- |
| Library (full width of this region) | `Min(3)` | The stage. Always drawn. |
| Transport line 1 | 1 | Play state, title, artist — album when it fits, seek, time, volume, repeat, shuffle |
| Transport line 2 | 1, or 0 when height < 20 | Signal path, same content as today |
| Hints | 1 | Key hints for the current focus. Never replaced by a toast. |

No tab bar. The old Now Playing block and the old status line are gone.

### Height fallback

| Terminal height | What drops |
| --- | --- |
| 20 or more | Everything |
| 12–19 | Transport line 2 (signal path), same threshold as today |
| Under 12 | Still library + one transport line + hints. Lists keep `Min(3)`. |

## Queue drawer

The drawer is an overlay. The library widget is still laid out in the full list region. The queue is painted afterwards on top of the right-hand side.

### Size

- Side overlay width: **36 columns**.
- If `list_width <= 36 + 40` (library strip would be under 40 columns), the drawer is a **full-width sheet** instead: it covers the whole list region until closed.
- The 40-column rule uses the list region width, not the whole terminal (transport and hints do not count).

### Open, close, focus

- Start: drawer **closed**, focus **library**.
- `]` toggles the drawer. Opening moves focus to the queue. Closing moves focus to the library.
- `Tab` with the drawer closed opens it and focuses the queue.
- `Tab` with the drawer open switches focus library ↔ queue. The drawer stays open.
- `BackTab` is the reverse of `Tab`. With the drawer closed it is a no-op (there is nothing to move back to).
- When the drawer is a **full-width sheet**, the library is not visible, so focus stays on the queue. `Tab` / `BackTab` do not move to the library. `]` or Esc (queue focused) closes the sheet.
- `1` / `2` / `3` are unbound.

Queue keys (`Enter`, `d` / `Del`, `C`, `o`) apply only while the queue is focused. Library keys (`/`, `Enter`, `a`, `A`, `Esc` as back, `R`) apply only while the library is focused. `j` / `k` / `g` / `G` / page keys move the focused list.

Adding from the library while the drawer is open updates the queue in place and shows a toast.

## Horizontal pan

Rows are composed for the **visible strip**, not for the width sitting under the overlay:

- Drawer closed, or full-width sheet: strip = inner width of the list region.
- Side overlay: strip = inner width minus 36.

Pinned in that strip:

- Left: track number (or the album-list equivalent).
- Right: duration (or album track-count / time).

The title (and artist) occupy the middle. If that text is wider than the slot, a horizontal offset (`usize` characters, one value for the whole library list) selects which slice is shown. Every row shares the offset so the column does not jitter. Clamped to `[0, max(0, longest_middle - slot_width)]`. Closing the drawer or widening the terminal clamps the offset down.

- `<` decreases the offset (toward the start of the title).
- `>` increases it (toward the end).
- These keys always pan the library when the search prompt is not open, including while the queue is focused.
- `h` / `l` and the arrows still seek. They do not pan.

Truncation with `…` still applies to the visible slice, counting characters, as `format::truncate` does today.

When the selected row's middle text is longer than the slot, moving the library cursor does not auto-pan. The user pans. Offset does not reset when opening or closing the drawer, except for the clamp above.

## Modals

`Modal` is `None`, `Help`, or `Devices`. Only one at a time. Opening help while devices is open replaces it, and the other way around. The drawer may stay open underneath.

- `?` opens help. **Any following key** closes help and is not executed (same as today).
- `,` toggles the devices overlay. `Enter` selects a device. `R` rescans. `j` / `k` move the device list while this modal is open.
- Help and devices are centered overlays using the same Clear-on-dim pattern as today's help.

Transport keys still work under the devices modal: Space, `s`, `n` / `N`, seek, volume, mute, repeat, shuffle, `q`.

## Esc

First match wins:

1. Search prompt open → cancel search (drawer and modal unchanged).
2. Devices modal open → close it.
3. Queue focused → close the drawer, focus library.
4. Library focused → existing back navigation (album or search → album list). Does **not** close the drawer.
5. Album list, nothing open → no-op.

`q` still quits. Esc never quits.

Help is not on this list because any key already dismisses it.

## Transport

Two lines at the bottom of the frame, above hints.

**Line 1:** play symbol and word, truncated title, `artist — album` when there is room after the title, seek bar, elapsed / total, volume bar and percent (or `muted`), repeat label, `shuffle` styled on/off as today.

**Line 2:** file format → device stream and the badge, identical rules to the current signal line (no device sample format; badge only while not stopped). Dropped when height < 20.

The output device name is not on this bar. It lives in the devices overlay. The signal line still shows the stream the device opened.

Title on line 1 truncates first when the line is tight; repeat and shuffle may drop before the seek times if the terminal is extremely narrow. The seek times and play symbol never drop.

## Toasts

`Toasts` already keeps up to eight messages. Draw up to **three** newest, stacked in the bottom-right of the list region (above the transport), each line truncated to `min(40, 40% of terminal width)` characters. Info / warn / error styles stay as they are. Lifetimes stay 4s / 8s for errors.

The hint line always shows hints. It never shows a toast.

## State

Replace `Pane` as three full-screen homes with:

```text
focus: Library | Queue          Queue only while the drawer is open
queue_open: bool
modal: None | Help | Devices
library.h_offset: usize
```

`show_help: bool` becomes `modal == Help`. Device list state stays on `App`. Queue cursor stays on `App`. Library browse state stays in `LibraryPane`.

Start-up: `focus = Library`, `queue_open = false`, `modal = None`, `h_offset = 0`.

## Draw order

Each frame, after the vertical split:

1. Library into the full list region.
2. Queue drawer (right overlay or full-width sheet) if `queue_open`.
3. Transport, then hints.
4. Devices or help modal, if any.
5. Toasts.

The immediate-mode loop is unchanged: drain player events, draw, wait for a key or tick.

## Key dispatch

1. Help open → close help, stop.
2. Search prompt open → search keys only (Esc, Enter, Backspace, characters). Transport keys do not run, same as today.
3. Ctrl-c quit, Ctrl-d / Ctrl-u page the focused list.
4. Global: quit, `?`, `,`, `]`, Tab / BackTab, `<` `>`, Space / stop / next / previous / seek / volume / mute / repeat / shuffle, then j/k/g/G/page on the focused list (or the device list when that modal is open).
5. Focus-specific: library keys or queue keys. Devices keys other than movement are handled while the devices modal is open (`Enter`, `R`).

`keys.rs` remains the single table for help text and the hint line. Update `GLOBAL`, drop pane-cycle bindings, add drawer / pan / devices-modal bindings. Hints follow **focus** (library, queue, devices), not a tab index.

## Errors

No change to `send_blocking` or to turning player events into toasts. Failures still appear on screen; they appear in the toast stack instead of on the hint line. ALSA stderr redirection is unchanged.

## Tests

Keep the rule: drawing must not panic at small sizes, and keys must be testable without a terminal.

- `tests/render.rs` — draw library home (drawer closed), drawer overlay, full-width sheet (narrow width), devices modal, help, toasts, at the same size set as today (down to 10×3). Assert the tab bar is absent, a library heading is present on a fresh `App`, and the hint line is not the toast text when both exist.
- `tests/keys.rs` — replace pane-cycle cases. Cover: default focus library; `]` open/close; Tab open; Tab swap focus with drawer staying open; BackTab no-op when closed; `,` modal; Esc stack (search, devices, drawer, library back); `<` `>` change offset and clamp; `h` still seeks; `1` `2` `3` do not change focus.
- `tests/library_browse.rs` — leave browse/search/add behaviour; only drop assumptions that the app starts on the queue.
- Unit tests for strip width (overlay vs sheet) and for offset clamping.
- `examples/preview.rs` — library home, drawer open, devices modal, help. No tab-bar frames.

## Wiki

Implementation updates `wiki/Architecture/TUI.md` (layout diagram, panes, messages) and the Phase 2.5 bullets in `wiki/Plans/Roadmap.md` so they describe library-home and the drawer instead of three tabs. That is part of the same change, not a later docs pass.

## Files likely to move

| File | Change |
| --- | --- |
| `znicz-tui/src/app.rs` | Focus, drawer, modal, dispatch, default library |
| `znicz-tui/src/views/mod.rs` | Frame: no tabs, library stage, overlays, two-line transport |
| `znicz-tui/src/views/library.rs` | Visible-strip rows, `h_offset` |
| `znicz-tui/src/library_pane.rs` | `h_offset`, clamp |
| `znicz-tui/src/views/now_playing.rs` | Split into transport lines; header block removed |
| `znicz-tui/src/views/status.rs` | Hints only; toasts leave this file |
| `znicz-tui/src/views/queue.rs` | Renders into the drawer rect |
| `znicz-tui/src/views/devices.rs` | Centered modal |
| `znicz-tui/src/views/help.rs` | Keymap copy only |
| `znicz-tui/src/keys.rs` | New bindings and hints |
| `znicz-tui/src/toast.rs` | Drawing helper for the stack (logic already exists) |
| tests and `examples/preview.rs` | As above |
