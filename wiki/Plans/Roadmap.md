# Roadmap

High-level phases for Znicz. Each phase builds on the previous ones.

| Phase | Name | Status | Doc |
|-------|------|--------|-----|
| **1** | Local playback MVP | **Done** | [README](../../README.md), [Playback pipeline](../Domain/Playback-Pipeline.md) |
| **2** | Library and metadata | Planned | [Formats and metadata](../Domain/Formats-and-Metadata.md) |
| **3** | Playlists | Planned | [Formats and metadata](../Domain/Formats-and-Metadata.md#playlists-phase-3) |
| **4** | Radio streams | Planned | [Formats and metadata](../Domain/Formats-and-Metadata.md#radio-phase-4) |
| **5** | Album art in the TUI | Planned | [Phase 5 plan](Phase-5-Album-Art.md) |
| **6** | MusicBrainz enrichment | Planned | MCP stub `enrich_metadata` in `znicz-mcp` |

## Phase 1 (done)

- Local file playback (Symphonia + cpal)
- TUI transport, queue, now playing (file name + format)
- MCP server with tools, resources, prompts, skills
- Bit-perfect-friendly device rate matching

## Phase 2

- Tag reading (title, artist, album) via lofty
- Folder scan and SQLite library
- MCP: `search_library`, `get_track`, `browse_album`

Phase 5 **reuses** Phase 2 picture extraction; see [Phase 5 — Album art](Phase-5-Album-Art.md).

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
