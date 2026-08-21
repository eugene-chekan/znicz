---
name: library-management
description: Scan, index, and search a local music library in znicz. Use when organizing files, finding tracks by artist or album, browsing album track lists, or checking library size.
---

# Library Management

## Status

Available. The library is a SQLite index of local files, filled by scanning folders.

## Tools

| Tool | Use it for |
|------|------------|
| `scan_library` | Index a folder (and its subfolders) |
| `search_library` | Find tracks by title, artist, or album |
| `get_track` | Full metadata for one file path |
| `browse_album` | Track list of an album, in track order |
| `list_albums` | Every album with a track count |
| `library_stats` | How many tracks and albums are indexed |
| `library_prune` | Drop entries whose files were deleted |

## Typical workflow

1. **Scan first.** `scan_library { "path": "/home/user/Music" }` walks the folder
   and reads tags. The report tells you how many files were added, updated,
   unchanged, or failed.
2. **Find something.** `search_library { "query": "portishead", "limit": 20 }`
   matches title, artist, album, and album artist. Matching is partial and
   case-insensitive.
3. **Play it.** Take `path` from a search result and pass it to `play`, or send
   several paths to `queue_add`.
4. **Browse an album.** `browse_album { "album": "Dummy" }` returns the tracks
   sorted by disc and track number.

## Notes

- Rescanning is cheap. Files whose modification time has not changed are
  skipped, so pointing `scan_library` at a large folder again is fast.
- `get_track` also works for files that were never scanned: it reads the tags
  straight from disk and sets `in_library: false`.
- If a tool answers "no music library configured", the user needs
  `[library].path` in `~/.config/znicz/config.toml`, or should run
  `znicz scan <dir>` once.
- Tracks with no tags fall back to the file name as the title.

## Related

- `audiophile-playback` — check sample rate and bit depth before playing
- `playlist-curation` — Phase 3, still stubbed
