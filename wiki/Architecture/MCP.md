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

Later-phase tools (`search_library`, radio, …) exist but return “not implemented”.

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
