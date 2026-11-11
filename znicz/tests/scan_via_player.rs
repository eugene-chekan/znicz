//! `znicz scan` runs through the player process, which remembers the folder.

use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant};

use znicz_core::{ClientRole, IpcClient, LibraryHost};

struct Isolated {
    // Removed after `Drop` has stopped the player, since fields drop last.
    dir: tempfile::TempDir,
}

impl Isolated {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("music")).unwrap();
        fs::write(
            dir.path().join("config.toml"),
            format!(
                "[library]\npath = \"{}\"\n",
                dir.path()
                    .join("library.db")
                    .display()
                    .to_string()
                    .replace('\\', "/")
            ),
        )
        .unwrap();
        Self { dir }
    }

    fn ipc(&self) -> PathBuf {
        self.dir.path().join("ipc.toml")
    }

    fn znicz(&self) -> Command {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_znicz"));
        cmd.arg("--config")
            .arg(self.dir.path().join("config.toml"))
            .env("ZNICZ_IPC_PATH", self.ipc())
            .env("ZNICZ_SESSION_PATH", self.dir.path().join("session.toml"))
            .env("XDG_DATA_HOME", self.dir.path().join("data"));
        cmd
    }
}

impl Drop for Isolated {
    fn drop(&mut self) {
        if let Ok(client) = IpcClient::connect(self.ipc(), ClientRole::Agent) {
            let _ = client.shutdown();
            // The player saves its session after it answers, and releases the
            // lock last; wait for that before the folder is removed.
            let lock = self.dir.path().join("player.lock");
            let start = Instant::now();
            while lock.exists() && start.elapsed() < Duration::from_secs(10) {
                std::thread::sleep(Duration::from_millis(50));
            }
        }
    }
}

#[test]
fn scan_goes_through_the_player_and_remembers_the_folder() {
    let iso = Isolated::new();
    let music = std::path::absolute(iso.dir.path().join("music")).unwrap();

    let output = iso
        .znicz()
        .arg("scan")
        .arg(&music)
        .output()
        .expect("run scan");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "{stdout}\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
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
        .arg(iso.dir.path().join("nope"))
        .output()
        .expect("run scan");
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("not a folder"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn a_bad_folder_among_good_ones_scans_nothing() {
    let iso = Isolated::new();
    let output = iso
        .znicz()
        .arg("scan")
        .arg(iso.dir.path().join("music"))
        .arg(iso.dir.path().join("nope"))
        .output()
        .expect("run scan");
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("not a folder"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    // Validation fails before the player is even contacted.
    assert!(!iso.ipc().exists(), "player was started");
}
