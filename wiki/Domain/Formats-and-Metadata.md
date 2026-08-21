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

**Tags** (title, album, artist) live inside the file. Phase 1 shows the **file name**. Phase 2 will scan tags with a crate like [lofty](https://github.com/Serial-ATA/lofty-rs).

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

## Library (Phase 2)

A library is a **database of tracks** (path, tags, duration) plus search. Typical stack later: walk folders → read tags → SQLite.

Embedded **album art** is read as part of tag scanning. Display in the TUI is [Phase 5](../Plans/Phase-5-Album-Art.md).

## Extra reading

- [FLAC format](https://xiph.org/flac/documentation.html)
- [M3U (Wikipedia)](https://en.wikipedia.org/wiki/M3U)
- [Icecast](https://icecast.org/)
