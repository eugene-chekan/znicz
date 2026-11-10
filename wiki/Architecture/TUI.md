# TUI (`znicz-tui`)

Znicz is a terminal player first, so this crate is where most of the day-to-day
experience lives. The interface is an **immediate-mode** loop:

1. Drain player events (turning failures into on-screen messages)
2. Draw the whole screen from a fresh `PlayerState`
3. Wait for a key, a mouse event, or for the tick that advances the seek bar
4. Repeat until `q`

While the TUI is up it also **captures the mouse**. A left click on a visible
library, queue, or overlay list row **selects** that row (it does not play or
open). The wheel moves the focused list one row, like `j` / `k`. Help,
devices, playlists, and radio close with the top-right **X**, or
with `Esc`, or a clickable footer hint — not by clicking outside. The queue
drawer closes with the **X**, global `]`, or a clickable footer `] close` /
`] queue` hint; `Esc` closes it when the drawer has queue focus, or on Stage regardless
of focus (with library focus in Browse, `Esc` goes back in the library). Footer hint segments are
clickable and run that key action. Click the library pane's right-border column
to open the queue drawer only. Library search and playlist/radio typing prompts
still cancel on an outside click. Clicks on the transport and cover do nothing.
Terminals that never send mouse events keep working from the keyboard. `?` stays
keyboard-only.

`q` closes the TUI only. Playback keeps going in `znicz player` until you
`s` (stop), `znicz player stop`, or the player is already Stopped and idle
with no UI connected.

