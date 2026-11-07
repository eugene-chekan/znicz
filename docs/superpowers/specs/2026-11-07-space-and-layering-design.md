# Space and layering

**Date:** 2026-11-07
**Status:** Approved
**Crate:** `znicz-tui`

Part 2 of the five-part TUI rework. Part 1 (keymap and discoverability,
0.6.0) is [here](2026-11-07-keymap-discoverability-design.md); the plan for all
five parts is in that spec's "Context" section and on the
[roadmap](../../../wiki/Plans/Roadmap.md).

## Problem

- Devices, Playlists, and Radio size themselves at 70 % of the whole window, so
  they cover the transport and the footer. Help and the inspector also centre on
  the whole window.
- Toasts float as up to three boxes in the bottom-right of the list, drawn last,
  on top of everything including modals.
- On short terminals the cover shrinks to 4–7 rows and squeezes the list: at
  48×14 the cover takes 8 rows and the list gets 3.
- A list without focus shows no selection at all. In column view, once focus
  moves to Albums nothing says which artist is selected; with the queue drawer
  open the library loses its selection.
- Titles only say `Library` in columns and tree; paging shows one level.
- The library does not mark the playing track, and there is no way to jump to
  it.
- Carried over from part 1: closing help always returns to the library, even
  when it replaced Devices, Playlists, or Radio; the help border still reads
  `/ filter · Esc close` while a filter is being typed.

## Goals

1. Modals open inside the list region. The transport and the footer are never
   covered.
2. Messages move into a transport row, with a history list (`M`) and an unread
   marker for warnings and errors.
3. The cover appears only on tall windows; short windows keep the list.
4. Focus is shown by colour, and every list keeps its selection visible.
5. Titles carry a breadcrumb.
6. The library marks the playing track, its album, and its artist; `J` jumps
   there.
7. Help returns to the modal it replaced; its border matches the filter state.

## Non-goals

- Stage mode, removing the cover from Browse (part 4).
- Scan from the TUI (part 3).
- Row reordering, volume steps (part 5).
- Renaming `Toasts` / `toast.rs`.
- Wall-clock timestamps in the message history.

## Layout

### Modals

Every modal (Help, Devices, Inspector, Playlists, Radio, Messages) is centred in
the **list region** (the area above the transport):

| Modal | Size inside the list region |
| --- | --- |
| Devices, Playlists, Radio, Messages | 70 % wide (at least 40 columns) × 70 % tall (at least 8 rows) |
| Help | up to 64 wide × the full list-region height |
| Inspector | its content size |

All sizes are clamped to the list region. When the list region is shorter than
**10 rows**, a modal takes the whole list region. One function,
`layout::modal_rect(list, want_w, want_h)`, does this for every modal; the
per-modal `centered_modal` / `centered` / `popup_rect` helpers go away.

Help's popup no longer needs its one-row inset: the list region already excludes
the footer.

### Cover and transport height

The cover is shown, at **8 rows**, only when `show_cover` is on and the window
is **at least 24 rows** tall. Otherwise the text transport is used. The 4–7 row
shrinking cover is removed.

| Window height | Cover | Transport rows |
| --- | --- | --- |
| ≥ 24, `show_cover` on | 8-row cover | 8: cover beside title, artist — album, seek, signal, message |
| ≥ 20 (cover off or < 24) | none | 3: title + seek, signal, message |
| < 20 | none | 2: title + seek, message |

The 3- and 2-row text transports are today's 2- and 1-row ones plus the message
row. The list region keeps `Min(3)`.

## Messages

### Message row

The transport's message row is always reserved. It shows, in order of
preference:

1. **The newest live message**: level mark and colour as today (`●` info blue,
   `●` success green, `▲` warn yellow, `x` error red), then the text, truncated
   with `…`. When other live messages exist, a dim `+N` is right-aligned.
2. **The unread marker**, when no message is live and unread alerts exist:
   right-aligned `▲ 2 unread · M` (or `x 2 unread · M`), coloured by the worst
   unread level (error beats warning).
3. **Blank.**

Lifetimes are unchanged: 4 s, errors 8 s.

The floating toast boxes are removed: `render_toasts`, `toast_areas`,
`toast_boxed`, `toast_width_for`, and the `TOAST_*` constants are deleted.

### History and unread

