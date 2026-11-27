# Changelog

## 0.20.0 - 2026-11-27

- Spotify control through the Web API. You bring your own Spotify developer
  app and its client ID (Development Mode is enough). Playback control needs
  Spotify Premium. Browsing and search work without it.
- `znicz spotify login`, `logout` and `status`. Login over SSH works by pasting
  the address you were sent to, or by forwarding the port with `ssh -L`.
- Settings → Spotify (tab 6) sets the client ID, the redirect port, when a
  browser opens and the playback device, and has a login panel.
- Spotify mode on `O`: browse playlists, liked songs and albums, search, play
  in context, use the transport keys, queue tracks, and pick a Spotify Connect
  device with `,`. `O` again returns.
- The Stage and the transport show what Spotify plays, with its cover and the
  signal rows (source, device, Spotify Connect path).
- Znicz and Spotify pause each other. Starting Spotify pauses Znicz, and
  starting a local track in Znicz pauses Spotify.
- MCP: `spotify_*` tools, a `znicz://spotify/now-playing` resource and a bundled
  skill.
- Spotify's 2026 Development Mode limits apply. Tracks of a playlist are listed
  only for playlists you own or collaborate on, artist top tracks are not
  available, and search returns at most 10 results per type.
- Settings keeps the comments in `config.toml` when it saves.

## 0.19.0 - 2026-11-22

- Crossfade: when one track follows another on its own, the two overlap for
  the set time with an equal-power curve (`[audio.fade] crossfade_ms`,
  Settings → Playback; off by default, up to 12 s). Repeat-one crossfades
  into itself. Radio, a change of sample rate or channels, a current track of
  unknown length, or a next track shorter than twice the crossfade join as
  before. Manual changes keep their fades.
- Gapless playback also loops a repeat-one track without reopening the stream.
- Settings → Playback rows are sliders: `h` / `l` step (50 ms for fades,
  0.5 s for crossfade). Enter or Space types an exact value (`150`, `0.2s`,
  `3.5`).

## 0.18.0 - 2026-11-18

- Fades: play and resume fade in; pause, stop, Next/Previous, choosing a new
  track, removing the playing row and switching the output device fade out
  first. Times are set in `[audio.fade]` (milliseconds, 0 = off) and in
  Settings → Playback, where a change applies at once. Defaults: in 150 ms,
  pause 150 ms, stop 300 ms, track change 150 ms, mute 500 ms. Automatic track
  changes and gapless joins do not fade. Steady playback stays bit-perfect.
- Settings has tabs: Display, Audio, Playback, Library, General. `Tab` /
  `Shift-Tab` or `1`–`5` switch tabs; `↑` on the first row moves to the tab
  bar, where `←` / `→` switch.

## 0.17.1 - 2026-11-15

- Radio covers are found for titles with a bracketed note after the song name,
  such as a composer (`Lover Man (Ramirez)`), `(Live)` or `[2011 Remaster]`:
  the artist / title search tries again without it.

## 0.17.0 - 2026-11-14

- Gapless playback: consecutive local tracks with the same sample rate and
  channel count play into the same output stream, with no silence or click
  between them. The title, cover and seek bar switch when the next track is
  heard. A format change, radio and repeat-one keep the old stop-and-reopen.
- AAC (M4A) files are trimmed of encoder delay and padding (`iTunSMPB`), as
  MP3 and Ogg Vorbis already were.

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
- The footer always shows the running version (`? more · v0.16.0`), and
  `v0.16.0 → 0.17.0 available` while an update is known. Help no longer shows
  the version.
- On the Stage, cover changes made while Settings (or another modal) is open
  show when it closes, so the cover never draws over the modal. Cover protocol
  offers only what the terminal supports.
- The seek bar moves in half-cell steps (`━━━╾───`), twice as smoothly.
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
