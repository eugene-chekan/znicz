//! Installs a release the way the dist installer did, without axoupdater: read the
//! install receipt, download `znicz-installer.{sh,ps1}` for the version, run it with
//! the receipt's install directory.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use serde::Deserialize;

const RECEIPT_FILE: &str = "znicz-receipt.json";
const REPO_DOWNLOADS: &str = "https://github.com/eugene-chekan/znicz/releases/download";
const MAX_INSTALLER_BYTES: u64 = 2 * 1024 * 1024;
const TIMEOUT: Duration = Duration::from_secs(30);
const TAIL_LINES: usize = 5;

/// The fields of dist's `znicz-receipt.json` that the installer runner uses.
#[derive(Debug, Deserialize)]
pub(crate) struct Receipt {
    pub install_prefix: PathBuf,
    pub version: String,
}

/// Where the receipt lives: `XDG_CONFIG_HOME/znicz`; on Windows `LOCALAPPDATA/znicz`;
/// otherwise `~/.config/znicz`.
pub(crate) fn receipt_dir(
    xdg_config_home: Option<PathBuf>,
    home: Option<PathBuf>,
    local_app_data: Option<PathBuf>,
) -> Option<PathBuf> {
    if let Some(xdg) = xdg_config_home {
        return Some(xdg.join("znicz"));
    }
    if cfg!(windows)
        && let Some(local) = local_app_data
    {
        return Some(local.join("znicz"));
    }
    home.map(|home| home.join(".config").join("znicz"))
}

/// `znicz-receipt.json` in `receipt_dir`, from the process environment.
pub(crate) fn receipt_path() -> Option<PathBuf> {
    let non_empty = |name: &str| std::env::var_os(name).filter(|value| !value.is_empty());
    let dir = receipt_dir(
        non_empty("XDG_CONFIG_HOME").map(PathBuf::from),
        dirs::home_dir(),
        non_empty("LOCALAPPDATA").map(PathBuf::from),
    )?;
    Some(dir.join(RECEIPT_FILE))
}

/// The receipt at `path`; a missing or unreadable file gives `None`.
pub(crate) fn load_receipt(path: &Path) -> Option<Receipt> {
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

/// Whether `exe` sits in the receipt's install directory or its `bin` subdirectory.
pub(crate) fn is_installed_copy(exe: &Path, receipt: &Receipt) -> bool {
    let Some(exe_dir) = exe.parent().and_then(|dir| dir.canonicalize().ok()) else {
        return false;
    };
    let Ok(prefix) = receipt.install_prefix.canonicalize() else {
        return false;
    };
    if exe_dir == prefix {
        return true;
    }
    prefix
        .join("bin")
        .canonicalize()
        .is_ok_and(|bin| bin == exe_dir)
}

/// The installer's URL for `version` on this platform.
pub(crate) fn installer_url(version: &str) -> String {
    installer_url_for(version, cfg!(windows))
}

pub(crate) fn installer_url_for(version: &str, windows: bool) -> String {
    let ext = if windows { "ps1" } else { "sh" };
    format!("{REPO_DOWNLOADS}/v{version}/znicz-installer.{ext}")
}

/// Downloads the installer at `url` into `dir` (named after the URL's last segment).
pub(crate) fn download_installer(url: &str, dir: &Path) -> Result<PathBuf, String> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(TIMEOUT))
        .build()
        .into();
    let mut response = agent
        .get(url)
        .header("User-Agent", concat!("znicz/", env!("CARGO_PKG_VERSION")))
        .call()
        .map_err(|e| e.to_string())?;
    let bytes = response
        .body_mut()
        .with_config()
        .limit(MAX_INSTALLER_BYTES)
        .read_to_vec()
        .map_err(|e| e.to_string())?;
    let name = url
        .rsplit('/')
        .next()
        .filter(|name| !name.is_empty())
        .unwrap_or("znicz-installer.sh");
    let path = dir.join(name);
    std::fs::write(&path, bytes).map_err(|e| e.to_string())?;
    Ok(path)
}

/// The installer run: `powershell -File` on Windows, `sh` elsewhere, installing
/// into `install_dir` without touching `PATH` again.
fn installer_command(script: &Path, install_dir: &Path, windows: bool) -> Command {
    let mut command = if windows {
        let mut command = Command::new("powershell");
        command
            .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"])
            .arg(script);
        // PowerShell 7 sets PSModulePath to its own modules; Windows PowerShell
        // 5.1 inherits it and fails to load them (PowerShell/PowerShell#18530).
        // Without it, 5.1 uses its default module path.
        command.env_remove("PSModulePath");
        command
    } else {
        let mut command = Command::new("sh");
        command.arg(script);
        command
    };
    command
        .env("ZNICZ_INSTALL_DIR", install_dir)
        .env("ZNICZ_NO_MODIFY_PATH", "1")
        .stdin(Stdio::null());
    command
}

