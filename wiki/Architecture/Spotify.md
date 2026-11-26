# Spotify (`znicz-spotify`)

Znicz can control the Spotify app through the Spotify Web API (Spotify
Connect). Spotify decodes and plays the audio on whichever device it runs on;
Znicz only sends commands and reads what is playing.

## What you need

- **Spotify Premium.** The Web API's playback calls refuse free accounts
  (`needs Spotify Premium`). Browsing and search work without it. Spotify no
  longer reports an account's plan, so Znicz shows it only after the first
  playback call has been accepted or refused.
- **Your own Spotify app.** Spotify does not let open-source desktop apps
  share one client ID, so register a free app at
  <https://developer.spotify.com/dashboard>, add the redirect URI
  `http://127.0.0.1:8898/callback` (or your port), and give Znicz the client
  ID. No client secret is needed: login uses PKCE. The app's owner needs
  Premium, and an app in Development Mode allows at most 5 users (add each
  under *User Management*).

## Development Mode limits (February 2026)

Spotify's rules for Development Mode apps (in force since March 9, 2026)
shape what Znicz can browse:

- A playlist's tracks are listed only for playlists you own or collaborate on
  (`/playlists/{id}/items`). Others still play with `Enter`; opening one says
  so.
- Artists have no top-track list any more; an artist opens to their albums.
- Search returns at most 10 results per type.

## The crate

| Module | Job |
| --- | --- |
| `pkce` | Verifier, S256 challenge, `state` |
| `redirect` | Read the code from a pasted address or callback query |
| `token` | `spotify_token.json` (data folder, owner-only, written atomically) |
| `http` | Blocking `ureq` transport; status → `SpotifyError` |
| `auth` | Authorize URL, code exchange, refresh |
| `model` | Web API JSON → `znicz-core` types |
| `api` | One method per endpoint |
| `client` | Settings, token, login session; refresh before expiry and once on 401 |
| `listener` | One-shot `127.0.0.1` callback listener |
| `service` | `SpotifyService`: polling thread and `SpotifyHost` |
| `browser` | `OpenBrowser` (`auto` / `always` / `never`), the rule for when a front end opens a browser, and the platform opener |

The shared types (`SpotifyOp`, `SpotifyReply`, `SpotifyState`, …) and the
`SpotifyHost` trait live in `znicz-core` (`spotify_host.rs`), like
`LibraryHost`, so core does not depend on this crate.

## Where it runs

The **player process** (`znicz player`) hosts the `SpotifyService`, the same way
it hosts the library service. It is built from `[spotify]` and the token file.
With no client ID it still runs and reports `not configured`. With no data
folder for the token file it is not built: the player logs a warning, and every
`znicz spotify` command prints `Spotify is unavailable: …` and exits 1. The
player does not exit for idleness while a login is waiting.

The TUI, MCP and `znicz spotify` reach it over IPC. Playback and browse ops are
`Spotify { op }`; the snapshot is `SpotifyStatus`. A change op (anything that is
not a read) carries a change id, so a resend after a lost reply is not applied
twice. A player without a Spotify service answers `no Spotify service in this
player`.

## Commands

```
znicz spotify login    # log in; prints the URL, opens a browser when it can
znicz spotify logout   # forget the token
znicz spotify status   # login, user, Premium, device, what is playing, last error
```

`login` sends the current `[spotify]` settings to the player first, so a client
ID added after the player started takes effect without a restart; `status` and
`logout` never change the settings. Changing `client_id` logs you out once the
change is confirmed, because the old token cannot be refreshed with another
app. The player refuses such a change unless it is confirmed: `login` (after
its question) and the TUI's confirmed Client ID change send
`replace_client_id`; every other `Configure` is refused while the player has a
different ID, and a refused `Configure` changes nothing (not the port, not the
device). `login` refuses to run while
`config.toml` has an error, and it says so if you are already logged in. Each
command starts the player if it is not running.

`login` checks the client ID before it starts the player. With no client ID it
prints how to get one, names the redirect URI to register, and exits 1. Once
logged in it exits 0; otherwise it exits 1. `status` prints `Premium: unknown
(learned at the first play)` until a playback call has been accepted or
refused.

