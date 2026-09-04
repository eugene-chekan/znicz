# Music library (`znicz-library`)

The library is a **searchable index of your local files**. It does not play
anything and it does not move or edit your music. It only records what is where.

## Why a separate crate

`znicz-core` is the audio engine. A database is a different concern, and SQLite
is compiled from C source, so keeping it apart means the audio engine stays small
and quick to build. The library depends on core (for tag reading), never the
other way round.

```
znicz-library ──uses──> znicz-core (tags)
      ▲
      └── znicz-tui, znicz-mcp, znicz (CLI)
```

## What gets stored

One row per audio file, in a table called `tracks`:

| Column | Where it comes from |
|--------|--------------------|
| `path` | The file location. Unique, so a file appears once |
| `title`, `artist`, `album`, `album_artist`, `genre`, `year` | Tags |
| `track_number`, `disc_number` | Tags, used for album ordering |
| `sample_rate`, `channels`, `bits_per_sample`, `duration_secs` | File header |
| `modified_secs` | File modification time, used to skip unchanged files |
| `title_folded`, `artist_folded`, `album_folded`, `album_artist_folded` | Unicode-lowercased copies of the tag fields, used only by search |

Files with no tags still get a row: the title falls back to the file name.

## Scanning

`Library::scan(root)` walks the folder with `walkdir`, keeps files whose
extension looks like audio, and reads each one with lofty.

Two things make a rescan cheap:

1. **Modification time check.** If the file's mtime matches the stored value, the
   file is skipped without opening it. Reading tags is the slow part.
2. **Upsert.** A changed file updates its existing row instead of creating a
   duplicate, because `path` is unique.

The scan returns a report: how many files were `seen`, `added`, `updated`,
`unchanged`, and `failed`.

Deleted files are not noticed during a scan — they are simply not walked. Use
`library_prune` (or `znicz scan --prune`) to drop rows whose file is gone.

### Why lofty and not Symphonia here

Symphonia has to look at the audio stream to answer questions, which is fine for
one track you are about to play but slow across thousands of files. Lofty reads
the header and the tag block only. Playback still uses Symphonia.

## Searching

There are two search APIs:

1. **`search`** (CLI `znicz search`, MCP `search_library`) — a flat list of
   **tracks** where the query matches title, artist, album, or album artist.
2. **`search_entities`** (TUI `/` search) — mixed **entity hits**: distinct
   artists, distinct albums, then tracks whose **title** matched. An artist-name
   query returns one artist row (not every track by that artist). Enter on an
   artist or album leaves search and focuses that entity in the artist-first
   browse view. `a` queues that entity; `A` queues title-matched tracks only.

Browse APIs for the TUI (not MCP/CLI):

- **`browse_artists`** — distinct browse artists, including a synthetic
  **Various Artists** root when compilation albums exist (tagged
  `album_artist = "Various Artists"`, or untagged multi-artist albums).
- **`albums_for_browse_artist`** — albums attributed to one browse artist.

Both use the same Unicode fold. Matching uses **Unicode-lowercased** copies of
the tag fields (`title_folded`, and the same for artist / album / album artist).
SQLite's own `LIKE` only folds ASCII `A–Z`, so a lowercase Cyrillic query would
otherwise miss capitalized tags.

```sql
-- flat search
WHERE title_folded LIKE '%query%' OR artist_folded LIKE '%query%' ...

-- entity tracks (title only)
WHERE title_folded LIKE '%query%'
```

The query is lowercased in Rust the same way (`str::to_lowercase`) before it is
escaped and bound. Display columns stay as tagged; only the folded copies are
for search. Opening an older database adds the folded columns and fills them
from the existing tags — no rescan required.

`%` and `_` are wildcards in SQL, so a query containing them is escaped first —
searching for "100%" looks for the literal text.

Flat track results are ordered by artist, album, disc, track number, then title.
Entity results are artists (by name), then albums (by name), then title hits in
the same track order.

Full-text search (SQLite FTS5) would rank results better and is a sensible later
upgrade. `LIKE` with an index is enough for a personal library.

## Where the database lives

| Platform | Default path |
|----------|--------------|
| Linux | `~/.local/share/znicz/library.db` (respects `XDG_DATA_HOME`) |
| Windows | `%APPDATA%\znicz\library.db` |

Override it in `~/.config/znicz/config.toml`:

```toml
[library]
path = "~/music-index.db"
```

The database uses **WAL mode**, so reading works while a scan is writing.

## Using it

From the command line:

```bash
znicz scan ~/Music          # index a folder
znicz scan ~/Music --prune  # index, then drop missing files
znicz search portishead     # find tracks
znicz albums                # list albums
```

From an AI agent over MCP: `scan_library`, `search_library`, `get_track`,
`browse_album`, `list_albums`, `library_stats`, `library_prune`. See
[MCP](MCP.md) and the bundled `library-management` skill.

`get_track` is deliberately forgiving: if a path is not indexed, it reads the
tags from disk and marks the answer `in_library: false`. An agent can ask about
any file without a scan first.

## Concurrency

The MCP server holds its library behind a mutex, because a scan writes and
SQLite connections are not shared across threads. The TUI holds its own
`Library` on the UI thread (`LibraryPane`) and queries it on keypress. Playback
runs on its own thread and never touches the database, so a long scan cannot
disturb audio.

## Extra reading

- [lofty](https://github.com/Serial-ATA/lofty-rs) — tag reading
- [rusqlite](https://docs.rs/rusqlite/) — SQLite for Rust
- [SQLite WAL mode](https://www.sqlite.org/wal.html)
- [walkdir](https://docs.rs/walkdir/) — recursive folder walking