/// Runs the installer with `ZNICZ_INSTALL_DIR` set to `install_dir`. With `capture`,
/// the output is collected and the error carries its last lines; otherwise it is shown.
pub(crate) fn run_installer(
    script: &Path,
    install_dir: &Path,
    capture: bool,
) -> Result<(), String> {
    let mut command = installer_command(script, install_dir, cfg!(windows));

    if !capture {
        let status = command.status().map_err(|e| e.to_string())?;
        return if status.success() {
            Ok(())
        } else {
            Err(format!("installer exited with {status}"))
        };
    }
    let output = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| e.to_string())?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = last_lines(&output.stderr);
    let detail = if stderr.is_empty() {
        last_lines(&output.stdout)
    } else {
        stderr
    };
    if detail.is_empty() {
        return Err(format!("installer exited with {}", output.status));
    }
    Err(format!("installer exited with {}: {detail}", output.status))
}

/// The last `TAIL_LINES` non-empty lines of `bytes`, joined with newlines.
fn last_lines(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    let lines: Vec<&str> = text
        .lines()
        .map(str::trim_end)
        .filter(|line| !line.trim().is_empty())
        .collect();
    lines[lines.len().saturating_sub(TAIL_LINES)..].join("\n")
}

/// `znicz.exe` → `znicz.exe.old-<stamp>`. The stamp keeps a name free even when
/// an earlier old copy is still in use by a running player.
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn old_exe_path(exe: &Path, stamp: u128) -> PathBuf {
    let mut name = exe.file_name().unwrap_or_default().to_os_string();
    name.push(format!(".old-{stamp}"));
    exe.with_file_name(name)
}

/// The files in `dir` named `<exe_name>.old…`: old copies left by Windows updates.
pub(crate) fn old_exe_names(dir: &Path, exe_name: &str) -> Vec<PathBuf> {
    let prefix = format!("{exe_name}.old");
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .file_name()
                .to_str()
                .is_some_and(|name| name.starts_with(&prefix))
        })
        .map(|entry| entry.path())
        .collect()
}

/// Deletes the old copies of the current exe left by earlier Windows updates.
/// Best effort: a copy still in use stays until a later start.
pub(crate) fn remove_old_exe() {
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    let (Some(dir), Some(name)) = (exe.parent(), exe.file_name().and_then(|n| n.to_str())) else {
        return;
    };
    for old in old_exe_names(dir, name) {
        let _ = std::fs::remove_file(old);
    }
}

/// Whether this copy is the one the receipt's installer put down. Errors mean no.
pub(crate) fn can_install() -> bool {
    let Some(receipt) = receipt_path().and_then(|path| load_receipt(&path)) else {
        return false;
    };
    std::env::current_exe()
        .is_ok_and(|exe| is_installed_copy(&crate::daemon::live_exe_path(exe), &receipt))
}

/// Installs `version` into the receipt's install directory. `capture` as in `run_installer`.
pub(crate) fn install_release(version: &str, capture: bool) -> Result<(), String> {
    let receipt = receipt_path()
        .and_then(|path| load_receipt(&path))
        .ok_or("no install receipt")?;
    tracing::debug!(
        "installing {version} into {} (receipt says {})",
        receipt.install_prefix.display(),
        receipt.version
    );
    let dir = tempfile::tempdir().map_err(|e| e.to_string())?;
    let script = download_installer(&installer_url(version), dir.path())?;

    #[cfg(windows)]
    let aside = move_running_exe_aside()?;
    let result = run_installer(&script, &receipt.install_prefix, capture);
    #[cfg(windows)]
    if result.is_err() {
        restore_running_exe(&aside);
    }
    result
}

/// Windows: a running exe can be renamed but not overwritten, so the installer writes
/// the new one at the original path after this moves the old one aside.
#[cfg(windows)]
fn move_running_exe_aside() -> Result<(PathBuf, PathBuf), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis());
    let old = old_exe_path(&exe, stamp);
    std::fs::rename(&exe, &old).map_err(|e| e.to_string())?;
    Ok((exe, old))
}

