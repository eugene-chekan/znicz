# Spotify control

**Version:** the next minor over `main` at merge time (0.20.0 if this merges
before playlist editing, else 0.21.0).

## Goal

Control the Spotify app, wherever it runs, from Znicz: see what it plays,
use the usual transport keys on it, browse and search Spotify, and start a
track, album, or playlist. The TUI, the CLI, and MCP agents all get this,
including on a server reached over SSH.

Spotify is reached through the **Spotify Web API** (Spotify Connect), so it
works the same on Linux and Windows. It needs:

- **Spotify Premium** for playback control. Without Premium, browsing and
  search still work; control keys and tools say `needs Spotify Premium`.
  Spotify no longer reports an account's plan, so Premium shows as unknown
  until the first playback call is accepted or refused.
- **The user's own Spotify developer app.** Spotify does not allow a shared
  client ID for open-source desktop apps, so each user registers a free app at
  developer.spotify.com, adds the redirect URI Znicz shows, and gives Znicz the
  client ID. Login is OAuth 2.0 Authorization Code with PKCE (no client
  secret). The app's owner needs Premium, and a Development Mode app allows
  at most 5 users.

**Development Mode limits** (Spotify, February 2026, in force since March 9,
2026):

- A playlist's tracks (`/playlists/{id}/items`) are listed only for playlists
  the user owns or collaborates on. Others still play.
- Artists have no top-track list.
- Search returns at most 10 results per type.

## Out of scope

- Spotify tracks in Znicz's own queue or `.m3u` playlists.
- Editing Spotify playlists, liking or saving items.
- Podcasts and audiobooks (a playing episode shows as `unsupported item`).
- Lyrics.
- Reordering or removing Spotify queue items (the Web API cannot).

## Architecture

```
TUI (O mode, Settings 6) ─┐
MCP spotify_* tools ──────┼── IPC Spotify { op } ──> znicz player ──> SpotifyService ──> Web API
znicz spotify CLI ────────┘                              │
                                                         └── pauses the engine when Spotify starts
```

### `znicz-spotify` (new crate)

Everything that talks to Spotify. Blocking `ureq`, like the cover fetchers; no
async runtime.

- `auth`: PKCE verifier, challenge and `state`; the authorize URL; the
  one-shot callback listener; the code exchange; token refresh before expiry;
  the token file.
- `api`: typed calls: `/me`, search, the user's playlists, liked songs, saved
  albums, album / playlist tracks, artist top tracks and albums, devices,
  playback state, queue, play / pause / next / previous / seek / volume /
  repeat / shuffle, add to queue, transfer playback. The base URL is injected
  so tests can use a fake server.
- `service`: `SpotifyService`, a worker thread that owns the client, polls
  playback state, and keeps a `SpotifyState` snapshot: login status, user,
  premium, devices, now playing (track, album, year, track number, image URL,
  duration, progress, is playing, repeat, shuffle, volume, device), and the
  last error.

### `znicz-core`

- A `SpotifyHost` trait and a `SpotifyOp` enum, mirroring `LibraryHost` and
  `LibraryOp`, so core does not depend on `znicz-spotify`.
- IPC: `Spotify { op, id }`, with the same change-id dedupe as `Library`,
  and a `SpotifyStatus` query that returns `SpotifyState`.
- `CoverKey::Spotify(url)` in the cover pipeline: fetch, cache, letterbox and
  protocol handling as for other covers.

### `znicz` (binary)

- `daemon.rs` builds the `SpotifyService` inside `znicz player` and gives it to
  the IPC server as the host. It is always built; with no client ID it refuses
  ops with `Spotify is not configured: set a client ID in Settings → Spotify`.
- `znicz spotify login | logout | status`.

### One rule in the player

Starting or resuming Spotify playback (`SpotifyOp` play / resume / transfer
with play) pauses Znicz's engine once Spotify accepts the command. A refused
command leaves Znicz playing. Starting or resuming local playback
pauses Spotify, if Spotify is playing on the user's account. Both paths live in
the player, so the TUI, CLI and MCP behave the same.

