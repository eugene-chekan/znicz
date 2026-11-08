# Library service and in-TUI scanning

**Date:** 2026-11-08
**Status:** Approved
**Crates:** `znicz-core`, `znicz-library`, `znicz-tui`, `znicz-mcp`, `znicz`

Part 3 of the five-part TUI rework (see the
[roadmap](../../../wiki/Plans/Roadmap.md)). Parts 1 and 2 shipped in 0.6.0 and
0.7.0.

## Problem

- The only ways to fill the library are `znicz scan <dir>` in a shell and the MCP
  `scan_library` tool. The TUI says "run `znicz scan <dir>`" and stops there.
- Scanned folders are not remembered, so every rescan means retyping paths.
- Nothing owns the library. The TUI, the MCP server, and the CLI each open
  `library.db` directly, and three of them write (`znicz scan`, MCP
  `scan_library`, MCP `library_prune`). They coordinate only through SQLite's
  WAL: there is no shared progress and nothing stops two scans at once.

## Goals

1. The `znicz player` process becomes the **single owner of library writes**.
   Reads stay direct.
2. Folders are remembered; one key rescans all of them.
3. An empty library opens with a folder prompt; progress shows in the TUI.
4. `znicz scan` and the MCP write tools go through the player.

## Non-goals

- A folder browser widget (a path prompt with completion is enough for a
  once-per-location action).
- Managing remembered folders (listing or forgetting them) in the TUI.
- Cancelling a running scan.
- Routing read queries over IPC.
- A separate library daemon process.
- Watching folders for changes.

## Library service

### Placement

| Crate | Adds |
| --- | --- |
| `znicz-core` | Protocol types `LibraryOp`, `LibraryStatus`, `ScanProgress`, `ScanSummary`; trait `LibraryHost`; optional host on the IPC server; client methods |
| `znicz-library` | `LibraryService: LibraryHost` (single write connection, `folders` table, one scan worker thread); a progress callback on `Library::scan` |
| `znicz` | `znicz player` builds the service from `[library].path` and passes it to the IPC server |

`znicz-core` does not depend on `znicz-library`; `znicz-library` already depends
on `znicz-core`.

### Protocol types (`znicz-core`)

```rust
pub enum LibraryOp {
    /// Remember the folder and scan it. The path is absolute (clients expand `~`).
    AddFolder { path: PathBuf },
    /// Scan every remembered folder that exists, then drop entries for deleted
    /// files under the folders that were scanned.
    RescanAll,
    /// Drop entries whose files no longer exist, anywhere in the library.
    Prune,
}

pub struct ScanProgress {
    pub folder: PathBuf,
    pub seen: usize,
    pub added: usize,
    pub updated: usize,
}

pub struct ScanSummary {
    pub seen: usize,
    pub added: usize,
    pub updated: usize,
    pub unchanged: usize,
    pub failed: usize,
    /// Remembered folders that did not exist and were skipped.
    pub skipped: Vec<PathBuf>,
    /// Entries removed by a prune, if one ran.
    pub removed: usize,
    /// Set when the job failed outright.
    pub error: Option<String>,
}

pub struct LibraryStatus {
    pub folders: Vec<PathBuf>,
    /// The job running now, if any.
    pub scanning: Option<ScanProgress>,
    /// Jobs waiting behind it.
    pub queued: usize,
    /// The most recent finished job.
    pub last: Option<ScanSummary>,
    /// Goes up by one each time a job finishes (success or failure).
    pub scans_finished: u64,
}

pub trait LibraryHost: Send + Sync {
    /// Validate and enqueue. Errors are immediate problems (not a folder, no
    /// library); scan failures are reported in `LibraryStatus::last`.
    fn apply(&self, op: LibraryOp) -> Result<()>;
    fn status(&self) -> LibraryStatus;
}
```

All types are `Serialize + Deserialize + Clone + Debug + PartialEq`.

### Behaviour

- **One job at a time.** Jobs run in order on one worker thread. `AddFolder` and
  `Prune` while a job runs are queued. `RescanAll` while a `RescanAll` is
  running or queued is ignored.
- **`AddFolder`**: errors at once if the path is not a directory
  (`not a folder: <path>`). Otherwise canonicalises it, stores it in `folders`
  (duplicates ignored by canonical path), and queues a scan of it.
- **`RescanAll`**: scans each remembered folder that exists. Missing folders are
  listed in `ScanSummary::skipped` and their tracks are **kept**. Afterwards it
  removes entries for files that no longer exist **only under the folders it
  scanned**.
- **`Prune`**: `Library::remove_missing()`, as `znicz scan --prune` does today.
- **Progress**: the scan calls back at least every 100 files and on each new
  folder; the service copies the numbers into `scanning`.
- **`folders` table** in `library.db`: `path TEXT PRIMARY KEY, added_at INTEGER`.
- **Idle exit**: the player does not idle-exit while `scanning` is `Some` or
  `queued > 0`.

### IPC

- New request `Library { token, op }` → `Ok` or `Err { message }`. Without a
  host: `Err("no music library configured")`.
- `IpcResponse::Ok` gains `library: Option<LibraryStatus>`, filled on every
  response when a host exists. `IpcClient` keeps the latest one;
  `IpcClient::library_status() -> Option<LibraryStatus>` returns it and
  `IpcClient::library(op) -> Result<()>` sends an op.