`Toasts` keeps the live list as today and adds a **history of the last 50
messages**, newest first, kept after they expire. Each history entry has the
text, level, the `Instant` it arrived, and an `unread` flag.

`unread` is set only for **warnings and errors**. Info and success messages
confirm something the user just did and are never unread.

### Messages list (`M`)

`M` opens a modal listing the history, newest first:

```
┌ Messages ────────────────────────────────────X┐
│ x  12s  device refused 96 kHz, resampling      │
│ ●  14s  added 4 tracks                         │
│ ●   3m  queue cleared                          │
└──────────────────────────── C clear · Esc close┘
```

- Each row: level mark (coloured), age right-aligned in 3 columns (`12s`, `3m`,
  `1h`; seconds under a minute, minutes under an hour, hours after), the text.
- Text is truncated; `Alt-←` / `Alt-→` pan the highlighted row like other lists.
- Rows that were unread when the list opened are bold until it closes.
- Opening the list marks every entry read.
- Keys: movement keys move the highlight; `C` clears the history; `Esc` or `M`
  closes.
- Empty history: a single dim row, `No messages yet`.
- Mouse: a click selects a row, the wheel moves the highlight, X closes.

### Footer and help

- New `Context::Messages` with hints `C clear` · `Esc close`, plus `? more`.
- `Library` and `LibraryTree` hint lists gain `M messages` as their **last**
  hint, so it is dropped first on narrow terminals.
- While unread alerts exist, that hint reads `M messages (N)`.
- `GLOBAL` gains `M` — "messages". A new `MESSAGES` table holds `C` — "clear
  the history", `Esc / M` — "close", and help shows it as a section like the
  other modals.

## Library

### Focus styling

| Pane | Selected row | Border and title |
| --- | --- | --- |
| Focused | accent background (cyan, black bold text) | accent |
| Unfocused (other columns, library behind the drawer, anything behind a modal) | grey background (dark grey, normal text) | muted |

The queue drawer and the modal lists (devices, playlists, radio, messages) use
the same pair.

### Breadcrumbs

| Layout | Title |
| --- | --- |
| Columns | `Artists`; `Albums · <artist>`; `Tracks · <album>` (each names its parent; just `Albums` / `Tracks` when nothing is selected) |
| Tree | `Library › <artist> › <album>` for the cursor row (artist row: `Library › <artist>`; nothing: `Library`) |
| Paging | `Library › <artist>` on albums, `Library › <artist> › <album>` on tracks, `Library` on artists |
| Search | `Library › search "<query>"` |
| All tracks | `Library › all tracks` |

Separator ` › `. When the title is wider than the space for it, middle segments
are shortened first (each with `…`), then the last; `Library` is never
shortened.

### Playing marker

- The playing **track** row shows `▶` in place of its track number, and the row
  text uses `theme::playing()`. This applies in columns, tree, paging, search
  results, and the all-tracks list.
- The playing track's **album** and **artist** rows get ` ♪` after the name in
  `theme::playing()`.
- Which album and artist: when the current track's path changes, the TUI looks
  the path up in the library (`Library::get_by_path`). A hit gives the browse
  artist (album artist, else artist) and album. A miss falls back to the
  current track's own tags. Streams and an empty player have no marker.

### `J` — jump to the playing track

| Layout | Effect |
| --- | --- |
| Columns | Select the artist, album, and track; focus Tracks |
| Tree | Expand the artist and album; cursor on the track |
| Paging | Drill to the album's tracks with the track selected |

If search results are showing, `J` leaves search first. Nothing playing or a
stream: info message `nothing playing in the library`. A file not in the
library: info message `playing track is not in the library`. `J` only acts with
library focus and no modal open.

`LIBRARY` gains `J` — "jump to the playing track".

## Help carry-overs

- `HelpState` gains `return_to: Modal`. `open_help` records the modal open at
  the time (`None`, `Devices`, `Inspector`, `Playlists`, `Radio`, `Messages`).
  Closing help by `Esc`, `?`, `q`, the X button, or a footer hint restores it.
- The help popup's bottom-right border text reads ` Enter keep · Esc clear `
  while `help.editing`, and ` / filter · Esc close ` otherwise.

## Architecture

### `layout.rs`

- `modal_rect(list: Rect, want_w: u16, want_h: u16) -> Rect` and
  `MODAL_FULL_BELOW: u16 = 10`.
