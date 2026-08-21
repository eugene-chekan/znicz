# Znicz

Cross-platform audiophile TUI music player with a native MCP server for AI agent control.

## Features

- Local file playback (FLAC, WAV, ALAC, MP3, and more via Symphonia)
- Bit-perfect-friendly output via cpal (ALSA on Linux, WASAPI on Windows)
- TUI with now playing (title, artist, album), queue, transport controls
- Music library: folder scan, tag indexing, search and album browse
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

## Keybindings (TUI)

| Key | Action |
|-----|--------|
| Space | Play / Pause |
| n | Next track |
| p | Previous track |
| + / - | Volume |
| Left / Right | Seek ±5s |
| ? | Help |
| q | Quit |

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
