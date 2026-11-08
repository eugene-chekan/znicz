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
        Job::Prune => library
            .remove_missing()
            .map(|removed| summary.removed = removed),
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
            assert_eq!(status.folders, std::slice::from_ref(&jazz));
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
        assert_eq!(last.skipped, std::slice::from_ref(&away));
        assert_eq!(last.removed, 1);
        assert!(reader
            .get_by_path(&away.join("kept.flac"))
            .unwrap()
            .is_some());
        assert!(reader
            .get_by_path(&here.join("deleted.flac"))
            .unwrap()
            .is_none());
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
