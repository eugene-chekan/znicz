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
HTTP / Icecast lines are still skipped in this version (later radio).

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

Radio here is an **HTTP or HTTPS Icecast-style byte stream**: the player
downloads audio and feeds it to Symphonia, the same decoder used for files.
HLS (`.m3u8` segments) is not in this version.

Stations live in `stations.toml` beside the library database:

- Linux: `~/.local/share/znicz/stations.toml`
- Windows: `%APPDATA%\znicz\stations.toml`

Override with `ZNICZ_STATIONS_PATH`. Names must be unique. A URL must start
with `http://` or `https://`. Playing a station **clears the queue** and starts
that stream.

Open the list in the player with `R`. Same list from the CLI and MCP:

```bash
znicz station list
znicz station add "Example" https://example.com/stream
znicz station play Example
```

CLI also has `remove`, `rename`, and `url`. MCP tools: `list_stations`, `add_radio_station`, `play_station`,
`rename_radio_station`, `set_station_url`, `remove_radio_station`, plus
resource `znicz://stations`.

M3U playlists still **skip** `http://` / `https://` lines. **Later:** ICY song
titles on the transport, HLS, those M3U URL lines as playable streams, and a
mixed queue of files and stations. See the
[roadmap](../Plans/Roadmap.md#later-radio-after-phase-4).

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
