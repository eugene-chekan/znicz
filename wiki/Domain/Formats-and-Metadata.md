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

## Playlist formats

A **playlist** is a small text (or XML) file that lists tracks in order. It is
not the music itself. Players disagree on which formats they open; the common
ones are:

| Format | What it is | Znicz |
|--------|------------|--------|
| **M3U** | One path or URL per line. `#` starts a comment. **Extended M3U** adds `#EXTM3U` and `#EXTINF:duration,title` before a row. Encoding is not guaranteed. | **Yes.** UTF-8 `.m3u` / `.m3u8` files of local paths and `http(s)` streams. |
| **M3U8** | Same grammar as M3U, saved as UTF-8. Winamp used the `8` to mean UTF-8. | **Yes**, as a UTF-8 M3U list. |
| **HLS `.m3u8`** | A *different* use of the same suffix: `#EXT-X-…` tags and a list of **media segments** (short `.ts` / `.m4s` files) that a live or VOD stream is cut into. | **Not yet.** A saved playlist named `.m3u8` is still an M3U list. An `http(s)` URL that is really an HLS playlist is enqueued as a stream and play fails until HLS exists. |
| **PLS** | INI-style (`[playlist]`, `File1=`, `Title1=`, `Length1=`). Common with Winamp / Icecast “listen” links. | **Later** (see [roadmap](../Plans/Roadmap.md#later-radio-after-phase-4)). |
| **XSPF** | XML Shareable Playlist Format (`<trackList>`, `<location>`). Can hold local paths or URLs. | **Later** (same roadmap section). |
| **CUE** | Indexes inside one audio file (CD images), not a list of separate files. | Not a playlist in this sense. Out of scope. |
| **ASX / WPL** | Older Windows XML lists. | Out of scope. |

Relative paths in a playlist file resolve against **that file’s directory**.

## Playlists (Phase 3)

Znicz playlists are **M3U / M3U8 files** of local paths and `http://` /
`https://` stream rows. PLS and XSPF are still later.

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
with `http://` or `https://`. Optional `art` is a **local image file** path
(not `http://` or `https://`); omit or leave empty for no station picture.
Playing a station **clears the queue** and starts that stream.

Open the list in the player with `R`. Same list from the CLI and MCP:

```bash
znicz station list
znicz station add "Example" https://example.com/stream
znicz station play Example
znicz station play Example --append
znicz station art "Example" ~/Pictures/example.png
znicz station art "Example" --clear
```

CLI also has `remove`, `rename`, `url`, `art`, and `copy`. MCP tools: `list_stations`, `add_radio_station`, `play_station`,
`rename_radio_station`, `set_station_url`, `set_station_art`, `copy_radio_station`, `remove_radio_station`, plus
resource `znicz://stations`.

The TUI overlay (`R`) uses the same saved-list keys as playlists: `n` new, `e`
edit (name, URL, and art on one form), `c` copy, `d` delete. Radio `a` appends the
station. Enter / `play_station` still replace the queue.

M3U URL lines play as streams. While a stream plays, the signal path shows a
**coded bitrate** (compressed bytes versus PCM duration, once a quarter second
has decoded). That is the stream’s audio rate, not Icecast `icy-br` and not
the decoded PCM rate (1411 kbps for CD-shape WAV). Icecast `StreamTitle`
replaces the now-playing title (and `tags.title`) when the station sends it.
An empty title falls back to the station name. Queue rows stay the station
name. Icecast `StreamUrl` may point at a cover image; when it decodes, the TUI
cover slot shows that picture instead of station `art` or the logo.
RadioTunes, DI.FM, RockRadio, JazzRadio, ClassicalRadio, and ZenRadio do not
send `StreamUrl`. For those hosts the TUI looks up the channel from the stream
URL and uses AudioAddict `art_url` as the song cover.

**Later:** HLS, PLS, XSPF. See the
[roadmap](../Plans/Roadmap.md#later-radio-after-phase-4).

## Session

The live queue is **`session.toml`** beside stations and the library database:

- Linux: `~/.local/share/znicz/session.toml`
- Windows: `%APPDATA%\znicz\session.toml`

Override with `ZNICZ_SESSION_PATH`. It holds file and stream rows, the queue
index, volume, mute, repeat, and shuffle. Not seek, not whether you were
playing, not the output device.

Bare `znicz` and `znicz mcp` autostart `znicz player`, which restores the
session **Stopped** (Space starts the current row). Missing local files are
skipped; streams stay. `znicz file.flac`, playlist play, and station play
without `--append` replace the live queue. `--append` keeps what is already
there, then appends. The player process writes the file shortly after those
fields change (about 500 ms of no further change) and again when it exits.

TUI and MCP read live state from the player process, not from this file.
See [MCP](../Architecture/MCP.md).

This is not `~/.cache/znicz/znicz-session.log` (TUI stderr). A later app-state
database may replace this file; see the [roadmap](../Plans/Roadmap.md).

## Library

A library is a **database of tracks** (path, tags, duration) plus search:
walk folders → read tags → SQLite. Znicz does this in the `znicz-library` crate;
see [Library architecture](../Architecture/Library.md).

```bash
znicz scan ~/Music
znicz search "kind of blue"
znicz albums
```

Embedded **album art** is part of the same tag data. The TUI reads the cover
from the file path on `TrackInfo` (not over IPC); see
[Phase 5](../Plans/Phase-5-Album-Art.md).

## Extra reading

- [FLAC format](https://xiph.org/flac/documentation.html)
- [M3U (Wikipedia)](https://en.wikipedia.org/wiki/M3U)
- [XSPF](https://xspf.org/)
- [PLS (Wikipedia)](https://en.wikipedia.org/wiki/PLS_(file_format))
- [HTTP Live Streaming](https://datatracker.ietf.org/doc/html/rfc8216)
- [Icecast](https://icecast.org/)