### Polling

- About every 1 s while a UI is in Spotify mode, or an MCP Spotify call came in
  during the last 60 s.
- About every 15 s otherwise (this feeds the `♫ Spotify:` line in Znicz mode).
- Not at all while logged out.
- Progress is advanced on the client between polls so seek bars move smoothly.

## Configuration

`[spotify]` in `config.toml`; the default config ships it commented out.

```toml
[spotify]
client_id = "0123456789abcdef0123456789abcdef"
redirect_port = 8898          # redirect URI http://127.0.0.1:8898/callback
device = ""                   # preferred device name; empty = active device
open_browser = "auto"         # auto | always | never
```

The token lives in `spotify_token.json` in the data folder
(`~/.local/share/znicz`, `%APPDATA%\znicz`).

### Ops

| Op | Effect |
| --- | --- |
| `Configure { client_id, redirect_port, device, open_browser }` | The player rebuilds its client from the new values; no restart |
| `BeginLogin` | Starts the callback listener; replies with the authorize URL |
| `CompleteLogin { redirect_url }` | Finishes login from a pasted address |
| `Logout` | Deletes the token file and clears the state |
| playback / browse ops | One per tool in [MCP](#mcp) |

Front ends that change settings also save them to `[spotify]` in place with
`toml_edit`, as Settings does today.

## Login

`BeginLogin` creates a PKCE session (verifier, `state`), kept in the player for
**10 minutes**. It starts a listener bound to `127.0.0.1:<redirect_port>` and
returns the authorize URL with the scopes:

`user-read-playback-state user-modify-playback-state
user-read-currently-playing user-library-read playlist-read-private
playlist-read-collaborative`

Login completes by whichever comes first; the others are cancelled:

1. **The callback.** The browser redirects to the listener. This happens in a
   local session, and over `ssh -L <port>:127.0.0.1:<port>`.
2. **Paste.** On any machine, the user opens the URL, approves, lands on
   `http://127.0.0.1:<port>/callback?code=…&state=…` ("can't connect"), and
   pastes that address. A bare `code=…&state=…` query is accepted too.
   `state` must match the session.

The listener accepts one request with the matching `state`, answers with a
short "You can close this tab" page, and shuts down.

### Opening the browser

The front end that starts login (TUI or CLI) decides, from its own
environment, never the player's:

| Signal | Result |
| --- | --- |
| `SSH_CONNECTION`, `SSH_CLIENT` or `SSH_TTY` set | Remote: do not open |
| Linux, none of those, `DISPLAY` or `WAYLAND_DISPLAY` set | Open with `xdg-open` |
| Linux, no display | Do not open |
| Windows, no SSH variables | Open with `start` |

`open_browser = "always"` or `"never"` overrides the guess (tmux reattached
over SSH, `ssh -X`). A failed open says so in one line and keeps waiting. The
URL and the paste prompt are always shown, so a wrong guess never blocks
login.

### CLI

`znicz spotify login` prints the URL, opens a browser under the same rule, and
waits for the callback or an address pasted on stdin. `status` prints the
login state, user, premium and device. `logout` removes the token.

## TUI

### Settings → Spotify (tab 6)

| Row | Kind | Behaviour |
| --- | --- | --- |
| Client ID | text prompt | 32 hex characters, else the prompt stays open and says why. The strip explains where to get one. |
| Redirect URI | number prompt (port, 1024–65535) | Shows `http://127.0.0.1:<port>/callback`; the strip says to add exactly this URI to the Spotify app. |
| Open browser | choice | `auto` / `always` / `never` |
| Account | action | `Not logged in` / `Logging in…` / `Logged in as <name>`, then `· Premium` or `· not Premium (control unavailable)` once a playback call has told / `Failed: <reason>`. `Enter` logs in; when logged in, it asks in the strip to log out (`Enter` confirms, `Esc` cancels). |
| Playback device | choice | `Active device` or a Spotify device by name, used when nothing is active. Refreshed when the tab opens. |

Until a client ID is set, the rows below Client ID are dimmed and the strip
points at Client ID. `1`–`6` select tabs.

**Login panel.** `Enter` on Account replaces the strip with:

```
Open this URL in any browser and approve:
https://accounts.spotify.com/authorize?client_id=…   (c copy · Enter paste)
Waiting for the callback, or paste the address you were sent to.
```

- `c` copies the URL with OSC 52 (kitty, WezTerm, iTerm2, Windows Terminal,
  tmux with `set-clipboard on`).
- The URL is also an OSC 8 hyperlink.
- `Enter` opens a paste prompt; a wrong address keeps it open with `that
  address is not from this login`.
- `Esc` cancels the login.
- The panel closes when the status turns `LoggedIn` or `Failed`.

### Spotify mode (`O`)

`O` toggles Spotify mode from Browse or Stage. The mode is a flag separate from
the view: `v` / `i` still switch Browse ↔ Stage inside it. When not logged in,
`O` toasts `Spotify: log in under Settings → Spotify (S, 6)`.

```
┌ Spotify · Eugene · ▸ Kitchen speaker ─────────────────────┐
│ ▸ Search                                                  │
│   Playlists                                               │
│   Liked songs                                             │
│   Albums                                                  │
└───────────────────────────────────────────────────────────┘
▶ So What — Miles Davis  ━━━●━━━━  1:02 / 9:22  70%
Spotify Connect → Kitchen speaker   shuffle · repeat all
● started Kind of Blue
Space pause · / search · Enter play · a queue · ] queue · O znicz · ? more
```

**Browser.** One drill-down column, like the single-column library layout.

- Top level: Search, Playlists, Liked songs, Albums.
- `l` / `Enter` in, `h` back.
- Lists load 50 rows at a time and fetch the next page near the end.
- `/` searches from anywhere in the mode. Results are grouped Tracks / Albums
  / Artists / Playlists, like library search (at most 10 per group). An
  artist opens to their albums.
- `Enter` on a track plays it in its context (album or playlist, offset to that
  track). Search results play with the result list as context (track URIs). `Enter` on an album or playlist plays it from the top. Opening a playlist
  the user neither owns nor collaborates on says `Spotify only lists the
  tracks of playlists you own or collaborate on; Enter still plays it`.
- `a` adds a track to Spotify's queue.
- `]` shows Spotify's queue, read-only.

**Keys go to Spotify.** `Space`, `n` / `p`, seek arrows, `+` / `-`, `e`
(repeat: off, all, one), `z` (shuffle), `m` (mute: volume 0, then the previous
volume on unmute). The devices key `,` lists Spotify Connect devices; choosing
one transfers playback.
Library-only keys (`A`, `L`, `F`, `U`, `J`) toast `not in Spotify mode`.

**Leaving the mode** keeps Spotify playing and returns the keys to the engine.
While Spotify plays in Znicz mode, the message row shows
`♫ Spotify: <title> — <artist> · O to switch` when no newer message is on it.

### One Stage, one transport

`views/stage.rs` renders a source-neutral `StageModel` instead of reading
`PlayerState`:

- title, artist, the `album · year · track N` line
- seek (position / duration, or live)
- volume, mute, repeat, shuffle
- Signal rows (label → value) and an optional explanation
- the cover key
- the empty-state text and hint

`StageModel::from_player(&PlayerState)` and
`StageModel::from_spotify(&SpotifyState)` build it. The Browse transport gets
the same split (`TransportModel`). The Znicz Stage and transport must render
exactly as before.

| Znicz | Spotify |
| --- | --- |
| Title, artist, `album · year · track N` | Same, from the Web API track |
| Seek line | Same widget |
| Volume / repeat / shuffle | Same line, Spotify's values |
| Signal `file` / `stream` | `spotify` |
| Signal `device` with rate, channels, format | `device <name> (<type>)` |
| Signal `path` bit perfect / resampled | `path Spotify Connect`: "decoded and played by the Spotify app on that device; Znicz does not touch the audio" |
| Cover from file / stream chain | Largest album image up to 640 px, as `CoverKey::Spotify(url)`; the previous cover stays while the next loads |
| Nothing playing: logo, "Nothing playing", hint | Logo, "Nothing playing on Spotify", `/ search · O znicz` |

Layouts (wide, narrow, no cover), `cover_size`, marquee, line drop order, and
the Kitty / Sixel redraw rules are shared. The Stage footer ends in `O znicz`
in Spotify mode.

## MCP

Every tool is a `SpotifyOp` sent with `send_blocking`; results are JSON. Every
browse item carries its Spotify `uri`.

| Tool | Arguments | Notes |
| --- | --- | --- |
| `spotify_status` | | `{configured, login, user, premium, device, last_error}` |
| `spotify_login_begin` | | `{authorize_url}`; the agent hands it to the user |
| `spotify_login_complete` | `redirect_url` | The paste path |
| `spotify_logout` | | |
| `spotify_search` | `query`, `types?`, `limit? = 10` | types: track, album, artist, playlist; at most 10 per type |
| `spotify_playlists` | `offset?`, `limit?` | |
| `spotify_liked_songs` | `offset?`, `limit?` | |
| `spotify_albums` | `offset?`, `limit?` | Saved albums |
| `spotify_tracks` | `uri` | Album or (own) playlist tracks |
| `spotify_artist_albums` | `uri`, `offset?`, `limit?` | An artist's albums |
| `spotify_play` | `uri?`, `context_uri?`, `offset?` | No arguments resumes. Pauses Znicz first. |
| `spotify_pause`, `spotify_next`, `spotify_previous` | | |
| `spotify_seek` | `position_ms` | |
| `spotify_set_volume` | `volume` 0.0–1.0 | Same scale as `set_volume` |
| `spotify_set_repeat` | `mode`: off, track, context | |
| `spotify_set_shuffle` | `on` | |
| `spotify_queue_add` | `uri` | |
| `spotify_devices` | | |
| `spotify_transfer` | `device_id`, `play?` | |

The client ID is not settable from MCP.

- **Resource:** `znicz://spotify/now-playing`, the `SpotifyState` snapshot.
- **Skill:** a bundled `spotify` skill. It tells the agent to:
  - check `spotify_status` first;
  - use the two login routes for headless use;
  - treat requests without "Spotify" as the local library;
  - read Premium and no-device errors.

## Errors

`znicz-spotify` has one `SpotifyError`. The player maps it to a message for the
message row, the Messages list, and `SpotifyState.last_error`.

| Case | Handling |
| --- | --- |
| No client ID | `Spotify is not configured: set a client ID in Settings → Spotify` |
| `invalid_client` / redirect URI mismatch | `Failed`, naming the exact URI to register |
| Pasted address with wrong `state` or no `code` | Prompt stays open: `that address is not from this login` |
| PKCE session older than 10 min | `login expired, press Enter to start again` |
| 401 | Refresh once and retry; on refresh failure delete the token, go to `not_logged_in`, `Spotify login expired` |
| 403 on playback | `needs Spotify Premium`; the account is marked not Premium |
| 403 / 404 on a playlist's tracks | `Spotify only lists the tracks of playlists you own or collaborate on; Enter still plays it` |
| 404 `NO_ACTIVE_DEVICE` | Transfer to the preferred device if set and online and retry, else `Spotify has no active device: open Spotify somewhere, or pick one with ,` |
| 429 | Wait `Retry-After`; slow polling; a user action is retried once after the wait |
| Network failure / 10 s timeout | Poll errors logged once and shown only when the state changes; user actions report at once |
| Corrupt token file | Logged out, one warning in the log |

**Security.**
- The token file is written atomically with owner-only permissions (`0600`
  on Unix).
- Tokens never appear in logs; `Debug` redacts them.
- The listener binds `127.0.0.1` only.

## Testing

No test calls the real API.

- **`znicz-spotify` unit tests:**
  - PKCE against the RFC 7636 test vector
  - building the authorize URL
  - parsing pasted addresses (full URL, bare query, wrong `state`, no `code`)
  - token expiry and refresh with an injected clock
  - mapping HTTP status codes and error bodies to `SpotifyError`
  - parsing API JSON fixtures (search, playlists, playback state, queue)
- **Service tests against a fake HTTP server** (a thread on a random port,
  with the base URL injected):
  - login by callback and by paste
  - 401 → refresh → retry
  - 429 backoff
  - choosing a device on `NO_ACTIVE_DEVICE`
  - the fast / slow polling switch
- **`znicz-core`:**
  - `Spotify` op dedupe with a `FakeSpotifyHost`
  - the pause rule in both directions
- **TUI:**
  - the Znicz Stage and transport unchanged after the model split
  - the Spotify Stage from a fixture in the wide, narrow and no-cover layouts
  - Settings tab 6 rows, Client ID checking, the login panel states
  - Spotify-mode keys mapping to `SpotifyOp`s
- **MCP:** tool schemas; calls through a fake host.
- **Manual checklist with a real Premium account:**
  - local browser login
  - paste login over SSH
  - `ssh -L` login
  - transport control
  - transferring between devices
  - a free account (browsing only)

## Docs and release

- New `wiki/Architecture/Spotify.md`. It covers developer-app setup, the login
  routes (with the SSH cases), why Premium is needed, and polling.
- Updates:
  - `wiki/Architecture/TUI.md`: `O` mode, Settings tab 6, the shared Stage
  - `wiki/Architecture/MCP.md`: the tools, resource and skill
  - `wiki/Architecture/Overview.md`: the new crate and IPC request
  - README features and configuration
  - `CHANGELOG.md`
  - the commented `[spotify]` block in `znicz/src/default-config.toml`
- Bump the minor version over `main` at merge time.

## Notes for parts 3–5 (from the part 2 review)

- **Give Spotify its own IPC connection.** `IpcClient` serialises requests on
  one connection, and its clones share it. A Spotify call can take about 40 s,
  so a front end that sent it on its shared client would block its own
  per-frame `state()` read: the TUI would freeze, and overlapping MCP tool
  calls would stall. MCP and the TUI connect a second `IpcClient` for
  `SpotifyHost`.
- **Typed errors.** A Spotify call that times out twice currently comes back
  as a Library-flavoured "outdated player" error. Part 3 decides on typed
  Spotify errors over IPC and fixes this with them.
- **Old players.** Against a player without Spotify support, every Spotify
  request costs a reconnect. The TUI stops polling `try_state` after the first
  "outdated player" or `NO_SPOTIFY` answer.
- **README.** The `[spotify]` configuration and `znicz spotify` usage go into
  the README with the final PR.

## Notes for parts 4–5 (from the part 3 review)

- **Two connections, split by speed.** Poll `SpotifyStatus` and send `Touch`
  on the main `IpcClient`: both answer at once. Send every other Spotify op on
  a second, Spotify-only `IpcClient` that runs off the UI thread. A slow op
  (up to about 40 s) must never stall a frame.
- **`network` on a change means "may still happen".** The player keeps one
  dedupe record for all Spotify changes and holds it while the change runs, so
  one client's change can wait behind another's slow one, time out, and still
  run later. After `network`, read the state instead of retrying blindly.
- **Stop polling an outdated player.** On `outdated_player` or `no_service`
  from `try_state`, stop polling Spotify and show why, instead of reconnecting
  every frame.
- **Poll errors carry no kind.** `SpotifyState.last_error` is plain text; the
  TUI gets kinds only from its own calls.
