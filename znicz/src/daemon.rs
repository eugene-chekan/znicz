//! Autostart and lock files for the shared `znicz player` process.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use znicz_core::{ClientRole, IpcClient};

/// How long clients wait for `ipc.toml` after spawning or detecting a live lock holder.
pub const PLAYER_START_TIMEOUT: Duration = Duration::from_secs(10);
const STARTUP_POLL: Duration = Duration::from_millis(50);

pub fn player_is_up(ipc: &Path) -> bool {
    IpcClient::connect(ipc, ClientRole::Agent).is_ok()
}

fn lock_holder_pid(lock_path: &Path) -> Option<u32> {
    let text = fs::read_to_string(lock_path).ok()?;
    text.lines().next()?.trim().parse().ok()
}

#[cfg(unix)]
fn pid_alive(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    unsafe {
        if libc::kill(pid as i32, 0) == 0 {
            return true;
        }
        // EPERM means the process exists but we cannot signal it.
        std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
    }
}

#[cfg(not(unix))]
fn pid_alive(pid: u32) -> bool {
    use std::process::Command;

    if pid == 0 {
        return false;
    }
    Command::new("tasklist")
        .args(["/FI", &format!("PID eq {pid}"), "/NH"])
        .output()
        .map(|output| String::from_utf8_lossy(&output.stdout).contains(&pid.to_string()))
        .unwrap_or(false)
}

fn lock_holder_alive(lock_path: &Path) -> bool {
    lock_path.exists() && lock_holder_pid(lock_path).is_some_and(pid_alive)
}

/// Drop stale `player.lock` / `ipc.toml` when nothing is listening.
///
/// A lock whose PID is still alive is left alone so we do not steal a daemon
/// that is still starting. A lock with no PID (older builds) is treated as dead.
pub fn clear_stale_player_files(lock_path: &Path, ipc: &Path) {
    if player_is_up(ipc) {
        return;
    }
    if lock_path.exists() && !lock_holder_alive(lock_path) {
        let _ = fs::remove_file(lock_path);
    }
    if ipc.exists() && !player_is_up(ipc) {
        let _ = fs::remove_file(ipc);
    }
}

pub struct PlayerLock {
    path: PathBuf,
    _file: File,
}

impl Drop for PlayerLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

pub fn acquire_player_lock(lock_path: &Path, ipc: &Path) -> color_eyre::Result<Option<PlayerLock>> {
    if let Some(parent) = lock_path.parent() {
        fs::create_dir_all(parent)?;
    }
    for _ in 0..2 {
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(lock_path)
        {
            Ok(mut file) => {
                writeln!(file, "{}", std::process::id())?;
                file.sync_all()?;
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    let _ = fs::set_permissions(lock_path, fs::Permissions::from_mode(0o600));
                }
                return Ok(Some(PlayerLock {
                    path: lock_path.to_path_buf(),
                    _file: file,
                }));
            }
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
                if player_is_up(ipc) {
                    return Ok(None);
                }
                if lock_holder_alive(lock_path) {
                    let start = Instant::now();
                    while start.elapsed() < PLAYER_START_TIMEOUT {
                        if player_is_up(ipc) {
                            return Ok(None);
                        }
                        if !lock_holder_alive(lock_path) {
                            break;
                        }
                        std::thread::sleep(STARTUP_POLL);
                    }
                    if player_is_up(ipc) || lock_holder_alive(lock_path) {
                        return Ok(None);
                    }
                }
                let _ = fs::remove_file(lock_path);
                let _ = fs::remove_file(ipc);
            }
            Err(e) => return Err(e.into()),
        }
    }
    Err(color_eyre::eyre::eyre!(
        "could not take player.lock at {}",
        lock_path.display()
    ))
}

pub fn spawn_detached_player_exe(
    exe: &Path,
    device: Option<&str>,
    config: Option<&Path>,
) -> color_eyre::Result<()> {
    let mut cmd = std::process::Command::new(exe);
    if let Some(config) = config {
        cmd.arg("--config").arg(config);
    }
    if let Some(device) = device {
        cmd.arg("--device").arg(device);
    }
    cmd.arg("player")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()?;
    Ok(())
}

