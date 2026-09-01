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

A playlist is an **M3U / M3U8 file** of local paths and `http://` / `https://`
stream rows. PLS and XSPF are still later.

| Line | Meaning |
|------|---------|
| Empty, or a `#` comment other than `#EXTINF:` | Ignored (`#EXTM3U`, `#EXT-X-…`) |
| `#EXTINF:…,Title` | Title for the **next** http(s) line only |
| Starts with `http://` or `https://` | Stream row. Name is the `#EXTINF` title, or the URL |
| Other `://` (`ftp://`, `file://`, …) | Skipped and counted |
| Anything else | A path. Relative paths resolve against the playlist file’s directory |

A UTF-8 BOM is stripped. Missing files are skipped and counted. If nothing
playable remains (no files and no http(s) lines), the queue is left alone.

**Writing** is UTF-8, no BOM. File rows are one absolute path per line. Stream
rows are the URL; if the queue name is not the URL, write `#EXTINF:-1,Name`
on the line before. Saved files live beside the library database:

- Linux: `~/.local/share/znicz/playlists/`
- Windows: `%APPDATA%\znicz\playlists\`

Override with `ZNICZ_PLAYLISTS_DIR`. The folder is created on first save.

Play has two actions, from the TUI (`P`), CLI (`znicz playlist …`), and MCP:

1. **Clear and play** — replace the queue and start the first track
2. **Add to queue** — append, do not start or stop playback

Rename, copy, and delete are the same three verbs from the TUI (`P`: `e` / `c` /
`d`), the CLI (`znicz playlist rename` / `copy` / `remove`), and MCP
(`rename_playlist`, `copy_playlist`, `remove_playlist`). Rename moves the file
in the playlists folder. Copy leaves the original and writes a second file. A
name that already exists is refused. If you omit `.m3u` / `.m3u8` on the new
name, the old suffix is kept. Delete is immediate.

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
znicz station play Example --append
```

CLI also has `remove`, `rename`, `url`, and `copy`. MCP tools: `list_stations`, `add_radio_station`, `play_station`,
`rename_radio_station`, `set_station_url`, `copy_radio_station`, `remove_radio_station`, plus
resource `znicz://stations`.

The TUI overlay (`R`) uses the same saved-list keys as playlists: `n` new, `e`
edit (name and URL on one form), `c` copy, `d` delete. Radio `a` appends the
station. Enter / `play_station` still replace the queue.

M3U URL lines play as streams. **Later:** ICY song titles on the transport,
HLS. See the
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
