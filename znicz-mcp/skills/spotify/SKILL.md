---
name: spotify
description: Control Spotify from znicz: log in, search, browse, play, pause, seek, set volume and move playback between devices. Use for any request that names Spotify, and for login or Premium questions. Requests that do not name Spotify are for the local library.
---

# Spotify

Spotify is a second source beside the local library. Its tools start with `spotify_`. Znicz controls the Spotify app on the user's account; it does not play Spotify audio itself.

## Always start with status

Call `spotify_status` first. It returns:

| Field | Meaning |
|-------|---------|
| `configured` | `false`: no client ID is set in `config.toml` |
| `login` | `not_configured`, `not_logged_in`, `logging_in`, `logged_in` or `failed` |
| `user` and `premium` | The account. `premium` is `null` until the first playback call has been accepted (then `true`) or refused for Premium (then `false`) |
| `device` | The device Spotify plays on, if any |
| `playing` | The current track and position, or `null` |
| `last_error` | The last Spotify error text |

## Logging in

If `login` is not `logged_in`, log in:

1. Call `spotify_login_begin`. It returns `authorize_url`.
2. Give `authorize_url` to the user. They open it in a browser and approve.
3. Then one of two routes:
   - **Local:** the browser redirects to `redirect_uri` on the user's machine, and the login finishes by itself. Check with `spotify_status`.
   - **Headless** (SSH, or the redirect does not reach the player): the browser lands on a page that says it cannot connect. Ask the user to copy the full address from the address bar and paste it to you. Call `spotify_login_complete` with `redirect_url` set to that address.
4. `spotify_logout` removes the stored login.

If `login` is `not_configured`, the user must set `client_id` under `[spotify]` in `config.toml` and run `znicz spotify login`. You cannot set the client ID from here.

## Local library or Spotify?

A request that does not say "Spotify" is for the local library: use `search_library`, `play_playlist` and the other library tools. A request that says Spotify is for the `spotify_*` tools.

## Searching and browsing

- `spotify_search` returns at most 10 results per type. To page, search one type at a time and use `offset`.
- `spotify_playlists`, `spotify_liked_songs` and `spotify_albums` list the user's own items.
- `spotify_tracks { uri }` lists an album's tracks, or the tracks of a playlist. Spotify lists a playlist's tracks only when the user owns it or collaborates on it. Any other playlist can still be played with `spotify_play`, but its tracks cannot be listed.
- `spotify_artist_albums { uri }` lists an artist's albums. Artists have no top-tracks list here.
- Every item carries a `uri`. Pass it to the other tools.

## Playing and controlling

- `spotify_play { uri }` plays one track. `spotify_play { context_uri, offset? }` plays an album or playlist from `offset`. `spotify_play {}` resumes.
- Once Spotify starts playback, Znicz's own playback is paused. A refused play (for example no active device) leaves Znicz playing. Starting Znicz playback pauses Spotify.
- `spotify_pause`, `spotify_next`, `spotify_previous`, `spotify_seek { position_ms }`, `spotify_set_volume { volume }` (0.0 to 1.0), `spotify_set_repeat { mode }` (off, track or context), `spotify_set_shuffle { on }`, and `spotify_queue_add { uri }`.
- Transport calls return `{"reply": "done"}`. `spotify_status` reflects the change within about a second.

## Devices

- `spotify_devices` lists the devices Spotify can see, with their ids.
- `spotify_transfer { device_id, play }` moves playback to a device. Use `play: true` to start it there.

## The now-playing resource

`znicz://spotify/now-playing` is the same snapshot as `spotify_status`, as JSON. Reading it does not speed up the player's polling, so when nobody is watching it can be up to about 15 s old. Prefer `spotify_status` when freshness matters.

## Errors

A failed Spotify tool returns its message and a `kind` in the error data. Act on the kind:

| kind | What to do |
|------|------------|
| `premium_required` | Tell the user that playback needs Spotify Premium. Search and browsing still work. |
| `no_active_device` | Ask the user to open Spotify on a phone or computer. Or call `spotify_devices`, pick one, and call `spotify_transfer` with `play: true`. |
| `not_configured` | The client ID is missing or wrong, or the Spotify app does not have the exact redirect URI registered. Tell the user what the message says, including the URI it names. The user sets `client_id` under `[spotify]` in `config.toml` and runs `znicz spotify login`. |
| `not_logged_in`, `login_expired` | Start the login steps above. |
| `wrong_address` | The pasted address is not from this login. Call `spotify_login_begin` again and ask for the new address. |
| `rate_limited` | Wait a little before the next call. Do not retry in a loop. |
| `unavailable` | Explain the message to the user. Spotify does not offer this for that item. |
| `network` | Spotify or the player did not answer. For a change such as play or pause, the change may still happen, so call `spotify_status` before retrying. For a read, try once more, then tell the user. |
| `outdated_player` | The running player is an older Znicz without Spotify support. Ask the user to run `znicz player stop` and try again. |
| `no_service` | The player was started without a data folder for the login token. It is not an older version. Tell the user, and ask them to restart the player with a data folder. |
| `other` | Show the message. |

Some messages end with a hint for the terminal UI, such as "Enter still plays it" or "pick one with ,". Those keys do not exist here: use the tools above instead, and do not repeat the hint to the user.
