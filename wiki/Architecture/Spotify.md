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

The shared types (`SpotifyOp`, `SpotifyReply`, `SpotifyState`, …) and the
`SpotifyHost` trait live in `znicz-core` (`spotify_host.rs`), like
`LibraryHost`, so core does not depend on this crate.

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
