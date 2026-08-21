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

A playlist is a list of paths or URLs.

| Format | Idea |
|--------|------|
| M3U / M3U8 | Simple text list |
| PLS | Older radio/playlist format |
| XSPF | XML playlist |

Znicz MCP already has **stub** tools (`import_playlist`, …) that return “not implemented” until Phase 3.

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