/// Put the old copy back after a failed install, unless the installer already
/// wrote a new copy at the original path (then the old one is left for later).
#[cfg(windows)]
fn restore_running_exe((exe, old): &(PathBuf, PathBuf)) {
    if exe.exists() {
        tracing::warn!(
            "not restoring {}: a new copy is already there",
            old.display()
        );
        return;
    }
    if let Err(e) = std::fs::rename(old, exe) {
        tracing::warn!("could not restore {}: {e}", exe.display());
    }
}

#[cfg(test)]
mod tests {

    #[test]
    fn the_windows_installer_does_not_inherit_powershell_7_modules() {
        // PowerShell 7 sets PSModulePath to its own modules, which Windows
        // PowerShell 5.1 then fails to load (PowerShell/PowerShell#18530).
        let command = installer_command(Path::new("i.ps1"), Path::new("C:/bin"), true);
        assert_eq!(command.get_program(), "powershell");
        let removed = command
            .get_envs()
            .any(|(key, value)| key == "PSModulePath" && value.is_none());
        assert!(removed, "PSModulePath must be removed for powershell");
    }

    #[test]
    fn the_unix_installer_runs_with_sh() {
        let command = installer_command(Path::new("i.sh"), Path::new("/bin"), false);
        assert_eq!(command.get_program(), "sh");
        let install_dir = command
            .get_envs()
            .find(|(key, _)| *key == "ZNICZ_INSTALL_DIR")
            .and_then(|(_, value)| value);
        assert_eq!(install_dir, Some(std::ffi::OsStr::new("/bin")));
    }
    use super::*;
    use crate::update::tests::serve;

    const REAL_RECEIPT: &str = r#"{"install_prefix":"/home/u/.local/bin","install_layout":"flat","binaries":["znicz"],"cdylibs":[],"cstaticlibs":[],"source":{"release_type":"github","owner":"eugene-chekan","name":"znicz","app_name":"znicz"},"version":"0.13.3","provider":{"source":"cargo-dist","version":"0.34.0"},"binary_aliases":{},"modify_path":true}"#;

    #[test]
    fn receipt_dir_prefers_xdg_config_home() {
        assert_eq!(
            receipt_dir(
                Some("/xdg".into()),
                Some("/home/u".into()),
                Some("/local".into())
            ),
            Some(PathBuf::from("/xdg/znicz"))
        );
    }

    #[test]
    fn receipt_dir_without_xdg_uses_the_platform_default() {
        let expected = if cfg!(windows) {
            PathBuf::from("/local/znicz")
        } else {
            PathBuf::from("/home/u/.config/znicz")
        };
        assert_eq!(
            receipt_dir(None, Some("/home/u".into()), Some("/local".into())),
            Some(expected)
        );
    }

    #[test]
    fn receipt_dir_needs_a_home_or_xdg() {
        assert_eq!(receipt_dir(None, None, None), None);
    }

    #[test]
    fn receipt_parses_the_fields_it_uses_and_ignores_the_rest() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(RECEIPT_FILE);
        std::fs::write(&path, REAL_RECEIPT).unwrap();

