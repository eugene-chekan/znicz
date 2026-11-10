# Roadmap

High-level phases for Znicz. Each phase builds on the previous ones.

| Phase | Name | Status | Doc |
|-------|------|--------|-----|
| **1** | Local playback MVP | **Done** | [README](../../README.md), [Playback pipeline](../Domain/Playback-Pipeline.md) |
| **2** | Library and metadata | **Done** | [Formats and metadata](../Domain/Formats-and-Metadata.md), [Library](../Architecture/Library.md) |
| **2.5** | TUI and UX | **Done** | [TUI architecture](../Architecture/TUI.md) |
| **3** | Playlists | **Done** | [Spec](../../docs/superpowers/specs/2026-08-27-playlist-files-design.md), [Formats and metadata](../Domain/Formats-and-Metadata.md#playlists-phase-3) |
| **4** | Radio streams | **Done** | [Spec](../../docs/superpowers/specs/2026-08-31-radio-streams-design.md), [Formats and metadata](../Domain/Formats-and-Metadata.md#radio-phase-4) |
| **5** | Album art in the TUI | **Done** | [Phase 5 plan](Phase-5-Album-Art.md), [spec](../../docs/superpowers/specs/2026-09-02-album-art-design.md), [TUI architecture](../Architecture/TUI.md) |
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
- Two-line transport; boxed toasts (blue info, green success, yellow warn,
  red error) inset from the pane border so they do not steal hints
  (replaced in 0.7.0 by the transport message row)
- Signal inspector (`i`) for the full file → device path, including sample format
  (replaced in 0.9.0 by the Stage Signal panel)
- Horizontal pan of long titles with `Alt-←` and `Alt-→`
- Library browsing and search from inside the player, not just the CLI
  (TUI search returns artist / album / title entity hits; MCP/CLI stay flat)
- Queue shows track titles, resolved on a background thread, and can be
  played from, reordered by removal, or cleared
- Signal-path line with a **bit perfect** or **resampled** badge
- Repeat, shuffle and mute
- Player errors shown on screen instead of vanishing into the log
- Vim keys alongside arrows; the help overlay is generated from the keymap
- Responsive layout down to very small windows

Details: [TUI architecture](../Architecture/TUI.md).

## Phase 3 (done)

M3U files for the queue. Spec:
**[Playlist files](../../docs/superpowers/specs/2026-08-27-playlist-files-design.md)**

- Save / play / rename / copy / delete from `~/.local/share/znicz/playlists/`
- Clear and play, or add to queue (`P` overlay, CLI, MCP)
- Overlay keys match Radio: `n` new, `e` edit, `c` copy, `d` delete
- `http(s)` lines in an M3U enqueue as streams; save writes URLs and `#EXTINF` names
- PLS / XSPF — [later radio](#later-radio-after-phase-4)

## Phase 4 (done)

HTTP/Icecast byte streams and a station list. Spec:
**[Radio streams](../../docs/superpowers/specs/2026-08-31-radio-streams-design.md)**

- Play an HTTP(S) stream (Symphonia on a blocking `Read`)
- Stations in `stations.toml` (TUI `R`, CLI `znicz station`, MCP tools)
- Playing a station clears the queue and starts that stream
- Overlay keys match Playlists (`n` / `e` / `c` / `d`); `a` appends a station; Enter still replaces
- Live **coded bitrate** on the transport while a stream plays (compressed bytes vs PCM time)
- Icecast **ICY now playing** — `StreamTitle` on the transport; queue rows stay the station name

### Later radio (after Phase 4)

Not in this version. Still later:

- **HLS** — `.m3u8` segment playlists (a second source type)
- **PLS** — INI-style playlist files (`File1=`, `Title1=`)
- **XSPF** — XML Shareable Playlist Format

Settings stay [#6](https://github.com/eugene-chekan/znicz/issues/6). Playlist formats themselves are described in [Formats and metadata](../Domain/Formats-and-Metadata.md#playlist-formats).

## Later playlists (parked)

Not in this version. Tracked as GitHub issues; see [Issues](../Issues.md).

- Browse and edit a saved playlist (view, delete rows, reorder) — [#18](https://github.com/eugene-chekan/znicz/issues/18) (keys: [#36](https://github.com/eugene-chekan/znicz/issues/36))
- Add a library item to a saved playlist — [#19](https://github.com/eugene-chekan/znicz/issues/19)

## Later app state

Session today is `session.toml` ([#20](https://github.com/eugene-chekan/znicz/issues/20), done).
TUI and MCP share one `znicz player` process ([#27](https://github.com/eugene-chekan/znicz/issues/27)).
That is the live bus. A **separate database** for app state and configuration (not `library.db`) is
later, with the settings overlay ([#6](https://github.com/eugene-chekan/znicz/issues/6)).

## Phase 5 (done)

Inline album cover in the TUI. Spec:
**[Album art in the TUI](../../docs/superpowers/specs/2026-09-02-album-art-design.md)**

- Eight-row cover slot beside stacked transport chrome (`show_cover` / height rules)
  (moved to the Stage view in 0.9.0)
- Kitty / Sixel / half-blocks via `ratatui-image`; logo when missing
- TUI reads embedded art from `TrackInfo.path` (no cover bytes on IPC)
- **No Kitty install** and no `icat`
- Config: `[tui]` `show_cover`, `cover_protocol`
- Station `art` (local path) and ICY `StreamUrl` image for stream covers (0.4.1)
- AudioAddict song covers for RadioTunes / DI.FM / RockRadio (0.4.2)

Details: [Phase 5 — Album art](Phase-5-Album-Art.md), [TUI architecture](../Architecture/TUI.md).

## Phase 6

- MusicBrainz lookup for missing tags and higher-resolution cover art from Cover Art Archive
- The MusicBrainz client for stream covers exists (`znicz-core/src/cover/musicbrainz.rs`, 0.11.0) and can be reused for tags. See [Stream covers](../Architecture/Cover-Sources.md).

## TUI reimagining (0.6+)

A five-part rework of the player experience. Each part has its own spec and PR.

| # | Part | Status |
| --- | --- | --- |
| 1 | Keymap and discoverability: `h` / `l` move, arrows seek, context footer, context-first help with a filter | **Done** in 0.6.0 ([spec](../../docs/superpowers/specs/2026-11-07-keymap-discoverability-design.md)) |
| 2 | Space and layering: modals inside the list region, toasts in a transport slot with history, focus styling, breadcrumb, playing marker | **Done** in 0.7.0 ([spec](../../docs/superpowers/specs/2026-11-07-space-and-layering-design.md)) |
| 3 | Empty-library onboarding: scan a folder from the TUI; the player process owns the library | **Done** in 0.8.0 ([spec](../../docs/superpowers/specs/2026-11-08-library-service-design.md)) |
| 4 | Stage mode ([#46](https://github.com/eugene-chekan/znicz/issues/46)): full-screen now playing; the inspector folds in; marquee ([#34](https://github.com/eugene-chekan/znicz/issues/34)) | **Done** in 0.9.0 ([spec](../../docs/superpowers/specs/2026-11-09-stage-mode-design.md)) |
| 5 | Reorder rows ([#36](https://github.com/eugene-chekan/znicz/issues/36)) and finer volume ([#47](https://github.com/eugene-chekan/znicz/issues/47)) | **Done** in 0.10.0 ([spec](../../docs/superpowers/specs/2026-11-09-queue-reorder-and-volume-design.md)); the five-part rework is complete |

Browse dropped the cover and gives the library the full height; the cover lives on Stage.

## Later TUI (parked)

Not in phases 3–6. Tracked as GitHub issues; see [Issues](../Issues.md).

- Settings overlay — [#6](https://github.com/eugene-chekan/znicz/issues/6)
- Colour themes for light and Solarized palettes — [#68](https://github.com/eugene-chekan/znicz/issues/68)
