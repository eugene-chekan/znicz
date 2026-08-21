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
| **Resources** | Read-only snapshots: `znicz://now-playing`, `skill://…` |
| **Prompts** | Ready-made instructions for the model |
| **Skills** | Longer how-to files (`SKILL.md`) the model loads when needed |

Tools call the same `Command`s as the TUI. No second player.

Library tools (`scan_library`, `search_library`, `get_track`, `browse_album`,
`list_albums`, `library_stats`, `library_prune`) talk to
[`znicz-library`](Library.md). Playlist and radio tools still return “not
implemented” until Phases 3 and 4.

## Tools wait for the player

A tool that changes something must report what actually happened. Sending a
command and immediately reading state gives the **previous** snapshot, because
the player thread has not run yet — that was [Issue #1](../Issues.md).

So mutating tools use `PlayerHandle::send_blocking`, which waits until the engine
has applied the command and returns the engine's own result:

- the returned state shows the change
- a real failure (missing file, unusable device) becomes an MCP error instead of
  a silent stale snapshot

The TUI still uses the non-blocking `send`, because it redraws on its own tick
and must never stall on a slow file.

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
