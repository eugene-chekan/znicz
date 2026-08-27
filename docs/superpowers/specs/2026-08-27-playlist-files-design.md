# Playlist files

**Date:** 2026-08-27
**Status:** Approved
**Crates:** `znicz-core`, `znicz-library`, `znicz-tui`, `znicz-mcp`, `znicz`

## Problem

The queue is a list of paths you build by hand. There is no way to keep a
setlist, reload yesterday’s queue, or open an `.m3u` from disk. MCP tools
`import_playlist`, `save_playlist`, and `play_playlist` return “not implemented”.

## Goals

1. A playlist is a **file** of local paths (M3U / M3U8). No new database table.
2. **Play** has two explicit actions: **clear and play**, and **add to queue**.
   A later settings screen (`#6`) can remember a default for Enter. This slice
   does not read or write that setting.
3. Saved playlists live under the user data directory. Any `.m3u` on disk can
   still be imported by path.
4. CLI, MCP, and the TUI all do the same two play actions.
5. Missing files and stream URLs are skipped and reported; they do not abort
   the rest of the list.

## Non-goals

- PLS, XSPF
- HTTP / Icecast / HLS lines (Phase 4)
- Playlist editor, drag-reorder, or tags inside the `.m3u` (`#EXTINF` is ignored)
- Settings UI or `config.toml` play-action default (`#6`)
- Copying an imported file into the saved folder unless the user saves
- Changing decode, output, or the SQLite library schema

## Daily motion

You open `P`, pick `evening.m3u`, press Enter. The current queue is replaced,
and the first track starts. Or you press `a` and the same list is appended
while whatever is playing keeps playing. `w` writes the current queue out as a
new file in the playlists folder.

## File format

One **UTF-8** `.m3u` or `.m3u8` file.

| Line | Meaning |
| --- | --- |
| Empty | Skip |
| Starts with `#` | Comment. Includes `#EXTM3U` and `#EXTINF:…`. Skip |
| Contains `://` | URL. Skip, count as skipped |
| Anything else | A path |

Relative paths resolve against the **directory of the playlist file**. A leading
UTF-8 BOM is stripped. After resolve, a path whose file does not exist is
skipped and counted.

`parse(text, base_dir) -> LoadResult` where `LoadResult` is:

- `paths: Vec<PathBuf>` — existing local files, in file order
- `skipped: usize` — comments do not count; URLs and missing files do

If `paths` is empty, **do not** clear the queue. Toast or CLI error:
`playlist had no playable files`.

### Writing

