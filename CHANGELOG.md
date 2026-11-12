# Changelog

## 0.16.0 - 2026-11-13

- Settings (`S`): display and audio options, the default output device, music
  folders (add, rescan, remove) and the update check. Each change is saved to
  `config.toml` in place, so your comments stay.
- Cover size on the Stage: 100, 75, 50 or 25 percent (`[tui] cover_size`).
- `config.toml` is written with the defaults on first start. `znicz config path`
  prints where it is; `znicz config init [--force]` writes the defaults again
  (the old file is kept as `config.toml.bak`).
- A `config.toml` that does not parse is reported with its line and column, and
  Znicz runs on the defaults.
- Removing a music folder (`d` in Settings) forgets it and deletes its tracks
  from the library. The files stay.
- README: a Nerd Font icons guide for `icons = "nerd"`.

## 0.15.0 - 2026-11-12

- The TUI no longer receives the whole queue from the player on every frame;
  the player sends it only when it changed. Long queues stay responsive.
- The queue and library lists draw only the rows on screen.
- Moving through artists no longer rereads the whole library; album lookups
  use case-insensitive indexes (older databases are migrated on open).

## 0.14.2 - 2026-11-12

- The footer keeps saying a new release is available (`? more · 0.14.2 available`) after the toast fades.
- Updating from a PowerShell 7 terminal on Windows works: the installer no
  longer inherits PowerShell 7's module path, which made Windows PowerShell
  fail with "Microsoft.PowerShell.Security could not be loaded".

## 0.14.1 - 2026-11-12

- `u` in Help checks for updates right away when none is known.
- Release notes in the Update window no longer start with a "Release Notes" heading.

## 0.14.0 - 2026-11-11

- The TUI checks GitHub for a new release once a day and says so in a toast.
  Help shows `znicz 0.13.4 → 0.14.0`; `u` in Help opens the release notes and
  installs the update, then offers to restart Znicz (and the player, resuming
  where it was). `[updates] check = false` turns the check off
  ([#83](https://github.com/eugene-chekan/znicz/issues/83)).
- `znicz update` installs the latest release; `znicz update --check` only
  reports it.
- The player writes its version into `ipc.toml`; a TUI newer than the player
  says the player updates when it next restarts.

## 0.13.4 - 2026-11-11

- Closing a drawer or modal on Stage no longer blinks: the repaint happens in
  one synchronized update and the cover is not re-encoded (except for Kitty).
- An autostarted player no longer lingers as a zombie under `znicz mcp`.
- Tests clean up their temporary files
  ([#79](https://github.com/eugene-chekan/znicz/issues/79)).
- Edition 2024.

## 0.13.3 - 2026-11-11

- AudioAddict covers match titles whose apostrophe the stream sends as a
  backtick (`` 90`s Love ``).

## 0.13.2 - 2026-11-11

- RadioTunes / DI.FM covers follow the song instead of showing the previous
  one.

## 0.13.1 - 2026-11-11

- Release builds and installers for Linux (x86-64, ARM64) and Windows.