        let receipt = load_receipt(&path).expect("receipt");
        assert_eq!(receipt.install_prefix, PathBuf::from("/home/u/.local/bin"));
        assert_eq!(receipt.version, "0.13.3");
    }

    #[test]
    fn receipt_is_none_when_missing_or_broken() {
        let dir = tempfile::tempdir().unwrap();
        assert!(load_receipt(&dir.path().join("nope.json")).is_none());
        let path = dir.path().join("broken.json");
        std::fs::write(&path, "{ not json").unwrap();
        assert!(load_receipt(&path).is_none());
    }

    #[test]
    fn installed_copy_is_in_the_prefix_or_its_bin() {
        let dir = tempfile::tempdir().unwrap();
        let prefix = dir.path().join("prefix");
        let bin = prefix.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let receipt = Receipt {
            install_prefix: prefix.clone(),
            version: "0.13.3".into(),
        };

        std::fs::write(prefix.join("znicz"), "").unwrap();
        assert!(is_installed_copy(&prefix.join("znicz"), &receipt));

        std::fs::write(bin.join("znicz"), "").unwrap();
        assert!(is_installed_copy(&bin.join("znicz"), &receipt));

        let elsewhere = dir.path().join("elsewhere");
        std::fs::create_dir_all(&elsewhere).unwrap();
        std::fs::write(elsewhere.join("znicz"), "").unwrap();
        assert!(!is_installed_copy(&elsewhere.join("znicz"), &receipt));
    }

    #[test]
    fn installed_copy_is_false_when_paths_do_not_exist() {
        let receipt = Receipt {
            install_prefix: PathBuf::from("/definitely/not/here/prefix"),
            version: "0.13.3".into(),
        };
        assert!(!is_installed_copy(
            Path::new("/definitely/not/here/znicz"),
            &receipt
        ));
    }

    #[test]
    fn installer_url_names_the_platform_script() {
        assert_eq!(
            installer_url_for("0.14.0", false),
            "https://github.com/eugene-chekan/znicz/releases/download/v0.14.0/znicz-installer.sh"
        );
        assert_eq!(
            installer_url_for("0.14.0", true),
            "https://github.com/eugene-chekan/znicz/releases/download/v0.14.0/znicz-installer.ps1"
        );
    }

    #[test]
    fn old_exe_path_names_the_old_copy_with_a_stamp() {
        assert_eq!(
            old_exe_path(Path::new("/opt/bin/znicz.exe"), 42),
            PathBuf::from("/opt/bin/znicz.exe.old-42")
        );
        assert_eq!(
            old_exe_path(Path::new("znicz"), 7),
            PathBuf::from("znicz.old-7")
        );
    }

    #[test]
    fn old_exe_names_lists_only_the_old_copies_of_this_exe() {
        let dir = tempfile::tempdir().unwrap();
        for name in [
            "znicz.exe",
            "znicz.exe.old-1",
            "znicz.exe.old-2",
            "other.old-1",
        ] {
            std::fs::write(dir.path().join(name), "").unwrap();
        }
        let mut found: Vec<PathBuf> = old_exe_names(dir.path(), "znicz.exe")
            .into_iter()
            .map(|p| p.file_name().unwrap().into())
            .collect();
        found.sort();
        assert_eq!(
            found,
            vec![
                PathBuf::from("znicz.exe.old-1"),
                PathBuf::from("znicz.exe.old-2")
            ]
        );
    }

    #[test]
    fn download_writes_the_body_under_the_url_name() {
        let (url, _) = serve(200, "echo installed\n");
        let url = format!("{url}/znicz-installer.sh");
        let dir = tempfile::tempdir().unwrap();

        let path = download_installer(&url, dir.path()).expect("download");
        assert_eq!(path, dir.path().join("znicz-installer.sh"));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "echo installed\n");
    }

    #[test]
    fn download_fails_on_404() {
        let (url, _) = serve(404, "missing");
        let url = format!("{url}/znicz-installer.sh");
        let dir = tempfile::tempdir().unwrap();

        assert!(download_installer(&url, dir.path()).is_err());
        assert!(!dir.path().join("znicz-installer.sh").exists());
    }

    #[cfg(unix)]
    fn write_script(dir: &Path, body: &str) -> PathBuf {
        let path = dir.join("znicz-installer.sh");
        std::fs::write(&path, body).unwrap();
        path
    }

    #[cfg(unix)]
    #[test]
    fn run_installer_passes_the_install_dir_and_no_path_edit() {
        let scripts = tempfile::tempdir().unwrap();
        let install = tempfile::tempdir().unwrap();
        let install_dir = install.path().display().to_string();
        let script = write_script(
            scripts.path(),
            &format!(
                "[ \"$ZNICZ_INSTALL_DIR\" = \"{install_dir}\" ] && [ \"$ZNICZ_NO_MODIFY_PATH\" = \"1\" ] || exit 9\nexit 0\n"
            ),
        );

        assert_eq!(run_installer(&script, install.path(), true), Ok(()));
        assert_eq!(run_installer(&script, install.path(), false), Ok(()));
    }

    #[cfg(unix)]
    #[test]
    fn run_installer_without_output_has_no_dangling_separator() {
        let scripts = tempfile::tempdir().unwrap();
        let install = tempfile::tempdir().unwrap();
        let script = write_script(scripts.path(), "exit 3\n");

        let err = run_installer(&script, install.path(), true).unwrap_err();
        assert!(err.starts_with("installer exited with"), "{err}");
        assert!(!err.ends_with(':') && !err.ends_with(' '), "{err:?}");
    }

    #[cfg(unix)]
    #[test]
    fn run_installer_reports_stderr_on_failure() {
        let scripts = tempfile::tempdir().unwrap();
        let install = tempfile::tempdir().unwrap();
        let script = write_script(scripts.path(), "echo boom >&2\nexit 3\n");

        let err = run_installer(&script, install.path(), true).unwrap_err();
        assert!(err.starts_with("installer exited with"), "{err}");
        assert!(err.contains("boom"), "{err}");
        assert!(run_installer(&script, install.path(), false).is_err());
    }
}