- `COVER_MIN_WINDOW: u16 = 24`. `cover_enabled(window, show_cover)` becomes
  `show_cover && window >= COVER_MIN_WINDOW`; `transport_height` returns 8 with
  the cover, else 3 (≥ 20) or 2 (< 20).
- Delete `COVER_TARGET`, `COVER_MIN`, and all toast-box items.

### `toast.rs`

`Toasts` gains `history: VecDeque<Entry>` (cap 50) where
`Entry { text, level, at: Instant, unread: bool }`, and:

- `live_extra() -> usize` — live messages beyond the newest.
- `history() -> impl Iterator<Item = &Entry>` — newest first.
- `unread_alerts() -> Option<(usize, Level)>` — count and worst level.
- `mark_read()`, `clear_history()`.

`visible()` is removed with the toast boxes.

### `views/now_playing.rs`

`message_line(toasts: &Toasts, width: usize) -> Line<'static>` builds the row
(pure). The stacked chrome draws it on row 4 (the cover only shows at ≥ 24 rows,
where the signal row is always present);
the text transport draws it on its last row.

### `views/messages.rs` (new)

`render_modal(frame, list, app)`. `App` gains `message_cursor: Cursor`,
`message_h_offset: usize`, `messages_list_state: ListState`, and
`messages_bold: usize` (how many of the newest rows were unread when the list
opened; reset on close). `Modal::Messages` is added; `blocks_list_focus` covers
it. Keys are handled in an `on_messages_key` next to the playlist and radio
handlers.

### `keys.rs`

`Context::Messages`, `MESSAGES` table, `M` in `GLOBAL`, `J` in `LIBRARY`,
`M messages` hint at the end of the library lists, `context_title(Messages)`.

### Footer labels

`PlacedHint` owns its label (`label: String`). `views/status.rs` first maps
hints to display labels (`M messages` → `M messages (N)` when
`unread_alerts()` is some), then lays out and registers clicks on those labels.

### Focus, breadcrumb, markers

- `theme::selected()` becomes the accent background; new
  `theme::selected_idle()` is the grey one. `views::highlight(focused) -> Style`
  replaces the `if focused { selected() } else { no_style() }` pattern in every
  list view.
- `views::breadcrumb(parts: &[&str], width: usize) -> String` is pure.
- `App` caches `playing: Option<Track>` with the path it was resolved for;
  refreshed in `render` when `state.current_track` path differs.
  `LibraryPane::locate(path) -> Option<Track>` wraps `Library::get_by_path`.
  Row builders receive the playing track and draw the markers.
- `LibraryPane::reveal(track: &Track, strip_inner) -> bool` implements `J`.

## Testing

Unit:

- `modal_rect`: centred, clamped, full list region below 10 rows.
- Transport height at 30, 22, and 15 rows, cover on and off; 48×14 has no cover.
- History cap 50; only warnings and errors are unread; `mark_read`;
  `unread_alerts` worst level; `clear_history`.
- `message_line`: live message, live with `+N`, unread marker, blank.
- `breadcrumb`: fits, middle shortened first, `Library` kept.
- `reveal` in columns, tree, paging, and from search; miss returns false.
- Message age formatting (`5s`, `3m`, `2h`).

Render:

- For every modal and every test size, the transport and footer rows are the
  same as with no modal open.
- The message row shows the newest message and `+N`; the unread marker once no
  message is live.
- The Messages modal lists entries, shows `No messages yet` when empty.
- Focused selection uses the accent background, unfocused the grey one (cell
  style check).
- Breadcrumbs in columns, tree, paging, search.
- `▶` on the playing track row; `♪` on its album and artist rows.
- `M messages (N)` in the footer with unread alerts.

Keys:

- `M` opens and closes Messages; opening clears unread; `C` clears history.
- `J` in each layout and from search; the two info messages.
- Help opened from Devices returns to Devices on `Esc`.

Mouse:

- Click selects a Messages row; wheel moves it; X closes.

## Delivery

- Bump `[workspace.package] version` to **0.7.0**.
- Update `wiki/Architecture/TUI.md` (layout diagram, responsive table, messages,
  modals, focus styling, breadcrumbs, playing marker, `J`), the README key table
  (`M`, `J`), and mark part 2 done on `wiki/Plans/Roadmap.md`.
- Check frames with `cargo run -p znicz-tui --example preview`.
