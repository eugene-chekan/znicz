<p align="center">
  <img src="znicz-tui/assets/logo-glass.png" alt="Znicz logo: a candle in a glass whose flame holds a play symbol" width="512">
</p>

<h1 align="center">Znicz</h1>

<p align="center">Cross-platform audiophile TUI music player with a native MCP server for AI agent control.</p>

## Features

- Local file playback (FLAC, WAV, ALAC, MP3, and more via Symphonia)
- Bit-perfect-friendly output via cpal (ALSA on Linux, WASAPI on Windows)
- TUI with a library browser, search, queue management and an output device picker
- Signal-path display: file format, the stream the device actually opened, and
  whether playback is bit perfect
- Music library: folder scan, tag indexing, search and album browse
- Repeat, shuffle, mute, and errors reported on screen rather than only in the log
- Radio stations: HTTP/Icecast byte streams from the TUI, CLI, and MCP
- Session restore: queue, volume, mute, repeat, and shuffle survive a restart
- Embedded album art in the TUI (Kitty / Sixel / half-blocks). Stream covers come from the ICY image, AudioAddict, an artist / title search (Deezer, then iTunes, then MusicBrainz and the Cover Art Archive), or the station's art, else the logo
- MCP server: tools, resources, prompts, and bundled Agent Skills (same live player as the TUI)

## Install

