# TUI (`znicz-tui`)

Znicz is a terminal player first, so this crate is where most of the day-to-day
experience lives. The interface is an **immediate-mode** loop:

1. Drain player events (turning failures into on-screen messages)
2. Draw the whole screen from a fresh `PlayerState`
3. Wait for a key, or for the tick that advances the seek bar
4. Repeat until `q`

Ratatui does not keep widgets as a tree you mutate. You describe the layout
**every frame**. That suits a player, where the position moves constantly.

## Layout

```
 1 Queue │ 2 Library │ 3 Devices          ← tab bar
┌ Now Playing ──────────────────────────┐
│ In My Time of Dying                   │  title
│ Led Zeppelin — Physical Graffiti      │  artist — album (from tags)
│ ━━━━━━━━───────────────  3:32 / 11:04 │  seek bar
│ FLAC 96 kHz 24-bit 2882 kbps stereo → │  signal path
│ 96 kHz stereo  ● bit perfect          │
└──────────────────────── track 2/4 ────┘
┌ Queue ────────────────────────────────┐
│  1   Led Zeppelin — Kashmir      8:28 │  the focused pane
│  2 ▶ Led Zeppelin — In My Time  11:04 │
└───────────────── 4 tracks · 33:43 ────┘
▶ playing  ██████▁▁ 70%  repeat all  …    ← status
Enter play · d remove · C clear · ? help  ← hints, or a message
```

Only one list pane is shown at a time, chosen with `Tab` or `1`/`2`/`3`. Giving
each pane the full width matters in a terminal: track titles are long, and split
columns would truncate everything.

### Responsive behaviour

The window is often small, so parts are dropped in order of importance:

| Height | What is shown |
| --- | --- |
| 20 rows or more | everything |
| 12–19 rows | the signal-path line is dropped |
| under 12 rows | the tab bar goes too |

Every row is also truncated to the window width with `…`, counting **characters
rather than bytes** so accented titles are never cut mid-character.

## The signal path

The line under the seek bar is the audiophile part of the interface. It reads
`file format → device stream`, then a badge. Example:

`FLAC 96 kHz 24-bit 2882 kbps stereo → 96 kHz stereo  ● bit perfect`

Left of the arrow is the file (codec name, sample rate, bit depth when the
codec has one, bitrate when known, channels). Right of the arrow is the stream
the device actually opened (rate and channels). The device sample format (`f32`
and similar) is kept in `PlayerState::output` for a later details view, not on
this line.

- `● bit perfect` — the device accepted the file's own sample rate and channel
  count, so nothing was converted
- `▲ resampled` — the device refused that rate, so [`RateConverter`](Core-Engine.md)
  is in the path

This is information the user cannot otherwise get. The device silently choosing
44.1 kHz for a 96 kHz file sounds fine but is no longer the original signal, so
the interface says so instead of hiding it. `PlayerState::output` carries the
details, filled in when the stream opens. See
[Audiophile basics](../Domain/Audiophile-Basics.md).

## Panes

### Queue

Shows **titles, not file names**. The player's queue is only a list of paths, so
`meta::MetaCache` resolves tags on a background thread: the UI asks for a path,
draws whatever is known now, and the worker fills the gap for the next frame.
Reading tags means opening and seeking each file, so doing it while drawing would
stutter on a long queue. Rows added from the library skip the worker entirely,
since the database already has the tags.

`Enter` plays a row, `d` removes one, `C` clears, `o` jumps back to whatever is
playing.

### Library

Albums by default; `Enter` opens one, `Esc` goes back, `/` searches, `a` queues
the selection (a whole album if the cursor is on one) and `A` queues everything
listed. Queries go straight to SQLite, which is fast enough to run while handling
the keypress. See [Library](Library.md).

A library whose files carry **no album tags** cannot be grouped, so the pane
falls back to a flat track list rather than looking empty.

### Devices

Lists the output devices and switches between them with `Enter`. The footer shows
what the open stream actually settled on, which is the quickest way to find a
device that will take your files unconverted.

## Messages instead of silence

The interface owns the screen, so anything written to the log or to stdout is
invisible. Previously a failed file looked exactly like nothing happening. Now
every player error and every action becomes a short-lived message on the bottom
line (`toast.rs`); errors stay up twice as long, since they need reading.

For this to work, key handlers use `PlayerHandle::send_blocking` rather than
`send`: the engine's own result comes back, so a missing file or an unusable
device is reported rather than dropped. It also means the next frame reads state
that already includes the change, instead of drawing the old volume.

The traffic goes the other way too. ALSA and other C libraries write warnings
**straight to file descriptor 2**, ignoring Rust's logging entirely, and in a
full-screen interface that means text appearing on top of the layout — during
device enumeration, and again whenever a stream is opened. So the binary points
stderr at `~/.cache/znicz/znicz-session.log` for as long as the TUI owns the
terminal, and restores it on exit. Nothing is lost; it just stops being drawn on
top of the player.

## Modules

| File | Role |
| --- | --- |
| `app.rs` | state, event loop, key dispatch |
| `views/` | drawing, one module per pane |
| `theme.rs` | every colour, in one place |
| `keys.rs` | the keymap as data |
| `cursor.rs` | list cursor movement |
| `meta.rs` | background tag cache |
| `toast.rs` | short-lived messages |
| `format.rs` | durations, rates, bars, truncation |
| `library_pane.rs` | browsing state |

`keys.rs` holds the bindings as data, and both the help overlay and the footer
hints are generated from it. The old help text had drifted from the code and
documented keys that did nothing; generating it removes that possibility.

Colours are the terminal's named 16 rather than RGB, so Znicz follows whatever
palette the user has chosen.

## Testing a UI

Two failure modes matter, and neither is caught by the compiler: a panic from a
width calculation that went negative, and a screen that draws nothing useful.
Both are testable without a terminal.

- `tests/render.rs` draws every pane at eight window sizes, down to 10×3, and
  reads the result back as text
- `tests/keys.rs` presses keys through `App::on_key` and checks the player state
- `tests/library_browse.rs` browses a real scanned library
- `cargo run -p znicz-tui --example preview` prints the interface to stdout, so
  layout can be checked without a sound card or a library

```bash
cargo run -p znicz-tui --example preview -- 120 40   # width, height
```

## Extra reading

- [Ratatui concepts](https://ratatui.rs/concepts/)
- [Ratatui testing with TestBackend](https://ratatui.rs/recipes/testing/snapshots/)
- [crossterm events](https://docs.rs/crossterm/latest/crossterm/event/index.html)
