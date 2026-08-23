# Znicz

Cross-platform audiophile TUI music player with a native MCP server for AI agent control.

## Features

- Local file playback (FLAC, WAV, ALAC, MP3, and more via Symphonia)
- Bit-perfect-friendly output via cpal (ALSA on Linux, WASAPI on Windows)
- TUI with a library browser, search, queue management and an output device picker
- Signal-path display: file format, the stream the device actually opened, and
  whether playback is bit perfect
- Music library: folder scan, tag indexing, search and album browse
- Repeat, shuffle, mute, and errors reported on screen rather than only in the log
- MCP server: tools, resources, prompts, and bundled Agent Skills

## Build

### Linux

```bash
# Debian/Ubuntu
sudo apt install pkg-config libasound2-dev

cargo build --release
```

### Windows

```powershell
cargo build --release
```

Uses WASAPI via cpal. For ASIO or exclusive WASAPI, see future roadmap.

## Documentation

Start at **[wiki/Home.md](wiki/Home.md)**. It covers digital audio, the playback pipeline, crate layout, and the Rust ideas (ownership, threads, traits) used in this repo.

Future work is tracked in **[wiki/Plans/Roadmap.md](wiki/Plans/Roadmap.md)** (Phase 5: [album art in the TUI](wiki/Plans/Phase-5-Album-Art.md)).

## Usage

```bash
# Play files in TUI mode
znicz track.flac
znicz album/*.flac

# List audio devices
znicz --list-devices

# Select device
znicz --device "device-name" track.flac

# MCP server (stdio)
znicz mcp
```

### Music library

```bash
# Index a folder (subfolders included). Rescans skip unchanged files.
znicz scan ~/Music

# Index, then drop entries whose files were deleted
znicz scan ~/Music --prune

# Search by title, artist or album
znicz search portishead

# List albums
znicz albums
```

The index is a SQLite file, by default `~/.local/share/znicz/library.db`
(`%APPDATA%\znicz\library.db` on Windows).

## The interface

Three panes, switched with `Tab` or `1`/`2`/`3`:

- **Queue** — what plays next, by track title rather than file name
- **Library** — albums, album tracks and search results
- **Devices** — pick the output, and see what the open stream settled on

Above them sits the now-playing header with the seek bar and the signal path,
for example `FLAC 96 kHz 24-bit 2882 kbps stereo → 96 kHz stereo  ● bit perfect`.
When the device will not take the file's own rate the badge reads `▲ resampled`,
so a silent conversion never goes unnoticed.

Press `?` in the player for the full keymap. The essentials:

| Key | Action |
|-----|--------|
| Space | Play / pause |
| s | Stop |
| n / N | Next / previous track |
| → ← or l h | Seek ±5s (`L` / `H` for ±30s) |
| + / - | Volume, `m` to mute |
| r / z | Repeat (off, all, one) / shuffle |
| j k, g G, Ctrl-d Ctrl-u | Move in the list |
| Enter | Play the selection, or open an album |
| a / A | Add the selection / everything listed to the queue |
| d / C | Remove from the queue / clear it |
| / | Search the library |
| Tab, 1 2 3 | Switch pane |
| ? / q | Help / quit |

To see the layout without starting the player:

```bash
cargo run -p znicz-tui --example preview -- 120 40
```

While the TUI is running, stderr is redirected to `~/.cache/znicz/znicz-session.log`
so that ALSA's warnings cannot draw over the interface. Look there if the player
misbehaves.

## Configuration

`~/.config/znicz/config.toml`:

```toml
[audio]
device = "default"
volume = 1.0
bit_perfect = true

[mcp]
skills_dirs = []

[library]
# Optional. Defaults to the user data directory.
path = "~/.local/share/znicz/library.db"
```

## MCP (Cursor)

Add to `.cursor/mcp.json`:

```json
{
  "mcpServers": {
    "znicz": {
      "command": "znicz",
      "args": ["mcp"]
    }
  }
}
```

Build first and ensure `znicz` is on `PATH`, or use the full path to `target/release/znicz`.

## Checking playback speed

If music sounds too fast or too slow, measure it:

```bash
cargo run --release -p znicz-core --example timing -- /path/to/track.flac
```

It prints the negotiated device rate, position drift, and a speed factor. Only
about `1.000x` is correct. Run with `RUST_LOG=info` to see whether the stream is
bit perfect or resampled.

## Workspace crates

| Crate | Role |
|-------|------|
| `znicz-core` | Audio engine, player state, tag reading |
| `znicz-library` | SQLite music index: scan, search, albums |
| `znicz-tui` | Ratatui frontend |
| `znicz-mcp` | MCP server + skills |
| `znicz` | CLI binary |

## License

MIT OR Apache-2.0
