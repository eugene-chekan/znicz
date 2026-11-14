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
    RemoveFolder(PathBuf),
    #[cfg(test)]
    PanicForTest,
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
        // Disk work happens before the lock. Paths are kept as written (only
        // made absolute): symlinks stay, so files indexed by older versions
        // under the same path are not duplicated.
        let folder = match &op {
            LibraryOp::AddFolder { path } => Some(
                std::path::absolute(path)
                    .ok()
                    .filter(|p| p.is_dir())
                    .ok_or_else(|| {
                        ZniczError::Library(format!("not a folder: {}", path.display()))
                    })?,
            ),
            _ => None,
        };
        let mut state = self
            .shared
            .state
            .lock()
            .map_err(|_| ZniczError::Library("library lock poisoned".into()))?;
        match op {
            LibraryOp::AddFolder { .. } => {
                let folder = folder.expect("folder resolved for AddFolder");
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
            LibraryOp::RemoveFolder { path } => {
                // Only a remembered folder that is not a filesystem root. Compared
                // as the store compares, so a trailing separator does not matter.
                let key = crate::store::folder_match(&path).map(|(key, _)| key);
                let allowed = key.as_deref().is_some_and(|key| {
                    Path::new(key).parent().is_some()
                        && state.status.folders.iter().any(|folder| {
                            crate::store::folder_match(folder)
                                .map(|(k, _)| k)
                                .as_deref()
                                == Some(key)
                        })
                });
                if !allowed {
                    return Err(ZniczError::Library(format!(
                        "not a music folder: {}",
                        path.display()
                    )));
                }
                state.queue.push_back(Job::RemoveFolder(path));
            }
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
                    state.status.running = true;
                    state.rescan_running = matches!(job, Job::RescanAll);
                    break job;
                }
                state = match shared.wake.wait(state) {
                    Ok(state) => state,
                    Err(_) => return,
                };
            }
        };
        let summary = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            run_job(&mut library, &shared, job)
        }))
        .unwrap_or_else(|_| {
            tracing::error!("library job panicked");
            ScanSummary {
                error: Some("library job crashed".into()),
                ..ScanSummary::default()
            }
        });
        // The database is the truth: a job may have changed the folder list
        // (a removal also forgets nested folders), even if it then failed.
        let folders = library.folders().ok();
        let Ok(mut state) = shared.state.lock() else {
            return;
        };
        if let Some(folders) = folders {
            let merged = folders_with_queued_adds(folders, &state.queue);
            state.status.folders = merged;
        }
        state.status.scanning = None;
        state.status.running = false;
        state.rescan_running = false;
        state.status.last = Some(summary);
        state.status.scans_finished += 1;
    }
}

/// The database's folders plus those of `Job::Add`s still queued: they are
/// listed on `apply`, but not yet remembered, so a resync must not drop them.
fn folders_with_queued_adds(mut folders: Vec<PathBuf>, queue: &VecDeque<Job>) -> Vec<PathBuf> {
    for job in queue {
        if let Job::Add(folder) = job {
            if !folders.contains(folder) {
                folders.push(folder.clone());
            }
        }
    }
    folders.sort();
    folders
}