### Logging in over SSH

The login finishes by whichever comes first:

1. **The redirect reaches the listener.** On the same machine, the browser
   redirects to `127.0.0.1:<port>` and the listener finishes the login. Over
   SSH, forward the port first, from your own machine:

   ```
   ssh -L 8898:127.0.0.1:8898 user@server
   ```

   Then run `znicz spotify login` on the server. Approve in the browser on your
   machine; the redirect goes through the tunnel.

2. **You paste the address.** Without a tunnel, approve the URL in any browser.
   The browser lands on `http://127.0.0.1:8898/callback?code=…` and shows "can't
   connect". That is expected. Copy the whole address from the address bar and
   paste it into the waiting `znicz spotify login` (on its stdin). Each
   non-empty line on stdin is one address.

The login prints "Waiting for the browser; or paste the address you were sent
to." when the listener is up. If the port is busy, it prints "Paste the address
you were sent to." instead.

Over SSH (`SSH_CONNECTION`, `SSH_CLIENT` or `SSH_TTY` set), `login` does not
open a browser. It always prints the URL and the paste prompt, so a wrong guess
never blocks login. `open_browser` overrides the guess.

## Configuration

```toml
[spotify]
client_id = "0123456789abcdef0123456789abcdef"  # 32 hex characters
redirect_port = 8898       # http://127.0.0.1:8898/callback
device = "Kitchen speaker" # used when nothing is active; unset = none
open_browser = "auto"      # auto | always | never
```

Defaults: `redirect_port` 8898, `device` empty (none), `open_browser` `auto`.
The default config ships this block commented out. `open_browser` is read only
by front ends (the CLI, and the TUI later); the player never opens a browser. An
unknown `open_browser` value logs a warning and means `auto`.

Browser rule for `auto`: no browser when `SSH_CONNECTION`, `SSH_CLIENT` or
`SSH_TTY` is set; on Linux, a browser only if `DISPLAY` or `WAYLAND_DISPLAY` is
set; elsewhere, a browser.

## The pause rule

Znicz and Spotify never play together. The player enforces this, so the TUI,
CLI and MCP behave the same.

- **Spotify starts** (a `Play` or a `Transfer` with play, sent through Znicz):
  the player pauses its own engine once Spotify accepts the command, but only
  if the engine is playing. A refused command (no active device, Premium
  required, a network error) leaves Znicz playing. A Stopped engine is left
  alone, because `Pause` would show it as paused.
- **Znicz starts:** the player samples the engine every 250 ms. A start is a
  move into `Playing` from a status that was not `Playing` on either of the two
  previous samples. A track change (a one-sample gap) does not count. Each start
  queues one pause request for a single worker thread, which asks Spotify to
  pause if it is playing. Requests coalesce while one is queued, so none is lost
  and a flapping engine cannot pile up calls. A call can take about 40 s, so it
  never runs on the watcher's thread.
- If Znicz is already playing when the player starts, Spotify is not paused.

The two directions cannot loop. Spotify starting only moves the engine out of
`Playing`, and the watcher acts only on a move *into* `Playing`. Spotify's own
pause does not go through the "Spotify starts" path.

## Login