Download a release build from
[GitHub Releases](https://github.com/eugene-chekan/znicz/releases/latest). It is
one self-contained binary; settings and the library live in your user folders.

**Linux** (x86-64 and ARM64, glibc 2.35 or newer, ALSA — present on every desktop
distribution, PipeWire and PulseAudio included):

```sh
curl -LsSf https://github.com/eugene-chekan/znicz/releases/latest/download/znicz-installer.sh | sh
```

**Windows** (x86-64, PowerShell):

```powershell
irm https://github.com/eugene-chekan/znicz/releases/latest/download/znicz-installer.ps1 | iex
```

Both put `znicz` in `~/.local/bin` (`%USERPROFILE%\.local\bin` on Windows) and add
it to your `PATH`; open a new terminal afterwards. Set `ZNICZ_INSTALL_DIR` to pick
another folder, or `ZNICZ_NO_MODIFY_PATH=1` to leave `PATH` alone.

To update later, run `znicz update`, or press `?` then `u` in the TUI when it
says a new release is out (`znicz update --check` only reports).

- **By hand:** download the `.tar.xz` (Linux) or `.zip` (Windows) for your
  platform, check it against its `.sha256`, and put `znicz` somewhere on `PATH`.
- **Update:** run the install command again.
- **Uninstall:** delete the binary (and `~/.config/znicz/env.*` plus the `PATH`
  line the installer added). Your data stays in `~/.local/share/znicz` /
  `%APPDATA%\znicz` until you remove it.
- Windows may show a SmartScreen warning for a browser-downloaded `znicz.exe`:
  release builds are not code-signed yet.

## Build from source

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

GitHub Actions on `main` and pull requests run rustfmt, Clippy, and
`cargo test --workspace` on Linux and Windows.
Pushing a `vX.Y.Z` tag runs the release workflow (see
[Releasing](wiki/Rust/Cargo-Workspace.md#releasing)).

## Documentation

Start at **[wiki/Home.md](wiki/Home.md)**. It covers digital audio, the playback pipeline, crate layout, and the Rust ideas (ownership, threads, traits) used in this repo.

Future work is tracked in **[wiki/Plans/Roadmap.md](wiki/Plans/Roadmap.md)** (Phase 5 album art is done: [Phase 5](wiki/Plans/Phase-5-Album-Art.md)).

## Usage

```bash
# Play files in TUI mode
znicz track.flac
znicz album/*.flac

# List audio devices
znicz --list-devices

# Select device
znicz --device "device-name" track.flac

# Print the app version
znicz --version

# MCP server (stdio; autostarts znicz player)
znicz mcp

# Shared player process (usually autostarted)
znicz player
znicz player stop
```

### Music library

```bash
# Index a folder (subfolders included). Rescans skip unchanged files.
# Runs through the player process, which is started if needed.
znicz scan ~/Music

# Index, then drop entries whose files were deleted
znicz scan ~/Music --prune

# Search by title, artist or album
znicz search portishead

# List albums
znicz albums

# Playlists (M3U files in ~/.local/share/znicz/playlists/)
znicz playlist list
znicz playlist play evening
znicz playlist rename evening night
znicz playlist copy night night-backup
znicz playlist remove night-backup
znicz playlist import ~/sets/club.m3u
znicz playlist import ~/sets/club.m3u --append

# Radio stations (stations.toml beside the library, or ZNICZ_STATIONS_PATH)
znicz station list
znicz station add "Example" https://example.com/stream
znicz station play Example
znicz station play Example --append
znicz station copy Example Example2
znicz station art "Example" ~/Pictures/example.png
znicz station art "Example" --clear
```

On first run with an empty library the TUI asks for a folder (pre-filled with
your music folder) and scans it. `F` adds another folder and `U` rescans every
remembered folder. `znicz scan` and the MCP `scan_library` tool go through the
same `znicz player` process, which is the only writer of the index and
remembers each folder you scan. Missing folders are skipped on a rescan and
their tracks are kept.

The index is a SQLite file, by default `~/.local/share/znicz/library.db`
(`%APPDATA%\znicz\library.db` on Windows).

## The interface

The library is the home screen. The queue is a side overlay (`]`), devices
are a modal (`,`), `S` opens Settings, `P` opens saved playlists, and `R` opens radio stations.
Those two overlays share a keymap: `n` new, `e` edit, `c` copy, `d` delete.
While they are open, `n` and `e` are not next/repeat; close with Esc to get
the global keys back.
`v` (or `i`) switches between two views. **Browse** is the library with the queue drawer and a
text transport; **Stage** is full-screen now playing: cover, title, artist, album, seek bar, volume,
repeat and shuffle, and a Signal panel with the full file → device path, including the device sample
format and whether the path is bit perfect. The TUI always starts on Browse. In Stage, `Esc` closes
an open modal or the queue first, then returns to Browse; library keys do nothing there.

Browse transport has no cover: three lines (title + seek, signal, message) on windows at least 20
rows tall, two below that. Example signal path:
`FLAC 96 kHz 24-bit 2882 kbps stereo → 96 kHz stereo  ● bit perfect`.
When the device will not take the file's own rate the badge reads `▲ resampled`,
so a silent conversion never goes unnoticed. Messages appear on the transport's last row rather than over the list.
On Stage the cover sits left of the details on a wide terminal and above them on a narrow one; streams
use the ICY image when one decodes, else an AudioAddict song cover on those networks, else an artist / title
search (Deezer, then iTunes, then MusicBrainz), else station art, else the Znicz logo. The cover area takes your terminal's background colour when the terminal reports it. Long now-playing titles scroll (set `marquee = false` to truncate with `…`).
Long list titles pan with `Alt-←` / `Alt-→` on the highlighted row.
Under an artist, album rows read `Kind of Blue (1959)` and track rows `1. So What` (`2-3.` on
multi-disc albums); the artist appears only when a track's artist differs (compilations, guests).
An artist's albums run oldest first, albums without a year last. Search results keep `— Artist`.
The volume bar starts with a marker (`◢`, `×` when muted); `icons = "emoji"` or `"nerd"` swaps in
🔈 🔉 🔊 / 🔇 or Nerd Font glyphs (`"nerd"` needs a [Nerd Font](#nerd-font-icons) in the terminal).

Press `?` in the player for the keymap: the keys for where you are come first,
and `/` filters by key or action. The overlay footer shows the running version.
The essentials:

| Key | Action |
|-----|--------|
| Space | Play / pause |
| s | Stop |
| n | Next track |
| p | Previous track |
| P | Playlists |
| R | Radio |
| → ← | Seek ±5s (Shift-→ / Shift-← for ±30s) |
| + / - | Volume in 1 % steps; `m` mutes (fades out and in over half a second) |
| e / z | Repeat (off, all, one) / shuffle |
| r | Reload the list in front |
| j k, g G, Ctrl-d Ctrl-u | Move in the list |
| h l | Left / right: column, level, back / into (never plays) |
| Enter | Open / expand browse, or play a track |
| a / A | Add the selection / everything listed to the queue |
| d / C | Remove from the queue / clear it |
| / | Search the library |
| Space / o | Expand or collapse a tree node |
| ] | Open / close the queue |
| F | Add a music folder to the library (scan it) |
| U | Rescan the remembered folders |
| M | Messages: recent history; unread warnings and errors are counted |
| J | Jump the library to the playing track |
| Tab | Next column when three-column browse is active; library ↔ queue while the drawer is open (only `]` opens it) |
| Shift-Tab | Previous column (or queue focus when the drawer is open) |
| Alt-↑ / Alt-↓ | Move the highlighted queue row up / down (queue drawer focused) |
| Alt-← / Alt-→ | Pan a long title |
| v / i | Stage: full-screen now playing (cover, details, signal path); again to return to Browse |
| , | Devices |
| S | Settings: display, audio, music folders, update check (saved to `config.toml`) |
| ? / q | Help / quit TUI (`s` stops playback) |

Some terminals do not send Alt-arrows (for example macOS Terminal until you
enable "Use Option as Meta key"); `Alt-↑` / `Alt-↓` and the `Alt-←` / `Alt-→` pan need that.

To see the layout without starting the player:

```bash
cargo run -p znicz-tui --example preview -- 120 40
```

While the TUI is running, stderr is redirected to `~/.cache/znicz/znicz-session.log`
so that ALSA's warnings cannot draw over the interface. Look there if the player
misbehaves. That log is not `session.toml` (the saved queue).

## Configuration

Znicz writes this file with the defaults below on first start, unless a file is already there.
Edit it by hand or in the TUI (`S`, Settings). Settings writes the same file in place and keeps your comments.

| Platform | Path |
|----------|------|
| Linux | `~/.config/znicz/config.toml` (or `$XDG_CONFIG_HOME/znicz/config.toml`) |
| macOS | `~/Library/Application Support/znicz/config.toml` |
| Windows | `%APPDATA%\znicz\config.toml` |

Default file (`znicz/src/default-config.toml`):

```toml
# Znicz settings. Edit here or in the TUI (S).
# Reference: https://github.com/eugene-chekan/znicz#configuration

[audio]
# Output device id; leave unset for the system default (see `znicz devices`).
# device = "default"
volume = 1.0
# Open the device at each file's own sample rate and channels.
bit_perfect = true

[player]
# Seconds the player stays up while stopped with no UI; 0 never exits.
idle_secs = 900

[library]
# Where the library database lives; unset uses the user data directory.
# path = "~/.local/share/znicz/library.db"

[tui]
show_cover = true          # album art on the Stage
cover_size = 100           # Stage art size, % of the largest fit: 100 | 75 | 50 | 25
cover_protocol = "auto"    # auto | kitty | sixel | halfblocks | off
library_layout = "columns" # columns | tree
marquee = true             # scroll long now-playing titles; false cuts with …
icons = "unicode"          # unicode | emoji | nerd (needs a Nerd Font: see README "Nerd Font icons")

[covers]
search = true              # look radio covers up by artist / title (Deezer, iTunes, MusicBrainz)

[updates]
check = true               # ask GitHub for a new release once a day

[mcp]
skills_dirs = []
```

- `znicz config path` prints the file Znicz uses (`--config FILE` on the command line overrides it).
- `znicz config init` writes the defaults to that file. It refuses to overwrite an existing file;
  `znicz config init --force` copies the old file to `config.toml.bak` first.
- A file that does not parse is not used: Znicz runs on the defaults and shows a toast with the line
  and column, for example `config.toml has an error (line 2, column 5): expected a value. Using the defaults.`
  While the file is broken, Settings applies changes for the run but does not save them.

A station in `stations.toml` can opt out with `cover_search = false`.
With search on, the playing stream's ICY title is sent to Deezer, then to Apple
(iTunes) and MusicBrainz when the earlier sources find nothing. Set
`search = false` to keep titles local.
See [Stream covers](wiki/Architecture/Cover-Sources.md).

**Colours.** The interface uses your terminal's 16 ANSI colours and is tuned
for dark palettes. On light palettes, and on some dark ones such as Solarized
Dark, body text or dim hints can be hard to read. Selectable themes are planned
([#68](https://github.com/eugene-chekan/znicz/issues/68)).

The last queue (and volume, mute, repeat, shuffle) is
`~/.local/share/znicz/session.toml`. Override with `ZNICZ_SESSION_PATH`.
The player process writes it after those fields settle, and on exit.
Opening the player restores it **Stopped**; it does not auto-play.
`znicz file.flac` / playlist or station play without `--append` does not load
the old queue. This is not the stderr log above.

## Nerd Font icons

`icons = "nerd"` (in `[tui]`, or the Icons row in Settings) draws the volume marker with
Nerd Font glyphs. A Nerd Font is a font patched with extra icon glyphs. The terminal has to
use one, or the glyphs show as boxes or `?`.

1. **Pick a font.** JetBrainsMono Nerd Font is one option. Download it from
   [nerdfonts.com/font-downloads](https://www.nerdfonts.com/font-downloads), or from the
   [`ryanoasis/nerd-fonts` releases](https://github.com/ryanoasis/nerd-fonts/releases)
   (`JetBrainsMono.zip` or `JetBrainsMono.tar.xz`).
2. **Install it.**
   - **Arch Linux:** `sudo pacman -S ttf-jetbrains-mono-nerd` (package in the Extra repository).
   - **Other Linux:** unzip into `~/.local/share/fonts/`, then run `fc-cache -f`.
   - **macOS:** `brew install --cask font-jetbrains-mono-nerd-font`.
   - **Windows:** `winget install --id DEVCOM.JetBrainsMonoNerdFont --exact`, or unzip the
     archive, select the `.ttf` files, right-click, and choose Install.
3. **Set it as the terminal's font.** Kitty: `font_family JetBrainsMono Nerd Font` in `kitty.conf`.
   WezTerm: `config.font = wezterm.font 'JetBrainsMono Nerd Font'`. Alacritty: under `[font.normal]`,
   `family = "JetBrainsMono Nerd Font"`. Other terminals: choose the font in the profile or
   preference settings. Restart the terminal afterwards.
4. **Check it.** In bash or zsh, `printf '\U000F057E\n'`. In PowerShell,
   `[char]::ConvertFromUtf32(0xF057E)`. U+F057E is the volume-high glyph (󰕾). A speaker means
   the font works; boxes or `?` mean it does not.
5. **Turn it on.** Open Settings (`S`), set Icons to `nerd`, or write `icons = "nerd"` under `[tui]`.

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
TUI and MCP share one `znicz player` process (autostarted if needed).

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
| `znicz-library` | SQLite music index: scan, search, albums, library service |
| `znicz-tui` | Ratatui frontend |
| `znicz-mcp` | MCP server + skills |
| `znicz` | CLI binary |

## License

MIT OR Apache-2.0, at your option: [LICENSE-MIT](LICENSE-MIT),
[LICENSE-APACHE](LICENSE-APACHE). Release builds also carry
[THIRD-PARTY-LICENSES.html](THIRD-PARTY-LICENSES.html), the licences of the Rust
crates compiled into the binary.
