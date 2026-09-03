//! Recovery of `znicz player` when `player.lock` / `ipc.toml` are stale (#40).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use znicz_core::{ClientRole, IpcClient};

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Isolated {
    dir: PathBuf,
    ipc: PathBuf,
    lock: PathBuf,
}

impl Isolated {
    fn new() -> Self {
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "znicz-autostart-{}-{}-{n}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).expect("temp dir");
        let ipc = dir.join("ipc.toml");
        let lock = dir.join("player.lock");
        Self { dir, ipc, lock }
    }

    fn command(&self, exe: &str) -> Command {
        let mut cmd = Command::new(exe);
        cmd.env("ZNICZ_IPC_PATH", &self.ipc)
            .env("ZNICZ_SESSION_PATH", self.dir.join("session.toml"))
            .env("XDG_DATA_HOME", self.dir.join("data"))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        cmd
    }
}

impl Drop for Isolated {
    fn drop(&mut self) {
        if let Ok(client) = IpcClient::connect(&self.ipc, ClientRole::Agent) {
            let _ = client.shutdown();
        }
        let _ = fs::remove_dir_all(&self.dir);
    }
}

fn wait_up(ipc: &Path, timeout: Duration) -> bool {
    let start = Instant::now();
    while start.elapsed() < timeout {
        if IpcClient::connect(ipc, ClientRole::Agent).is_ok() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    IpcClient::connect(ipc, ClientRole::Agent).is_ok()
}

fn wait_gone(path: &Path, timeout: Duration) -> bool {
    let start = Instant::now();
    while start.elapsed() < timeout {
        if !path.exists() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    !path.exists()
}

fn spawn_mcp(iso: &Isolated, exe: &str) -> Child {
    iso.command(exe)
        .arg("mcp")
        .stdin(Stdio::piped())
        .spawn()
        .expect("spawn znicz mcp")
}

#[test]
fn mcp_autostarts_player_when_runtime_dir_is_empty() {
    let exe = env!("CARGO_BIN_EXE_znicz");
    let iso = Isolated::new();

    let mut mcp = spawn_mcp(&iso, exe);
    assert!(
        wait_up(&iso.ipc, Duration::from_secs(15)),
        "player should advertise at {}",
        iso.ipc.display()
    );
    let _ = mcp.kill();
    let _ = mcp.wait();
}

#[test]
fn mcp_recovers_from_stale_lock_and_advertise() {
    let exe = env!("CARGO_BIN_EXE_znicz");
    let iso = Isolated::new();
    fs::write(&iso.lock, "999999999\n").unwrap();
    fs::write(&iso.ipc, "port = 1\ntoken = \"dead\"\n").unwrap();

    let mut mcp = spawn_mcp(&iso, exe);
    assert!(
        wait_up(&iso.ipc, Duration::from_secs(15)),
        "stale lock should not block autostart at {}",
        iso.ipc.display()
    );
    let lock_text = fs::read_to_string(&iso.lock).unwrap_or_default();
    assert_ne!(lock_text.trim(), "999999999");
    let _ = mcp.kill();
    let _ = mcp.wait();
}

#[cfg(unix)]
#[test]
fn sigterm_clears_lock_and_advertise() {
    let exe = env!("CARGO_BIN_EXE_znicz");
    let iso = Isolated::new();

    let mut child = iso
        .command(exe)
        .arg("player")
        .spawn()
        .expect("spawn znicz player");
    assert!(
        wait_up(&iso.ipc, Duration::from_secs(15)),
        "player should start before SIGTERM"
    );

    unsafe {
        libc::kill(child.id() as i32, libc::SIGTERM);
    }
    assert!(
        wait_gone(&iso.ipc, Duration::from_secs(5)),
        "SIGTERM should remove ipc.toml"
    );
    assert!(
        wait_gone(&iso.lock, Duration::from_secs(2)),
        "SIGTERM should remove player.lock"
    );
    let _ = child.wait();
}