A login keeps a PKCE session for 10 minutes and starts a listener on
`127.0.0.1:<port>`. The listener binds only that address and answers one
finished login. The redirect or a pasted address finishes it, whichever comes
first (see [Logging in over SSH](#logging-in-over-ssh)). A login nobody finishes
expires after the 10 minutes, and its listener lets go of the port.

Logging in again replaces a pending login. The old listener lets go of the
port first, so the new one can bind it.

`BeginLogin` says whether the listener is listening. If the port is busy, the
reply says so and the error names the port, so the user knows to paste the
address instead.

The token file is `spotify_token.json` in the data folder
(`~/.local/share/znicz` on Linux, `%APPDATA%\znicz` on Windows), unless
`ZNICZ_SPOTIFY_TOKEN_PATH` is set.

The `state` in the address must match the session.

**Which token is kept.** A token belongs to the client ID it was issued for,
and the token file records that ID. A token saved before the ID was recorded
has none and matches any ID. The token file is deleted only by:

- `znicz spotify logout`, or the TUI's Logout;
- a confirmed client-ID replacement (`Configure` with `replace_client_id`);
- a refresh that Spotify reports as revoked, under the rule below.

A configuration change on its own never deletes it. A token for another ID
stays on disk, the app is not logged in with it, and switching back to that ID
logs in again until you log in with another ID: the file holds one token. The
player does not log in with another ID's token either.

**Revoked refresh.** A token that fails to refresh with Spotify's revoked
answer is dropped from memory at once. The file is deleted only when the token
belongs to the current app (its client ID is the configured one, not a legacy
token without one) and the file still holds that same refresh token. If another
player rotated the token in the meantime, or the file is missing or unreadable,
the file is left alone. A pending login is kept.

**The CLI asks.** `znicz spotify login` first sends `Configure` without
replacing. When the player refuses a different ID (`client_id_differs`), it asks
`The player uses a different Spotify client ID. Replace it with the one in
config.toml and log out? [y/N]`. Only `y` or `yes` (any case) sends
`replace_client_id`; anything else prints `Not changed.` and exits 1.

**The TUI shows it.** A refused ID is shown on the Settings Spotify tab as a
mismatch, not as a generic error; see [TUI: Spotify](TUI.md#spotify).

### Logging in from the TUI

Settings → Spotify, `Enter` on Account: the TUI asks the player for the login
(`BeginLogin`) and shows the URL in a panel. `c` copies the URL with OSC 52,
`Enter` opens a prompt for the address you were redirected to (the paste route
in [Logging in over SSH](#logging-in-over-ssh)), and `Esc` cancels the login.
The browser opens by the `open_browser` rule: `auto` does not open one over SSH,
and on Linux opens one only with `DISPLAY` or `WAYLAND_DISPLAY` set. `always`
and `never` override it. The panel and its keys are in
[TUI: Spotify](TUI.md#spotify).

## Polling

About every second while someone watches (a `Touch`, or any op in the last
minute), every 15 s otherwise, and not at all while logged out. Devices are
re-read at most every 15 s. After a 429 the service skips polls until
`Retry-After` has passed; a user command waits it out (up to 10 s) and retries
once. The same poll error is shown once, not every second.

A transfer waits up to about 5 s for the device to take over, and otherwise reports that it did not.

## Errors

| Case | Message |
| --- | --- |
| No client ID | `Spotify is not configured: set a client ID in Settings → Spotify` |
| Refused client ID / redirect URI | names the exact URI to register |
| Revoked refresh token, at any point | `Spotify login expired`; the token is dropped, and the token file is deleted only under the revoked-refresh rule above |
| Different client ID, not confirmed | `the player uses a different Spotify client ID` (kind `client_id_differs`); nothing changes |
| 403 on playback | `needs Spotify Premium` |
| No active device | retries on the preferred device if set and online, else `Spotify has no active device: …` |

The TUI's Account row shows `Spotify is unavailable: …` and ends with `(switch
tab to retry)` when the player has no Spotify service or is an older version.
The TUI then stops polling the status until the Spotify tab is shown again.

Tokens never appear in logs (`Token`'s `Debug` redacts them).

### Typed errors

`SpotifyError` maps to a `SpotifyErrorKind`. The core error is
`ZniczError::Spotify { kind, message }`, and its text is the message. Over IPC the
kind is sent as `spotify_kind` next to the message, so the CLI and the MCP tools
act on the kind and show the message. The kinds are `not_configured`,
`not_logged_in`, `login_expired`, `wrong_address`, `premium_required`,
`no_active_device`, `rate_limited`, `unavailable`, `network`, `no_service`,
`outdated_player` and `other`. An unknown kind from a newer player reads as
`other`.

- A player that does not answer within the 30 s read timeout is `network`, with
  the text "the player did not answer in time". The client reconnects and resends
  a change once first. A player that is not reachable at all is `network` too.
- A connection lost mid-request is `outdated_player`: the player is an older
  version that does not know Spotify requests. The text says to run
  `znicz player stop` and try again.
- `no_service` is a player without a Spotify service: one started without a data
  folder for the token. It is not an older version.
- A resent change keeps its kind, because the player stores the failure with it.
