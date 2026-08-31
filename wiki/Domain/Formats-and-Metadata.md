# Formats, tags, playlists, radio

## Local files (Phase 1)

Symphonia can decode many containers. Common ones:

| Extension | Typical codec | Notes |
|-----------|---------------|--------|
| `.flac` | FLAC | Lossless. Favourite for libraries |
| `.wav` | PCM | Uncompressed |
| `.m4a` | ALAC or AAC | ALAC is lossless; AAC is lossy |
| `.mp3` | MP3 | Lossy |
| `.ogg` | Vorbis or Opus | Lossy |

## Tags

**Tags** are the text stored inside a music file: title, artist, album, year,
track number. Different formats keep them in different places (ID3 in MP3,
Vorbis comments in FLAC, MP4 atoms in M4A), which is why we use
[lofty](https://github.com/Serial-ATA/lofty-rs) — it reads all of them through
one interface.

Znicz reads tags when a track loads, so the TUI shows the real title and an
"Artist — Album" line. Files with no tags fall back to the file name, because
playing music matters more than metadata.

## Playlists (Phase 3)

A playlist is an **M3U / M3U8 file of local paths**. PLS and XSPF are still later.
HTTP / Icecast lines are skipped (Phase 4).

| Line | Meaning |
|------|---------|
| Empty, or starts with `#` | Ignored (`#EXTM3U`, `#EXTINF:…`) |
| Contains `://` | URL. Skipped and counted |
| Anything else | A path. Relative paths resolve against the playlist file’s directory |

A UTF-8 BOM is stripped. Missing files are skipped and counted. If nothing
playable remains, the queue is left alone.

**Writing** is UTF-8, no BOM, one absolute path per line (the real path on
disk, which on Windows can look different from the path you typed). Saved files live beside
the library database:

- Linux: `~/.local/share/znicz/playlists/`
- Windows: `%APPDATA%\znicz\playlists\`

Override with `ZNICZ_PLAYLISTS_DIR`. The folder is created on first save.

Play has two actions, from the TUI (`P`), CLI (`znicz playlist …`), and MCP:

1. **Clear and play** — replace the queue and start the first track
2. **Add to queue** — append, do not start or stop playback

Parsing lives in `znicz-core::playlist`. The engine has no extra commands:
callers send `QueueClear` / `QueueAdd` / `QueuePlayIndex(0)`.

## Radio (Phase 4)

Radio is often an **HTTP stream** (Icecast): the player downloads audio forever instead of a file. HLS (`.m3u8`) is a playlist of short segments. Same decoder, different “source”.

## Library

A library is a **database of tracks** (path, tags, duration) plus search:
walk folders → read tags → SQLite. Znicz does this in the `znicz-library` crate;
see [Library architecture](../Architecture/Library.md).

```bash
znicz scan ~/Music
znicz search "kind of blue"
znicz albums
```

Embedded **album art** is part of the same tag data. Displaying it in the TUI is
[Phase 5](../Plans/Phase-5-Album-Art.md).

## Extra reading

- [FLAC format](https://xiph.org/flac/documentation.html)
- [M3U (Wikipedia)](https://en.wikipedia.org/wiki/M3U)
- [Icecast](https://icecast.org/)
