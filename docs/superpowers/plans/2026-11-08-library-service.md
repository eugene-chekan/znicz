# Library Service Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** The `znicz player` process becomes the single writer of `library.db` (remembered folders, scans, prunes), and the TUI, CLI, and MCP scan through it; the TUI gains a first-run folder prompt, `F` / `U` keys, and scan progress.

**Architecture:** `znicz-core` defines the protocol (`LibraryOp`, `LibraryStatus`, …), a `LibraryHost` trait, and IPC support (a `Library` request, status on every `Ok` response, `IpcClient: LibraryHost`). `znicz-library` implements `LibraryService: LibraryHost` with one worker thread that owns the write connection. `znicz player` plugs the service into its IPC server; `znicz scan`, MCP, and the TUI talk to it through `LibraryHost`.

**Tech Stack:** Rust 2021, rusqlite (WAL), serde, ratatui 0.30, crossterm 0.28, rmcp, cargo test.

**Spec:** [`docs/superpowers/specs/2026-11-08-library-service-design.md`](../specs/2026-11-08-library-service-design.md)

**Commands:** per-crate `cargo test -p <crate> [--lib|--test <name>] <filter>`; lint `cargo clippy --workspace --all-targets -- -D warnings`; format `cargo fmt --all -- --check`.

Every commit message ends with a blank line then `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.

---

### Task 1: Commit the plan

- [x] **Step 1:** `git add docs/superpowers/plans/2026-11-08-library-service.md && git commit -m "Add library service implementation plan"`

---

### Task 2: Protocol types, `LibraryHost`, and `wait_for_library` (`znicz-core`)

**Files:**
- Create: `znicz-core/src/library_host.rs`
- Modify: `znicz-core/src/lib.rs`, `znicz-core/src/error.rs`

- [ ] **Step 1: Write the module with its failing tests**

Create `znicz-core/src/library_host.rs`:

```rust
//! What a library writer offers to clients: the operations, the status they
//! read back, and the trait both the in-process service and the IPC client
//! implement.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::error::{Result, ZniczError};

/// A write to the music library.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum LibraryOp {
    /// Remember the folder and scan it. The path is absolute (clients expand `~`).
    AddFolder { path: PathBuf },
    /// Scan every remembered folder that exists, then drop entries for deleted
    /// files under the folders that were scanned.
    RescanAll,
    /// Drop entries whose files no longer exist, anywhere in the library.
    Prune,
}

/// The job running now.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScanProgress {
    pub folder: PathBuf,
    pub seen: usize,
    pub added: usize,
    pub updated: usize,
}

/// What a finished job did.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
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

/// Everything a client needs to show the library writer's state.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LibraryStatus {
    pub folders: Vec<PathBuf>,
    pub scanning: Option<ScanProgress>,
    pub queued: usize,
    pub last: Option<ScanSummary>,
    /// Goes up by one each time a job finishes, success or failure.
    pub scans_finished: u64,
}

impl LibraryStatus {
    /// A job is running or waiting.
    pub fn busy(&self) -> bool {
        self.scanning.is_some() || self.queued > 0
    }
}

/// The single writer of the library, in process or over IPC.
pub trait LibraryHost: Send + Sync {
    /// Validate and enqueue. Errors are immediate problems (not a folder, no
    /// library); scan failures are reported in `LibraryStatus::last`.
    fn apply(&self, op: LibraryOp) -> Result<()>;
    fn status(&self) -> LibraryStatus;
}

