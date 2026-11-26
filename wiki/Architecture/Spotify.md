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
it hosts the library service. It is always built from `[spotify]` and the token
file. With no client ID it still runs and reports `not configured`.

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

Each command first sends the current `[spotify]` settings to the player, so a
client ID added after the player started takes effect without a restart. The
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
  the player pauses its own engine first, but only if the engine is playing.
  A Stopped engine is left alone, because `Pause` would show it as paused.
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
finished login. It finishes by whichever comes first:

1. the browser's redirect reaching the listener (a local session, or
   `ssh -L <port>:127.0.0.1:<port>`);
2. the address the browser landed on, pasted back (works anywhere; the
   browser shows "can't connect", which is expected).

Logging in again replaces a pending login. The old listener lets go of the
port first, so the new one can bind it.

`BeginLogin` says whether the listener is listening. If the port is busy, the
reply says so and the error names the port, so the user knows to paste the
address instead.

The token file is `spotify_token.json` in the data folder
(`~/.local/share/znicz` on Linux, `%APPDATA%\znicz` on Windows), unless
`ZNICZ_SPOTIFY_TOKEN_PATH` is set.

The `state` in the address must match the session. A different client ID
logs out, because the old token cannot be refreshed with it.

## Polling

About every second while someone watches (a `Touch`, or any op in the last
minute), every 15 s otherwise, and not at all while logged out. Devices are
re-read at most every 15 s. After a 429 the service skips polls until
`Retry-After` has passed; a user command waits it out (up to 10 s) and retries
once. The same poll error is shown once, not every second.

## Errors

| Case | Message |
| --- | --- |
| No client ID | `Spotify is not configured: set a client ID in Settings → Spotify` |
| Refused client ID / redirect URI | names the exact URI to register |
| Revoked refresh token, at any point | `Spotify login expired`; you are logged out and the token file is deleted |
| 403 on playback | `needs Spotify Premium` |
| No active device | retries on the preferred device if set and online, else `Spotify has no active device: …` |

Tokens never appear in logs (`Token`'s `Debug` redacts them).
