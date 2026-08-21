# Znicz wiki

Welcome. This wiki explains **what Znicz is**, **how digital audio works**, and **the Rust ideas** used in this project.

Read in this order if you are new:

1. [What is Znicz?](Home.md) (this page)
2. [Digital audio in plain words](Domain/Digital-Audio.md)
3. [How playback works](Domain/Playback-Pipeline.md)
4. [Project map](Architecture/Overview.md)
5. [Rust: ownership](Rust/Ownership.md)

## Project

- [Roadmap](Plans/Roadmap.md) — phases 1–6
- [Issues](Issues.md) — open bugs and loose ends

## What is Znicz?

Znicz is a **music player that lives in the terminal**. You start it from the command line. It shows a text screen (a TUI) and plays files on your speakers or DAC.

It also has an **MCP server**. MCP lets an AI assistant (Cursor, Claude, and others) control the player with tools like “play this file” or “what is playing now”.

Goals:

- Sound quality first (keep the original sample rate when we can)
- Work on **Linux** and **Windows**
- Keep UI, audio engine, and AI tools in **separate crates**

## Folders

| Folder | What it is |
|--------|------------|
| `znicz-core` | The brain: decode files, send samples to the sound card, keep queue state |
| `znicz-tui` | The screen: keys, now playing, progress bar |
| `znicz-mcp` | The AI interface: tools, resources, prompts, skills |
| `znicz` | The program you run (`znicz`, `znicz mcp`) |
| `wiki/` | This documentation |

## Wiki index

### Domain (music and audio)

- [Digital audio](Domain/Digital-Audio.md) — sample rate, bit depth, PCM
- [Audiophile ideas](Domain/Audiophile-Basics.md) — bit-perfect, DACs, why resampling matters
- [Playback pipeline](Domain/Playback-Pipeline.md) — file → decode → ring → speakers
- [Formats and tags](Domain/Formats-and-Metadata.md) — FLAC, WAV, playlists, radio
- [TUI players](Domain/TUI-Players.md) — why a terminal UI

### Architecture (this codebase)

- [Overview](Architecture/Overview.md)
- [Audio engine](Architecture/Core-Engine.md)
- [Threads and the realtime rule](Architecture/Audio-Threading.md)
- [TUI](Architecture/TUI.md)
- [MCP server](Architecture/MCP.md)

### Plans

- [Roadmap](Plans/Roadmap.md)
- [Phase 5 — Album art in the TUI](Plans/Phase-5-Album-Art.md)

### Rust (language theory used here)

- [Ownership and borrowing](Rust/Ownership.md)
- [Threads, channels, atomics](Rust/Threads-and-Channels.md)
- [Traits](Rust/Traits.md)
- [Errors](Rust/Error-Handling.md)
- [Cargo workspaces](Rust/Cargo-Workspace.md)
- [Realtime audio in Rust](Rust/Real-Time-Audio.md)

## Run it

```bash
# Linux build tools
sudo apt install pkg-config libasound2-dev

cargo build --release
./target/release/znicz --list-devices
./target/release/znicz your-track.flac
./target/release/znicz mcp
```

See the root [README.md](../README.md) for keys and config.

## Extra reading (outside this repo)

- [The Rust Book](https://doc.rust-lang.org/book/)
- [Rust by Example](https://doc.rust-lang.org/rust-by-example/)
- [cpal](https://github.com/RustAudio/cpal) — talking to sound devices
- [Symphonia](https://github.com/pdeljanov/symphonia) — decoding audio files in Rust
- [Ratatui](https://ratatui.rs/) — terminal UIs
- [Model Context Protocol](https://modelcontextprotocol.io/)
- [xiph.org: Digital Audio](https://wiki.xiph.org/index.php/Digital_Audio) — deep but clear