/// Poll `host` until at least `target` jobs have finished and nothing runs or
/// waits. `on_poll` sees every status read (for progress output).
///
/// `None` waits for as long as it takes. If other clients queue jobs at the
/// same time, the returned `last` is whichever job finished last.
pub fn wait_for_library<H: LibraryHost + ?Sized>(
    host: &H,
    target: u64,
    timeout: Option<Duration>,
    mut on_poll: impl FnMut(&LibraryStatus),
) -> Result<LibraryStatus> {
    let start = Instant::now();
    loop {
        let status = host.status();
        on_poll(&status);
        if status.scans_finished >= target && !status.busy() {
            return Ok(status);
        }
        if timeout.is_some_and(|t| start.elapsed() >= t) {
            return Err(ZniczError::Library(
                "timed out waiting for the library scan".into(),
            ));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// Finishes one job per status read.
    struct Ticking {
        finished: Mutex<u64>,
    }

    impl LibraryHost for Ticking {
        fn apply(&self, _op: LibraryOp) -> Result<()> {
            Ok(())
        }
        fn status(&self) -> LibraryStatus {
            let mut finished = self.finished.lock().unwrap();
            *finished += 1;
            LibraryStatus {
                scans_finished: *finished,
                ..LibraryStatus::default()
            }
        }
    }

    struct Stuck;

    impl LibraryHost for Stuck {
        fn apply(&self, _op: LibraryOp) -> Result<()> {
            Ok(())
        }
        fn status(&self) -> LibraryStatus {
            LibraryStatus {
                queued: 1,
                ..LibraryStatus::default()
            }
        }
    }

    #[test]
    fn waiting_returns_once_the_target_is_reached() {
        let host = Ticking {
            finished: Mutex::new(0),
        };
        let mut polls = 0;
        let status = wait_for_library(&host, 3, Some(Duration::from_secs(5)), |_| polls += 1)
            .expect("finished");
        assert_eq!(status.scans_finished, 3);
        assert_eq!(polls, 3);
    }

    #[test]
    fn waiting_gives_up_after_the_timeout() {
        let err = wait_for_library(&Stuck, 1, Some(Duration::from_millis(150)), |_| {})
            .expect_err("times out");
        assert!(err.to_string().contains("timed out"), "{err}");
    }

    #[test]
    fn ops_serialise_with_a_tag() {
        let json = serde_json::to_string(&LibraryOp::AddFolder {
            path: PathBuf::from("/music"),
        })
        .unwrap();
        assert_eq!(json, r#"{"op":"add_folder","path":"/music"}"#);
        let back: LibraryOp = serde_json::from_str(r#"{"op":"rescan_all"}"#).unwrap();
        assert_eq!(back, LibraryOp::RescanAll);
    }
}
```

In `znicz-core/src/lib.rs` add `pub mod library_host;` and
`pub use library_host::{wait_for_library, LibraryHost, LibraryOp, LibraryStatus, ScanProgress, ScanSummary};`.

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p znicz-core --lib library_host`
Expected: compile error, `no variant Library` on `ZniczError`.

- [ ] **Step 3: Add the error variant**

In `znicz-core/src/error.rs`, add to `ZniczError`:

```rust
    /// A library write was refused or failed; the message is shown as is.
    #[error("{0}")]
    Library(String),
```

If a `match` over `ZniczError` elsewhere becomes non-exhaustive, add the arm next to `Player`.

- [ ] **Step 4: Run and verify**

Run: `cargo test -p znicz-core --lib library_host` — PASS. Then `cargo test -p znicz-core`.

- [ ] **Step 5: Commit**

```bash
git add znicz-core/src
git commit -m "Add the library host protocol"
```

---

### Task 3: IPC — `Library` request, status on responses, idle wait, client host (`znicz-core`)

**Files:**
- Modify: `znicz-core/src/player/ipc.rs`

- [ ] **Step 1: Write the failing tests**

Add to the tests module of `znicz-core/src/player/ipc.rs`:

```rust
    use crate::library_host::{LibraryHost, LibraryOp, LibraryStatus, ScanProgress};

    #[derive(Default)]
    struct FakeHost {
        ops: Mutex<Vec<LibraryOp>>,
        busy: AtomicBool,
    }

    impl LibraryHost for FakeHost {
        fn apply(&self, op: LibraryOp) -> Result<()> {
            if op == LibraryOp::Prune {
                return Err(ZniczError::Library("refused".into()));
            }
            self.ops.lock().unwrap().push(op);
            Ok(())
        }
        fn status(&self) -> LibraryStatus {
            LibraryStatus {
                scans_finished: self.ops.lock().unwrap().len() as u64,
                scanning: self
                    .busy
                    .load(Ordering::SeqCst)
                    .then(ScanProgress::default),
                ..LibraryStatus::default()
            }
        }
    }

    #[test]
    fn library_ops_reach_the_host_and_status_comes_back() {
        let path = advertise_path();
        let (player, _thread) = spawn_player(AudioConfig::default());
        let host = Arc::new(FakeHost::default());
        let _server = IpcServer::start_with_library(player, &path, Duration::ZERO, host.clone())
            .expect("start ipc");
        let client = IpcClient::connect(&path, ClientRole::Agent).expect("connect");

        client
            .apply(LibraryOp::AddFolder {
                path: PathBuf::from("/music"),
            })
            .expect("apply");
        assert_eq!(host.ops.lock().unwrap().len(), 1);
        assert_eq!(client.status().scans_finished, 1);

        let err = client.apply(LibraryOp::Prune).expect_err("host refused");
        assert_eq!(err.to_string(), "refused");
    }

    #[test]
    fn without_a_host_library_requests_fail() {
        let path = advertise_path();
        let (player, _thread) = spawn_player(AudioConfig::default());
        let _server = IpcServer::start(player, &path).expect("start ipc");
        let client = IpcClient::connect(&path, ClientRole::Agent).expect("connect");
        let err = client.apply(LibraryOp::RescanAll).expect_err("no host");
        assert!(err.to_string().contains("no music library"), "{err}");
        assert_eq!(client.status(), LibraryStatus::default());
    }

    #[test]
    fn a_running_scan_holds_off_idle_exit() {
        let path = advertise_path();
        let (player, _thread) = spawn_player(AudioConfig::default());
        let host = Arc::new(FakeHost::default());
        host.busy.store(true, Ordering::SeqCst);
        let _server =
            IpcServer::start_with_library(player, &path, Duration::from_millis(80), host.clone())
                .expect("start ipc");
        thread::sleep(Duration::from_millis(500));
        assert!(path.exists(), "must not idle-exit while scanning");
        host.busy.store(false, Ordering::SeqCst);
        let start = Instant::now();
        while path.exists() && start.elapsed() < Duration::from_secs(3) {
            thread::sleep(Duration::from_millis(50));
        }
        assert!(!path.exists(), "idles out once the scan is done");
    }
```

(`Mutex`, `Arc`, `AtomicBool`, `Ordering`, `thread`, `Instant`, `PathBuf` come from the module's `use super::*;` — add explicit `use` lines in the tests module for any that are not in scope.)

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test -p znicz-core --lib ipc`
Expected: compile errors (`start_with_library`, `LibraryHost` for `IpcClient`).

- [ ] **Step 3: Implement**

In `znicz-core/src/player/ipc.rs`:

1. Imports: `use crate::library_host::{LibraryHost, LibraryOp, LibraryStatus};`.
2. `IpcRequest`: add `Library { token: String, op: LibraryOp },`.
3. `IpcResponse::Ok` becomes:

```rust
    Ok {
        state: Box<PlayerState>,
        /// The library writer's status, when this player hosts one.
        #[serde(default)]
        library: Option<LibraryStatus>,
    },
```

4. `Shared`: add `library: Option<Arc<dyn LibraryHost>>,`, and a helper:

```rust
impl Shared {
    fn ok(&self) -> IpcResponse {
        IpcResponse::Ok {
            state: Box::new(self.player.state()),
            library: self.library.as_ref().map(|host| host.status()),
        }
    }
}
```

Replace every `IpcResponse::Ok { state: Box::new(shared.player.state()) }` in `handle_session` with `shared.ok()`.

5. Constructors: change `start_inner` to take `library: Option<Arc<dyn LibraryHost>>` and store it in `Shared`. `start` / `start_with_idle` / `start_with_session` pass `None`. Add:

```rust
    /// A server whose clients can also write the library through `host`.
    pub fn start_with_library(
        player: PlayerHandle,
        advertise: impl Into<PathBuf>,
        idle: Duration,
        host: Arc<dyn LibraryHost>,
    ) -> Result<Self> {
        Self::start_inner(player, advertise, idle, None, Some(host))
    }

    /// The `znicz player` setup: session persistence plus an optional library writer.
    pub fn start_with_session_and_library(
        player: PlayerHandle,
        advertise: impl Into<PathBuf>,
        idle: Duration,
        session_path: impl Into<PathBuf>,
        debounce: Duration,
        host: Option<Arc<dyn LibraryHost>>,
    ) -> Result<Self> {
        let mut persister = SessionPersister::new(session_path, debounce);
        persister.sync_from_player(&player);
        Self::start_inner(player, advertise, idle, Some(persister), host)
    }
```

and make `start_with_session` call `start_with_session_and_library(…, None)`.

6. Token extraction: add `| IpcRequest::Library { token, .. }`.
7. Handle the request:

```rust
            IpcRequest::Library { op, .. } => {
                let response = match &shared.library {
                    None => IpcResponse::Err {
                        message: "no music library configured".into(),
                    },
                    Some(host) => match host.apply(op) {
                        Ok(()) => shared.ok(),
                        Err(e) => IpcResponse::Err {
                            message: e.to_string(),
                        },
                    },
                };
                let _ = write_json_line(&mut writer, &response);
            }
```

8. `should_idle_exit`: after the zero-idle check, add

```rust
    let scanning = shared
        .library
        .as_ref()
        .is_some_and(|host| host.status().busy());
```

and require `!scanning` together with `stopped && no_ui`.

9. Client: patterns `IpcResponse::Ok { .. }` / `IpcResponse::Ok { state, .. }` where they match today. Add:

```rust
impl LibraryHost for IpcClient {
    fn apply(&self, op: LibraryOp) -> Result<()> {
        match self.rpc(|token| IpcRequest::Library {
            token: token.to_string(),
            op: op.clone(),
        })? {
            IpcResponse::Ok { .. } => Ok(()),
            IpcResponse::Err { message } => Err(ZniczError::Library(message)),
        }
    }

    /// Fresh from the player; default when it hosts no library or is unreachable.
    fn status(&self) -> LibraryStatus {
        match self.rpc(|token| IpcRequest::State {
            token: token.to_string(),
        }) {
            Ok(IpcResponse::Ok { library, .. }) => library.unwrap_or_default(),
            _ => LibraryStatus::default(),
        }
    }
}
```

- [ ] **Step 4: Run and verify**

Run: `cargo test -p znicz-core` and `cargo clippy -p znicz-core --all-targets -- -D warnings`. Expected: PASS, clean.

- [ ] **Step 5: Commit**

```bash
git add znicz-core/src/player/ipc.rs
git commit -m "Carry library writes and status over the player IPC"
```

---

### Task 4: Scan progress, remembered folders, scoped prune (`znicz-library`)

**Files:**
- Modify: `znicz-library/src/scan.rs`, `znicz-library/src/store.rs`

- [ ] **Step 1: Write the failing tests**

Add to the tests module of `znicz-library/src/store.rs` (create `#[cfg(test)] mod tests { use super::*; … }` if absent):

```rust
    fn temp_db(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "znicz-store-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("library.db")
    }

    #[test]
    fn remembered_folders_survive_a_reopen_and_ignore_duplicates() {
        let db = temp_db("folders");
        {
            let mut library = Library::open(&db).unwrap();
            library.remember_folder(Path::new("/music/b")).unwrap();
            library.remember_folder(Path::new("/music/a")).unwrap();
            library.remember_folder(Path::new("/music/a")).unwrap();
        }
        let library = Library::open(&db).unwrap();
        assert_eq!(
            library.folders().unwrap(),
            [PathBuf::from("/music/a"), PathBuf::from("/music/b")]
        );
    }

    #[test]
    fn scoped_prune_only_touches_the_given_folders() {
        let mut library = Library::open_in_memory().unwrap();
        for path in ["/gone-a/x.flac", "/gone-b/y.flac"] {
            library
                .upsert_track_for_test(Path::new(path), "t", None, None, None)
                .unwrap();
        }
        let removed = library
            .remove_missing_under(&[PathBuf::from("/gone-a")])
            .unwrap();
        assert_eq!(removed, 1);
        assert!(library.get_by_path(Path::new("/gone-b/y.flac")).unwrap().is_some());
        assert!(library.get_by_path(Path::new("/gone-a/x.flac")).unwrap().is_none());
    }
```

Add to `znicz-library/src/scan.rs` (tests module at the bottom; create it if absent):

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_reports_growing_counts() {
        let dir = std::env::temp_dir().join(format!(
            "znicz-scan-progress-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        // Audio by extension, unreadable inside: seen and failed, never indexed.
        for i in 0..250 {
            std::fs::write(dir.join(format!("t{i}.flac")), b"not audio").unwrap();
        }
        let mut library = Library::open_in_memory().unwrap();
        let mut seen = Vec::new();
        let report = library
            .scan_with_progress(&dir, |r| seen.push(r.seen))
            .unwrap();
        assert_eq!(report.seen, 250);
        assert_eq!(seen.first(), Some(&0), "reports at the start");
        assert_eq!(seen.last(), Some(&250), "and at the end");
        assert!(seen.contains(&100) && seen.contains(&200), "{seen:?}");
        assert!(seen.windows(2).all(|w| w[0] <= w[1]), "{seen:?}");
        std::fs::remove_dir_all(&dir).ok();
    }
}
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test -p znicz-library --lib` — compile errors (`remember_folder`, `folders`, `remove_missing_under`, `scan_with_progress`).

- [ ] **Step 3: Implement**

`store.rs`:

- Append to `SCHEMA` (inside the string):

```sql
CREATE TABLE IF NOT EXISTS folders (
    path     TEXT PRIMARY KEY,
    added_at INTEGER NOT NULL
);
```

- In `impl Library`:

```rust
    /// Remember a scanned folder. Remembering it twice is a no-op.
    pub fn remember_folder(&mut self, path: &Path) -> Result<()> {
        self.conn.execute(
            "INSERT OR IGNORE INTO folders (path, added_at)
             VALUES (?1, CAST(strftime('%s', 'now') AS INTEGER))",
            params![path.to_string_lossy()],
        )?;
        Ok(())
    }

    /// Remembered folders, sorted.
    pub fn folders(&self) -> Result<Vec<PathBuf>> {
        let mut statement = self.conn.prepare("SELECT path FROM folders ORDER BY path")?;
        let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
        Ok(rows
            .collect::<rusqlite::Result<Vec<_>>>()?
            .into_iter()
            .map(PathBuf::from)
            .collect())
    }

    /// Like `remove_missing`, but only for files under one of `roots`.
    pub fn remove_missing_under(&mut self, roots: &[PathBuf]) -> Result<usize> {
        let paths: Vec<String> = {
            let mut statement = self.conn.prepare("SELECT path FROM tracks")?;
            let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
        };
        let mut removed = 0;
        let transaction = self.conn.transaction()?;
        for path in paths {
            let file = Path::new(&path);
            if roots.iter().any(|root| file.starts_with(root)) && !file.exists() {
                transaction.execute("DELETE FROM tracks WHERE path = ?1", params![path])?;
                removed += 1;
            }
        }
        transaction.commit()?;
        Ok(removed)
    }
```

`scan.rs`:

- Add `/// How often the progress callback runs, in audio files seen.\nconst PROGRESS_EVERY: usize = 100;`
- Replace `pub fn scan` with:

```rust
    /// Walk a folder and index every audio file inside it.
    ///
    /// Files that have not changed since the last scan are left alone, so
    /// rescanning a large library is cheap.
    pub fn scan(&mut self, root: &Path) -> Result<ScanReport> {
        self.scan_with_progress(root, |_| {})
    }

    /// `scan`, calling `progress` at the start, every `PROGRESS_EVERY` audio
    /// files, and at the end.
    pub fn scan_with_progress(
        &mut self,
        root: &Path,
        mut progress: impl FnMut(&ScanReport),
    ) -> Result<ScanReport> {
```

with the old body, plus: `progress(&report);` right after `let mut report = ScanReport::default();`; after the `match self.index_file(…)` block, `if report.seen % PROGRESS_EVERY == 0 { progress(&report); }`; and `progress(&report);` just before the final `Ok(report)`.

- [ ] **Step 4: Run and verify**

`cargo test -p znicz-library` — PASS.

- [ ] **Step 5: Commit**

```bash
git add znicz-library/src
git commit -m "Remember folders and report scan progress"
```

---

### Task 5: `LibraryService` (`znicz-library`)

**Files:**
- Create: `znicz-library/src/service.rs`
- Modify: `znicz-library/src/lib.rs`

- [ ] **Step 1: Write the service with failing tests**

Create `znicz-library/src/service.rs`:

```rust
//! The one writer of `library.db`: remembered folders and a scan queue, run on
//! a worker thread. `znicz player` hosts it; clients reach it as a
//! `LibraryHost`.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};

use znicz_core::{LibraryHost, LibraryOp, LibraryStatus, ScanProgress, ScanSummary, ZniczError};

use crate::error::Result;
use crate::scan::ScanReport;
use crate::store::Library;

enum Job {
    Add(PathBuf),
    RescanAll,
    Prune,
}

#[derive(Default)]
struct State {
    queue: VecDeque<Job>,
    status: LibraryStatus,
    rescan_running: bool,
    stop: bool,
}

struct Shared {
    state: Mutex<State>,
    wake: Condvar,
}

pub struct LibraryService {
    shared: Arc<Shared>,
}

impl LibraryService {
    /// Open the database for writing and start the worker.
    pub fn open(path: &Path) -> Result<Self> {
        let library = Library::open(path)?;
        let status = LibraryStatus {
            folders: library.folders()?,
            ..LibraryStatus::default()
        };
        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                status,
                ..State::default()
            }),
            wake: Condvar::new(),
        });
        let worker = shared.clone();
        std::thread::Builder::new()
            .name("znicz-library".into())
            .spawn(move || run(library, worker))?;
        Ok(Self { shared })
    }
}

impl Drop for LibraryService {
    fn drop(&mut self) {
        if let Ok(mut state) = self.shared.state.lock() {
            state.stop = true;
        }
        self.shared.wake.notify_all();
    }
}

impl LibraryHost for LibraryService {
    fn apply(&self, op: LibraryOp) -> znicz_core::Result<()> {
        let mut state = self
            .shared
            .state
            .lock()
            .map_err(|_| ZniczError::Library("library lock poisoned".into()))?;
        match op {
            LibraryOp::AddFolder { path } => {
                let folder = std::fs::canonicalize(&path)
                    .ok()
                    .filter(|p| p.is_dir())
                    .ok_or_else(|| {
                        ZniczError::Library(format!("not a folder: {}", path.display()))
                    })?;
                if !state.status.folders.contains(&folder) {
                    state.status.folders.push(folder.clone());
                    state.status.folders.sort();
                }
                state.queue.push_back(Job::Add(folder));
            }
            LibraryOp::RescanAll => {
                let pending = state.queue.iter().any(|job| matches!(job, Job::RescanAll));
                if !pending && !state.rescan_running {
                    state.queue.push_back(Job::RescanAll);
                }
            }
            LibraryOp::Prune => state.queue.push_back(Job::Prune),
        }
        state.status.queued = state.queue.len();
        self.shared.wake.notify_all();
        Ok(())
    }

    fn status(&self) -> LibraryStatus {
        self.shared
            .state
            .lock()
            .map(|state| state.status.clone())
            .unwrap_or_default()
    }
}

fn run(mut library: Library, shared: Arc<Shared>) {
    loop {
        let job = {
            let Ok(mut state) = shared.state.lock() else {
                return;
            };
            loop {
                if state.stop {
                    return;
                }
                if let Some(job) = state.queue.pop_front() {
                    state.status.queued = state.queue.len();
                    state.rescan_running = matches!(job, Job::RescanAll);
                    break job;
                }
                state = match shared.wake.wait(state) {
                    Ok(state) => state,
                    Err(_) => return,
                };
            }
        };
        let summary = run_job(&mut library, &shared, job);
        let Ok(mut state) = shared.state.lock() else {
            return;
        };
        state.status.scanning = None;
        state.rescan_running = false;
        state.status.last = Some(summary);
        state.status.scans_finished += 1;
    }
}

fn run_job(library: &mut Library, shared: &Shared, job: Job) -> ScanSummary {
    let mut summary = ScanSummary::default();
    let result = match job {
        Job::Add(folder) => library
            .remember_folder(&folder)
            .and_then(|()| scan_into(library, shared, &folder, &mut summary)),
        Job::RescanAll => rescan_all(library, shared, &mut summary),
        Job::Prune => library.remove_missing().map(|removed| summary.removed = removed),
    };
    if let Err(e) = result {
        tracing::warn!(error = %e, "library job failed");
        summary.error = Some(e.to_string());
    }
    summary
}

fn scan_into(
    library: &mut Library,
    shared: &Shared,
    folder: &Path,
    summary: &mut ScanSummary,
) -> Result<()> {
    set_progress(shared, folder, &ScanReport::default());
    let report = library.scan_with_progress(folder, |r| set_progress(shared, folder, r))?;
    summary.seen += report.seen;
    summary.added += report.added;
    summary.updated += report.updated;
    summary.unchanged += report.unchanged;
    summary.failed += report.failed;
    Ok(())
}

fn rescan_all(library: &mut Library, shared: &Shared, summary: &mut ScanSummary) -> Result<()> {
    let mut scanned = Vec::new();
    for folder in library.folders()? {
        if folder.is_dir() {
            scan_into(library, shared, &folder, summary)?;
            scanned.push(folder);
        } else {
            // An unplugged drive: keep its tracks, just say it was skipped.
            summary.skipped.push(folder);
        }
    }
    summary.removed = library.remove_missing_under(&scanned)?;
    Ok(())
}

fn set_progress(shared: &Shared, folder: &Path, report: &ScanReport) {
    if let Ok(mut state) = shared.state.lock() {
        state.status.scanning = Some(ScanProgress {
            folder: folder.to_path_buf(),
            seen: report.seen,
            added: report.added,
            updated: report.updated,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "znicz-service-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::canonicalize(&dir).unwrap()
    }

    fn music(root: &Path, name: &str, files: usize) -> PathBuf {
        let dir = root.join(name);
        std::fs::create_dir_all(&dir).unwrap();
        for i in 0..files {
            std::fs::write(dir.join(format!("t{i}.flac")), b"not audio").unwrap();
        }
        dir
    }

    fn wait(service: &LibraryService, target: u64) -> LibraryStatus {
        znicz_core::wait_for_library(service, target, Some(Duration::from_secs(10)), |_| {})
            .expect("job finished")
    }

    fn add(service: &LibraryService, path: &Path) {
        service
            .apply(LibraryOp::AddFolder {
                path: path.to_path_buf(),
            })
            .expect("add folder");
    }

    #[test]
    fn adding_a_folder_scans_and_remembers_it() {
        let root = temp_dir("add");
        let jazz = music(&root, "jazz", 3);
        let db = root.join("library.db");
        {
            let service = LibraryService::open(&db).unwrap();
            add(&service, &jazz);
            let status = wait(&service, 1);
            assert_eq!(status.folders, [jazz.clone()]);
            assert_eq!(status.last.as_ref().unwrap().seen, 3);
            assert!(status.scanning.is_none());
        }
        let reopened = LibraryService::open(&db).unwrap();
        assert_eq!(reopened.status().folders, [jazz]);
    }

    #[test]
    fn a_path_that_is_not_a_folder_is_refused() {
        let root = temp_dir("refuse");
        let service = LibraryService::open(&root.join("library.db")).unwrap();
        let err = service
            .apply(LibraryOp::AddFolder {
                path: root.join("nope"),
            })
            .expect_err("refused");
        assert!(err.to_string().starts_with("not a folder: "), "{err}");
        assert_eq!(service.status().scans_finished, 0);
    }

    #[test]
    fn each_queued_folder_finishes() {
        let root = temp_dir("queue");
        let service = LibraryService::open(&root.join("library.db")).unwrap();
        add(&service, &music(&root, "a", 1));
        add(&service, &music(&root, "b", 1));
        let status = wait(&service, 2);
        assert_eq!(status.folders.len(), 2);
    }

    #[test]
    fn rescan_skips_missing_folders_and_prunes_only_what_it_scanned() {
        let root = temp_dir("rescan");
        let db = root.join("library.db");
        let here = music(&root, "here", 0);
        let away = music(&root, "away", 0);
        let service = LibraryService::open(&db).unwrap();
        add(&service, &here);
        add(&service, &away);
        wait(&service, 2);

        // Rows for files that do not exist, under both folders.
        let mut reader = Library::open(&db).unwrap();
        for path in [here.join("deleted.flac"), away.join("kept.flac")] {
            reader
                .upsert_track_for_test(&path, "t", None, None, None)
                .unwrap();
        }
        std::fs::remove_dir_all(&away).unwrap();

        service.apply(LibraryOp::RescanAll).unwrap();
        let last = wait(&service, 3).last.unwrap();
        assert_eq!(last.skipped, [away.clone()]);
        assert_eq!(last.removed, 1);
        assert!(reader.get_by_path(&away.join("kept.flac")).unwrap().is_some());
        assert!(reader.get_by_path(&here.join("deleted.flac")).unwrap().is_none());
    }

    #[test]
    fn prune_drops_missing_files_anywhere() {
        let root = temp_dir("prune");
        let db = root.join("library.db");
        let service = LibraryService::open(&db).unwrap();
        Library::open(&db)
            .unwrap()
            .upsert_track_for_test(Path::new("/nowhere/x.flac"), "t", None, None, None)
            .unwrap();
        service.apply(LibraryOp::Prune).unwrap();
        assert_eq!(wait(&service, 1).last.unwrap().removed, 1);
    }
}
```

In `znicz-library/src/lib.rs`: `mod service;` and `pub use service::LibraryService;`.

- [ ] **Step 2: Run the tests**

Run: `cargo test -p znicz-library --lib service`. Write the tests first with an empty `impl LibraryHost` (`todo!()` bodies) if you want to see them fail; the required evidence is that each test exercises the behaviour named in its title. Expected after implementation: PASS.

- [ ] **Step 3: Lint and commit**

```bash
cargo clippy -p znicz-library --all-targets -- -D warnings
git add znicz-library/src
git commit -m "Add the library service: one writer with a scan queue"
```

---

### Task 6: `znicz player` hosts the service; `znicz scan` goes through it (`znicz`)

**Files:**
- Modify: `znicz/src/main.rs`
- Create: `znicz/tests/scan_via_player.rs`

- [ ] **Step 1: Write the failing integration test**

Create `znicz/tests/scan_via_player.rs`:

```rust
//! `znicz scan` runs through the player process, which remembers the folder.

use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant};

use znicz_core::{ClientRole, IpcClient, LibraryHost};

struct Isolated {
    dir: PathBuf,
}

impl Isolated {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!(
            "znicz-scan-cli-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(dir.join("music")).unwrap();
        fs::write(
            dir.join("config.toml"),
            format!(
                "[library]\npath = \"{}\"\n",
                dir.join("library.db").display().to_string().replace('\\', "/")
            ),
        )
        .unwrap();
        Self { dir }
    }

    fn ipc(&self) -> PathBuf {
        self.dir.join("ipc.toml")
    }

    fn znicz(&self) -> Command {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_znicz"));
        cmd.arg("--config")
            .arg(self.dir.join("config.toml"))
            .env("ZNICZ_IPC_PATH", self.ipc())
            .env("ZNICZ_SESSION_PATH", self.dir.join("session.toml"))
            .env("XDG_DATA_HOME", self.dir.join("data"));
        cmd
    }
}

impl Drop for Isolated {
    fn drop(&mut self) {
        if let Ok(client) = IpcClient::connect(self.ipc(), ClientRole::Agent) {
            let _ = client.shutdown();
        }
        let _ = fs::remove_dir_all(&self.dir);
    }
}

#[test]
fn scan_goes_through_the_player_and_remembers_the_folder() {
    let iso = Isolated::new();
    let music = fs::canonicalize(iso.dir.join("music")).unwrap();

    let output = iso.znicz().arg("scan").arg(&music).output().expect("run scan");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success(), "{stdout}\n{}", String::from_utf8_lossy(&output.stderr));
    assert!(stdout.contains("0 files: 0 added"), "{stdout}");

    let start = Instant::now();
    let client = loop {
        if let Ok(client) = IpcClient::connect(iso.ipc(), ClientRole::Agent) {
            break client;
        }
        assert!(start.elapsed() < Duration::from_secs(10), "player is up");
        std::thread::sleep(Duration::from_millis(50));
    };
    assert_eq!(client.status().folders, [music]);
}

#[test]
fn scanning_a_missing_folder_fails() {
    let iso = Isolated::new();
    let output = iso
        .znicz()
        .arg("scan")
        .arg(iso.dir.join("nope"))
        .output()
        .expect("run scan");
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("not a folder"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
```

Check `--config` is a global flag accepted before the subcommand (it is `cli.config` in `main.rs`); if it must follow the subcommand, move `.arg("--config")…` after `.arg("scan")` in both tests.

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p znicz --test scan_via_player`
Expected: FAIL (the player hosts no library; `status().folders` is empty).

- [ ] **Step 3: Host the service in `znicz player`**

In `znicz/src/main.rs`:

- Change `run_player_daemon(audio_config: AudioConfig, idle_secs: u64)` to also take `library_path: Option<PathBuf>`, and the call site to `run_player_daemon(audio_config, config.player.idle_secs.unwrap_or(900), library_path.clone())`.
- In `run_player_daemon`, before starting the server:

```rust
    // A broken library must not stop playback; library writes then report it.
    let library: Option<std::sync::Arc<dyn znicz_core::LibraryHost>> =
        library_path.and_then(|path| match znicz_library::LibraryService::open(&path) {
            Ok(service) => Some(std::sync::Arc::new(service) as _),
            Err(e) => {
                tracing::warn!(path = %path.display(), error = %e, "library service unavailable");
                None
            }
        });
```

and replace `IpcServer::start_with_session(player.clone(), ipc, idle, session_path()?, SESSION_SAVE_DEBOUNCE)` with `IpcServer::start_with_session_and_library(player.clone(), ipc, idle, session_path()?, SESSION_SAVE_DEBOUNCE, library)`.

- [ ] **Step 4: `znicz scan` through the player**

Replace `fn scan_library` with:

```rust
fn scan_library(
    library_path: Option<PathBuf>,
    dirs: &[PathBuf],
    prune: bool,
    device: Option<&str>,
    config: Option<&Path>,
) -> color_eyre::Result<()> {
    if dirs.is_empty() && !prune {
        println!("nothing to do: pass one or more folders, or --prune");
        return Ok(());
    }

    let player = connect_agent(device, config)?;
    let live = std::io::IsTerminal::is_terminal(&std::io::stdout());

    for dir in dirs {
        let folder = std::fs::canonicalize(dir)
            .ok()
            .filter(|p| p.is_dir())
            .ok_or_else(|| color_eyre::eyre::eyre!("not a folder: {}", dir.display()))?;
        println!("scanning {} ...", folder.display());
        let report = run_library_job(&player, LibraryOp::AddFolder { path: folder }, live)?;
        println!(
            "  {} files: {} added, {} updated, {} unchanged, {} failed",
            report.seen, report.added, report.updated, report.unchanged, report.failed
        );
    }

    if prune {
        let report = run_library_job(&player, LibraryOp::Prune, live)?;
        println!("removed {} missing entries", report.removed);
    }

    let library = open_library(library_path)?;
    println!("library now holds {} tracks", library.track_count()?);
    Ok(())
}

/// Send one library job to the player and wait for it, drawing progress on a terminal.
fn run_library_job(
    player: &IpcClient,
    op: LibraryOp,
    live: bool,
) -> color_eyre::Result<ScanSummary> {
    let start = player.status().scans_finished;
    player.apply(op).map_err(|e| color_eyre::eyre::eyre!("{e}"))?;
    let status = wait_for_library(player, start + 1, None, |status| {
        if let (true, Some(p)) = (live, status.scanning.as_ref()) {
            print!(
                "\r  {} files · {} new · {} updated",
                p.seen, p.added, p.updated
            );
            let _ = std::io::Write::flush(&mut std::io::stdout());
        }
    })
    .map_err(|e| color_eyre::eyre::eyre!("{e}"))?;
    if live {
        print!("\r\x1b[2K");
    }
    let summary = status.last.unwrap_or_default();
    if let Some(error) = &summary.error {
        color_eyre::eyre::bail!("scan failed: {error}");
    }
    Ok(summary)
}
```

Imports: `use znicz_core::{wait_for_library, LibraryHost, LibraryOp, ScanSummary};` (merge with the existing `znicz_core` import). Update the call site to `scan_library(library_path, &dirs, prune, cli.device.as_deref(), cli.config.as_deref())` — use whatever device/config variables the other `connect_agent` call sites use in `main`.

- [ ] **Step 5: Run and verify**

Run: `cargo test -p znicz` and `cargo clippy -p znicz --all-targets -- -D warnings`. Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add znicz/src/main.rs znicz/tests/scan_via_player.rs
git commit -m "Host the library service in the player and scan through it"
```

---

### Task 7: MCP tools through the service (`znicz-mcp`)

**Files:**
- Modify: `znicz-mcp/src/server.rs`

- [ ] **Step 1: Write the failing tests**

In the tests module of `znicz-mcp/src/server.rs`, add a helper next to `hosted_player()`:

```rust
    /// A player that hosts a library service over a temp database, plus an MCP
    /// server reading the same database.
    fn hosted_with_library(name: &str) -> (IpcServer, ZniczMcpServer, PathBuf) {
        let root = std::env::temp_dir().join(format!(
            "znicz-mcp-lib-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let db = root.join("library.db");
        let advertise = root.join("ipc.toml");
        let (player, _thread) = znicz_core::spawn_player(znicz_core::AudioConfig::default());
        let service = std::sync::Arc::new(znicz_library::LibraryService::open(&db).unwrap());
        let host = IpcServer::start_with_library(player, &advertise, Duration::ZERO, service)
            .expect("start ipc");
        let client = IpcClient::connect(&advertise, ClientRole::Agent).expect("connect");
        let server = ZniczMcpServer::with_library(client, Vec::new(), Library::open(&db).unwrap());
        (host, server, root)
    }

    #[test]
    fn scan_library_without_waiting_reports_queued() {
        let (_host, server, root) = hosted_with_library("nowait");
        let music = root.join("music");
        std::fs::create_dir_all(&music).unwrap();
        let result = server
            .scan_library(Parameters(ScanParams {
                path: music.to_string_lossy().into_owned(),
                wait: Some(false),
            }))
            .expect("scan");
        let json: serde_json::Value = serde_json::from_str(&result_text(&result)).unwrap();
        assert_eq!(json["queued"], true);
    }

    #[test]
    fn library_status_lists_remembered_folders() {
        let (_host, server, root) = hosted_with_library("status");
        let music = std::fs::canonicalize({
            let m = root.join("music");
            std::fs::create_dir_all(&m).unwrap();
            m
        })
        .unwrap();
        server
            .scan_library(Parameters(ScanParams {
                path: music.to_string_lossy().into_owned(),
                wait: None,
            }))
            .expect("scan");
        let status = server.library_status().expect("status");
        let json: serde_json::Value = serde_json::from_str(&result_text(&status)).unwrap();
        assert_eq!(json["folders"][0], music.to_string_lossy().as_ref());
    }

    #[test]
    fn library_prune_reports_removed_entries() {
        let (_host, server, root) = hosted_with_library("prune");
        Library::open(&root.join("library.db"))
            .unwrap()
            .upsert_track_for_test(std::path::Path::new("/nowhere/x.flac"), "t", None, None, None)
            .unwrap();
        let result = server.library_prune().expect("prune");
        let json: serde_json::Value = serde_json::from_str(&result_text(&result)).unwrap();
        assert_eq!(json["removed"], 1);
    }
```

Update the existing scan test (the one around the `scan_library(Parameters(ScanParams { path: … }))` call that asserts `report["added"] == 1`): build its server with `hosted_with_library(…)` instead of an in-memory library, keep its fixture folder and assertions, and add `wait: None` to its `ScanParams`.

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test -p znicz-mcp` — compile errors (`wait` field, `library_status`).

- [ ] **Step 3: Implement**

- `ScanParams`: add

```rust
    /// Wait for the scan to finish (default) or return as soon as it is queued.
    #[serde(default)]
    pub wait: Option<bool>,
```

- Replace the bodies:

```rust
    #[tool(description = "Scan a folder into the music library (through the player, which remembers the folder)")]
    fn scan_library(
        &self,
        Parameters(params): Parameters<ScanParams>,
    ) -> Result<rmcp::model::CallToolResult, McpError> {
        let path = std::fs::canonicalize(&params.path)
            .map_err(|_| McpError::invalid_params(format!("not a folder: {}", params.path), None))?;
        let summary = self.run_library_job(LibraryOp::AddFolder { path }, params.wait.unwrap_or(true))?;
        match summary {
            None => Self::json_result(&serde_json::json!({ "queued": true })),
            Some(s) => Self::json_result(&serde_json::json!({
                "seen": s.seen,
                "added": s.added,
                "updated": s.updated,
                "unchanged": s.unchanged,
                "failed": s.failed,
            })),
        }
    }
```

```rust
    #[tool(description = "Drop library entries whose files no longer exist")]
    fn library_prune(&self) -> Result<rmcp::model::CallToolResult, McpError> {
        let summary = self.run_library_job(LibraryOp::Prune, true)?.unwrap_or_default();
        Self::json_result(&serde_json::json!({ "removed": summary.removed }))
    }

    #[tool(description = "Remembered music folders, the running scan's progress, and the last result")]
    fn library_status(&self) -> Result<rmcp::model::CallToolResult, McpError> {
        Self::json_result(&LibraryHost::status(&self.player))
    }
```

and a private helper in a plain `impl ZniczMcpServer` block:

```rust
    /// Send a library job to the player; with `wait`, return what it did.
    fn run_library_job(
        &self,
        op: LibraryOp,
        wait: bool,
    ) -> Result<Option<ScanSummary>, McpError> {
        let start = LibraryHost::status(&self.player).scans_finished;
        LibraryHost::apply(&self.player, op)
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        if !wait {
            return Ok(None);
        }
        let status = wait_for_library(&self.player, start + 1, None, |_| {})
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        let summary = status.last.unwrap_or_default();
        if let Some(error) = &summary.error {
            return Err(McpError::internal_error(format!("scan failed: {error}"), None));
        }
        Ok(Some(summary))
    }
```

Imports: `use znicz_core::{wait_for_library, LibraryHost, LibraryOp, ScanSummary};`. If `json_result` requires `Serialize` and `LibraryStatus` is serialisable, `library_status` works as written. Keep the old `library_prune` tool name and description semantics. Remove now-unused imports.

- [ ] **Step 4: Run and verify**

`cargo test -p znicz-mcp` and clippy. Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add znicz-mcp/src
git commit -m "Scan and prune through the player from MCP; add library_status"
```

---

### Task 8: Folder path helpers and scan texts (`znicz-tui`, pure)

**Files:**
- Create: `znicz-tui/src/folder_complete.rs`, `znicz-tui/src/scan_status.rs`
- Modify: `znicz-tui/src/lib.rs`, `znicz-tui/Cargo.toml`

- [ ] **Step 1: Write both modules with tests**

`znicz-tui/Cargo.toml` `[dependencies]`: add `dirs.workspace = true`.

Create `znicz-tui/src/folder_complete.rs`:

```rust
//! Typing a folder path: `~` handling and `Tab` completion against the disk.

use std::path::{Path, PathBuf};

/// The result of completing what was typed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Completion {
    /// The new prompt text.
    pub text: String,
    /// Folder names that matched, sorted (empty when none did).
    pub matches: Vec<String>,
}

/// `~` and `~/…` become paths under `home`; anything else is taken as is.
pub fn expand(input: &str, home: &Path) -> PathBuf {
    if input == "~" {
        home.to_path_buf()
    } else if let Some(rest) = input.strip_prefix("~/") {
        home.join(rest)
    } else {
        PathBuf::from(input)
    }
}

/// `path` with the home folder shown as `~`.
pub fn tilde(path: &Path, home: &Path) -> String {
    match path.strip_prefix(home) {
        Ok(rest) if rest.as_os_str().is_empty() => "~".to_string(),
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => path.display().to_string(),
    }
}

/// Complete the last path segment against the folders in its parent.
///
/// One match completes it with a trailing `/`; several complete their common
/// prefix. Hidden folders only match a segment that starts with `.`.
pub fn complete(input: &str, home: &Path) -> Completion {
    let cut = input
        .rfind(['/', std::path::MAIN_SEPARATOR])
        .map_or(0, |i| i + 1);
    let (parent_text, partial) = input.split_at(cut);
    let parent = if parent_text.is_empty() {
        PathBuf::from(".")
    } else {
        expand(parent_text, home)
    };
    let mut names: Vec<String> = std::fs::read_dir(&parent)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|entry| entry.path().is_dir())
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|name| name.starts_with(partial))
        .filter(|name| partial.starts_with('.') || !name.starts_with('.'))
        .collect();
    names.sort();
    let text = match names.as_slice() {
        [] => input.to_string(),
        [one] => format!("{parent_text}{one}/"),
        many => format!("{parent_text}{}", common_prefix(many)),
    };
    Completion {
        text,
        matches: names,
    }
}

fn common_prefix(names: &[String]) -> String {
    let first = &names[0];
    let mut len = first.chars().count();
    for name in &names[1..] {
        len = len.min(
            first
                .chars()
                .zip(name.chars())
                .take_while(|(a, b)| a == b)
                .count(),
        );
    }
    first.chars().take(len).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree() -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "znicz-complete-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        for dir in ["Music/Classical", "Music/Classic Rock", "Music/Jazz", "Music/.hidden"] {
            std::fs::create_dir_all(root.join(dir)).unwrap();
        }
        std::fs::write(root.join("Music/Clash.txt"), b"a file").unwrap();
        root
    }

    #[test]
    fn tilde_expands_against_home_and_back() {
        let home = Path::new("/home/u");
        assert_eq!(expand("~", home), PathBuf::from("/home/u"));
        assert_eq!(expand("~/Music", home), PathBuf::from("/home/u/Music"));
        assert_eq!(expand("/srv/m", home), PathBuf::from("/srv/m"));
        assert_eq!(tilde(Path::new("/home/u/Music"), home), "~/Music");
        assert_eq!(tilde(Path::new("/srv/m"), home), "/srv/m");
    }

    #[test]
    fn a_single_match_completes_with_a_slash() {
        let home = tree();
        let done = complete("~/Music/Ja", &home);
        assert_eq!(done.text, "~/Music/Jazz/");
        assert_eq!(done.matches, ["Jazz"]);
    }

    #[test]
    fn several_matches_complete_their_common_prefix_and_skip_files() {
        let home = tree();
        let done = complete("~/Music/Cl", &home);
        assert_eq!(done.text, "~/Music/Classic");
        assert_eq!(done.matches, ["Classic Rock", "Classical"]);
    }

    #[test]
    fn hidden_folders_need_a_dot() {
        let home = tree();
        assert!(!complete("~/Music/", &home)
            .matches
            .contains(&".hidden".to_string()));
        assert_eq!(complete("~/Music/.h", &home).text, "~/Music/.hidden/");
    }

    #[test]
    fn no_match_leaves_the_text_alone() {
        let home = tree();
        let done = complete("~/Music/Zz", &home);
        assert_eq!(done.text, "~/Music/Zz");
        assert!(done.matches.is_empty());
    }
}
```

(If `"Classic Rock" < "Classical"` does not hold under `sort()` — it does, since `' '` sorts before `'a'` — adjust the expected order to the sorted order.)

Create `znicz-tui/src/scan_status.rs`:

```rust
//! The words the TUI shows for a library scan.

use std::path::Path;

use znicz_core::{ScanProgress, ScanSummary};

use crate::folder_complete::tilde;
use crate::format;
use crate::toast::Level;

/// `1240` → `1,240`.
pub fn grouped(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// `⟳ scanning ~/Music/Jazz · 1,240 files · 37 new · 2 updated`, with the folder
/// cut from the left so the counts stay visible in `width` columns.
pub fn progress_text(progress: &ScanProgress, home: Option<&Path>, width: usize) -> String {
    let folder = match home {
        Some(home) => tilde(&progress.folder, home),
        None => progress.folder.display().to_string(),
    };
    let head = "⟳ scanning ";
    let tail = format!(
        " · {} files · {} new · {} updated",
        grouped(progress.seen),
        grouped(progress.added),
        grouped(progress.updated)
    );
    let room = width.saturating_sub(head.chars().count() + tail.chars().count());
    format::truncate(&format!("{head}{}{tail}", cut_left(&folder, room)), width)
}

fn cut_left(text: &str, width: usize) -> String {
    let count = text.chars().count();
    if count <= width {
        return text.to_string();
    }
    if width == 0 {
        return String::new();
    }
    let keep: String = text.chars().skip(count - (width - 1)).collect();
    format!("…{keep}")
}

/// The message posted when a job finishes.
pub fn finish_message(summary: &ScanSummary, home: Option<&Path>) -> (Level, String) {
    if let Some(error) = &summary.error {
        return (Level::Error, format!("scan failed: {error}"));
    }
    let mut text = format!(
        "scan done: {} new, {} updated, {} failed",
        grouped(summary.added),
        grouped(summary.updated),
        grouped(summary.failed)
    );
    if summary.removed > 0 {
        text.push_str(&format!(", {} removed", grouped(summary.removed)));
    }
    if summary.skipped.is_empty() {
        return (Level::Success, text);
    }
    let skipped: Vec<String> = summary
        .skipped
        .iter()
        .map(|p| match home {
            Some(home) => tilde(p, home),
            None => p.display().to_string(),
        })
        .collect();
    text.push_str(&format!(" · skipped missing {}", skipped.join(", ")));
    (Level::Warn, text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn progress() -> ScanProgress {
        ScanProgress {
            folder: PathBuf::from("/home/u/Music/Jazz"),
            seen: 1240,
            added: 37,
            updated: 2,
        }
    }

    #[test]
    fn numbers_are_grouped_in_thousands() {
        assert_eq!(grouped(7), "7");
        assert_eq!(grouped(1240), "1,240");
        assert_eq!(grouped(1_234_567), "1,234,567");
    }

    #[test]
    fn progress_reads_naturally_when_it_fits() {
        assert_eq!(
            progress_text(&progress(), Some(Path::new("/home/u")), 80),
            "⟳ scanning ~/Music/Jazz · 1,240 files · 37 new · 2 updated"
        );
    }

    #[test]
    fn a_long_folder_is_cut_from_the_left() {
        let text = progress_text(&progress(), Some(Path::new("/home/u")), 50);
        assert_eq!(text, "⟳ scanning …azz · 1,240 files · 37 new · 2 updated");
        assert_eq!(text.chars().count(), 50);
    }

    #[test]
    fn finish_messages_follow_the_outcome() {
        let home = Some(Path::new("/home/u"));
        let ok = ScanSummary {
            added: 412,
            updated: 3,
            ..ScanSummary::default()
        };
        assert_eq!(
            finish_message(&ok, home),
            (Level::Success, "scan done: 412 new, 3 updated, 0 failed".into())
        );
        let pruned = ScanSummary {
            removed: 5,
            ..ScanSummary::default()
        };
        assert!(finish_message(&pruned, home).1.ends_with(", 5 removed"));
        let skipped = ScanSummary {
            added: 1,
            skipped: vec![PathBuf::from("/home/u/nas/music")],
            ..ScanSummary::default()
        };
        assert_eq!(
            finish_message(&skipped, home),
            (
                Level::Warn,
                "scan done: 1 new, 0 updated, 0 failed · skipped missing ~/nas/music".into()
            )
        );
        let failed = ScanSummary {
            error: Some("disk gone".into()),
            ..ScanSummary::default()
        };
        assert_eq!(
            finish_message(&failed, home),
            (Level::Error, "scan failed: disk gone".into())
        );
    }
}
```

`Level` must derive `PartialEq` (it does) for the tuple comparisons.

In `lib.rs` add `pub mod folder_complete;` and `pub mod scan_status;`.

- [ ] **Step 2: Run and verify**

`cargo test -p znicz-tui --lib folder_complete` and `--lib scan_status` — write the tests first and see them fail to compile, then PASS.

- [ ] **Step 3: Commit**

```bash
git add znicz-tui/Cargo.toml znicz-tui/src Cargo.lock
git commit -m "Add folder completion and scan wording for the TUI"
```

---

### Task 9: TUI state, keys, and footer for scanning (`znicz-tui`)

**Files:**
- Modify: `znicz-tui/src/app.rs`, `znicz-tui/src/keys.rs`, `znicz-tui/src/library_pane.rs`, `znicz-tui/src/views/mod.rs`
- Test: `znicz-tui/tests/scan.rs` (new), `znicz-tui/src/keys.rs` tests

- [ ] **Step 1: Write the failing integration tests**

Create `znicz-tui/tests/scan.rs`:

```rust
//! Scanning from the TUI through an in-process library service.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::backend::TestBackend;
use ratatui::Terminal;
use znicz_core::{spawn_player, wait_for_library, AudioConfig, LibraryHost};
use znicz_library::{Library, LibraryService};
use znicz_tui::{views, App};

struct Setup {
    app: App,
    root: PathBuf,
    service: Arc<LibraryService>,
}

fn setup(name: &str) -> Setup {
    let root = std::env::temp_dir().join(format!(
        "znicz-tui-scan-{name}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let root = std::fs::canonicalize(&root).unwrap();
    let db = root.join("library.db");
    let service = Arc::new(LibraryService::open(&db).unwrap());
    let (player, _thread) = spawn_player(AudioConfig::default());
    let mut app = App::with_library(player, Some(Library::open(&db).unwrap()));
    app.library_host = Some(service.clone());
    Setup { app, root, service }
}

fn press(app: &mut App, code: KeyCode) {
    app.on_key(KeyEvent::new(code, KeyModifiers::NONE));
}

fn type_text(app: &mut App, text: &str) {
    for c in text.chars() {
        press(app, KeyCode::Char(c));
    }
}

fn draw(app: &mut App) -> String {
    let state = app.state();
    let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
    terminal
        .draw(|frame| views::render(frame, app, &state))
        .unwrap();
    let buffer = terminal.backend().buffer();
    let mut out = String::new();
    for y in 0..buffer.area().height {
        for x in 0..buffer.area().width {
            out.push_str(buffer[(x, y)].symbol());
        }
        out.push('\n');
    }
    out
}

fn wait(service: &LibraryService, target: u64) {
    wait_for_library(service, target, Some(Duration::from_secs(10)), |_| {}).expect("scan");
}

fn music(root: &Path) -> PathBuf {
    let dir = root.join("music");
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn an_empty_library_opens_the_folder_prompt() {
    let mut s = setup("first-run");
    let screen = draw(&mut s.app);
    assert!(s.app.folder_prompt.is_some());
    assert!(screen.contains("No music yet. Which folder holds your music?"), "{screen}");
    assert!(screen.contains("folder: "), "{screen}");
}

#[test]
fn esc_skips_the_prompt_for_the_session() {
    let mut s = setup("skip");
    draw(&mut s.app);
    press(&mut s.app, KeyCode::Esc);
    let screen = draw(&mut s.app);
    assert!(s.app.folder_prompt.is_none(), "does not reopen");
    assert!(screen.contains("press F to add a folder"), "{screen}");
}

#[test]
fn enter_adds_the_folder_and_the_library_reloads_when_done() {
    let mut s = setup("add");
    let music = music(&s.root);
    draw(&mut s.app);
    s.app.folder_prompt = Some(znicz_tui::line_edit::LineEdit::new());
    type_text(&mut s.app, &music.display().to_string());
    press(&mut s.app, KeyCode::Enter);
    assert!(s.app.folder_prompt.is_none());
    wait(&s.service, 1);
    draw(&mut s.app);
    assert_eq!(s.app.library_status.folders, [music]);
    let message = s.app.toasts.current().expect("finish message");
    assert!(message.text.starts_with("scan done: "), "{}", message.text);
}

#[test]
fn a_path_that_is_not_a_folder_keeps_the_prompt_open() {
    let mut s = setup("bad");
    draw(&mut s.app);
    s.app.folder_prompt = Some(znicz_tui::line_edit::LineEdit::new());
    type_text(&mut s.app, "/definitely/not/here");
    press(&mut s.app, KeyCode::Enter);
    assert!(s.app.folder_prompt.is_some());
    assert_eq!(
        s.app.toasts.current().unwrap().text,
        "not a folder: /definitely/not/here"
    );
}

#[test]
fn capital_f_opens_the_prompt_and_capital_u_rescans() {
    let mut s = setup("keys");
    let music = music(&s.root);
    s.service
        .apply(znicz_core::LibraryOp::AddFolder { path: music })
        .unwrap();
    wait(&s.service, 1);
    draw(&mut s.app);
    press(&mut s.app, KeyCode::Esc); // dismiss the first-run prompt
    press(&mut s.app, KeyCode::Char('F'));
    assert!(s.app.folder_prompt.is_some());
    press(&mut s.app, KeyCode::Esc);
    press(&mut s.app, KeyCode::Char('U'));
    assert_eq!(s.app.toasts.current().unwrap().text, "rescanning 1 folder");
    wait(&s.service, 2);
}

#[test]
fn u_without_folders_explains() {
    let mut s = setup("no-folders");
    draw(&mut s.app);
    press(&mut s.app, KeyCode::Esc);
    press(&mut s.app, KeyCode::Char('U'));
    assert_eq!(
        s.app.toasts.current().unwrap().text,
        "no music folders yet — press F to add one"
    );
}

#[test]
fn the_folder_prompt_has_its_own_footer() {
    let mut s = setup("footer");
    let screen = draw(&mut s.app);
    let footer = screen.lines().last().unwrap();
    assert!(footer.contains("Tab complete · Enter scan · Esc cancel"), "{footer}");
}
```

(`App::state()` exists and is public; adjust names to the actual public API if any differ.)

In `keys.rs` tests: add `Context::FolderPrompt` to `ALL_CONTEXTS` (size +1) and to the `None` list in `more_shows_only_where_question_mark_opens_help`; append `"F add folder"` before `"M messages"` in `library_hints_lead_with_pause_and_label_p_and_r`; raise the width bound in `every_context_fits_ninety_columns_with_more` to 110 and rename it `every_context_fits_a_wide_footer_with_more` (comment: later hints drop first on narrow terminals).

- [ ] **Step 2: Run to verify they fail**

`cargo test -p znicz-tui --test scan` and `--lib keys` — compile errors (`library_host`, `folder_prompt`, `library_status`, `Context::FolderPrompt`).

- [ ] **Step 3: Keymap**

In `keys.rs`:
- `Context`: add `FolderPrompt,` (after `Search`).
- Hints: `const ADD_FOLDER: Hint = hint("F add folder", KeyCode::Char('F'));`, `const TAB_COMPLETE: Hint = hint("Tab complete", KeyCode::Tab);`, `const ENTER_SCAN: Hint = hint("Enter scan", KeyCode::Enter);`.
- `context_hints`: `ADD_FOLDER` before `MESSAGES_OPEN` in both `Library` and `LibraryTree`; `Context::FolderPrompt => &[TAB_COMPLETE, ENTER_SCAN, CANCEL],`.
- `more_hint`: add `Context::FolderPrompt` to the no-`? more` set.
- `context_title`: `Context::FolderPrompt => None`.
- `LIBRARY`: add `b("F", "add a music folder"),` and `b("U", "rescan music folders"),` after `J`.

- [ ] **Step 4: Library pane helper and notice**

In `library_pane.rs`:

```rust
    /// The library exists but holds no music yet.
    pub fn has_no_music(&self) -> bool {
        self.library.is_some()
            && self.artists.is_empty()
            && self.tracks.is_empty()
            && !matches!(self.mode, Mode::Search(_))
    }
```

In `reload_browse`, change the empty-library notice to `"No music yet — press F to add a folder"`.

- [ ] **Step 5: App state and keys**

In `app.rs`:

- Imports: `use std::sync::Arc;` and `use znicz_core::{LibraryHost, LibraryOp, LibraryStatus};`.
- Fields on `App`:

```rust
    /// Where library writes go: the player over IPC, or an in-process service in tests.
    pub library_host: Option<Arc<dyn LibraryHost>>,
    /// The writer's status as of the last frame.
    pub library_status: LibraryStatus,
    /// The folder prompt, while it is open.
    pub folder_prompt: Option<LineEdit>,
    /// The first-run prompt was dismissed; do not offer it again this session.
    pub(crate) first_run_dismissed: bool,
    /// `scans_finished` at the last poll, to notice finished jobs.
    scans_seen: Option<u64>,
```

initialised to `None`, `LibraryStatus::default()`, `None`, `false`, `None`. `with_remote` sets `library_host: Some(Arc::new(player.clone()))` (clone the `IpcClient` before wrapping it in `Engine::Remote`).

- Methods:

```rust
    /// Read the library writer's status: reload and report finished jobs, and
    /// offer the first-run prompt. Called once per frame.
    pub fn poll_library(&mut self) {
        let Some(host) = self.library_host.clone() else {
            return;
        };
        let status = host.status();
        if self.scans_seen.is_some_and(|seen| seen != status.scans_finished) {
            self.library.reload_browse();
            self.playing = None;
            if let Some(last) = &status.last {
                let home = dirs::home_dir();
                let (level, text) = crate::scan_status::finish_message(last, home.as_deref());
                match level {
                    crate::toast::Level::Error => self.toasts.error(text),
                    crate::toast::Level::Warn => self.toasts.warn(text),
                    crate::toast::Level::Success => self.toasts.success(text),
                    crate::toast::Level::Info => self.toasts.info(text),
                }
            }
        }
        self.scans_seen = Some(status.scans_finished);
        self.library_status = status;
        if self.wants_first_run_prompt() {
            self.open_folder_prompt();
        }
    }

    fn wants_first_run_prompt(&self) -> bool {
        !self.first_run_dismissed
            && self.folder_prompt.is_none()
            && self.modal == Modal::None
            && !self.library.is_typing()
            && !self.library_status.busy()
            && self.library.has_no_music()
    }

    fn open_folder_prompt(&mut self) {
        if self.library_host.is_none() {
            self.toasts.error("no music library configured");
            return;
        }
        self.folder_prompt = Some(LineEdit::from_text(default_folder_text()));
    }

    fn on_folder_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => {
                self.folder_prompt = None;
                self.first_run_dismissed = true;
            }
            KeyCode::Enter => self.submit_folder(),
            KeyCode::Tab => self.complete_folder(),
            _ => {
                if let Some(edit) = self.folder_prompt.as_mut() {
                    edit.on_key(key);
                }
            }
        }
    }

    fn submit_folder(&mut self) {
        let Some(typed) = self.folder_prompt.as_ref().map(|e| e.as_str().trim().to_string()) else {
            return;
        };
        let home = dirs::home_dir().unwrap_or_default();
        let path = crate::folder_complete::expand(&typed, &home);
        if !path.is_dir() {
            self.toasts.error(format!("not a folder: {typed}"));
            return;
        }
        let Some(host) = self.library_host.clone() else {
            self.toasts.error("no music library configured");
            return;
        };
        match host.apply(LibraryOp::AddFolder { path }) {
            Ok(()) => {
                self.folder_prompt = None;
                self.first_run_dismissed = true;
                self.toasts.info(format!("scanning {typed}"));
            }
            Err(e) => self.toasts.error(e.to_string()),
        }
    }

    fn complete_folder(&mut self) {
        let Some(edit) = self.folder_prompt.as_mut() else {
            return;
        };
        let home = dirs::home_dir().unwrap_or_default();
        let done = crate::folder_complete::complete(edit.as_str(), &home);
        *edit = LineEdit::from_text(done.text);
        if done.matches.len() > 1 {
            let mut shown: Vec<&str> = done.matches.iter().take(5).map(String::as_str).collect();
            if done.matches.len() > 5 {
                shown.push("…");
            }
            self.toasts.info(shown.join(" · "));
        }
    }

    fn rescan_folders(&mut self) {
        let Some(host) = self.library_host.clone() else {
            self.toasts.error("no music library configured");
            return;
        };
        let count = host.status().folders.len();
        if count == 0 {
            self.toasts.info("no music folders yet — press F to add one");
            return;
        }
        match host.apply(LibraryOp::RescanAll) {
            Ok(()) => self.toasts.info(match count {
                1 => "rescanning 1 folder".to_string(),
                n => format!("rescanning {n} folders"),
            }),
            Err(e) => self.toasts.error(e.to_string()),
        }
    }
```

and a free function:

```rust
/// The folder prompt's starting text: the platform music folder, else home.
fn default_folder_text() -> String {
    let home = dirs::home_dir();
    match (dirs::audio_dir(), home.as_deref()) {
        (Some(music), Some(home)) if music.is_dir() => {
            crate::folder_complete::tilde(&music, home)
        }
        (Some(music), None) if music.is_dir() => music.display().to_string(),
        _ => "~/".to_string(),
    }
}
```

- `on_key`: right after the `if self.library.is_typing() { self.on_search_key(key); return; }` block, add `if self.folder_prompt.is_some() { self.on_folder_key(key); return; }`.
- `on_global_key`'s early `return false` guard: also return false when `self.folder_prompt.is_some()`.
- `on_library_key`: add `KeyCode::Char('F') => self.open_folder_prompt(),` and `KeyCode::Char('U') => self.rescan_folders(),`.
- `context()`: right after the `is_typing` → `Search` check, `if self.folder_prompt.is_some() { return Context::FolderPrompt; }`.
- `on_left_click`: after the footer-hit block, `if self.folder_prompt.is_some() { return; }` (clicks elsewhere do nothing while typing a folder).

In `views/mod.rs` `render`, call `app.poll_library();` right after `app.refresh_playing(state);`.

- [ ] **Step 6: Run and verify**

`cargo test -p znicz-tui` (the rendering assertions in `tests/scan.rs` that need Task 10 — the welcome text and `folder: ` line — may still fail; if so, mark them `#[ignore = "drawn in Task 10"]` temporarily and remove the attribute in Task 10). Clippy clean.

- [ ] **Step 7: Commit**

```bash
git add znicz-tui
git commit -m "Add folder prompt, F and U keys, and scan polling to the TUI"
```

---

### Task 10: TUI drawing — welcome, prompt row, progress line (`znicz-tui`)

**Files:**
- Modify: `znicz-tui/src/views/library.rs`, `znicz-tui/src/views/now_playing.rs`
- Test: `znicz-tui/tests/scan.rs`, `now_playing.rs` tests

- [ ] **Step 1: Write the failing tests**

Remove any `#[ignore]` added in Task 9. In `now_playing.rs` tests, update existing `message_line(&toasts, N)` calls to `message_line(&toasts, None, N)` and add:

```rust
    #[test]
    fn scan_progress_shows_when_no_message_is_live() {
        let toasts = crate::toast::Toasts::new();
        let progress = znicz_core::ScanProgress {
            folder: std::path::PathBuf::from("/srv/music"),
            seen: 12,
            added: 3,
            updated: 0,
        };
        let row = text(&message_line(&toasts, Some(&progress), 60));
        assert!(row.starts_with("⟳ scanning /srv/music · 12 files · 3 new"), "{row:?}");
        assert_eq!(row.chars().count(), 60);
    }

    #[test]
    fn a_live_message_beats_scan_progress() {
        let mut toasts = crate::toast::Toasts::new();
        toasts.info("added 4 tracks");
        let progress = znicz_core::ScanProgress::default();
        let row = text(&message_line(&toasts, Some(&progress), 60));
        assert!(row.starts_with("● added 4 tracks"), "{row:?}");
    }
```

Add to `tests/scan.rs`:

```rust
#[test]
fn the_prompt_sits_on_top_when_the_library_has_music() {
    let mut s = setup("top");
    Library::open(&s.root.join("library.db"))
        .unwrap()
        .upsert_track_for_test(
            Path::new("/m/a.flac"),
            "So What",
            Some("Miles".into()),
            Some("Kind of Blue".into()),
            None,
        )
        .unwrap();
    s.app.library.reload_browse();
    s.app.on_key(KeyEvent::new(KeyCode::Char('F'), KeyModifiers::NONE));
    let screen = draw(&mut s.app);
    let row = screen.lines().nth(1).unwrap_or_default();
    assert!(row.contains("folder: "), "prompt on the pane's first row:\n{screen}");
    assert!(screen.contains("Miles"), "{screen}");
}
```

- [ ] **Step 2: Run to verify they fail**

`cargo test -p znicz-tui --lib now_playing` (signature), `--test scan` (welcome text, top prompt).

- [ ] **Step 3: Message row**

In `now_playing.rs`, change `message_line` to

```rust
pub(crate) fn message_line(
    toasts: &crate::toast::Toasts,
    scanning: Option<&znicz_core::ScanProgress>,
    width: usize,
) -> Line<'static> {
```

and between the live-message branch and the unread branch insert:

```rust
    if let Some(progress) = scanning {
        let home = dirs::home_dir();
        let text = crate::scan_status::progress_text(progress, home.as_deref(), width);
        let pad = width.saturating_sub(text.chars().count());
        return Line::from(vec![
            Span::styled(text, theme::progress()),
            Span::raw(" ".repeat(pad)),
        ]);
    }
```

Callers (`render_transport`, `render_stacked_chrome`) pass `app.library_status.scanning.as_ref()`; `render_stacked_chrome` gains a `scanning: Option<&ScanProgress>` parameter (clone the status into a local first if the borrow checker objects).

- [ ] **Step 4: Library pane**

In `views/library.rs` `render`:

- Replace the prompt-area condition with

```rust
    let folder_on_top = app.folder_prompt.is_some() && !app.library.has_no_music();
    let (prompt_area, list_area) = match app.library.is_typing() || folder_on_top {
```

- In the `if let Some(prompt_area)` block, after the search prompt case, draw the folder prompt:

```rust
        } else if let Some(edit) = app.folder_prompt.clone() {
            frame.render_widget(
                Paragraph::new(views::prompt_line("folder: ", &edit)),
                prompt_area,
            );
        }
```

(restructure the existing `if let Some(edit) = app.library.prompt()` into this `if / else if`).

- `render_empty`: build the body with a helper:

```rust
fn empty_body(app: &App, width: usize) -> Vec<Line<'static>> {
    if app.library.has_no_music() {
        if let Some(edit) = &app.folder_prompt {
            return vec![
                Line::from("No music yet. Which folder holds your music?"),
                Line::from(""),
                views::prompt_line("folder: ", edit),
                Line::from(""),
                views::placeholder("Tab completes · Enter scans · Esc skips"),
            ];
        }
        if let Some(progress) = &app.library_status.scanning {
            let home = dirs::home_dir();
            return vec![views::placeholder(&crate::scan_status::progress_text(
                progress,
                home.as_deref(),
                width,
            ))];
        }
    }
    let notice = app
        .library
        .notice()
        .unwrap_or("Nothing here. Press / to search, or Esc to go back.");
    vec![views::placeholder(notice)]
}
```

and in `render_empty` use `Paragraph::new(empty_body(app, views::inner_width(area))).block(block)`.

- [ ] **Step 5: Run and verify**

`cargo test -p znicz-tui`, clippy, fmt. Look at `cargo run -q -p znicz-tui --example preview -- 100 30 2>/dev/null` (it has no library host, so no prompt — just check it still builds and draws).

- [ ] **Step 6: Commit**

```bash
git add znicz-tui
git commit -m "Draw the folder prompt, welcome, and scan progress"
```

---

### Task 11: Version and docs

**Files:** `Cargo.toml`, `Cargo.lock`, `README.md`, `wiki/Architecture/Overview.md`, `wiki/Architecture/Library.md`, `wiki/Architecture/MCP.md`, `wiki/Architecture/TUI.md`, `wiki/Plans/Roadmap.md`, `wiki/Rust/Cargo-Workspace.md`

- [ ] **Step 1:** Version `0.7.0` → `0.8.0` in the root `Cargo.toml`; `cargo build -p znicz`; update the "currently" version in `wiki/Rust/Cargo-Workspace.md`.
- [ ] **Step 2: README.** In the "Music library" section, say that the TUI offers to scan your music folder on first run, `F` adds a folder and `U` rescans remembered folders, and that `znicz scan` (and MCP `scan_library`) now run through the `znicz player` process, which remembers each folder. Add `F` and `U` rows to the key table.
- [ ] **Step 3: Wiki.**
  - `Architecture/Overview.md`: the player process is also the single writer of `library.db` (`LibraryService`, reached through `LibraryHost` over IPC); TUI, MCP, and CLI read directly.
  - `Architecture/Library.md`: the service (one worker, job queue, `AddFolder` / `RescanAll` / `Prune`), the `folders` table, rescan rules (missing folders skipped and kept, prune only under scanned folders), progress every 100 files.
  - `Architecture/MCP.md`: `scan_library` (`wait`), `library_prune`, new `library_status`.
  - `Architecture/TUI.md`: first-run prompt, folder prompt keys and completion, `F` / `U`, progress on the message row and in the empty pane, finish messages, the `FolderPrompt` footer context; modules table gains `folder_complete.rs` and `scan_status.rs`.
  - `Plans/Roadmap.md`: part 3 → `**Done** in 0.8.0 ([spec](../../docs/superpowers/specs/2026-11-08-library-service-design.md))`.
- [ ] **Step 4:** `cargo test --workspace`, `cargo fmt --all -- --check`; commit `Bump to 0.8.0 and document the library service`.

---

### Task 12: Verify and open the PR

- [ ] **Step 1:** `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace` — all clean.
- [ ] **Step 2:** Tick every checkbox in this plan; commit `Mark library service plan complete`.
- [ ] **Step 3:** Push `feat/library-service`; open a PR "Library service and in-TUI scanning (TUI rework part 3)" summarising the spec goals, ending with `🤖 Generated with [Claude Code](https://claude.com/claude-code)`.
