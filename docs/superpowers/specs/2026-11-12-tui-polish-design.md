# TUI polish: Stage repaint, Tab, Browse labels, volume icon

**Date:** 2026-11-12
**Status:** Approved (decisions recorded in issues #70, #71, #72, #74)
**Crates:** `znicz-tui`, `znicz-library`, `znicz`

One release (0.13.0, one PR) for four TUI issues.

## #70 Stage: a closed modal stays painted over the cover

Kitty and Sixel images are drawn outside ratatui's cell diff, so cells a modal
covered are not rewritten when it closes. Today the run loop clears the
terminal only when *every* overlay (modal or queue drawer) is gone.

**Behaviour:** on Stage, force a full repaint (`terminal.clear()` and
`cover_draw_key = None`) when a modal closes, or when the drawer closes,
independently of the other. `needs_full_redraw(view, before, after)` takes two
`Overlays { modal: bool, drawer: bool }` values and returns true on Stage when
`before.modal && !after.modal` or `before.drawer && !after.drawer`.

## #71 Tab never opens the queue drawer

`]` is the only key that opens or closes the drawer. Tab and Shift-Tab only
move:

- Browse, column layout: next / previous column (unchanged).
- Drawer open, overlay layout, Browse: library ↔ queue (unchanged).
- Drawer closed, anywhere else: nothing.
- Sheet layout, and Stage with the drawer open: nothing (unchanged).

Help text: `Tab` reads "next column, or library ↔ queue".

## #72 Browse labels

A row's label depends on where it sits:

- **Grouped** rows sit under an artist: tree album and track rows, the album and
  track columns, and the narrow paging list's album and track levels.
- **Flat** rows have no artist context: search hits, the all-tracks list and the
  single Browse track list.

Rules:

- Album, grouped: `Kind of Blue (1959)`. Album, flat: `Kind of Blue (1959) — Miles Davis` (unchanged).
- Track, grouped: `N. Title`, plus ` — Artist` only when the track artist
  differs (case-insensitive, trimmed) from the browse artist the row sits under
  (so compilations under Various Artists keep their artists).
- Track, flat: `Title — Artist` (unchanged), no number.
- **Numbering** is per album (the track list it belongs to): `N.`; when any
  track in the list has `disc_number > 1`, `D-N.` (disc defaults to 1). Labels
  are right-aligned to the widest label in the list and followed by one space
  (` 9. ` / `10. `). A track without a number gets blanks of the same width, so
  titles stay aligned. A list with no numbers at all gets no prefix.
- The right-hand column of track rows keeps its three cells: ` ▶ ` when playing,
  blanks otherwise, then the duration. The number moves to the left.
- The pan width (`middle_len`) uses the same label as the row.
- **Album order:** `albums_for_browse_artist` sorts by year ascending, then name
  (case-insensitive, then exact); albums without a year go last.

## #74 Volume icon

A marker before the volume bar on Stage and in the Browse transport.

- `[tui] icons = "unicode" | "emoji" | "nerd"` in `config.toml`, default
  `unicode`; unknown values fall back to `unicode`.
- Markers (`format::volume_icon(icons, volume, muted)`):

  | Set | 0–33 % | 34–66 % | 67–100 % | muted |
  |-----|--------|---------|----------|-------|
  | unicode | `◢` | `◢` | `◢` | `×` |
  | emoji | 🔈 | 🔉 | 🔊 | 🔇 |
  | nerd | `\u{f057f}` (󰕿) | `\u{f0580}` (󰖀) | `\u{f057e}` (󰕾) | `\u{f075f}` (󰝟) |

  Levels use the rounded percentage: ≤ 33 low, ≤ 66 medium, else high.
- Muted: marker, then the bar at the level it returns to (dimmed style), then
  `muted` instead of the percentage: `× ███▁▁▁ muted`. Not muted:
  `◢ ███▁▁▁  50%`.
- Widths use `unicode-width` display width, so the transport still drops the
  volume segment first (after toggles) at narrow widths with a two-cell emoji.

## Testing

- `needs_full_redraw`: modal closes with the drawer open → repaint; drawer
  closes with no modal → repaint; Browse never; opening never.
- Keys: Tab with the drawer closed on Stage, in the tree layout and in search
  leaves the drawer closed and focus on the library; Tab with the drawer open
  swaps focus; column cycling unchanged.
- Labels: grouped album; grouped track with same artist, different artist,
  compilation; numbering, padding, multi-disc, missing number, no numbers;
  flat album and track unchanged. `albums_for_browse_artist` year order with
  year-less last. Render: expanded tree shows `Kind of Blue (1959)` and
  `1. So What`; search still shows `— Miles Davis`. `middle_len` matches.
- Volume: marker per level and muted for each set; transport at narrow widths
  drops volume first with an emoji marker; Stage shows the marker.

## Delivery

Version **0.13.0**. README (config key), `wiki/Architecture/TUI.md` (Tab, Browse
labels, volume marker, `[tui] icons`), and any config reference in `wiki/`.
