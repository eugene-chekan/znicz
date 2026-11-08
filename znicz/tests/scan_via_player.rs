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
                dir.join("library.db")
                    .display()
                    .to_string()
                    .replace('\\', "/")
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
