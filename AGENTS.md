# AGENTS.md

## Superpowers

If the Superpowers skills are installed (`~/.agents/skills/` globally or `.agents/skills/` in the repo), invoke the `using-superpowers` skill before starting a task. It explains which of the other skills to use and when.

- Designs go in `docs/superpowers/specs/` and implementation plans in `docs/superpowers/plans/`, matching the existing file naming.
- Track work with the plan's `- [ ]` checkboxes.
- On Zed, skill instructions written for Claude Code map onto different tool names — see `~/.agents/skills/using-superpowers/references/zed-tools.md`.
- After upgrading the Superpowers plugin, re-apply the Zed wiring with `scripts/sync-superpowers.sh`.

## Project rules

The always-apply rules in `.cursor/rules/` apply to every agent, not just Cursor:

- `finish-with-pr.mdc` — end a development cycle by pushing and opening a PR, not by asking how to integrate.
- `version-bump.mdc` — bump `[workspace.package] version` in the root `Cargo.toml` in the same change as shipped work.
- `wiki-sync.mdc` — update `wiki/` in the same change whenever behaviour, keys, crates, config, or MCP tools change.
