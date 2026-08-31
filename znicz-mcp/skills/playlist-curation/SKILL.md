---
name: playlist-curation
description: Import, save, play, rename, copy, and delete M3U playlists in znicz. Use when loading a setlist, restoring yesterday’s queue, writing the current queue to a file, or renaming a saved playlist.
---

# Playlist Curation

Playlists are **M3U / M3U8 files of local paths**. There is no playlist table in
SQLite. PLS, XSPF, and stream URLs are out of scope.

Saved files live beside the library database:
`~/.local/share/znicz/playlists/` on Linux, `%APPDATA%\znicz\playlists\` on
Windows. Import-by-path can point anywhere.

## Tools

| Tool | Parameters | What it does |
|------|------------|----------------|
| `list_playlists` | none | Names of saved `.m3u` / `.m3u8` files |
| `import_playlist` | `path`, `append` (default false) | Load that file |
| `play_playlist` | `name`, `append` (default false) | Load a saved name |
| `save_playlist` | `name` | Write the current queue; error if empty |
| `rename_playlist` | `name`, `new_name` | Rename a saved file |
| `copy_playlist` | `name`, `new_name` | Duplicate a saved file |
| `remove_playlist` | `name` | Delete a saved file |

`append: false` **clears the queue and starts the first track**.
`append: true` **adds the paths and does not start or stop playback**.

Each play/import result includes `loaded`, `skipped`, and `state`. Comments and
blank lines are ignored. Lines with `://` and missing files are skipped and
counted. If nothing playable remains, the tool errors and the queue is unchanged.

## Typical workflow

1. `list_playlists` to see saved names.
2. `play_playlist { "name": "evening" }` to replace the queue and start.
3. Or `play_playlist { "name": "evening", "append": true }` to keep what is
   already playing and add the list.
4. `import_playlist { "path": "/home/user/sets/club.m3u" }` for a file outside
   the saved folder.
5. After building a queue (`queue_add` / `queue_get`), `save_playlist { "name": "evening" }`
   writes `evening.m3u` (overwrite is allowed).
6. `rename_playlist { "name": "evening", "new_name": "night" }` renames that file.
7. `copy_playlist { "name": "night", "new_name": "night-backup" }` duplicates it.
8. `remove_playlist { "name": "night-backup" }` deletes that file.

## Notes

- Relative paths in a file resolve against that file’s directory.
- The TUI does the same two play actions: `P` then Enter (clear and play) or `a`
  (add). `n` saves. `e` renames. `c` copies. `d` deletes.
- Do not invent stream playback from `http://` lines; they are skipped.

## Related

- `library-management` — find tracks to put on a playlist
- `audiophile-playback` — check the device after the first track starts
