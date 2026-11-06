#!/usr/bin/env bash
#
# Sync the Superpowers skills into the directory Zed reads, then re-apply the
# Zed-specific wiring that a plain copy would lose.
#
# Re-run this after upgrading the Superpowers plugin.

set -euo pipefail

SOURCE=${SUPERPOWERS_SOURCE:-}
DEST=${SUPERPOWERS_DEST:-$HOME/.agents/skills}

usage() {
    cat <<'USAGE'
Usage: scripts/sync-superpowers.sh [--source DIR] [--dest DIR]

  --source DIR  Superpowers skills directory (default: the skills directory of
                the installed Claude Code plugin)
  --dest DIR    Where Zed discovers skills (default: ~/.agents/skills)

Environment: SUPERPOWERS_SOURCE, SUPERPOWERS_DEST

The copy itself is straight from the plugin. Everything the script writes
afterwards (references/zed-tools.md and the "In Zed" notes in
using-superpowers/SKILL.md) is owned by this script: local edits to those
parts are overwritten on every run.
USAGE
}

die() {
    echo "error: $*" >&2
    exit 1
}

while [ $# -gt 0 ]; do
    case "$1" in
        --source)
            [ $# -ge 2 ] || die "--source needs a directory"
            SOURCE=$2
            shift 2
            ;;
        --dest)
            [ $# -ge 2 ] || die "--dest needs a directory"
            DEST=$2
            shift 2
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        *)
            usage >&2
            die "unknown argument: $1"
            ;;
    esac
done

command -v python3 >/dev/null 2>&1 || die "python3 is required to patch using-superpowers/SKILL.md"

if [ -z "$SOURCE" ]; then
    SOURCE=$(python3 - "$HOME/.claude/plugins/installed_plugins.json" <<'PY'
import json
import sys
from pathlib import Path

registry = Path(sys.argv[1])
if registry.is_file():
    data = json.loads(registry.read_text(encoding="utf-8"))
    for name, installs in data.get("plugins", {}).items():
        if not name.startswith("superpowers@"):
            continue
        for entry in installs:
            skills = Path(entry["installPath"]) / "skills"
            if skills.is_dir():
                print(skills)
                raise SystemExit(0)
raise SystemExit(1)
PY
    ) || SOURCE=
fi

if [ -z "$SOURCE" ]; then
    cache_root="$HOME/.claude/plugins/cache/claude-plugins-official/superpowers"
    if [ -d "$cache_root" ]; then
        SOURCE=$(find "$cache_root" -mindepth 2 -maxdepth 2 -type d -name skills | sort -V | tail -n 1)
    fi
fi

[ -n "$SOURCE" ] || die "could not find the Superpowers plugin; pass --source DIR"
[ -f "$SOURCE/using-superpowers/SKILL.md" ] || die "not a Superpowers skills directory: $SOURCE"

mkdir -p "$DEST"
cp -R "$SOURCE"/. "$DEST"/

skill_dir="$DEST/using-superpowers"
[ -f "$skill_dir/SKILL.md" ] || die "copy did not produce $skill_dir/SKILL.md"

mkdir -p "$skill_dir/references"
cat > "$skill_dir/references/zed-tools.md" <<'MARKDOWN'
# Zed Tool Mapping

Skills use Claude Code tool names. When you encounter these in a skill, use your Zed equivalent:

| Skill references | Zed equivalent |
|-----------------|----------------|
| `Read` (file reading) | `read_file` (use `start_line`/`end_line` for large files) |
| `Write` (file creation) | `write_file` |
| `Edit` (file editing) | `edit_file` (pass a list of exact `old_text`/`new_text` edits) |
| `Bash` (run commands) | `terminal` (set the `cd` argument instead of `cd` in the command) |
| `Grep` (search file content) | `grep` |
| `Glob` (search files by name) | `find_path` |
| `LS` (list a directory) | `list_directory` |
| `Skill` tool (invoke a skill) | `skill` |
| `WebFetch` | `fetch` |
| `Task` tool (dispatch subagent) | `spawn_agent` |
| Multiple `Task` calls (parallel) | Multiple `spawn_agent` calls in one message |
| Follow-up on a subagent | `spawn_agent` with the returned `session_id` |
| `TodoWrite` (task tracking) | No equivalent - keep a markdown checklist in your reply and update it as you go |
| `AskUserQuestion` | No equivalent - ask in plain chat and stop |
| `EnterPlanMode` / `ExitPlanMode` | No equivalent - present the plan in chat and wait for approval |
| `WebSearch` | No equivalent - `fetch` a known URL, or ask the user for sources |
| `NotebookEdit` | No equivalent |
| Slash commands (`/superpowers:brainstorm`) | Zed's `/` menu exposes each skill by name (e.g. `/brainstorming`) |

## Additional Zed tools

These have no Claude Code counterpart and are often useful when executing a skill:

| Tool | Purpose |
|------|---------|
| `diagnostics` | Compiler/linter errors for a file or the whole project - prefer this over "it should compile" in `verification-before-completion` |
| `create_directory` | Create a directory tree |
| `move_path` / `copy_path` / `delete_path` | Move, copy, delete files or directories |
| `search_code`, `search_repositories`, `search_issues` | GitHub code and issue search |
| `create_pull_request`, `pull_request_read`, `merge_pull_request` | GitHub PR operations from `finishing-a-development-branch` |
| `assign_copilot_to_issue`, `request_copilot_review` | Delegate work or request an automated review |

## Behavioral differences

- **Skill discovery is per conversation.** Zed loads the skill catalog when a conversation starts, so a skill installed mid-conversation is only visible in the next one.
- **No session hooks.** Claude Code injects `using-superpowers` at session start via a hook. On Zed that context is missing, which is why `AGENTS.md` asks you to invoke `using-superpowers` first.
- **`skill` returns text, it does not execute anything.** Read the returned instructions, then use the tools above to act on them.
- **Subagents are one-shot.** `spawn_agent` returns only the agent's final message and sees nothing of your conversation, so the prompt must carry all file paths, requirements, and constraints.
- **Terminal output is capped on request, not on execution.** Long builds and test suites still run in full; pass `timeout_ms` and read `head_lines`/`tail_lines` rather than piping through `head`/`tail`.
- **Read-only git commands need `--no-pager`** (and `GIT_EDITOR=true` for anything that may open an editor) or the terminal can hang.
MARKDOWN

python3 - "$skill_dir/SKILL.md" <<'PY'
import sys
from pathlib import Path

path = Path(sys.argv[1])
text = path.read_text(encoding="utf-8")

zed_note = (
    "**In Zed:** Use the `skill` tool. Skills are discovered from "
    "`~/.agents/skills/` and `.agents/skills/` at the start of each "
    "conversation. See `references/zed-tools.md` for tool equivalents.\n\n"
)
platform_anchor = (
    "**In other environments:** Check your platform's documentation for how "
    "skills are loaded."
)
platform_old = (
    "see `references/copilot-tools.md` (Copilot CLI), "
    "`references/codex-tools.md` (Codex) for tool equivalents."
)
platform_new = (
    "see `references/copilot-tools.md` (Copilot CLI), "
    "`references/codex-tools.md` (Codex), and `references/zed-tools.md` (Zed) "
    "for tool equivalents."
)

if "**In Zed:**" not in text:
    if platform_anchor not in text:
        sys.exit(
            f"error: could not patch {path}: the 'In other environments' "
            "anchor is gone (upstream changed it?)"
        )
    text = text.replace(platform_anchor, zed_note + platform_anchor, 1)

text = text.replace(platform_old, platform_new, 1)
path.write_text(text, encoding="utf-8")
PY

skill_count=$(find "$DEST" -mindepth 1 -maxdepth 1 -type d | wc -l | tr -d ' ')
echo "Synced $skill_count skills from $SOURCE"
echo "Applied Zed wiring in $DEST/using-superpowers"