- Readers (TUI browse and search, MCP read tools) keep their own read
  connections.

## TUI

### First run

When the library has no artists and no tracks, the library pane shows a welcome
with the folder prompt open:

```
┌ Library ────────────────────────────────────────────────┐
│ No music yet. Which folder holds your music?            │
│                                                         │
│ folder: ~/Music█                                        │
│                                                         │
│ Tab completes · Enter scans · Esc skips                 │
└─────────────────────────────────────────────────────────┘
```

- Pre-fill: the platform music folder (`dirs::audio_dir()`), shown with `~`
  when it is under the home folder, if it exists; otherwise `~/`.
- `Esc` closes the prompt; the pane then reads
  `No music yet — press F to add a folder`. The prompt does not reopen by
  itself for the rest of the session.
- A library with no host (no `[library].path` and no data directory) keeps
  today's notice.

### Folder prompt

The same `LineEdit` as search.

- `Tab` completes the last path segment against the folders in its parent. One
  match: completed with a trailing `/`. Several: completed to their common
  prefix, and the message row lists up to five (`Classical · Classic Rock · …`).
  Folders starting with `.` only match when the segment starts with `.`.
- `Enter` expands a leading `~` and sends `AddFolder`. An error keeps the prompt
  open and shows `x not a folder: ~/Musc`.
- `Esc` cancels.

### Keys

Library focus, no modal open:

| Key | Action |
| --- | --- |
| `F` | Add a folder: open the folder prompt |
| `U` | Update: rescan every remembered folder (`RescanAll`) |

`r` still reloads the list from the database. The library also reloads itself
whenever `scans_finished` changes.

### Progress and results

- The message row shows, when no newer live message is showing and a scan is
  running: `⟳ scanning <folder> · 1,240 files · 37 new · 2 updated` (folder
  shortened with `~`, truncated from the left when long). Order of preference:
  live message, scan progress, unread marker, blank.
- The empty-library pane shows the same line in its body while scanning.
- When `scans_finished` changes, the TUI posts a message:
  - success: `scan done: 412 new, 3 updated, 0 failed` (plus `, 5 removed` when
    a prune ran);
  - with skipped folders: a **warning**,
    `scan done: … · skipped missing ~/nas/music`;
  - `error` set: an **error**, `scan failed: <reason>`.

### Footer and help

- `Library` and `LibraryTree` hint lists gain `F add folder` before
  `M messages`.
- `LIBRARY` table gains `F` — "add a music folder" and `U` — "rescan music
  folders".
- New `Context::FolderPrompt` with hints `Tab complete` · `Enter scan` ·
  `Esc cancel` (no `? more`).

## CLI

`znicz scan <dir>... [--prune]`:

- Autostarts the player (`ensure_player`), like the TUI.
- Sends `AddFolder` for each folder, then `Prune` with `--prune`.
- While stdout is a terminal, rewrites one progress line in place; otherwise
  prints nothing until the end.
- Waits until `scans_finished` has advanced past its starting value by the
  number of jobs it sent and nothing is scanning or queued, then prints the
  report in today's format (per folder: seen / added / updated / unchanged /
  failed; then removed).
- Exits non-zero on an immediate error or a `ScanSummary::error`.
- If the player cannot start, fails with that reason. There is no direct-write
  fallback.

## MCP

- `scan_library { path, wait? }`: sends `AddFolder`. With `wait` (default
  `true`) it waits as the CLI does and returns today's JSON report
  (`seen`, `added`, `updated`, `unchanged`, `failed`). With `wait: false` it
  returns `{"queued": true}`.
- `library_prune`: sends `Prune`, waits, returns `{"removed": N}`.
- New `library_status`: returns `LibraryStatus` as JSON.
- Read tools keep their direct read connection.

## Testing

- `znicz-library`: progress callback fires with growing counts; `folders`
  dedupes by canonical path; service: `AddFolder` scans and remembers, queues
  behind a running job, rejects a non-folder; `RescanAll` skips a missing
  folder, keeps its tracks, lists it in `skipped`, prunes only under scanned
  folders; `Prune` removes missing files; `scans_finished` advances once per
  job.
- `znicz-core` IPC: a fake `LibraryHost` answers `Library { op }`; `State`
  responses carry `library`; no host → error; idle exit waits for a running scan.
- `znicz-tui` (local engine with an in-process host): empty library opens the
  pre-filled prompt; `Esc` skips; `F` opens it; `U` sends `RescanAll`; `Tab`
  completion (single, common prefix, hidden); `~` expansion; progress on the
  message row; reload and message when `scans_finished` changes; footer context.
- `znicz-mcp`: `scan_library` (wait and `wait: false`), `library_prune`,
  `library_status` against a host.
- `znicz`: integration test runs `znicz scan <tempdir>` against an autostarted
  player and checks the report and the remembered folder.

## Delivery

- Bump `[workspace.package] version` to **0.8.0**.
- Wiki: `Architecture/Overview.md` (library writes go through the player),
  `Architecture/Library.md` (service, folders, rescan rules),
  `Architecture/MCP.md` (changed tools, `library_status`),
  `Architecture/TUI.md` (first run, `F` / `U`, progress), roadmap part 3 done.
- README: scanning from the TUI; `znicz scan` goes through the player.
