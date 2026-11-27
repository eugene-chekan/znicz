# TUI (`znicz-tui`)

Znicz is a terminal player first, so this crate is where most of the day-to-day
experience lives. The interface is an **immediate-mode** loop:

1. Drain player events (turning failures into on-screen messages)
2. Draw the whole screen from a fresh `PlayerState`
3. Wait for a key, a mouse event, or for the tick that advances the seek bar
4. Repeat until `q`

The seek bar (`format::progress_bar`) fills in half-cell steps: `━` for each
played cell, `╾` for a played half cell, then `─`. The player publishes the
position every 200 ms, and the TUI redraws on the same tick.

While the TUI is up it also **captures the mouse**. A left click on a visible
library, queue, or overlay list row **selects** that row (it does not play or
open). The wheel moves the focused list one row, like `j` / `k`. Help,
devices, settings, playlists, and radio close with the top-right **X**, or
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

A frame reads the player state once and passes it down. Lists (the queue, the
library columns, tree, paged and single lists) build rows only for what is on
screen: `views::window::visible_window` follows ratatui's `List` scrolling rule
(keep the offset until the selection leaves the screen), and the window start is
stored as the list's offset for the next frame and for mouse hit-testing. The
queue's title width and total length come from one pass per frame that formats
no strings.

## Views

Two views, toggled with `v` or `i`. The TUI always starts on **Browse**.

