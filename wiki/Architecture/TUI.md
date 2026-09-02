# TUI (`znicz-tui`)

Znicz is a terminal player first, so this crate is where most of the day-to-day
experience lives. The interface is an **immediate-mode** loop:

1. Drain player events (turning failures into on-screen messages)
2. Draw the whole screen from a fresh `PlayerState`
3. Wait for a key, or for the tick that advances the seek bar
4. Repeat until `q`

`q` closes the TUI only. Playback keeps going in `znicz player` until you
`s` (stop), `znicz player stop`, or the player is already Stopped and idle
with no UI connected.

The **player process** writes `session.toml` shortly after queue or transport
extras change, and again when it exits (idle or `znicz player stop`). Bare
start restores that session **Stopped**.
See [Formats and metadata](../Domain/Formats-and-Metadata.md#session).

The TUI is a **UI client**. It does not host the engine or write `ipc.toml`.
`znicz` autostarts `znicz player` and connects with Hello `role=ui`. See
[MCP](MCP.md).

Ratatui does not keep widgets as a tree you mutate. You describe the layout
**every frame**. That suits a player, where the position moves constantly.

## Layout

```
┌ Library / Dummy ─────────────────────────────────────────┐
│  1  Mysterons — Portishead                          5:02 │
│  2  Sour Times — Portishead                         4:11 │
│  3  Wandering Star — Portishead  ┌ Queue ────────────────┤
│  4  It Could Be Sweet — Portis.  │  1 ▶ Sour Times  4:11 │
│  5  Numb — Portishead            │  2   Strangers   3:58 │
│                                  ┌───────────────────────┤
│                                  │ × device refused 96 kHz
│                                  └───────────────────────┘
└ 12 tracks · Alt-← → pan ─────────┴ 3 tracks · ] close ───┘
▶ Sour Times  Portishead — Dummy  ━━━━━── 1:02/4:11  70%
  FLAC 96 kHz 24-bit 2882 kbps stereo → 96 kHz stereo  ● bit perfect
  Space pause  a add  ] queue  i inspect  Alt-← → pan  , devices  ? help
```

The library is the stage and always fills the list region. The queue is an
overlay drawer on the right (`]` toggles it). Long titles pan horizontally with
`Alt-←` and `Alt-→` on the highlighted row only. `<` and `>` are unbound. Transport is two lines at the bottom; hints stay on their own line
and are never replaced by a toast.

### Responsive behaviour

The window is often small, so parts are dropped in order of importance:

| Height | What is shown |
| --- | --- |
| 20 rows or more | everything, including both transport lines |
| 12–19 rows | the signal-path line (transport line 2) is dropped |
| under 12 rows | library + one transport line + hints |

Every row is also truncated to the window width with `…`, counting **characters
rather than bytes** so accented titles are never cut mid-character.

## The signal path

The second transport line is the audiophile part of the interface. It reads
`file format → device stream`, then a badge. Example:

`FLAC 96 kHz 24-bit 2882 kbps stereo → 96 kHz stereo  ● bit perfect`

Left of the arrow is the file (codec name, sample rate, bit depth when the
codec has one, bitrate when known, channels). Right of the arrow is the stream
the device actually opened (rate and channels). The device sample format (`f32`
and similar) is kept off this line; press `i` for the [signal inspector](#signal-inspector).

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

### Library

The home screen. Albums by default; `Enter` opens one, `Esc` goes back, `/`
opens a search prompt, `a` queues the selection (a whole album if the cursor is
on one) and `A` queues everything listed. While the prompt is open, Left and
Right move the caret the same way they do when naming a playlist or a station.
Queries go straight to SQLite, which is fast enough to run while handling the
keypress. See [Library](Library.md).

A library whose files carry **no album tags** cannot be grouped, so the pane
falls back to a flat track list rather than looking empty.

### Queue

An overlay drawer (`]`). Shows **titles, not file names**. A queue row is a
`QueueItem`: a local file or a radio station. For files, `meta::MetaCache`
resolves tags on a background thread: the UI asks for a path, draws whatever is
known now, and the worker fills the gap for the next frame. Reading tags means
opening and seeking each file, so doing it while drawing would stutter on a
long queue. Rows added from the library skip the worker entirely, since the
database already has the tags. A station row shows the **station name**; it has
no duration (`—`).

`Enter` plays a row, `d` removes one, `C` clears, `o` jumps back to whatever is
playing. `d` on the playing row starts the next remaining one, or stops if
that was the last row.

### Devices

A centered modal (`,`). Lists the output devices and switches between them with
`Enter`. The footer shows what the open stream actually settled on, which is the
quickest way to find a device that will take your files unconverted.

### Signal inspector

A centered modal (`i`). The transport line is a summary; this overlay is the
full path: file format, title, device name, the open stream **including sample
format** (`f32` and similar), and whether playback is bit perfect or resampled.
Nothing is invented when a field is missing — an unknown sample format shows as
`--`. Esc or `i` closes it. Transport keys still work underneath.

### Playlists

A centered modal (`P`, shift-p — not `p`, which is previous track). Lists
`.m3u` / `.m3u8` files in the playlists folder. A playlist file may list
local paths and http(s) URLs; save writes both. The overlay uses the same
saved-list keys as Radio. While it is open (and you are not typing a prompt),
those keys win over the global map: `n` is new, not next track; `e` is edit,
not repeat. Space, `s`, seek, and volume stay global. After Esc, `n` and `e`
are next and repeat again.

| Key | Action |
| --- | --- |
| Enter | Clear the queue and play |
| `a` | Add the highlighted file to the queue |
| `n` | New: name a file and write the current queue (`save: █`) |
| `e` | Edit: rename the highlighted file (`rename: █`, pre-filled) |
| `c` | Copy the file to a new name (`copy: █`, pre-filled) |
| `d` | Delete the highlighted file (immediate, no confirm) |
| Esc | Close the overlay, or cancel a prompt |

`s` still stops while the list is showing; while naming a file, every letter
(including `s`, which is stop everywhere else) is part of the name. Left and
Right move the caret; Home and End jump to the ends. The footer switches to
type/cancel so the global keymap is not what you type into.

### Radio

A centered modal (`R`). Lists stations from `stations.toml`. Same saved-list
keys as Playlists: overlay keys win until Esc. If the URL cannot be opened,
playback stops so the previous file does not keep playing.

| Key | Action |
| --- | --- |
| Enter | Clear the queue and play |
| `a` | Add the highlighted station to the queue (does not start or stop playback) |
| `n` | New station: empty two-field form (name and URL) |
| `e` | Edit: the same form, filled from the highlighted station |
| `c` | Copy name+URL, then prompt for a new name |
| `d` | Delete (immediate, no confirm) |
| `r` | Reload `stations.toml` |
| Esc | Close the overlay, or cancel a prompt |

Tab (or Down) moves between name and URL; BackTab (or Up) goes back. Enter
saves both fields. Copy is name-only; the same name as the original is an
error. While typing, letters (including keys that mean something else globally)
are part of the text. Every prompt uses the same one-line editor
(`line_edit.rs`): Left and Right move the caret, Home and End jump to the
ends, Backspace and Delete edit at the caret. A typo at the start of a name
does not mean retyping the whole line. The unfocused field of the form shows
the text without a caret.

Transport shows Icecast `StreamTitle` when the station sends it, otherwise the
station name. Queue rows stay the station name. Duration is unknown, so the
total time is `—` and the seek bar stays empty. Seek is refused. After a short
decode, the signal-path line includes the stream’s coded bitrate (`N kbps`).
Playing a station puts that stream on the queue, so the queue can show a
station name.

`r` on the library, playlists, devices, or radio reloads **the list in front**.
Repeat is `e`, shuffle is `z`. The full keymap lives in `znicz-tui/src/keys.rs`
and in the `?` overlay.

## Messages instead of silence

The interface owns the screen, so anything written to the log or to stdout is
invisible. Previously a failed file looked exactly like nothing happening. Now
every player error and every action becomes a short-lived **boxed** message in
the list corner (`toast.rs`), inset so it does not sit on the pane border. The
box grows to the message, up to the list width, so a reason like
`playlist had no playable files` is not cut off. A coloured mark and
matching outline show the level at a glance: blue info, green success, yellow
warn, red error. Errors stay up twice as long, since they need reading. Hints
on the bottom line are never replaced.

For this to work, key handlers use `send_blocking` rather than
`send`: the engine's own result comes back, so a missing file or an unusable
device is reported rather than dropped. It also means the next frame reads state
that already includes the change, instead of drawing the old volume.

The traffic goes the other way too. ALSA and other C libraries write warnings
**straight to file descriptor 2**, ignoring Rust's logging entirely, and in a
full-screen interface that means text appearing on top of the layout — during
device enumeration, and again whenever a stream is opened. So the binary points
stderr at `~/.cache/znicz/znicz-session.log` for as long as the TUI owns the
terminal, and restores it on exit. That file is a log, not the saved queue
(`session.toml`). Nothing is lost; it just stops being drawn on
top of the player.

## Modules

| File | Role |
| --- | --- |
| `app.rs` | state, event loop, key dispatch |
| `layout.rs` | list region, drawer overlay, toast boxes, modal placement |
| `views/` | drawing, one module per pane |
| `views/now_playing.rs` | two-line transport (play state, seek, signal path) |
| `views/inspector.rs` | full signal-path overlay (`i`) |
| `views/playlists.rs` | saved M3U overlay (`P`) |
| `views/radio.rs` | saved stations overlay (`R`) |
| `views/status.rs` | key hints only |
| `theme.rs` | every colour, in one place |
| `keys.rs` | the keymap as data |
| `cursor.rs` | list cursor movement |
| `line_edit.rs` | one-line prompt caret (search, playlist save, radio) |
| `meta.rs` | background tag cache |
| `toast.rs` | boxed, level-coloured messages |
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
