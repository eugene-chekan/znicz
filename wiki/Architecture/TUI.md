# TUI (`znicz-tui`)

The UI is an **immediate-mode** loop:

1. Draw the whole screen from `PlayerState`
2. Wait a short time for a key (or a tick)
3. Repeat until `q`

Ratatui does not keep widgets as a tree you mutate. You describe layout **every frame**. That is a good fit for a player: position and peak meters change all the time.

## Layout

- **Now playing** — title, format line, progress (Phase 5 adds **album cover** on the left — see [Phase 5 plan](../Plans/Phase-5-Album-Art.md))
- **Queue** — file names, current row marked
- **Status** — playing/paused, volume, device, key hints
- **Help overlay** — `?`

## Wiring

`App` holds a `PlayerHandle`. Keys call `player.send(Command::…)`. Events from the engine are drained so errors can be logged.

The binary restores the terminal with `ratatui::restore()` even if drawing failed.

## Extra reading

- [Ratatui concepts](https://ratatui.rs/concepts/)
- [crossterm events](https://docs.rs/crossterm/latest/crossterm/event/index.html)
