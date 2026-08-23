# Roadmap

High-level phases for Znicz. Each phase builds on the previous ones.

| Phase | Name | Status | Doc |
|-------|------|--------|-----|
| **1** | Local playback MVP | **Done** | [README](../../README.md), [Playback pipeline](../Domain/Playback-Pipeline.md) |
| **2** | Library and metadata | **Done** | [Formats and metadata](../Domain/Formats-and-Metadata.md), [Library](../Architecture/Library.md) |
| **2.5** | TUI and UX | **Done** | [TUI architecture](../Architecture/TUI.md) |
| **3** | Playlists | Planned | [Formats and metadata](../Domain/Formats-and-Metadata.md#playlists-phase-3) |
| **4** | Radio streams | Planned | [Formats and metadata](../Domain/Formats-and-Metadata.md#radio-phase-4) |
| **5** | Album art in the TUI | Planned | [Phase 5 plan](Phase-5-Album-Art.md) |
| **6** | MusicBrainz enrichment | Planned | MCP stub `enrich_metadata` in `znicz-mcp` |

## Phase 1 (done)

- Local file playback (Symphonia + cpal)
- TUI transport, queue, now playing (file name + format)
- MCP server with tools, resources, prompts, skills
- Bit-perfect-friendly device rate matching

## Phase 2 (done)

- Tag reading (title, artist, album, year, track number) via lofty
- TUI shows the real title and an "Artist — Album" line
- `znicz-library` crate: folder scan into SQLite, search, album browse
- CLI: `znicz scan`, `znicz search`, `znicz albums`
- MCP: `scan_library`, `search_library`, `get_track`, `browse_album`,
  `list_albums`, `library_stats`, `library_prune`

Details: [Library architecture](../Architecture/Library.md).

Phase 5 **reuses** this tag reading for embedded cover art; see
[Phase 5 — Album art](Phase-5-Album-Art.md).

## Phase 2.5 — TUI and UX (done)

Znicz is a terminal player first, so the interface caught up with the engine:

- Library as home; queue as an overlay drawer (`]`); devices as an overlay
  modal (`,`)
- Two-line transport; floating toasts that do not steal hints
- Horizontal pan of long titles with `<` and `>`
- Library browsing and search from inside the player, not just the CLI
- Queue shows track titles, resolved on a background thread, and can be
  played from, reordered by removal, or cleared
- Signal-path line with a **bit perfect** or **resampled** badge
- Repeat, shuffle and mute
- Player errors shown on screen instead of vanishing into the log
- Vim keys alongside arrows; the help overlay is generated from the keymap
- Responsive layout down to very small windows

Details: [TUI architecture](../Architecture/TUI.md).

## Phase 3

- M3U / PLS / XSPF import and export
- MCP playlist tools

## Phase 4

- HTTP/Icecast radio sources
- Station list in config or DB

## Phase 5

- Inline album cover in the TUI
- Graphics protocols (Kitty, Sixel, iTerm2) with universal half-block fallback
- **No Kitty install required** — Rust emits protocol sequences directly

Full plan: **[Phase 5 — Album art in the TUI](Phase-5-Album-Art.md)**

## Phase 6

- MusicBrainz lookup for missing tags and higher-resolution cover art from Cover Art Archive
