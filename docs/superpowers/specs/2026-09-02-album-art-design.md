# Album art in the TUI

**Date:** 2026-09-02
**Status:** Approved
**Crates:** `znicz-core` (picture bytes), `znicz-tui` (worker + layout + draw), `znicz` (`[tui]` config)
**Version:** 0.3.10 → **0.4.0** (Phase 5)
**Plan sketch:** [wiki/Plans/Phase-5-Album-Art.md](../../../wiki/Plans/Phase-5-Album-Art.md) (this spec wins where they differ: no cover bytes on `PlayerState`)

## Problem

Now playing is text only. Files already carry a front cover. The TUI should show it in the transport, sharp in Kitty / WezTerm / Ghostty, and as half-blocks everywhere else. Radio has no embedded picture yet.

## Goals

1. Show the current file’s **embedded front cover** in an **8-row** slot on the left of a taller transport. The library stays full width above. Hints stay one line under the transport.
2. When there is no picture (stream, missing tags, read/decode error), keep that slot and draw a **bundled Znicz logo**. The list does not jump.
3. Do **not** put image bytes on the JSON IPC tick. The TUI reads the file from `TrackInfo.path`, off the UI thread, same idea as `MetaCache`.
4. Detect graphics protocol at TUI start (`ratatui-image` picker). Never spawn `kitty icat`.

## This slice does not include

- `folder.jpg` / `cover.jpg` beside the file
- MusicBrainz / Cover Art Archive (Phase 6)
- MCP `cover://current`
- Disk cache of decoded bitmaps
- Radio/stream art (ICY, playlist image, URL fetch, or a lookup). `CoverSource` stays extendable; today it is `Embedded` or `Logo` only
- A key to hide the cover
- Animated covers or visualizers

## Layout

Frame stays **list / transport / hints**. Queue drawer still overlays the list only.

With cover on, transport is **square cover | stacked chrome**. Same fields as today (title, artist/album, seek, volume, toggles, signal path), wrapped in the right column. Cover keeps aspect (`Resize::Fit`). Typical cells make an 8-row square about 16 columns; real width comes from the picker font size. If font size is unknown (tests), use `width = 2 * height` columns.

```
┌ Library ─────────────────────────────────────────────────┐
│  tracks                                                  │
└──────────────────────────────────────────────────────────┘
┌ cover ┐  ▶ So What
│ 8 rows│  Miles Davis — Kind of Blue
│       │  ━━━●━━━━  1:02 / 9:22  70%
│       │  FLAC 24/96 → DAC  ● bit perfect
└───────┘
Space pause  a add  ] queue  i inspect  …
```

`show_cover = false`: today’s 1- or 2-line full-width transport. Takes effect on TUI start. No new keys.

### Height

Always reserve **3** list rows and **1** hint row. `available = height - 4`. Compact (`height < 20`) still **drops the signal line**, as today, whether or not a cover is showing.

| `show_cover` | `available` | Transport height |
| --- | --- | --- |
| false | (any) | 2 if `height >= 20`, else 1 |
| true | ≥ 8 | 8 (cover 8 rows) |
| true | 4–7 | `available` (shrunk square) |
| true | < 4 | cover off; same 1- or 2-line rule as `show_cover = false` |

## Data flow

```
PlayerState.current_track.path
        │
        ▼
CoverCache worker (TUI)
        │  znicz_core::read_cover(path)
        ▼
CoverSource::Embedded or CoverSource::Logo
        │  decode + cap longest side at 512 px on the worker
        ▼
ratatui-image StatefulImage in the cover rect
```

- **IPC and `PlayerState` do not change.** No `CoverArt` field on `TrackInfo`.
- **Library scan does not read pictures.** `read_metadata` stays text + audio properties.
- **Player thread does not decode images.**
- `CoverCache` is in-memory, keyed by path, cap **16** entries (skip-back without a disk cache). Current path only is requested each frame, like `MetaCache`.
- Streams and missing `path` never call `read_cover`. Stopped with a file still on `current_track` still shows that file’s cover.

`CoverSource` is TUI-only:

```text
Embedded | Logo
```

Later: `Stream { url }` or a lookup, without moving art onto IPC or changing chrome.

## Core helper

`znicz_core::read_cover(path) -> Option<CoverArt>` where `CoverArt { mime: String, bytes: Vec<u8> }`.

- Open with lofty. Prefer `PictureType::CoverFront`. Else the first picture with non-empty data.
- Missing file, no tag, no picture, or lofty error: `None`. Log at `debug`. Never `Err` to the TUI.
- Mime from the picture when present, else sniff PNG/JPEG magic, else `"application/octet-stream"`.

Tiny tests write a temp FLAC/MP3 with a 1×1 PNG (lofty), plus a file with two pictures (front wins), plus a file with none.

## TUI draw

1. Enter the alternate screen, **then** `Picker::from_query_stdio()`. On error (including `TestBackend`), use `Picker::halfblocks()`.
2. Decode JPEG/PNG with the `image` crate on the worker. Cap longest side at **512 px** there so `StatefulImage` fit in the slot is cheap. Do not decode or scale JPEG on the render thread.
3. Rebuild the resize protocol when the path changes or the cover rect size changes.
4. `cover_protocol = "off"`: keep the slot, draw the logo, skip picker bitmap protocols.
5. Forced `kitty` / `sixel` / `halfblocks`: construct the picker for that protocol. `auto`: query, then half-blocks if the query fails.
6. Logo: `include_bytes!` PNG in `znicz-tui`. Decode once at start. Replace the file later; layout stays.

Failures (no path, `None` from `read_cover`, decode error): logo. Log `debug`. No toast.

## Config

`~/.config/znicz/config.toml`, read by the `znicz` binary and passed into `App`:

```toml
[tui]
show_cover = true
cover_protocol = "auto"   # auto | kitty | sixel | halfblocks | off
```

Defaults: `show_cover = true`, `cover_protocol = "auto"`. Unknown `cover_protocol` values behave as `auto`. Log the chosen renderer once at `info`.

## Tests

- Core: `read_cover` fixtures as above.
- Worker: file → bytes; no path / `None` / bad bytes → `Logo`.
- Layout (`znicz-tui/tests/render.rs`): existing sizes must not panic. **Update** tests that assume two transport rows (they will break at 80×24 with cover on). With cover on, 80×24 uses transport height 8. With `show_cover = false`, 80×24 stays two transport rows. 40×12 and smaller still draw. Do not pixel-diff Kitty/Sixel.

Manual (not CI): WezTerm with a real embedded cover; plain xterm half-blocks; a stream shows the logo; resize keeps a square.

## Wiki (same change as the feature)

- [Phase-5-Album-Art.md](../../../wiki/Plans/Phase-5-Album-Art.md): TUI reads the file; logo placeholder; this spec linked; milestones 5.1–5.3
- [Architecture/TUI.md](../../../wiki/Architecture/TUI.md): chrome diagram and height table
- [Roadmap.md](../../../wiki/Plans/Roadmap.md): Phase 5 in progress, then done
- README: one feature line
- [keys.rs](../../../znicz-tui/src/keys.rs): unchanged

## Dependencies

| Crate | Role |
| --- | --- |
| `lofty` (already) | Picture blocks |
| `image` | Decode / resize on the worker |
| `ratatui-image` | Picker + `StatefulImage` |

No `viuer`. No `icat`.
