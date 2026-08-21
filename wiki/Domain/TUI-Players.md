# TUI music players

**TUI** = Terminal User Interface. The app draws boxes and text in a terminal, not a window with pixels.

Why people like this for music:

- Keyboard first (`space` to pause, `n` / `p` for tracks)
- Works over SSH
- Low distraction
- Easy to script and pair with tools like MCP

Znicz uses [Ratatui](https://ratatui.rs/) to draw, and [crossterm](https://github.com/crossterm-rs/crossterm) to read keys.

The TUI **does not decode audio**. It only:

1. Sends **commands** (`Play`, `Pause`, `Seek`) to `znicz-core`
2. Reads **state** (title, position, volume) and paints it

That split means we can run the same engine from MCP with no screen.

## Album art (Phase 5)

The now-playing screen will gain a **cover panel** when Phase 5 ships: embedded JPEG/PNG from tags, rendered with Kitty/Sixel-style protocols where the terminal supports them, and a half-block fallback elsewhere. No Kitty install is required — Znicz emits the graphics protocol from Rust. See [Phase 5 plan](../Plans/Phase-5-Album-Art.md).

## Keys (Phase 1)

| Key | Action |
|-----|--------|
| Space | Play / pause |
| n | Next |
| p | Previous |
| ← / → | Seek 5 seconds |
| + / − | Volume |
| ? | Help |
| q | Quit |

## Extra reading

- [Ratatui book](https://ratatui.rs/concepts/why-ratatui/)
- [Immediate-mode UI](https://github.com/ocornut/imgui/wiki#about-the-imgui-paradigm) — Ratatui is “draw everything each frame”