`save` writes UTF-8, no BOM, one **absolute** path per line, nothing else.
The file name is `NAME.m3u` under the playlists directory. `NAME` is the stem
the user typed: trim whitespace, reject empty, `/`, `\`, and `..`. If the user
omits `.m3u`, add it. Overwrite without a second confirm; toast `saved NAME`.

## Where they live

```
Linux:   ~/.local/share/znicz/playlists/
Windows: %APPDATA%\znicz\playlists\
```

Same parent as `library.db`. Create the folder on first save. Listing the
overlay is `*.m3u` and `*.m3u8` in that folder, sorted by name. Import-by-path
does not have to live there.

`znicz_library::default_playlists_dir()` returns that folder (same data-dir
helper as the database). Parsing and writing live in `znicz-core` as
`playlist::{parse, write, LoadResult}` so tests do not need SQLite. The engine
does not grow new `Command` variants: callers issue `QueueClear` / `QueueAdd` /
`QueuePlayIndex(0)` themselves.

## Play actions

| Action | Commands | Playback |
| --- | --- | --- |
| **Clear and play** | `QueueClear`, then `QueueAdd(paths)`, then `QueuePlayIndex(0)` | Starts the first track |
| **Add to queue** | `QueueAdd(paths)` only | Does not start or stop playback |

Both use `send_blocking` in the TUI and MCP, in that order.

## TUI

`P` toggles `Modal::Playlists`, a centered overlay like devices.

```
┌ Playlists ─────────────────────────────────┐
│  evening                                   │
│  weekend-jazz                              │
│  (empty)  —  w to save the queue           │  ← when the folder has no files
└ Enter play · a add · w save · Esc close ───┘
```

| Key | When the list is showing | When naming a save |
| --- | --- | --- |
| Enter | Clear and play the highlighted file | Confirm the name and write |
| `a` | Add to queue | Type the letter `a` |
| `w` | Open the name prompt (`save: █`) | Type `w` |
| Esc | Close the overlay | Cancel the prompt, stay on the list |
| `P` | Close the overlay | Ignored as a letter in the name (prompt is plain text; `P` inserts `P`) |
| j / k / g / G / arrows | Move in the playlist list | — |
| `s` | Still **stop** (global, before modal keys) | Still stop |

`Char('P')` is unbound today (`p` / `N` are previous track). Opening the
overlay while Help is showing first closes Help, same as `,` and `i`.

List movement while this modal is open must move the **playlist** cursor, not
the library, same pattern as `Modal::Devices`.

Toasts: success `playing evening` / `added 12 tracks from evening`; warn if
some rows were skipped (`12 tracks, 2 skipped`). `w` with an empty queue does
not open the prompt: toast `queue is empty`.

## CLI

```
znicz playlist list
znicz playlist import FILE [--append]
znicz playlist save NAME
znicz playlist play NAME [--append]
```

A fresh process has no queue, so `play` and `import` **start the TUI** (same as
`znicz track.flac`) after loading the list:

- default: clear and play (the playlist is the queue, first track starts)
- `--append`: add to queue and **do not** auto-play (you land in the library
  with the drawer fillable; Space starts it)

`list` prints stems, one per line, and exits.

`save NAME` writes the queue of a **running** player. From the CLI there is no
queue, so it exits non-zero: `save the queue from the player (P then w, or MCP
save_playlist)`. Do not spawn an empty player to write an empty file.

Exit non-zero when the file is missing, the name is illegal, or nothing
playable loaded on a clear-and-play.

## MCP

Replace the three stubs. Add `list_playlists` so an agent can see names.

| Tool | Parameters | Behaviour |
| --- | --- | --- |
| `list_playlists` | none | `{ "playlists": ["evening", …] }` |
| `import_playlist` | `path: string`, `append: bool` (default false) | Load that file |
| `play_playlist` | `name: string`, `append: bool` (default false) | Load saved `name` |
| `save_playlist` | `name: string` | Write current queue; error if queue empty |

`append: false` is clear and play. `append: true` is add to queue.
Return JSON `{ "loaded": N, "skipped": M }` plus the player state via `apply`
after the commands. Empty playable list is an error, queue unchanged.

Update `znicz-mcp/skills/playlist-curation/SKILL.md` so it is no longer stubbed.

## Tests

- `znicz-core` unit tests: comments, relative paths, BOM, `://` skip, missing
  files, empty result, write then parse round-trip of absolute paths
- `znicz-tui` keys: `P` opens/closes; Esc closes; Enter on a fixture playlist
  issues clear-and-play (queue replaced); `a` appends; `s` still stops
- `znicz-tui` render: overlay draws at the usual size set; title `Playlists`
- MCP: `play_playlist` with `append: false` starts playback; `append: true`
  does not clear an existing queue
- Parser tests use temp dirs, not the user’s real playlists folder

## Docs

Same change as the code (wiki-sync):

- [Formats and metadata](../../../wiki/Domain/Formats-and-Metadata.md) — M3U
  behaviour, where files live, PLS/XSPF still later
- [TUI](../../../wiki/Architecture/TUI.md) — `P` overlay, Enter vs `a`
- [MCP](../../../wiki/Architecture/MCP.md) — tools no longer stubs
- [Roadmap](../../../wiki/Plans/Roadmap.md) Phase 3 — this spec, then **Done**
  when the code lands
- README essentials table: `P` playlists

## Risks

- `P` vs `p`: people may hit shift-p meaning previous. Help text must say
  `P` playlists and `p` previous. If that proves painful, bind playlists to
  another key in a follow-up, not in this spec.
- `QueueClear` does not stop the current decoder. Clear and play still
  `QueuePlayIndex(0)` immediately after, so the new first track opens. Do not
  add a new command for that.