fn wait_until_up(ipc: &Path, timeout: Duration) -> bool {
    let start = Instant::now();
    while start.elapsed() < timeout {
        if player_is_up(ipc) {
            return true;
        }
        std::thread::sleep(STARTUP_POLL);
    }
    player_is_up(ipc)
}

pub fn ensure_player(
    ipc: &Path,
    lock_path: &Path,
    device: Option<&str>,
    config: Option<&Path>,
) -> color_eyre::Result<()> {
    ensure_player_exe(ipc, lock_path, &std::env::current_exe()?, device, config)
}

pub fn ensure_player_exe(
    ipc: &Path,
    lock_path: &Path,
    exe: &Path,
    device: Option<&str>,
    config: Option<&Path>,
) -> color_eyre::Result<()> {
    if player_is_up(ipc) {
        return Ok(());
    }

    for attempt in 0..2 {
        clear_stale_player_files(lock_path, ipc);
        if player_is_up(ipc) {
            return Ok(());
        }

        if !lock_holder_alive(lock_path) {
            spawn_detached_player_exe(exe, device, config)?;
        }

        if wait_until_up(ipc, PLAYER_START_TIMEOUT) {
            return Ok(());
        }

        if attempt == 0 {
            clear_stale_player_files(lock_path, ipc);
        }
    }

    Err(color_eyre::eyre::eyre!(
        "could not start znicz player (no advertise at {})",
        ipc.display()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT: AtomicU64 = AtomicU64::new(0);

    struct TempPaths {
        dir: PathBuf,
        lock: PathBuf,
        ipc: PathBuf,
    }

    impl TempPaths {
        fn new() -> Self {
            let n = NEXT.fetch_add(1, Ordering::Relaxed);
            let dir = std::env::temp_dir().join(format!("znicz-daemon-{}-{n}", std::process::id()));
            fs::create_dir_all(&dir).unwrap();
            Self {
                lock: dir.join("player.lock"),
                ipc: dir.join("ipc.toml"),
                dir,
            }
        }
    }

    impl Drop for TempPaths {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.dir);
        }
    }

    fn temp_paths() -> TempPaths {
        TempPaths::new()
    }

    #[test]
    fn dead_lock_holder_files_are_cleared() {
        let tmp = temp_paths();
        fs::write(&tmp.lock, "999999999\n").unwrap();
        fs::write(&tmp.ipc, "port = 1\ntoken = \"x\"\n").unwrap();

        clear_stale_player_files(&tmp.lock, &tmp.ipc);

        assert!(!tmp.lock.exists());
        assert!(!tmp.ipc.exists());
    }

    #[test]
    fn lock_without_pid_is_treated_as_stale() {
        let tmp = temp_paths();
        fs::write(&tmp.lock, "").unwrap();
        fs::write(&tmp.ipc, "port = 1\ntoken = \"x\"\n").unwrap();

        clear_stale_player_files(&tmp.lock, &tmp.ipc);

        assert!(!tmp.lock.exists());
        assert!(!tmp.ipc.exists());
    }

    #[test]
    fn live_lock_holder_is_not_cleared() {
        let tmp = temp_paths();
        fs::write(&tmp.lock, format!("{}\n", std::process::id())).unwrap();
        fs::write(&tmp.ipc, "port = 1\ntoken = \"x\"\n").unwrap();

        clear_stale_player_files(&tmp.lock, &tmp.ipc);

        assert!(tmp.lock.exists());
        assert!(!tmp.ipc.exists());
    }

    #[test]
    fn acquire_lock_after_dead_holder() {
        let tmp = temp_paths();
        fs::write(&tmp.lock, "999999999\n").unwrap();

        let held = acquire_player_lock(&tmp.lock, &tmp.ipc).expect("acquire");
        assert!(held.is_some());
        assert!(tmp.lock.exists());
        let text = fs::read_to_string(&tmp.lock).expect("pid");
        assert_eq!(text.trim(), std::process::id().to_string());
        drop(held);
        assert!(!tmp.lock.exists());
    }

    #[test]
    fn temp_paths_directory_does_not_leak() {
        let dir = {
            let tmp = temp_paths();
            tmp.dir.clone()
        };
        assert!(
            !dir.exists(),
            "test runtime dir leaked at {}",
            dir.display()
        );
    }
}
