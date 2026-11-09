# Stage mode

**Date:** 2026-11-09
**Status:** Approved
**Crate:** `znicz-tui`
**Issues:** [#46](https://github.com/eugene-chekan/znicz/issues/46) (full-screen now playing), [#34](https://github.com/eugene-chekan/znicz/issues/34) (auto-pan now-playing text)

Part 4 of the five-part TUI rework (see the
[roadmap](../../../wiki/Plans/Roadmap.md)). Parts 1–3 shipped in 0.6.0–0.8.0.

## Problem

- The only place for cover art is a small slot in the transport, which part 2
  limited to windows of 24+ rows. It competes with the library for height.
- Signal details live in a modal inspector (`i`), separate from what is playing.
- Long titles, ICY stream titles, and `Artist — Album` are cut with `…` on the
  transport, and there is no way to read them without leaving the player.

## Goals

1. A **Stage** view: a full-screen now-playing view with a large cover, track
   details, seek, volume and modes, and the signal path.
2. The inspector folds into Stage; `v` and `i` both toggle Stage.
3. Long now-playing titles scroll automatically (marquee).
4. Browse drops the cover: the library gets the height.

## Non-goals

- An "Up next" list on Stage (the queue drawer covers it).
- Remembering the view across restarts, or opening on Stage at startup. The TUI
  always starts on Browse.
- Library navigation from Stage.
- New cover sources.
- Marquee on library or queue rows (`Alt-←` / `Alt-→` stays manual).

## Views and keys

`App` gains `pub view: View` with `enum View { Browse, Stage }`, starting at
`Browse`. Modals (`Modal`) still open on top of either view.

| Key | Browse | Stage |
| --- | --- | --- |
| `v` | open Stage | back to Browse |
| `i` | open Stage | back to Browse |
| `Esc` | as today | closes an open modal or the queue drawer first; otherwise back to Browse |
| `]` | queue drawer | queue drawer, drawn over the right of Stage |
| Playback keys (Space, `s`, `n` / `p`, arrows and Shift-arrows, `+` / `-`, `m`, `e`, `z`), `?`, `,`, `P`, `R`, `M`, `q` | as today | work |
| Library keys (`/`, `a`, `A`, `J`, `F`, `U`, `h` / `l`, list movement with library focus) | as today | ignored |

`J` from Stage does nothing; `v` first, then `J`.

Removed: `Modal::Inspector`, `views/inspector.rs` (its line builders move to
Stage), `keys::INSPECTOR`, `Context::Inspector`.

Keymap:

- `GLOBAL`: `v` — "now playing (stage)"; `i` relabelled "now playing (stage)".
- New `STAGE` table: `v / i / Esc` — "back to browse"; help shows it as a
  "Stage" section.
- New `Context::Stage` (Stage, no modal, drawer closed or library focus) with
  hints `Space pause` · `n next` · `v browse` · `] queue`, plus `? more`.

## Stage layout

Stage uses the list region **and** the transport region; only the message row
(last row above the footer) and the footer remain below it.

### Wide (Stage area ≥ 70 columns and ≥ 14 rows)

```
┌──────────────────────────┐
│                          │   In My Time of Dying
│                          │   Led Zeppelin
│          COVER           │   Physical Graffiti · 1975 · track 3
│   square, full height    │
│                          │   ▶ ━━━━━━━━━━●─────────────── 3:32 / 11:04
│                          │   ▆▆▆▁ 70%   repeat all   shuffle
│                          │
│                          │   Signal
│                          │     file    FLAC 96 kHz 24-bit 2882 kbps stereo
│                          │     device  Topping E30 II · 96 kHz stereo f32
│                          │     path    ● bit perfect
│                          │             the device took the file's own rate
└──────────────────────────┘
```

- Cover: square, the full Stage height, width from the cell aspect
  (`layout::cover_width` logic), capped at half the Stage width; one blank column
  between cover and details.
- Details, top to bottom: title (bold, marquee), artist, `album · year · track N`
  (each part only when known), blank, seek row (status mark, bar, times), volume /
  repeat / shuffle row, blank, `Signal` heading, `file`, `device`, `path` lines
  and the path explanation (the inspector's content).

### Stacked (narrower than 70 columns or shorter than 14 rows)

- Cover on top, centred, at most half the Stage height; details below.
- When rows run short, details drop from the bottom in this order: the path
  explanation, the signal panel, the volume row. Title, artist, and seek always
  stay.
- Under 8 rows: no cover.

### `show_cover = false`

No cover; details are centred in the Stage area (wide or stacked rules apply to
the details alone).

### Streams

- Title: the ICY title when present, else the station name (as the transport
  does today).
- Second line: station name. Third line: stream URL, dim.
- Seek row: `● live` and the elapsed time instead of a bar.
- Signal `file` line includes the live coded bitrate.

### Nothing playing

The logo as cover; details read `Nothing playing` and, dim,
`Enter a track in Browse (v) to start`.

## Marquee

- Applies to the Browse transport's title segment (title plus `— artist —
  album` when shown) and the Stage title. Text that fits never moves.
- Motion: hold 2 s at the start, move 1 column per 200 ms, hold 2 s at the end,
  jump back to the start, repeat.
- The offset is a pure function of elapsed time:
  `marquee_offset(text_len, width, elapsed) -> usize`.
- `Marquee { text, width, started: Instant }` keeps the start time and resets
  when the text or width changes (new track, new ICY title, resize).
- Config: `[tui] marquee = true` by default; `false` keeps `…` truncation.

## Browse without a cover

- The Browse transport is always the text transport: 3 rows (chrome, signal,
  message) on windows ≥ 20 rows, 2 rows (chrome, message) below.
- `show_cover` now controls the Stage cover only; `cover_protocol` applies to
  Stage. The config keys keep their names.
- `layout::cover_enabled`, `COVER_MIN_WINDOW`, and the stacked transport chrome
  are removed; the cover drawing moves to `views/stage.rs`.

## Architecture

- `app.rs`: `View`, `App.view`, `toggle_stage()` (`v`, `i`), Esc handling,
  library-key guard in Stage, `Context::Stage`.
- `views/stage.rs` (new): `render(frame, area, app, state)`; pure
  `detail_lines(state, marquee_offset, width)`, `signal_lines(state)`,
  `stream_lines(track)`; cover drawing moved from `now_playing.rs` (same
  `CoverCache`, picker, draw-key caching).
- `views/mod.rs`: Browse draws library + drawer in the list region and the text
  transport; Stage draws `stage::render` over list region + transport region
  minus the message row, then the message row; drawer and modals on top.
- `views/now_playing.rs`: text transport only (`chrome_line`, `signal_line`,
  `message_line`), marquee on the title segment.
- `marquee.rs` (new): `marquee_offset` and `Marquee`.
- `layout.rs`: `transport_height(window)`; `stage_layout(area, font_w, font_h,
  show_cover) -> StageLayout { cover: Option<Rect>, details: Rect }`.
- `keys.rs`: `Context::Stage`, `STAGE`, `v` and `i` in `GLOBAL`; remove
  `INSPECTOR` and `Context::Inspector`.
- `tui_config.rs` and the `znicz` config: `marquee: bool` (default `true`).

## Testing

Unit:

- `marquee_offset`: fits → 0; within the start hold → 0; then +1 per 200 ms; end
  hold at the max offset; wraps to 0 after the end hold.
- `Marquee` resets on text change and width change.
- `stage_layout` at 120×40 (wide, cover), 60×30 (stacked, cover), 100×12
  (stacked, cover ≤ half), 50×7 (no cover); `show_cover = false` → no cover.
- `detail_lines` for a file, a stream (live marker, URL), and nothing playing.
- `signal_lines` for bit perfect, resampled, and no stream open (moved from the
  inspector tests).

Render:

- Stage draws at every size in `SIZES`.
- Wide Stage shows title, artist, seek, and the `Signal` heading; stacked keeps
  title and seek.
- Stream Stage shows `● live` and the URL.
- Browse at 80×24 has no cover and `list_height` 20.
- Stage footer shows `v browse`.
- A long title with a given marquee offset shows the shifted slice.

Keys:

- `v` and `i` toggle; Esc closes the drawer first, then leaves Stage; library
  keys do nothing in Stage; Space and `n` work in Stage; `]` opens the drawer in
  Stage.

## Delivery

- Bump `[workspace.package] version` to **0.9.0**.
- README: key table (`v`, `i`), `show_cover` meaning, marquee and its config.
- Wiki `Architecture/TUI.md`: views, Stage layout, marquee, transport rows,
  modules (remove `inspector.rs`, add `stage.rs`, `marquee.rs`); roadmap part 4
  done.
- PR closes #46 and #34.
