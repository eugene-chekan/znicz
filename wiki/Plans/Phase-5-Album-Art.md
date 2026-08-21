# Phase 5 — Album art in the TUI

**Status:** Planned  
**Depends on:** [Phase 2](../Domain/Formats-and-Metadata.md#library-phase-2) (tag and picture extraction)  
**Primary crates:** `znicz-core` (read embedded art), `znicz-tui` (draw it)

## Goal

Show the album cover for the current track inside the Znicz TUI — sharp where the terminal allows it, with a safe fallback everywhere else.

Phase 1 shows the **file name** only. Phase 2 adds tags (title, artist, album) from files. Phase 5 adds the **cover image** to the now-playing screen.

This does not change audio quality. It is a display feature only.

## What “full quality” means in a terminal

Embedded album art in FLAC/MP3 is often a JPEG or PNG (500×500 up to 3000×3000). In a TUI:

| Layer | Full quality? |
|-------|----------------|
| **Read from file** | Yes — use the original embedded bytes, no recompression |
| **On screen** | Depends on terminal size and rendering method |

A terminal is a grid of cells, not a pixel canvas. You always **scale** the image to fit the layout. With a graphics-capable terminal (Kitty, WezTerm, Ghostty), the result can look almost as good as a small widget in a GUI app. With character-cell fallback, you get a recognizable thumbnail, not hi-res.

**Bottom line:** keep the full source image in memory or on disk cache; display at the best quality the terminal supports.

## Three ways to show images

### 1. Graphics protocols (preferred — best look)

The terminal draws real bitmaps via escape sequences.

| Protocol | Terminals | Notes |
|----------|-----------|--------|
| **Kitty graphics** | Kitty, WezTerm, Ghostty | Best quality; widely used in modern terminals |
| **Sixel** | mlterm, foot, WezTerm, some xterms | Older; still useful |
| **iTerm2 inline images** | iTerm2 (macOS) | Good on Mac |

**Rust approach:** emit protocol sequences from Znicz (e.g. [`viuer`](https://crates.io/crates/viuer) or [`ratatui-image`](https://crates.io/crates/ratatui-image)). Decode JPEG/PNG with the [`image`](https://crates.io/crates/image) crate.

**No Kitty install required** for this path. The app speaks the protocol; only the **terminal** must understand it.

### 2. Character-cell rendering (universal fallback)

Map pixels onto terminal cells using half-blocks, braille dots, or shade blocks (similar to [`chafa`](https://github.com/hpjansson/chafa) or [`viu`](https://github.com/atanunq/viu)).

- Works in **any** terminal, including plain SSH sessions
- Always downscaled (roughly one column = one “pixel” wide, two rows = one “pixel” tall with half-blocks)
- Low CPU, no special terminal features

Use when graphics protocols are unavailable or the user disables them in config.

### 3. External viewer (out of scope for Phase 5)

Spawning `feh`, `viu`, or `kitty +kitten icat` in a separate window can show full resolution but is **not** part of the inline TUI layout. Do not rely on this as the default.

## Kitty `icat` — do we need Kitty installed?

**Only if Znicz shells out to `kitty +kitten icat`.** That command is a helper bundled with the Kitty terminal. It is not a separate package on most systems.

| Approach | Kitty installed? | Graphics-capable terminal? |
|----------|------------------|----------------------------|
| Spawn `kitty +kitten icat` | **Yes** | Yes (Kitty, WezTerm, Ghostty, …) |
| Rust library (`viuer`, `ratatui-image`) | **No** | Yes |
| Half-block fallback | No | No (any terminal) |

**Znicz should use the Rust library path**, not `icat`. That keeps WezTerm and Ghostty users working without installing Kitty. Detect terminal capability at runtime; never hard-depend on an external `icat` binary.

## Cross-platform terminals

| OS | Good inline art | Character fallback only |
|----|-----------------|-------------------------|
| **Linux** | WezTerm, Kitty, Ghostty, foot | GNOME Terminal, Konsole (unless Sixel enabled), xterm |
| **Windows** | WezTerm, Windows Terminal (evolving) | Classic conhost, older terminals |
| **SSH** | Client must support protocol | Always works with half-blocks |

Plan for **detect → best method → fallback**, not one code path for all users.

## Architecture

```
Track load (znicz-core)
    │
    ├─ lofty: read APIC / FLAC PICTURE / Vorbis METADATA_BLOCK_PICTURE
    ├─ store CoverArt { mime, bytes } on TrackInfo or side cache
    │
    ▼
PlayerState / PlayerEvent (optional CoverArtChanged)
    │
    ▼
TUI draw loop (znicz-tui)
    │
    ├─ detect: KITTY_GRAPHICS | SIXEL | ITERM2 | HalfBlock
    ├─ resize to panel (keep aspect ratio)
    └─ render in left column of “Now Playing”
```

### Core (`znicz-core`)

- Extend metadata loading (shared with Phase 2) to extract **front cover** picture blocks.
- Prefer embedded art; optional later: `folder.jpg` / `cover.jpg` next to the file.
- Expose `CoverArt` (mime type + bytes, or path in a disk cache keyed by file hash).
- Do **not** decode images on the player thread — read tags when opening a track on the player thread is OK for Phase 5 MVP; heavy work can move to a small worker if needed.

### TUI (`znicz-tui`)

- Split “Now Playing” horizontally: **cover | title / artist / progress / format**.
- Terminal capability probe once at startup (env vars like `TERM`, `KITTY_WINDOW_ID`, `WEZTERM_EXECUTABLE`, plus optional config override).
- Render pipeline with fallback chain (see below).
- Config: `show_cover = true`, `cover_protocol = "auto" | "kitty" | "sixel" | "halfblocks" | "off"`.

### MCP (`znicz-mcp`) — optional stretch

- Resource `cover://current` returning base64 JPEG for agents that want a preview.
- Not required for Phase 5 MVP.

## Suggested dependencies

| Crate | Role |
|-------|------|
| `lofty` | Read embedded pictures (Phase 2; reused here) |
| `image` | Decode JPEG/PNG, resize for display |
| `viuer` or `ratatui-image` | Kitty / iTerm2 / Sixel + half-block in one API |

Evaluate `ratatui-image` first for Ratatui integration; keep the image widget behind a small internal trait so we can swap backends.

## Milestones

### 5.1 — Extract embedded art

- [ ] `CoverArt` type in `znicz-core`
- [ ] Read front cover from FLAC, MP3, M4A via lofty when a track opens
- [ ] Unit tests with tiny fixture files (embedded 1×1 PNG/JPEG)
- [ ] `TrackInfo` or event carries cover reference (bytes or cache path)

### 5.2 — Terminal detection

- [ ] Probe graphics support at TUI startup
- [ ] Config keys in `~/.config/znicz/config.toml` under `[tui]`
- [ ] Log chosen renderer at `info` level once

### 5.3 — Inline display

- [ ] Layout: cover panel + text panel
- [ ] Kitty / Sixel / iTerm2 path via `viuer` or `ratatui-image`
- [ ] Half-block fallback when protocols unavailable
- [ ] Placeholder when no embedded art (`[no cover]` or empty frame)
- [ ] Resize on terminal resize; preserve aspect ratio

### 5.4 — Polish

- [ ] Disk cache for decoded/resized bitmaps (avoid re-decode on queue skip)
- [ ] MCP resource for current cover (optional)
- [ ] Wiki update: [TUI players](../Domain/TUI-Players.md) screenshot/description
- [ ] README: one line under features + link here

## Fallback chain (default)

```
1. User set cover_protocol explicitly → use that (or off)
2. auto: try Kitty graphics
3. else try Sixel
4. else try iTerm2 (macOS)
5. else half-block character art
6. else text placeholder only
```

Never spawn `kitty icat` in the default chain.

## Verification

Manual:

1. FLAC with large embedded cover in **WezTerm** — sharp image, no Kitty installed
2. Same file in **plain xterm** — half-block thumbnail or placeholder
3. File with **no** embedded art — placeholder, no crash
4. **Resize** terminal — cover rescales, layout stable
5. **SSH** to remote with WezTerm client — graphics if client supports it

Automated:

- Core: lofty extracts picture bytes from fixtures
- TUI: mock `CoverArt` + snapshot or dimension checks (full pixel diff is fragile across terminals; prefer “renderer selected” unit tests)

## Out of scope (Phase 5)

- Downloading art from the internet (MusicBrainz / Cover Art Archive → Phase 6)
- Animated covers, video, or visualizers
- External window viewer as default
- Requiring Kitty installation

## Related docs

- [Formats and metadata](../Domain/Formats-and-Metadata.md) — Phase 2 tags and library
- [TUI players](../Domain/TUI-Players.md) — why a terminal UI
- [TUI architecture](../Architecture/TUI.md) — current layout
- [Roadmap](Roadmap.md) — all phases

## External links

- [Kitty graphics protocol](https://sw.kovidgoyal.net/kitty/graphics-protocol/)
- [viuer](https://crates.io/crates/viuer)
- [ratatui-image](https://crates.io/crates/ratatui-image)
- [lofty — pictures](https://docs.rs/lofty/latest/lofty/)
