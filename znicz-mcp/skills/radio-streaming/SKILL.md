---
name: radio-streaming
description: Add, play, copy, and edit HTTP/Icecast radio stations in znicz.
---

# Radio Streaming

Stations are stored in `stations.toml` (override `ZNICZ_STATIONS_PATH`).

1. `add_radio_station` with `name` and `url` (`http://` or `https://`)
2. `list_stations` or resource `znicz://stations`
3. `play_station` with the exact name — this **clears the queue** and starts the stream
4. `rename_radio_station`, `set_station_url`, `copy_radio_station`, `remove_radio_station` to edit

Copy keeps the URL and asks for a new name. The same name as the original is an error.

This slice does not parse ICY titles or play HLS. Playlist `http://` lines are still skipped.
Mixing a station into an existing file queue is later; do not invent that.