- **Browse**: the library, the queue drawer (`]`), and the text transport.
- **Stage**: full-screen now playing (see [Stage](#stage)).

`O` switches to **Spotify mode**, a flag beside the view, not a third view. In
the mode `v` / `i` still switch between Browse and Stage. `O` needs a Spotify
login; without one it toasts the login line and stays out. `O` again, or `O
znicz`, leaves the mode. The keys, the footer and the list region are Spotify's
while the mode is on. See [Spotify mode](#spotify-mode-o).

In Znicz mode, on Stage, `Esc` closes an open modal or the queue drawer first,
then returns to Browse. Playback keys, `?`, `,`, `P`, `R`, `M`, `]`, and `q` work
there. Library keys (`/`, `a`, `A`, `J`, `F`, `U`, `h` / `l`, `Tab` column
cycling, list movement with library focus, Alt-arrow pan) do nothing. The Znicz
Stage footer is `Space pause · n next · v browse · ] queue · ? more`. The
Spotify Stage footer is in [Spotify mode](#spotify-mode-o).

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
`cover_size`, `cover_protocol`, `library_layout`, `marquee`, `icons`). The file is
written with the defaults on first start; see the README's Configuration section.

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
columns and wraps. With the drawer open (overlay layout), `Tab` swaps library ↔
queue focus; with it closed, outside the columns, `Tab` does nothing — only `]`
opens the drawer. `M` opens the messages list and `J` jumps to the playing
track.

The footer shows a short hint list for the current **context** (library,
library tree, search prompt, folder prompt, queue, devices, playlists, radio, the
Messages list, a name prompt, the station form, help, or the help filter), most important first,
as many as fit. Wherever `?` opens help, ` ? more ` is right-aligned and always
kept; search, the prompts, and help itself leave it out because `?` is typed
text there or help is already open. Each hint carries the key it sends, so
clicking one runs that key.

In Spotify mode the footer uses the Spotify contexts (browser, search, Stage,
queue drawer, device list), and the library keys above do not apply: `h` and `l`
belong to the browser there. The `SPOTIFY` help table in `keys.rs` lists every
Spotify mode key, so `?` in the mode shows them all. The full list is in
[Spotify mode](#spotify-mode-o).

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

**Row labels.** Rows under an artist (tree albums and tracks, the album and
track columns, the paging list's album and track levels) do not repeat it:
`Kind of Blue (1959)`, `1. So What`. Track numbers are padded per album
(` 9.` / `10.`); when any track has a disc above 1 they read `D-N.` (`2-3.`). A
track without a number gets blanks so titles line up. A grouped track adds
` — Artist` only when its artist differs from the browse artist (compilations,
guests). Flat lists (search, all tracks) keep `Title — Artist` and
`Album (Year) — Artist` and show no number. Albums under an artist sort by year,
then name; albums without a year go last (`albums_for_browse_artist`). The
right-hand column of a track row is the playing marker (` ▶ `) and the
duration. Labels come from `LibraryPane::album_middle` / `track_middle` with a
`RowContext` (`Flat` or `Grouped { artist, numbering }`); the pan width uses the
same label.

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
A marker comes before it, chosen by `[tui] icons` (`format::volume_icon`):

| `icons` | 0–33 % | 34–66 % | 67–100 % | muted |
|---------|--------|---------|----------|-------|
| `unicode` (default) | `◢` | `◢` | `◢` | `×` |
| `emoji` | 🔈 | 🔉 | 🔊 | 🔇 |
| `nerd` | 󰕿 | 󰖀 | 󰕾 | 󰝟 |

`nerd` needs a Nerd Font set as the terminal font; the README's
[Nerd Font icons](../../README.md#nerd-font-icons) section has the install steps and a check.
Muted shows the marker, the bar at the level it returns to (dimmed on Stage),
and `muted` instead of the percentage. Transport widths use display width
(`unicode-width`), so a two-cell emoji still lets the volume segment drop first
on a narrow line.

### Devices

A centered modal (`,`). Lists the output devices and switches between them with
`Enter`. The footer shows what the open stream actually settled on, which is the
quickest way to find a device that will take your files unconverted.

### Settings

`S` opens a centered modal with six tabs, in the order the tab bar shows
them: **Display** (the cover, cover search, library layout, marquee and icons), **Audio**
(the default output device and bit-perfect), **Playback** (the fade times and the crossfade),
**Library** (the music folders), **General** (the update check) and **Spotify**
(the client ID, login and playback device; see [Spotify](#spotify) below). The
modal opens on the tab you last used in this run, with the cursor on its first
row.

Focus is either on the tab bar (the first line inside the modal) or on the
rows, never both. The row under the cursor shows the selection bar only while
the rows have focus. The strip above the bottom border explains the row under the cursor, or,
while the tab bar has focus, how to use it.

| Key | Action |
| --- | --- |
| `Tab` / `Shift-Tab` | Next / previous tab, from either focus; wraps at the ends |
| `1`–`6` | Show that tab, from either focus |
| `↑` on the first row (`k`) | Move focus to the tab bar |
| `←` / `→` (`h` / `l`) on the tab bar | Previous / next tab; stops at the first and last tab |
| `↓` (`j`) or `Enter` on the tab bar | Move focus into the rows of the shown tab |
| Click a tab label | Show that tab; focus stays where it was |
| With the login panel open, `Tab` / `Shift-Tab` do not switch tabs | The panel keeps `c`, `Enter` and `Esc`, and the other keys fall through; a click on a tab closes the panel and cancels its login, as leaving the Spotify tab does |
| `↑` / `↓` (`j` / `k`), `g` / `G`, `Ctrl-d` / `Ctrl-u` | Move between rows; `↓` stops at the last row, and `↑` on the first row goes to the tab bar |
| `Enter` / `Space` | Change the highlighted row: a switch flips, a choice steps forward and wraps; a slider opens its typed-value prompt |
| `←` / `→` (`h` / `l`) | Step a choice or a slider back or forward and stop at the ends; turn a switch off or on |
| `d` | On a folder, ask to remove it (`Enter` confirms, `Esc` cancels) |
| `Esc` | Close the modal, or back out of what is open first: a slider or client ID prompt, a pending folder removal or Spotify question, or the Spotify login panel (in its paste prompt, back to the panel) |

The shown tab's own number key keeps the cursor where it is, so `1`–`6` only
moves the cursor when it goes to another tab. The tab shown when the modal
closes is the tab it opens on next time.

**Playback** has six rows, one per `[audio.fade]` time: *Fade in (play,
resume)*, *Fade on pause*, *Fade on stop*, *Fade on track change* and *Mute
fade*, then *Crossfade*. Each row is a slider: a bar about 16 cells wide, then
the value (`off`, `150 ms`, `3.5 s`). The fades run 0–2000 ms in 50 ms steps;
the crossfade runs 0–12 s in 0.5 s steps. `←` / `→` (`h` / `l`) move one step and
stop at the ends. A value that is not on the grid (typed, or in the file) moves
to the next grid value in the direction pressed.

`Enter` or `Space` opens a prompt in the strip with the current value, for
example `Crossfade (0–12 s): 3.5`. Type a number in the row's unit (seconds for
the crossfade, milliseconds for the fades), or with a unit: `3.5s`, `3500ms`,
`150 ms`. `Enter` saves, `Esc` cancels. A value that is not a number, or out of
range, keeps the prompt open and the strip says why (`must be 0–12 s`). While
the prompt is open the keys type into it (`←` / `→`, Home / End, Backspace,
Delete); tabs and rows do not move, `Ctrl-d` / `Ctrl-u` do nothing, and the
footer shows `Enter save · Esc cancel`.

A change saves to `[audio.fade]` and reaches the player at once. A new crossfade
time applies to the next crossfade, not one that is running. See
[Playback Pipeline](../Domain/Playback-Pipeline.md) for what each fade and the
crossfade cover.

Each change applies at once and is saved to `config.toml` in place, so comments
and other keys stay. A changed value keeps the comment on its line, and a removed
key keeps the comment lines above it (they often hold examples of other keys). The strip above the bottom border explains the highlighted
row in two lines; the border shows the config file's path. Bit-perfect is read
when the player starts, so after a change the row says `on restart`. A cover
protocol that needs the terminal's answer also says `on restart` after a change
when the TUI started without asking the terminal. Cover protocol offers only
the protocols the terminal reported at start (Windows Terminal: no `kitty`); a
value from the file that the terminal did not report is shown with `not
supported`.

On the Stage, cover changes wait while a modal, or a queue drawer that overlaps
the cover, is open: the row shows the new value at once with a dim `on close`
hint (Show cover, Cover size, Cover protocol, Cover search), and the cover
follows when the overlay closes. A Kitty or Sixel cover is sent as whole rows, so
drawing it again under an overlay would paint over the overlay. This also holds
for a new stream cover that arrives while Help or Settings is open.

With no config file the change applies for this run only,
and the first such change says so. A config file with an error is never written
to; the change applies for the run and the toast gives the line and column.

`Enter` on **Default output device** opens the Devices list with `System default`
first; choosing a device switches to it, saves `[audio] device`, and returns to
Settings. `System default` removes the key and switches to the device the OS
marks as default. `+ Add a folder…` opens the folder prompt and returns to
Settings when it closes, `Rescan all folders` runs the same rescan as `U`, and
`d` on a folder asks in the strip how many tracks it takes out. `Enter` removes
the folder and its tracks from the library (the files stay); `Esc` cancels.

### Spotify

The **Spotify** tab (tab 6) sets up the Spotify service that the player runs.
Its rows:

| Row | Shows | `Enter` does |
| --- | --- | --- |
| Client ID | The first 8 characters and `…`, or `not set`. The strip points at developer.spotify.com/dashboard | Opens a prompt. Accepts 32 hex characters, or empty to clear it |
| Redirect port | `http://127.0.0.1:<port>/callback`. Default 8898 | Opens a prompt for a port from 1024 to 65535 |
| Open browser | `auto`, `always` or `never` | Steps to the next value |
| Account | Not configured, Not logged in, Logging in…, or Logged in as *name*, with Premium or not once known | Logged out: starts a login. Logged in: asks in the strip, then logs out on `Enter` |
| Playback device | The active device, or a device Spotify lists. Logged in, a device that the list does not name is marked `offline`; logged out, the hint is `unknown while logged out` | Steps through the devices |

Account and Playback device are dimmed until a client ID is set. `Enter` on
either then says `Set a client ID first, in the Client ID row.` Redirect port
and Open browser stay editable without one.

When Spotify is unavailable, the Account row says `Spotify is unavailable: …`
and ends with `(switch tab to retry)`.

**Client ID.** Setting the first ID saves at once, when neither the screen nor
`config.toml` has one. Changing or clearing an ID that is already set always
asks first in the strip: `Changing the client ID logs you out of Spotify.
Enter confirms · Esc cancels.` Only this confirmed change tells the player it
may replace the ID (`Configure` carries `replace_client_id`), and the player
deletes the token only then. The player refuses any other change to a set ID
with `ClientIdDiffers`. The one other way to replace it is `znicz spotify
login`, which asks first (see [Spotify](Spotify.md)).

**Another ID in the player.** When the player refuses the file's ID that way,
the TUI keeps the file's value on screen and shows the mismatch:

- the Client ID row's hint says `player uses another ID` (dim, like `offline`);
- a toast says `The player uses another Spotify client ID. Enter it again under
  Client ID to switch (this logs out).`;
- the Redirect port and Playback device rows add the strip line `Not applied:
  the player uses another client ID.` in place of their explanation, because
  the player keeps its old port and device.

Entering the same ID again in the Client ID prompt asks in the strip, as a
change does, and confirming sends `replace_client_id`. Any successful
`Configure` clears the mismatch.

**Saving.** A row the player needs (Client ID, Redirect port and Playback
device) is written to `[spotify]` in `config.toml` in place. Then the TUI reads
`[spotify]` back from the file and takes that as the truth for the screen, and
only then tells the player (`Configure`) with what the file says. If the file
cannot be read back, nothing is sent and a toast says why. Open browser is read
only by the TUI, so it is saved but not sent. With a config error, or no config
file path, the row goes back to its old value, nothing is written, and nothing
is sent. An empty client ID would log the user out.

**Broken file at start.** If `config.toml` did not parse at start, the TUI runs
on the defaults. Showing the Spotify tab reads `[spotify]` from the file again,
and once the file parses, the Client ID, Redirect port, Open browser and
Playback device rows and Account show it. The first time that happens in a run,
a toast says `config.toml is fixed; restart Znicz to load all settings`. The
problem stays set, so saves and `Configure` stay refused until a restart: the
rest of the config is still the defaults.

**Login panel.** `Enter` on Account (logged out) starts a login. The panel
opens in the list area and shows the URL as plain wrapped text (not an OSC 8
link), with the line `Waiting for the browser; or paste the address you were
sent to.` The keys are:

| Key | Action |
| --- | --- |
| `c` | Copy the URL. The TUI writes an OSC 52 sequence to the terminal, so the terminal must allow it (tmux needs `set-clipboard on`). The terminal gives no answer, so the toast says `Sent`, not `Copied` |
| `Enter` | Open the paste prompt in the strip. `Enter` in the prompt sends the address you were redirected to. A wrong address keeps the prompt open with the reason |
| `Esc` | In the paste prompt, back to the panel. In the panel, cancel the login |

Other keys are not the panel's. `S` or `?` still closes Settings and drops the
panel, and the pending login is cancelled in the player, as `Esc` does, so the
Account row does not stay on Logging in. When the login ends, the panel closes
by itself. A success toasts `Logged in as …`, and a failure toasts the
reason. A cancelled login closes quietly. A `BeginLogin` reply that arrives after
Settings has closed cancels that login, so nothing is left pending.

**One request at a time.** While a `BeginLogin` is pending, or the panel is open,
`Enter` on Account (and a click on its footer hint) does nothing. Each
`BeginLogin` starts a new PKCE session, so a second one would invalidate the
first. Likewise a device list is requested once at a time: switching to tab 6
while it is pending sends nothing new. A device list that arrives after the
client ID changed is dropped.

**Opening the browser.** `auto` (the default) does not open a browser over SSH,
which is when `SSH_CONNECTION`, `SSH_CLIENT` or `SSH_TTY` is set. On Linux it
opens one only when `DISPLAY` or `WAYLAND_DISPLAY` is set. `always` and `never`
override that guess. If the open fails, one line says `Could not open a browser:
… Use the URL below.`, and the panel keeps waiting for the redirect or the
pasted address.

**Background calls.** Every Spotify call except the status read and `Touch` runs
on a background thread (`SpotifyWorker`), so no frame waits for Spotify. There
are two lanes, each with its own connection to the player (its own
`IpcClient`): the **browse lane** (Settings calls, logins, devices, browse and
the queue) and the **transport lane** (play, pause, next, previous, seek,
volume, repeat, shuffle, queue add and transfer). A slow search never holds up
Pause. The status is read every 500 ms while the Spotify tab, a login panel or
Spotify mode shows it, and every 3 s otherwise when a client ID is set. When the
player has no Spotify service, or is too old, the status reads stop and the
Account row says why, until the Spotify tab is shown again or `O` is pressed.
Devices are read when the tab opens, when a login finishes and when `,` opens
the device list in Spotify mode. The worker threads end when the TUI closes.

### Stage

Full-screen now playing (`v` / `i`), drawn by `views/stage.rs` with the geometry
in `layout::stage_layout`.

- **Wide** (Stage area at least 70 x 14): a square cover on the left at full
  height (capped at half the width), details on the right.
- **Narrow**: cover on top (at most half the height), details below.
- **Cover size** (`[tui] cover_size`, 100 / 75 / 50 / 25 %) scales the cover;
  it always stays in the top-left corner and the details follow it.
- **No cover**: under 8 rows, or `show_cover = false`; details are centred (at
  most 80 columns wide).

For files, missing embedded art draws the bundled logo. For streams the cover
is an ICY `StreamUrl` image when one decodes, else the AudioAddict current-song
cover (RadioTunes / DI.FM / RockRadio / JazzRadio / ClassicalRadio /
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

Kitty and Sixel images bypass ratatui's cell diff, so when a modal or the queue
drawer closes over the Stage, the next frame clears the terminal and redraws
everything in one synchronized update (`needs_full_redraw`); otherwise the
closed overlay would stay painted over the image. A modal closing while the
drawer stays open counts. Sixel and iTerm2 covers are re-sent from their encoded
data; a Kitty cover is encoded and transmitted again (`cover_needs_retransmit`).

Details, top to bottom: title (bold, scrolls), artist, `album · year · track N`,
the seek line, volume / repeat / shuffle, and a **Signal** panel: `file` or
`stream`, `device` with rate, channels and sample format, and `path` (`bit
perfect` or `resampled`, with an explanation). Unknown fields show `--`. When
height is short, lines drop in this order: the explanation, the Signal panel,
volume. Streams show the station name and URL, and the seek line reads `● live`
with the elapsed time. With nothing playing, Stage shows the logo, "Nothing
playing", and a hint.

**Spotify Stage.** In Spotify mode Stage draws what Spotify plays, from the same
`StageModel` and `TransportModel` (`stage_model.rs`) that Znicz uses, so the
layout is the same. The parts that differ:

- **Cover**: the track's album art, fetched by URL through `CoverKey::Url`. If
  the fetch fails, or the track has no art, the logo shows. The previous art
  stays only while the next one loads.
- **Signal panel**: three rows. `source` is `spotify`, `device` is `<name>
  (<kind>)`, and `path` is `Spotify Connect` (good tone). The explanation is
  "decoded and played by the Spotify app on that device; Znicz does not touch
  the audio".
- **Volume**: the device's percent. While `m` holds a remembered volume, the
  line shows muted.
- **Empty state**: with nothing playing the title reads `Nothing playing on
  Spotify`, the hint reads `/ search · O znicz`, and the logo shows.
- **Footer**: `Space pause · n next · v browse · O znicz`.

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

The footer always ends with the running version: `? more · v0.y.z`, or the
version alone at the right end where there is no `? more` (Help, prompts).
Hints give way to it on narrow terminals; it goes only when the row cannot hold
`? more · v0.y.z`. `znicz --version` prints the same number.

## Spotify mode (`O`)

`O` turns Spotify mode on from Browse or Stage, with no modal open. It needs a
login: without one it toasts `Spotify: log in under Settings → Spotify (S, 6)`
and stays out. Once the mode is on, `v` / `i` still switch Browse and Stage.
`O` again, or `O znicz`, leaves. Leaving sends nothing, so Spotify keeps playing
where it is. The browser's levels and the search prompt are cleared.

If the login is lost while the mode is on, the mode is left with the toast
`Spotify: logged out; O again after logging in`.

### Browser

The browser fills the list region, one column. The top level has four entries:
**Search**, **Playlists**, **Liked songs** and **Albums**. Artists are reached
through search. The title reads `Spotify · <user> · <device>`.

| Key | Action |
| --- | --- |
| `j` / `k`, arrows | Move the cursor. In search results the cursor skips the group headers |
| `g` / `G` | First / last row |
| PageUp / PageDown | Page |
| `Ctrl-d` / `Ctrl-u` | Half a page down / up (10 rows), as in the library |
| `Enter` | Act on the row (below) |
| `l` | Open the row: a playlist or album opens its tracks; an artist opens its albums |
| `h` | Back one level. Never past the top |
| `r` | Reload the list from its first page. Not in search results |
| `/` | Open the search prompt (the Search row does the same) |
| `Esc` | Close the search prompt first; then the queue drawer, if open; otherwise back one level; at the top it does nothing |
| `Tab`, `Shift-Tab` | Nothing: the browser is one column |

**Enter** plays what the row is:

- A playlist or album plays from its first track, as Spotify's context.
- A track in a playlist or album list plays from that track, in that context.
  The offset is the track's row in Spotify's list, so a local file skipped in
  the list still counts.
- A track from liked songs or from a search group plays the loaded tracks of
  that list as URIs, starting at the cursor. Liked songs have no context. Spotify
  takes at most 100 URIs per request, so only a window of up to 100 is sent: it
  starts 50 rows before the cursor, and the cursor's place in it is the offset.
- An artist opens its albums (as `l` does).

**Lists and paging.** A list is fetched 50 rows at a time. The tick asks for
the next page when the cursor is within 5 rows of the last loaded row and more
rows exist. The next page starts at the row after the last raw row Spotify
returned, so a skipped local file does not make a row appear twice. Search does
not page. A list shows `Loading…` while a request is
out. Each list level keeps its own requests: a page that arrives for a level
still on the stack fills that level, even after the user has gone back from it
or started a search above it. Only a level the user has left is dropped, and
its late pages are discarded. A page that fails stalls its list: the tick asks
for no more pages until the cursor moves or `r` reloads it. A page with no raw
rows from Spotify ends the list.

**Unavailable playlists.** Spotify only lists the tracks of playlists the user
owns or collaborates on. When a playlist's tracks are refused, the list shows
the notice `Spotify only lists the tracks of playlists you own or collaborate
on; Enter still plays it` and stays empty. The app does not know ownership in
advance, so `Enter` on that playlist still plays it.

**Search.** `/` opens the prompt in the strip, and typing does not search yet.
`Enter` searches. An empty or spaces-only query sends nothing and keeps the
prompt open. `Esc` closes it. While the prompt is open, every key is text, Ctrl
keys included, so `Ctrl-d` / `Ctrl-u` do not move the cursor.

A search sends one request per kind (tracks, albums, artists and playlists),
each for 10 rows at offset 0. The results are grouped under headers in that
order, at most 10 per group. There is no second page: Spotify's Development
Mode returns at most 10 per type.

**Mouse.** A click on a browser row moves the cursor there. It does not play or
load the row. The wheel moves the cursor.

### Playback keys

These work in Browse and in Stage.

| Key | Action |
| --- | --- |
| `Space` | Pause while playing. Otherwise resume the device's own track |
| `n` / `p` | Next / previous track |
| `→` / `←` | Seek 5 s forward / back; `Shift-→` / `Shift-←` seek 30 s. Clamped to the start and the track end. With nothing playing, a toast |
| `+` / `-` (also `=` / `_`) | Volume up / down 1 %, clamped to 0–100 %. A device with no volume reading gets a toast |
| `m` | Mute. Spotify has no mute, so `m` remembers the device's percent and sets 0 %. `m` again restores the remembered percent, or 50 % when none was saved. A device with no volume reading gets a toast and nothing changes |
| `e` | Repeat: off, then the context, then one track, then off |
| `z` | Shuffle on / off |
| `a` | Queue the highlighted track. An album, playlist or artist gets a toast: `Spotify's queue takes one track at a time` |
| `,` | Spotify Connect device list (a modal). `j` / `k` move, `Enter` moves playback to that device and starts it, `,` or `Esc` closes. Other keys do nothing while it is open, apart from the global ones |
| `]` | Spotify queue drawer, read-only. It is read again every 3 s while open. While it is open the browser behind it takes no keys and the wheel does nothing: `j` / `k`, `g` / `G`, `Enter`, `l` and the paging keys do nothing, so nothing plays and the Znicz queue cursor does not move. Its other keys (`d`, `C`, Alt-arrows) do nothing too: the Znicz queue is never changed from here, and nothing starts. A failed read shows its reason in the drawer in place of the rows (see Errors below) |
| `s` | A toast: `Spotify has no stop: Space pauses`. Nothing else changes |
| `A`, `L`, `F`, `U`, `J` | A toast: `not in Spotify mode`. Library keys only |
| `P`, `R`, `S`, `M`, `?`, `q`, `v`, `i`, `O` | Keep their global meaning |

The Stage in the mode takes the same transport keys. `r` and `/` belong to the
browser, so they do nothing in Stage.

### Message row

In the mode the message row shows toasts and scan progress. It does not show
the `♫ Spotify:` line that Znicz mode shows for a Spotify track.

### Polling

- In the mode the status is read every 500 ms, and each read touches the player
  so it keeps its fast poll.
- Outside the mode, with a client ID set, the status is read every 3 s without
  touching the player. So `O` knows the login state soon after launch. Until
  the first read, `O` shows the login toast.
- The queue drawer is read again every 3 s while it is open in the mode.
- The player polls Spotify on its own schedule too: see [Spotify](Spotify.md#polling).

### Lanes

Playback keys go to the transport lane and browser calls to the browse lane (see
[Background calls](#spotify)). Playback never waits behind a search.

### Errors

Errors of a change (a key that sends an op) or a read are toasts in the message
row.

| Case | What shows |
| --- | --- |
| The player's own message (for example `needs Spotify Premium`, or no active device) | The message as a toast |
| No answer in time, on a change | `Spotify did not answer: the change may still happen; the next status read shows the truth`. The status is read again at once |
| An old player, or a player without Spotify | The message as a toast. Status reads stop until the Spotify tab is shown or `O` is pressed |
| The request could not be sent (its lane is not running) | `Spotify is not connected in this session` |
| A queue read (the drawer's, every 3 s) fails | The reason, in place of the drawer's rows, until a read succeeds. A network error keeps reading; an old player or a player without Spotify stops the reads, as above |
| A browse list is `unavailable` | The notice in the list (above) |
| Any other browse error | A toast. The list keeps what it had |

A reply for a list the user has left changes nothing.

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

## Updates

When a newer release is known, a toast says so once, and `u` in Help opens the
**Update** modal: the release notes, `Enter` to install, then an offer to
restart Znicz (and the player, resuming where it was). While the update is
known, the footer's version reads `v0.16.0 → 0.17.0 available` (highlighted;
`v0.16.0 → 0.17.0` when space is short), so the news outlasts the toast.
When no update is known, `u` in Help checks
GitHub right away (even with `[updates] check = false`) and says what it found.
Details: [Updates](Updates.md).

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
| `settings.rs` | Settings rows, their values, and how a choice steps (pure) |
| `views/settings.rs` | Settings modal (`S`): list, explanation strip, config path |
| `spotify_rows.rs` | Spotify rows: account text, device choices, value checks, login panel lines (pure) |
| `spotify_worker.rs` | `SpotifyWorker<T>`: a thread with its own player connection that runs Spotify calls off the UI thread. Two lanes: browse and transport (`SpotifyAsk` tags both) |
| `stage_model.rs` | `StageModel` and `TransportModel`, built from `PlayerState` (Znicz) or `SpotifyState` (Spotify). The Stage and the transport draw only these |
| `spotify_browse.rs` | The Spotify browser as a pure state machine: levels, per-level request ids, paging and stalls, search groups (pure) |
| `spotify_mode.rs` | Spotify mode rules: keys, seek and volume arithmetic, mute plan, error plan, lanes, texts (pure) |
| `views/spotify.rs` | Spotify mode drawing: the browser column, search prompt, queue drawer, device list |
| `app/spotify_mode.rs` | Spotify mode in `App`: `O`, transport keys, mute, queue drawer, device list, leaving the mode |
| `app/spotify_browse_ui.rs` | Spotify mode in `App`: browser keys, loading, replies, search prompt, mouse rows |
| `osc52.rs` | The OSC 52 clipboard sequence that `c` writes in the login panel |
| `app/spotify_ui.rs` | Spotify tab and login panel: status poll, the lane each request takes, worker replies, login, logout, copy, browser |
| `app/settings_ui/spotify.rs` | Spotify rows of Settings: the `[spotify]` section as the file holds it, client ID, port and playback device saves, what each sends to the player |
| `help_state.rs` | help context, scroll, and filter text |
| `footer_hits.rs` | footer hint placement and click targets |
| `theme.rs` | every colour, in one place (16 ANSI colours, tuned for dark palettes; themes: [#68](https://github.com/eugene-chekan/znicz/issues/68)) |
| `keys.rs` | the keymap as data |
| `cursor.rs` | list cursor movement |
| `line_edit.rs` | one-line prompt caret (search, playlist save, radio, a Settings slider value) |
| `meta.rs` | background tag cache |
| `cover.rs` | background cover cache + logo |
| `tui_config.rs` | `show_cover` / `cover_protocol` / `library_layout` / `marquee` / `icons` |
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