fn run_job(library: &mut Library, shared: &Shared, job: Job) -> ScanSummary {
    let mut summary = ScanSummary::default();
    let result = match job {
        Job::Add(folder) => library
            .remember_folder(&folder)
            .and_then(|()| scan_into(library, shared, &folder, &mut summary).map(drop)),
        Job::RescanAll => rescan_all(library, shared, &mut summary),
        Job::Prune => library
            .remove_missing()
            .map(|removed| summary.removed = removed),
        Job::RemoveFolder(folder) => {
            summary.removed_folder = Some(folder.clone());
            library
                .remove_folder(&folder)
                .map(|removed| summary.removed = removed)
        }
        #[cfg(test)]
        Job::PanicForTest => panic!("injected library job panic"),
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
) -> Result<usize> {
    set_progress(shared, folder, &ScanReport::default());
    let report = library.scan_with_progress(folder, |r| set_progress(shared, folder, r))?;
    summary.seen += report.seen;
    summary.added += report.added;
    summary.updated += report.updated;
    summary.unchanged += report.unchanged;
    summary.failed += report.failed;
    Ok(report.seen)
}

fn rescan_all(library: &mut Library, shared: &Shared, summary: &mut ScanSummary) -> Result<()> {
    rescan_folders(library, summary, |library, folder, summary| {
        scan_into(library, shared, folder, summary)
    })
}

/// Rescan every remembered folder with `scan`, which returns how many files it
/// saw. A folder that is missing, fails to scan, or scans empty while the
/// library has tracks under it (an unmounted drive leaves an empty mount
/// point) is reported as skipped and keeps its tracks; the others go on.
fn rescan_folders(
    library: &mut Library,
    summary: &mut ScanSummary,
    mut scan: impl FnMut(&mut Library, &Path, &mut ScanSummary) -> Result<usize>,
) -> Result<()> {
    let mut scanned = Vec::new();
    for folder in library.folders()? {
        if !folder.is_dir() {
            // An unplugged drive: keep its tracks, just say it was skipped.
            summary.skipped.push(folder);
            continue;
        }
        match scan(library, &folder, summary) {
            Ok(0) if library.has_track_under(&folder)? => {
                tracing::warn!(folder = %folder.display(), "folder scanned empty; keeping its tracks");
                summary.skipped.push(folder);
            }
            Ok(_) => scanned.push(folder),
            Err(e) => {
                tracing::warn!(folder = %folder.display(), error = %e, "folder rescan failed");
                summary.skipped.push(folder);
            }
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

    fn temp_dir() -> tempfile::TempDir {
        // Canonical, so folder paths compare equal where the temp dir is a symlink.
        let base = std::fs::canonicalize(std::env::temp_dir()).unwrap();
        tempfile::tempdir_in(base).unwrap()
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

    impl LibraryService {
        fn apply_panic_for_test(&self) {
            let mut state = self.shared.state.lock().unwrap();
            state.queue.push_back(Job::PanicForTest);
            state.status.queued = state.queue.len();
            self.shared.wake.notify_all();
        }
    }

    #[test]
    fn queued_adds_stay_listed_after_a_resync() {
        let mut queue = VecDeque::new();
        queue.push_back(Job::Add(PathBuf::from("/b")));
        queue.push_back(Job::Prune);
        queue.push_back(Job::Add(PathBuf::from("/c")));
        queue.push_back(Job::Add(PathBuf::from("/a")));
        let merged = folders_with_queued_adds(vec![PathBuf::from("/a")], &queue);
        assert_eq!(
            merged,
            [
                PathBuf::from("/a"),
                PathBuf::from("/b"),
                PathBuf::from("/c")
            ]
        );
    }

    #[test]
    fn a_crashing_job_is_reported_and_the_worker_lives_on() {
        let dir = temp_dir();
        let root = dir.path();
        let service = LibraryService::open(&root.join("library.db")).unwrap();
        service.apply_panic_for_test();
        let status = wait(&service, 1);
        assert_eq!(
            status.last.as_ref().unwrap().error.as_deref(),
            Some("library job crashed")
        );
        assert!(!status.busy());
        service.apply(LibraryOp::Prune).unwrap();
        let last = wait(&service, 2).last.unwrap();
        assert!(last.error.is_none());
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
        let dir = temp_dir();
        let root = dir.path();
        let jazz = music(root, "jazz", 3);
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

    #[cfg(unix)]
    #[test]
    fn a_symlinked_folder_is_remembered_as_written() {
        let dir = temp_dir();
        let root = dir.path();
        let real = music(root, "real", 1);
        let link = root.join("link");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let service = LibraryService::open(&root.join("library.db")).unwrap();
        add(&service, &link);
        let status = wait(&service, 1);
        assert_eq!(status.folders, [link]);
    }

    #[test]
    fn a_path_that_is_not_a_folder_is_refused() {
        let dir = temp_dir();
        let root = dir.path();
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
        let dir = temp_dir();
        let root = dir.path();
        let service = LibraryService::open(&root.join("library.db")).unwrap();
        add(&service, &music(root, "a", 1));
        add(&service, &music(root, "b", 1));
        let status = wait(&service, 2);
        assert_eq!(status.folders.len(), 2);
    }

    #[test]
    fn rescan_skips_missing_folders_and_prunes_only_what_it_scanned() {
        let dir = temp_dir();
        let root = dir.path();
        let db = root.join("library.db");
        let here = music(root, "here", 1);
        let away = music(root, "away", 0);
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
        assert!(
            reader
                .get_by_path(&away.join("kept.flac"))
                .unwrap()
                .is_some()
        );
        assert!(
            reader
                .get_by_path(&here.join("deleted.flac"))
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn a_folder_that_scans_empty_keeps_its_tracks() {
        let dir = temp_dir();
        let root = dir.path();
        let db = root.join("library.db");
        let nas = music(root, "nas", 0);
        let service = LibraryService::open(&db).unwrap();
        add(&service, &nas);
        wait(&service, 1);
        let reader = {
            let mut library = Library::open(&db).unwrap();
            library
                .upsert_track_for_test(&nas.join("x.flac"), "t", None, None, None)
                .unwrap();
            library
        };

        service.apply(LibraryOp::RescanAll).unwrap();
        let last = wait(&service, 2).last.unwrap();
        assert_eq!(last.skipped, std::slice::from_ref(&nas));
        assert_eq!(last.removed, 0);
        assert!(reader.get_by_path(&nas.join("x.flac")).unwrap().is_some());
    }

    #[test]
    fn prune_drops_missing_files_anywhere() {
        let dir = temp_dir();
        let root = dir.path();
        let db = root.join("library.db");
        let service = LibraryService::open(&db).unwrap();
        Library::open(&db)
            .unwrap()
            .upsert_track_for_test(Path::new("/nowhere/x.flac"), "t", None, None, None)
            .unwrap();
        service.apply(LibraryOp::Prune).unwrap();
        assert_eq!(wait(&service, 1).last.unwrap().removed, 1);
    }

    #[test]
    fn removing_a_folder_drops_its_tracks_and_reports_the_count() {
        let dir = temp_dir();
        let root = dir.path();
        let db = root.join("library.db");
        let music = root.join("music");
        {
            let mut writer = Library::open(&db).unwrap();
            writer.remember_folder(&music).unwrap();
            writer.remember_folder(&root.join("music2")).unwrap();
            for path in [
                music.join("a.flac"),
                music.join("sub/b.flac"),
                root.join("music2/c.flac"),
            ] {
                writer
                    .upsert_track_for_test(&path, "t", None, None, None)
                    .unwrap();
            }
        }
        let service = LibraryService::open(&db).unwrap();
        service
            .apply(LibraryOp::RemoveFolder {
                path: music.clone(),
            })
            .unwrap();
        let status = wait(&service, 1);
        let last = status.last.unwrap();
        assert!(last.error.is_none(), "{last:?}");
        assert_eq!(last.removed, 2);
        assert_eq!(last.removed_folder, Some(music.clone()));
        assert_eq!(status.folders, [root.join("music2")]);
        let reader = Library::open(&db).unwrap();
        assert_eq!(reader.folders().unwrap(), [root.join("music2")]);
        assert_eq!(reader.track_count().unwrap(), 1);
    }

    #[test]
    fn removal_refuses_folders_that_are_not_remembered() {
        let dir = temp_dir();
        let root = dir.path();
        let db = root.join("library.db");
        let jazz = root.join("music").join("jazz");
        {
            let mut writer = Library::open(&db).unwrap();
            writer.remember_folder(&jazz).unwrap();
            writer
                .upsert_track_for_test(&jazz.join("a.flac"), "t", None, None, None)
                .unwrap();
        }
        let service = LibraryService::open(&db).unwrap();
        // A parent of a remembered folder, and a folder that was never remembered.
        for path in [root.join("music"), root.join("elsewhere")] {
            let err = service
                .apply(LibraryOp::RemoveFolder { path: path.clone() })
                .expect_err("refused");
            assert_eq!(
                err.to_string(),
                format!("not a music folder: {}", path.display())
            );
        }
        assert_eq!(service.status().queued, 0);
        assert_eq!(service.status().scans_finished, 0);
        assert_eq!(Library::open(&db).unwrap().track_count().unwrap(), 1);
    }

    #[test]
    fn removal_refuses_a_filesystem_root_even_when_remembered() {
        let dir = temp_dir();
        let db = dir.path().join("library.db");
        {
            let mut writer = Library::open(&db).unwrap();
            writer.remember_folder(Path::new("/")).unwrap();
            writer
                .upsert_track_for_test(Path::new("/a.flac"), "t", None, None, None)
                .unwrap();
        }
        let service = LibraryService::open(&db).unwrap();
        let err = service
            .apply(LibraryOp::RemoveFolder {
                path: PathBuf::from("/"),
            })
            .expect_err("refused");
        assert!(err.to_string().contains("not a music folder: /"), "{err}");
        assert_eq!(Library::open(&db).unwrap().track_count().unwrap(), 1);
    }

    #[test]
    fn a_pruning_service_never_looks_idle_before_it_finishes() {
        let dir = temp_dir();
        let root = dir.path();
        let db = root.join("library.db");
        let service = LibraryService::open(&db).unwrap();
        {
            let mut writer = Library::open(&db).unwrap();
            for i in 0..2000 {
                writer
                    .upsert_track_for_test(
                        Path::new(&format!("/nowhere/{i}.flac")),
                        "t",
                        None,
                        None,
                        None,
                    )
                    .unwrap();
            }
        }
        service.apply(LibraryOp::Prune).unwrap();
        let start = std::time::Instant::now();
        loop {
            let status = service.status();
            if status.scans_finished >= 1 {
                break;
            }
            assert!(status.busy(), "idle before the prune finished: {status:?}");
            assert!(start.elapsed() < Duration::from_secs(30), "prune timed out");
        }
    }

    #[test]
    fn a_failing_folder_does_not_stop_the_rest_of_a_rescan() {
        let dir = temp_dir();
        let root = dir.path();
        let bad = music(root, "bad", 0);
        let good = music(root, "good", 0);
        let mut library = Library::open(&root.join("library.db")).unwrap();
        library.remember_folder(&bad).unwrap();
        library.remember_folder(&good).unwrap();
        for path in [bad.join("kept.flac"), good.join("deleted.flac")] {
            library
                .upsert_track_for_test(&path, "t", None, None, None)
                .unwrap();
        }
        let mut summary = ScanSummary::default();
        let mut tried = Vec::new();
        rescan_folders(&mut library, &mut summary, |_, folder, _| {
            tried.push(folder.to_path_buf());
            if folder == bad {
                Err(crate::error::LibraryError::NotADirectory("boom".into()))
            } else {
                Ok(1)
            }
        })
        .unwrap();
        assert_eq!(tried, [bad.clone(), good.clone()]);
        assert_eq!(summary.skipped, std::slice::from_ref(&bad));
        assert_eq!(summary.removed, 1);
        assert!(
            library
                .get_by_path(&bad.join("kept.flac"))
                .unwrap()
                .is_some()
        );
        assert!(
            library
                .get_by_path(&good.join("deleted.flac"))
                .unwrap()
                .is_none()
        );
    }
}
