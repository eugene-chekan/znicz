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

impl LibraryOp {
    /// How to report that this job failed.
    pub fn failure_text(&self, error: &str) -> String {
        match self {
            Self::AddFolder { .. } | Self::RescanAll => format!("scan failed: {error}"),
            Self::Prune => format!("library job failed: {error}"),
        }
    }
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
    /// A job is running (scanning or pruning).
    #[serde(default)]
    pub running: bool,
}

impl LibraryStatus {
    /// A job is running or waiting.
    pub fn busy(&self) -> bool {
        self.running || self.scanning.is_some() || self.queued > 0
    }
}

/// The single writer of the library, in process or over IPC.
pub trait LibraryHost: Send + Sync {
    /// Validate and enqueue. Errors are immediate problems (not a folder, no
    /// library); scan failures are reported in `LibraryStatus::last`.
    fn apply(&self, op: LibraryOp) -> Result<()>;
    fn status(&self) -> LibraryStatus;

    /// Like `status`, but reports a host that cannot be reached.
    fn try_status(&self) -> Result<LibraryStatus> {
        Ok(self.status())
    }
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
    let mut first_seen: Option<u64> = None;
    loop {
        let status = host.try_status()?;
        let first = *first_seen.get_or_insert(status.scans_finished);
        if status.scans_finished < first {
            return Err(ZniczError::Library(
                "the player restarted while scanning".into(),
            ));
        }
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

    #[test]
    fn a_running_job_is_busy() {
        let status = LibraryStatus {
            running: true,
            ..Default::default()
        };
        assert!(status.busy());
        assert!(!LibraryStatus::default().busy());
    }

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

    /// Its player is gone: reading status fails.
    struct Gone;

    impl LibraryHost for Gone {
        fn apply(&self, _op: LibraryOp) -> Result<()> {
            Ok(())
        }
        fn status(&self) -> LibraryStatus {
            LibraryStatus::default()
        }
        fn try_status(&self) -> Result<LibraryStatus> {
            Err(ZniczError::Library("connection lost".into()))
        }
    }

    #[test]
    fn waiting_fails_when_the_player_goes_away() {
        let err = wait_for_library(&Gone, 1, None, |_| {}).expect_err("fails");
        assert!(err.to_string().contains("connection lost"), "{err}");
    }

    /// Reports a scripted sequence of finished counts, repeating the last.
    struct Scripted {
        counts: Mutex<Vec<u64>>,
    }

    impl LibraryHost for Scripted {
        fn apply(&self, _op: LibraryOp) -> Result<()> {
            Ok(())
        }
        fn status(&self) -> LibraryStatus {
            let mut counts = self.counts.lock().unwrap();
            let n = if counts.len() > 1 {
                counts.remove(0)
            } else {
                counts[0]
            };
            LibraryStatus {
                scans_finished: n,
                queued: 1,
                ..LibraryStatus::default()
            }
        }
    }

    #[test]
    fn waiting_fails_when_the_player_restarts() {
        let host = Scripted {
            counts: Mutex::new(vec![3, 3, 0]),
        };
        let err = wait_for_library(&host, 4, Some(Duration::from_secs(5)), |_| {})
            .expect_err("restart detected");
        assert!(err.to_string().contains("restarted"), "{err}");
    }

    #[test]
    fn failure_text_names_the_kind_of_job() {
        let add = LibraryOp::AddFolder { path: "/m".into() };
        assert_eq!(add.failure_text("boom"), "scan failed: boom");
        assert_eq!(
            LibraryOp::RescanAll.failure_text("boom"),
            "scan failed: boom"
        );
        assert_eq!(
            LibraryOp::Prune.failure_text("boom"),
            "library job failed: boom"
        );
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