The **player process** writes `session.toml` shortly after queue or transport
extras change, and again when it exits (idle or `znicz player stop`). Bare
start restores that session **Stopped**.
See [Formats and metadata](../Domain/Formats-and-Metadata.md#session).

The TUI is a **UI client**. It does not host the engine or write `ipc.toml`.
`znicz` autostarts `znicz player` and connects with Hello `role=ui`. If that
process restarts, the next key or tick re-reads the advertise file and Hellos
again. See [MCP](MCP.md).

Ratatui does not keep widgets as a tree you mutate. You describe the layout
**every frame**. That suits a player, where the position moves constantly.

## Views

Two views, toggled with `v` or `i`. The TUI always starts on **Browse**.

- **Browse**: the library, the queue drawer (`]`), and the text transport.
- **Stage**: full-screen now playing (see [Stage](#stage)).

On Stage, `Esc` closes an open modal or the queue drawer first, then returns to
Browse. Playback keys, `?`, `,`, `P`, `R`, `M`, `]`, and `q` work there. Library
keys (`/`, `a`, `A`, `J`, `F`, `U`, `h` / `l`, `Tab` column cycling, list
movement with library focus, Alt-arrow pan) do nothing. The Stage footer is
`Space pause · n next · v browse · ] queue · ? more`.

## Layout

```
┌ Library ─────────────────────────────────────────────────┐
│  tracks                                                  │
└──────────────────────────────────────────────────────────┘
▶ So What  ━━━●━━━━  1:02 / 9:22  70%
FLAC 24/96 → DAC  ● bit perfect
● added 4 tracks
Space pause · / search · a add · ] queue · P playlists · R radio · M messages
```

The library is the Browse view and always fills the list region. The queue is an
overlay drawer on the right (`]` toggles it). Every modal (help, devices,
playlists, radio, messages) opens **inside the list region**, so the
transport and the footer are never covered. The transport ends in a message row;
toasts no longer float over the list. Long titles pan horizontally with
`Alt-←` and `Alt-→` on the highlighted row only. `<` and `>` are unbound.
Browse transport is text only (no cover). Hints stay on their own line and are
never replaced by a message. Config: `[tui]` in `config.toml` (`show_cover`,
`cover_protocol`, `library_layout`, `marquee`).

### Responsive behaviour

The list region is `Min(3)`; the footer is 1 row and the transport takes the
rest of the fixed rows. The last transport row is always the message row.

| Window height | Transport rows |
| --- | --- |
| ≥ 20 | 3: title + seek, signal, message |
| < 20 | 2: title + seek, message |

When the list region is shorter than 10 rows, a modal takes the whole list
region instead of 70 % of it (`layout::modal_rect`).

Every row is also truncated to the window width with `…`, counting **characters
rather than bytes** so accented titles are never cut mid-character.

## Keys, footer, and help

Arrows seek (`→` / `←` 5 s, `Shift-→` / `Shift-←` 30 s). `h` / `l` move left
and right in the library and never play anything:

| Layout | `h` | `l` |
| --- | --- | --- |
| Columns | previous column (stops at Artists) | next column (stops at Tracks) |
| Tree | collapse an open node, else jump to the parent | expand a closed artist or album |
| Single column | back one level | into the artist or album |

They do nothing in search results, the queue, or modals. `Tab` still cycles
columns and wraps. `M` opens the messages list and `J` jumps to the playing
track.

The footer shows a short hint list for the current **context** (library,
library tree, search prompt, folder prompt, queue, devices, playlists, radio, the
Messages list, a name prompt, the station form, help, or the help filter), most important first,
as many as fit. Wherever `?` opens help, ` ? more ` is right-aligned and always
kept; search, the prompts, and help itself leave it out because `?` is typed
text there or help is already open. Each hint carries the key it sends, so
clicking one runs that key.

`?` opens help on the context it was opened from: that context's keys first,
then Playback and Navigation, then every other context dimmed. `j` / `k`,
`Ctrl-d` / `Ctrl-u`, `g` / `G`, and the wheel scroll. `/` filters by key or
action (case-insensitive); `Enter` keeps the filter, `Esc` clears it, and a
second `Esc`, `?`, or `q` closes help (`q` does not quit from here). Closing
returns to the modal help replaced (devices, playlists, radio, or
messages), or to the library. While you type a filter the help border reads
`Enter keep · Esc clear`; otherwise `/ filter · Esc close`. Other keys are
ignored while help is open, and the selection behind help is grey like any
other modal.

## The signal path

The second Browse transport line (and the Signal panel on Stage) is the audiophile part of the interface. It reads
`file format → device stream`, then a badge. Example:

`FLAC 96 kHz 24-bit 2882 kbps stereo → 96 kHz stereo  ● bit perfect`

Left of the arrow is the file (codec name, sample rate, bit depth when the
codec has one, bitrate when known, channels). Right of the arrow is the stream
the device actually opened (rate and channels). The device sample format (`f32`
and similar) is kept off this line; the Stage view's Signal panel shows it.

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

The home screen is **artist-first browse**: artists → albums → tracks. How that
is drawn depends on config and width:

- **`library_layout = "columns"`** (default) and a wide enough library strip:
  three columns side by side. **h** / **l** (or **Tab** / **Shift-Tab**)
  move focus between columns; arrows seek.
- Same preference but a **narrow** strip: single-column paging (Enter drills
  in, Esc steps back; **l** / **h** also drill in and step back). Still
  artist-first, not albums-as-home.
- **`library_layout = "tree"`**: expandable tree at any width. Enter or
  **Space** / **o** expands or collapses; open nodes are remembered for the
  session only. **l** expands, **h** collapses or jumps to the parent. Album
  lists are cached per expanded artist, so a redraw does not re-scan the
  library; **r** refreshes those lists.

`/` opens search. Results are entity hits — artists, albums, then
title-matched tracks. **Enter** on an artist or album leaves search and focuses
that entity in browse; **Enter** on a track plays it. `a` queues the selection;
`A` queues everything listed (`A` in search queues title-matched tracks only).
While the prompt is open, Left and Right move the caret. Queries go straight to
SQLite. See [Library](Library.md).

A library whose files carry **no album tags** cannot be grouped, so the pane
falls back to a flat track list rather than looking empty.

**Focus** is shown by colour. The focused pane has a cyan selection bar and an
accent border; an unfocused one (other columns, the library behind the drawer,
anything behind a modal) keeps its selection as a dark grey bar with a muted
border, so a selection is never invisible.

**Titles are breadcrumbs.** Columns title themselves `Artists`,
`Albums · <artist>`, and `Tracks · <album>`. The tree shows
`Library › <artist> › <album>` for the cursor row; paging shows the path of the
level you are on; search is `Library › search "<query>"` and the flat list is
`Library › all tracks`. When too wide, middle segments shorten first.

**Playing marker.** The playing track shows `▶` in place of its number (tree
tracks get a `▶ ` lead) and the playing colour. Its album and artist rows get
` ♪`. When the current path changes, the TUI looks it up in the library
(`Library::get_by_path`) to learn the browse artist and album, falling back to
the track's own tags. Streams and an idle player have no marker. `J` jumps to
the playing track: columns select artist, album, and track and focus Tracks;
the tree expands to the track; paging drills to the album's tracks. It leaves
search first, and says so in a message when nothing is playing or the file is
not in the library.

The queue drawer still overlays on the right; columns and the tree shrink under
it the same way the old single list did.

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
that was the last row. `Alt-↑` / `Alt-↓` move the highlighted row (the cursor
follows it, the playing track keeps playing, and the ends do nothing). It works
with the drawer focused on Browse or Stage. An older running player answers with
a hint to run `znicz player stop`.

`+` / `-` change volume by 1 % (snapped to whole percent). The volume bar has
eighth-cell resolution: 6 cells on the Browse transport, 10 on Stage.

### Devices

A centered modal (`,`). Lists the output devices and switches between them with
`Enter`. The footer shows what the open stream actually settled on, which is the
quickest way to find a device that will take your files unconverted.

### Stage

Full-screen now playing (`v` / `i`), drawn by `views/stage.rs` with the geometry
in `layout::stage_layout`.

- **Wide** (Stage area at least 70 x 14): a square cover on the left at full
  height (capped at half the width), details on the right.
- **Narrow**: cover on top (at most half the height), details below.
- **No cover**: under 8 rows, or `show_cover = false`; details are centred (at
  most 80 columns wide).

For files, missing embedded art draws the bundled logo. For streams the cover
is an ICY `StreamUrl` image when one decodes, else the AudioAddict current-song
`art_url` (RadioTunes / DI.FM / RockRadio / JazzRadio / ClassicalRadio /
ZenRadio), else an artist / title search (Deezer, then iTunes, then MusicBrainz), else the
station `art` file, else the logo. Streams resolve through `CoverKey::Stream`
and the chain in `znicz-core/src/cover/`; see [Stream covers](Cover-Sources.md).
While the next cover loads, Stage keeps the previous cover of the same stream.
The picture is
letterboxed onto an opaque canvas the size of the cover area, so a stream cannot
leave the previous cover on screen. The canvas takes the terminal's background
colour when the terminal answers the OSC 11 query at startup (asked with the
Kitty / Sixel probe), else a dark slate. The logo is transparent, so it sits
directly on the terminal colour. With Sixel (Windows Terminal, foot, …) the
uncovered pixels are left transparent instead: Sixel colours are whole
percentages, so a fill can never match the terminal exactly, and the Sixel
renderer erases the cells before drawing, so nothing old shows through. Kitty
keeps the opaque fill because it does not erase the cells. `cover_protocol`
applies here.

Details, top to bottom: title (bold, scrolls), artist, `album · year · track N`,
the seek line, volume / repeat / shuffle, and a **Signal** panel: `file` or
`stream`, `device` with rate, channels and sample format, and `path` (`bit
perfect` or `resampled`, with an explanation). Unknown fields show `--`. When
height is short, lines drop in this order: the explanation, the Signal panel,
volume. Streams show the station name and URL, and the seek line reads `● live`
with the elapsed time. With nothing playing, Stage shows the logo, "Nothing
playing", and a hint.

### Marquee

A now-playing title too long for its row (the Browse transport title segment,
which includes artist — album, and the Stage title) holds 2 s, scrolls one
column per 200 ms, holds 2 s at the end, then jumps back. The offset resets when
the text or width changes and when the view switches. `[tui] marquee = false`
keeps `…` truncation. Logic lives in `marquee.rs`.

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
`Enter save · Esc cancel` so the global keymap is not what you type into.

### Radio

A centered modal (`R`). Lists stations from `stations.toml`. Same saved-list
keys as Playlists: overlay keys win until Esc. If the URL cannot be opened,
playback stops so the previous file does not keep playing.

| Key | Action |
| --- | --- |
| Enter | Clear the queue and play |
| `a` | Add the highlighted station to the queue (does not start or stop playback) |
| `n` | New station: empty three-field form (name, URL, and art) |
| `e` | Edit: the same form, filled from the highlighted station |
| `c` | Copy URL and art; prompt asks for a new name only |
| `d` | Delete (immediate, no confirm) |
| `r` | Reload `stations.toml` |
| Esc | Close the overlay, or cancel a prompt |

Tab (or Down) cycles name → URL → art; BackTab (or Up) goes back. Enter
saves all three fields. Copy keeps URL and art; the prompt is name-only. The
same name as the original is an error. While typing, letters (including keys that mean something else globally)
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

The `?` overlay also shows the build in its bottom-left corner
(` znicz 0.y.z `); `znicz --version` prints the same number.

## Adding music

The TUI scans through the player's [library service](Library.md#the-library-service);
it never writes the database itself.

- **First run.** An empty library opens a welcome with a `folder:` prompt
  pre-filled with the platform music folder (`~/Music` style) or `~/`. `Enter`
  scans it; `Esc` skips the prompt for the session. If the text is not a folder
  the message `not a folder: …` appears and the prompt stays open.
- **`F`** opens the same prompt later (add a folder). **`U`** rescans every
  remembered folder.
- **Completion.** `Tab` completes folder names: it fills in the common prefix
  and, if several folders match, lists up to 5 as a message. Hidden folders
  show only after you type a dot.
- **Footer.** The `FolderPrompt` context reads `Tab complete · Enter scan ·
  Esc cancel`. The library footer has `F add folder` before `M messages`.
- **Progress.** While a job runs, the message row shows
  `⟳ scanning ~/Music · 1,240 files · 37 new · 2 updated`. The row prefers a
  live message, then progress, then the unread marker. An empty pane shows the
  same text.
- **Finish.** A message `scan done: …` (a warning when folders were missing:
  `skipped missing …`; an error `scan failed: …`). The library reloads once the
  last queued job is done, and not while search results are showing.
- **Polling.** The TUI asks for the library status at most every 500 ms and
  keeps the last status if the player cannot be reached.

## Messages

The interface owns the screen, so anything written to the log or to stdout is
invisible. Previously a failed file looked exactly like nothing happening. Now
every player error and every action becomes a short message (`toast.rs`) on the
**transport's last row**. It shows, in order of preference:

1. the newest live message, with a level mark and colour (blue info, green
   success, yellow warn, red error), truncated with `…`, and a dim `+N` on the
   right when more are live;
2. else, when unread alerts exist, `▲ 2 unread · M` (or `x`), coloured by the
   worst level;
3. else blank.

Messages live 4 s, errors 8 s. Hints on the bottom line are never replaced.

Every message enters a **history of 50** when it arrives, newest first, and stays there after it expires from the transport. **Unread** means
warnings and errors only; info and success confirm what you just did. `M` opens
the Messages list: a mark, an age (`12s`, `3m`, `1h`), and the text per row,
with rows that were unread when it opened in bold until it closes. `C` clears
the history; `Esc` or `M` closes; `Alt-←` / `Alt-→` pan the highlighted row.
Opening it marks everything read, and alerts that arrive while it is open count as
read too. The footer hint reads `M messages (N)` while unread alerts exist; on
80-column terminals `M messages` is the first hint dropped, and the transport's
unread marker still points to `M`.

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
| `layout.rs` | list region, drawer overlay, modal placement, transport height, Stage layout |
| `views/` | drawing, one module per pane |
| `views/now_playing.rs` | Browse transport (two-/three-line text) and the message row |
| `views/messages.rs` | Messages list (`M`) |
| `playing.rs` | which rows stand for the playing track, album, and artist |
| `views/stage.rs` | Stage view: cover, details, Signal panel |
| `marquee.rs` | scrolling of long now-playing titles |
| `views/playlists.rs` | saved M3U overlay (`P`) |
| `views/radio.rs` | saved stations overlay (`R`) |
| `views/status.rs` | footer hint line for the current context |
| `views/help.rs` | help overlay: context first, scroll, filter row |
| `help_state.rs` | help context, scroll, and filter text |
| `footer_hits.rs` | footer hint placement and click targets |
| `theme.rs` | every colour, in one place (16 ANSI colours, tuned for dark palettes; themes: [#68](https://github.com/eugene-chekan/znicz/issues/68)) |
| `keys.rs` | the keymap as data |
| `cursor.rs` | list cursor movement |
| `line_edit.rs` | one-line prompt caret (search, playlist save, radio) |
| `meta.rs` | background tag cache |
| `cover.rs` | background cover cache + logo |
| `tui_config.rs` | `show_cover` / `cover_protocol` / `library_layout` / `marquee` |
| `toast.rs` | live messages and history |
| `format.rs` | durations, rates, bars, truncation |
| `library_pane.rs` | browsing state |
| `folder_complete.rs` | `Tab` completion of folder names for the folder prompt |
| `scan_status.rs` | scan progress and finish text, polling of the library status |

`keys.rs` holds the bindings as data. Help is generated from those tables, and
the footer hints for each context are listed next to them; a unit test checks
that every hint names a key the context's help documents. The old help text had
drifted from the code and documented keys that did nothing; generating it
removes that possibility.

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
