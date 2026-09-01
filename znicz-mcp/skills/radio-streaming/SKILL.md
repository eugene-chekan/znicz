---
name: radio-streaming
description: Add, play, copy, and edit HTTP/Icecast radio stations in znicz.
---

# Radio Streaming

Stations are stored in `stations.toml` (override `ZNICZ_STATIONS_PATH`).

1. `add_radio_station` with `name` and `url` (`http://` or `https://`)
2. `list_stations` or resource `znicz://stations`
3. `play_station` with the exact name — default **clears the queue** and starts the stream. `append: true` adds the station and does not start or stop playback.
4. `rename_radio_station`, `set_station_url`, `copy_radio_station`, `remove_radio_station` to edit

Copy keeps the URL and asks for a new name. The same name as the original is an error.

M3U URL lines play as streams. `queue_add` also accepts `http(s)` URLs (name is
the URL; does not save a station). This slice does not play HLS.

`get_player_state` / now-playing `title` and `tags.title` follow Icecast
`StreamTitle` when the station sends it; empty falls back to the station name.
Queue rows stay the station name. `bitrate_kbps` appears once the stream has
decoded about a quarter second (coded bytes vs PCM time).
