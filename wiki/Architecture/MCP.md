# MCP server (`znicz-mcp`)

**MCP** (Model Context Protocol) is a standard so an AI app can talk to a local tool.

Znicz speaks MCP on **stdio** (stdin/stdout). The host starts:

```bash
znicz mcp
```

and sends JSON-RPC messages.

We use the Rust SDK [`rmcp`](https://crates.io/crates/rmcp).

## Four surfaces

| Surface | Job |
|---------|-----|
| **Tools** | Actions: `play`, `pause`, `seek`, `list_devices`, … |
| **Resources** | Read-only snapshots: `znicz://now-playing`, `znicz://stations`, `skill://…` |
| **Prompts** | Ready-made instructions for the model |
| **Skills** | Longer how-to files (`SKILL.md`) the model loads when needed |

Tools call the same `Command`s as the TUI. No second player.

Library tools (`scan_library`, `search_library`, `get_track`, `browse_album`,
`list_albums`, `library_stats`, `library_prune`) talk to
[`znicz-library`](Library.md). Playlist tools (`list_playlists`,
`import_playlist`, `save_playlist`, `play_playlist`) load and write M3U files;
see [Formats and metadata](../Domain/Formats-and-Metadata.md#playlists-phase-3).
Radio tools talk to `stations.toml`: `list_stations`, `add_radio_station`,
`play_station` (clears the queue and starts the stream), `rename_radio_station`,
`set_station_url`, `remove_radio_station`. Resource `znicz://stations` is the
same list. Serialised player state marks each queue row with `kind`: `file` or
`stream`.

## Tools wait for the player

A tool that changes something must report what actually happened. Sending a
command and immediately reading state gives the **previous** snapshot, because
the player thread has not run yet — that was [Issue #1](../Issues.md).

So mutating tools use `PlayerHandle::send_blocking`, which waits until the engine
has applied the command and returns the engine's own result:

- the returned state shows the change
- a real failure (missing file, unusable device) becomes an MCP error instead of
  a silent stale snapshot

The TUI uses `send_blocking` too: the frame after a keypress must show the new
volume, and a failure must become a toast instead of vanishing into the log.
Startup paths that do not need an immediate redraw can still use `send`.

## Skills (SEP-2640 style)

Bundled under `znicz-mcp/skills/`:

- `audiophile-playback`
- `library-management`
- `playlist-curation`
- `radio-streaming`
- `mcp-control`

They are also MCP resources (`skill://name/SKILL.md`) and listed by the `skills_list` tool.

## Cursor

[`.cursor/mcp.json`](../../.cursor/mcp.json) points at `znicz mcp`. Put `znicz` on your `PATH` or change `command` to the full binary path.

## Extra reading

- [MCP specification](https://modelcontextprotocol.io/)
- [Agent Skills](https://agentskills.io/)
- [rmcp docs](https://docs.rs/rmcp/)
