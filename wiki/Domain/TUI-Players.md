# TUI music players

**TUI** = Terminal User Interface. The app draws boxes and text in a terminal, not a window with pixels.

Why people like this for music:

- Keyboard first (`space` to pause, `n` / `p` for tracks)
- Works over SSH
- Low distraction
- Easy to script and pair with tools like MCP

Znicz uses [Ratatui](https://ratatui.rs/) to draw, and [crossterm](https://github.com/crossterm-rs/crossterm) to read keys.

The TUI **does not decode audio**. It only:

1. Sends **commands** (`Play`, `Pause`, `Seek`) to `znicz player`
2. Reads **state** (title, position, volume) and paints it

That split means MCP can drive the same engine with no screen. It also
means the interface never has its own copy of the truth to get out of date: each
frame reads the state fresh.

## Album art

The now-playing transport shows an **embedded cover** when the file has one:
Kitty / Sixel / half-blocks via `ratatui-image`, with a bundled logo when the
picture is missing (streams, no tags, read error). No Kitty install and no
`icat`. Config: `[tui]` in `config.toml`. See
[Phase 5](../Plans/Phase-5-Album-Art.md) and
[TUI architecture](../Architecture/TUI.md).

## What a terminal player has to get right

A terminal gives you a grid of characters and nothing else, which makes a few
problems specific to this kind of program:

- **Nowhere to print.** The app owns the screen, so a `println!` or a log line
  either corrupts the display or is never seen. Anything the user needs to know
  has to be drawn as part of the interface. Znicz turns player errors into
  boxed toasts in the list corner, not onto the hint line.
- **Unknown window size.** A terminal can be 200×60 or 40×10, and it changes
  while running. Layouts need an order in which parts are dropped.
- **Characters, not pixels.** Text has to be truncated to fit, and by character
  rather than byte, or a title like `Łódź nocą` breaks apart.
- **No pointer.** Every action needs a key, and the keys have to be discoverable,
  which is what the help overlay is for.

Conventions worth borrowing from other players: `Space` for pause, `/` to search,
`j`/`k` to move (from vim and `less`), `]` for the queue drawer, `i` for the
signal path, `P` for playlists, `?` for help. Mouse is [not wired yet](https://github.com/eugene-chekan/znicz/issues/8).

## Keys

See the [README table](../../README.md#the-interface) or press `?` in the player.
Both come from the same keymap in the code, so neither can go stale.

## Extra reading

- [Ratatui book](https://ratatui.rs/concepts/why-ratatui/)
- [Immediate-mode UI](https://github.com/ocornut/imgui/wiki#about-the-imgui-paradigm) — Ratatui is “draw everything each frame”
