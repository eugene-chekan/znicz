# Phase 5 — Album art in the TUI

**Status:** Done (0.4.0)  
**Depends on:** [Phase 2](../Domain/Formats-and-Metadata.md#library-phase-2) (tag and picture extraction)  
**Primary crates:** `znicz-core` (`read_cover`), `znicz-tui` (cache + draw), `znicz` (`[tui]` config)  
**Spec:** [Album art in the TUI](../../docs/superpowers/specs/2026-09-02-album-art-design.md)

## Goal

Show the album cover for the current track inside the Znicz TUI — sharp where the terminal allows it, with a safe fallback everywhere else.

Phase 1 shows the **file name** only. Phase 2 adds tags (title, artist, album) from files. Phase 5 adds the **cover image** to the now-playing transport.

This does not change audio quality. It is a display feature only.

## What shipped

- Embedded front cover (or first picture) via `znicz_core::read_cover`
- Eight-row cover slot; stacked chrome to the right; `show_cover = false` keeps the old one-/two-line transport
- Kitty / Sixel / half-blocks through `ratatui-image` (picker after alternate screen). `cover_protocol = "off"` keeps the slot and draws the logo
- Bundled Znicz logo when the path is missing, there is no picture, or decode fails (streams included). The picture is letterboxed onto an opaque canvas that fills the slot, so the previous cover cannot remain.
- In-memory cover cache (cap 16), decode off the UI thread, longest side capped at 512 px
- Config under `[tui]`: `show_cover`, `cover_protocol` (`auto` | `kitty` | `sixel` | `halfblocks` | `off`)

**No** cover bytes on IPC. **No** `icat`. **No** new keys. Radio/stream art is later.

## Architecture

```
PlayerState.current_track.path
        │
        ▼
CoverCache worker (TUI)
        │  znicz_core::read_cover(path)
        ▼
CoverSource::Embedded or CoverSource::Logo
        │
        ▼
ratatui-image StatefulImage in the cover rect
```

The TUI reads the file from `TrackInfo.path`. Library scan and the player thread do not decode images. Layout and height rules: [TUI architecture](../Architecture/TUI.md).

## Milestones

### 5.1 — Extract embedded art

- [x] `CoverArt` / `read_cover` in `znicz-core`
- [x] Prefer front cover, else first picture
- [x] Unit tests with tiny fixture files
- [x] No cover field on `PlayerState` / IPC (TUI reads the path)

### 5.2 — Terminal detection and config

- [x] `Picker` after alternate screen (`auto` → half-blocks on failure)
- [x] `[tui]` keys in `~/.config/znicz/config.toml`
- [x] Forced protocol / `off` = logo only in the slot

### 5.3 — Inline display

- [x] Cover slot + stacked chrome
- [x] Kitty / Sixel / half-blocks via `ratatui-image`
- [x] Logo placeholder when no embedded art
- [x] Resize keeps aspect (`Resize::Fit`)

### 5.4 — Polish (not in 0.4.0)

- [ ] Disk cache for decoded/resized bitmaps
- [ ] MCP resource for current cover (optional)
- [ ] Radio / stream covers

## Out of scope (still)

- `folder.jpg` / `cover.jpg` beside the file
- MusicBrainz / Cover Art Archive → Phase 6
- Animated covers, video, or visualizers
- External window viewer as default
- Requiring Kitty installation

## Related docs

- [Formats and metadata](../Domain/Formats-and-Metadata.md) — Phase 2 tags and library
- [TUI players](../Domain/TUI-Players.md) — why a terminal UI
- [TUI architecture](../Architecture/TUI.md) — cover layout
- [Roadmap](Roadmap.md) — all phases
- [Design spec](../../docs/superpowers/specs/2026-09-02-album-art-design.md)

## External links

- [Kitty graphics protocol](https://sw.kovidgoyal.net/kitty/graphics-protocol/)
- [ratatui-image](https://crates.io/crates/ratatui-image)
- [lofty — pictures](https://docs.rs/lofty/latest/lofty/)
