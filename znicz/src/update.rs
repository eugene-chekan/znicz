//! The binary's side of updates: the daily release check, the install through
//! dist's install receipt, and `znicz update`.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use color_eyre::eyre::{bail, eyre};
use serde::{Deserialize, Serialize};
use znicz_tui::{Release, Updates};

use crate::daemon;

pub(crate) mod installer;

pub(crate) use installer::remove_old_exe;

pub(crate) const API_URL: &str = "https://api.github.com/repos/eugene-chekan/znicz/releases/latest";
pub(crate) const CHECK_EVERY: Duration = Duration::from_secs(24 * 60 * 60);

/// Dist's install section starts at this heading; it is not release notes.
const INSTALL_SECTION: &str = "## Install znicz";
const NO_NOTES: &str = "No release notes.";
const STATE_FILE: &str = "update-check.toml";
const WINDOWS_INSTALLER: &str = r#"Run the installer again: powershell -ExecutionPolicy Bypass -c "irm https://github.com/eugene-chekan/znicz/releases/latest/download/znicz-installer.ps1 | iex""#;
const UNIX_INSTALLER: &str = "Run the installer again: curl --proto '=https' --tlsv1.2 -LsSf https://github.com/eugene-chekan/znicz/releases/latest/download/znicz-installer.sh | sh";
const CHECKOUT_HINT: &str = "Update the checkout and rebuild: git pull && cargo build --release";

/// `latest` is a release (no pre-release part) newer than `current`.
pub(crate) fn is_newer(current: &str, latest: &str) -> bool {
    match (
        semver::Version::parse(current),
        semver::Version::parse(latest),
    ) {
        (Ok(current), Ok(latest)) => latest.pre.is_empty() && latest > current,
        _ => false,
    }
}

/// The body up to dist's `## Install znicz` section, trimmed; empty → "No release notes.".
pub(crate) fn release_notes(body: &str) -> String {
    let notes: Vec<&str> = body
        .lines()
        .take_while(|line| !line.starts_with(INSTALL_SECTION))
        .collect();
    let notes = notes.join("\n");
    let notes = notes.trim();
    if notes.is_empty() {
        NO_NOTES.to_string()
    } else {
        notes.to_string()
    }
}

/// `v0.14.0` → `0.14.0`.
pub(crate) fn version_from_tag(tag: &str) -> &str {
    tag.strip_prefix('v').unwrap_or(tag)
}

/// Whether `exe` sits in cargo's output, `…/target/debug` or `…/target/release`.
fn is_cargo_output(exe: &Path) -> bool {
    let parts: Vec<_> = exe.components().map(|c| c.as_os_str()).collect();
    parts
        .windows(2)
        .any(|pair| pair[0] == "target" && (pair[1] == "debug" || pair[1] == "release"))
}

/// The hint for a copy the installer did not make.
pub(crate) fn update_hint(exe: &Path) -> String {
    if is_cargo_output(exe) {
        CHECKOUT_HINT.to_string()
    } else if cfg!(windows) {
        WINDOWS_INSTALLER.to_string()
    } else {
        UNIX_INSTALLER.to_string()
    }
}

/// What the daily check remembers between runs (`update-check.toml`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct CheckState {
    /// Unix seconds of the last successful check; 0 when never.
    pub checked_at: u64,
    pub latest: Option<String>,
    pub notes: Option<String>,
    pub url: Option<String>,
    /// The last version a toast was shown for.
    pub notified: Option<String>,
}

impl CheckState {
    /// Missing or unreadable state is the default (never checked).
    pub(crate) fn load(path: &Path) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|text| toml::from_str(&text).ok())
            .unwrap_or_default()
    }

    pub(crate) fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let text = toml::to_string(self).map_err(std::io::Error::other)?;
        std::fs::write(path, text)
    }

    /// Whether a check is due at unix time `now_secs`.
    pub(crate) fn due(&self, now_secs: u64) -> bool {
        now_secs.saturating_sub(self.checked_at) >= CHECK_EVERY.as_secs()
    }
}

#[derive(Deserialize)]
struct ApiRelease {
    tag_name: String,
    body: Option<String>,
    html_url: String,
}

/// `(version, notes, url)` of the latest release on `api_url`.
pub(crate) fn fetch_latest_at(api_url: &str) -> Result<(String, String, String), String> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(10)))
        .build()
        .into();
    let mut response = agent
        .get(api_url)
        .header("User-Agent", concat!("znicz/", env!("CARGO_PKG_VERSION")))
        .header("Accept", "application/vnd.github+json")
        .call()
        .map_err(|e| e.to_string())?;
    let text = response
        .body_mut()
        .read_to_string()
        .map_err(|e| e.to_string())?;
    let release: ApiRelease = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    Ok((
        version_from_tag(&release.tag_name).to_string(),
        release_notes(release.body.as_deref().unwrap_or("")),
        release.html_url,
    ))
}

/// The daily check and the install, behind the TUI's `Updates` trait.
pub(crate) struct GithubUpdates {
    state_path: PathBuf,
    check: bool,
    api_url: String,
    state: Mutex<CheckState>,
}

impl GithubUpdates {
    pub(crate) fn new(state_path: PathBuf, check: bool, api_url: String) -> Self {
        let state = Mutex::new(CheckState::load(&state_path));
        Self {
            state_path,
            check,
            api_url,
            state,
        }
    }

    fn state(&self) -> MutexGuard<'_, CheckState> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Fetches the latest release now and records it. An error leaves the state as it was.
    pub(crate) fn fetch_now(&self) -> Result<Release, String> {
        let (version, notes, url) = fetch_latest_at(&self.api_url)?;
        let mut state = self.state();
        state.checked_at = now_secs();
        state.latest = Some(version.clone());
        state.notes = Some(notes.clone());
        state.url = Some(url.clone());
        save_state(&state, &self.state_path);
        Ok(Release {
            version,
            notes,
            url,
        })
    }

    /// The stored release, when it is newer than this build.
    fn stored_release(state: &CheckState) -> Option<Release> {
        let version = state.latest.clone()?;
        if !is_newer(env!("CARGO_PKG_VERSION"), &version) {
            return None;
        }
        Some(Release {
            version,
            notes: state.notes.clone().unwrap_or_else(|| NO_NOTES.to_string()),
            url: state.url.clone().unwrap_or_default(),
        })
    }
}

impl Updates for GithubUpdates {
    fn latest(&self) -> Option<Release> {
        if !self.check {
            return None;
        }
        let due = self.state().due(now_secs());
        if due {
            if let Err(e) = self.fetch_now() {
                tracing::debug!("update check failed: {e}");
            }
        }
        Self::stored_release(&self.state())
    }

    fn can_install(&self) -> bool {
        installer::can_install()
    }

    fn install(&self, version: &str) -> Result<(), String> {
        installer::install_release(version, true)
    }

    fn hint(&self) -> String {
        let exe = std::env::current_exe().unwrap_or_default();
        update_hint(&daemon::live_exe_path(exe))
    }

    fn notified(&self) -> Option<String> {
        self.state().notified.clone()
    }

    fn set_notified(&self, version: &str) {
        let mut state = self.state();
        state.notified = Some(version.to_string());
        save_state(&state, &self.state_path);
    }
}

fn save_state(state: &CheckState, path: &Path) {
    if let Err(e) = state.save(path) {
        tracing::warn!("could not save {}: {e}", path.display());
    }
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0)
}

/// The state file: `update-check.toml` next to the session file.
pub(crate) fn state_path() -> Option<PathBuf> {
    znicz_library::default_session_path().map(|path| path.with_file_name(STATE_FILE))
}

/// `znicz update` (`check_only`: `--check`).
pub(crate) fn run_cli(check_only: bool) -> color_eyre::Result<()> {
    let state_path = state_path().ok_or_else(|| {
        eyre!("cannot work out where to keep update-check.toml; set ZNICZ_SESSION_PATH")
    })?;
    let updates = GithubUpdates::new(state_path, true, API_URL.to_string());
    let current = env!("CARGO_PKG_VERSION");
    let release = updates
        .fetch_now()
        .map_err(|e| eyre!("could not check for a new release: {e}"))?;

    if !is_newer(current, &release.version) {
        println!("znicz {current} is up to date");
        return Ok(());
    }
    if check_only {
        println!("znicz {current}, latest {}", release.version);
        println!();
        println!("{}", release.notes);
        println!();
        println!("{}", release.url);
        return Ok(());
    }
    if !updates.can_install() {
        bail!(
            "This copy was not installed by the Znicz installer.\n{}",
            updates.hint()
        );
    }
    installer::install_release(&release.version, false).map_err(|e| eyre!("update failed: {e}"))?;
    println!(
        "Updated to {}. A running player keeps the old version until it restarts (znicz player stop).",
        release.version
    );
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::Arc;
    use std::thread;
    use std::time::SystemTime;

    /// Answers every request with `status` and `body`. Returns the URL to call and the
    /// number of requests that arrived.
    pub(crate) fn serve(status: u16, body: &'static str) -> (String, Arc<Mutex<usize>>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("local addr");
        let hits = Arc::new(Mutex::new(0));
        let counter = hits.clone();
        thread::spawn(move || {
            for mut stream in listener.incoming().flatten() {
                let mut buf = [0u8; 4096];
                let _ = stream.read(&mut buf);
                *counter.lock().unwrap() += 1;
                let text = match status {
                    200 => "200 OK",
                    404 => "404 Not Found",
                    _ => "500 Internal Server Error",
                };
                let head = format!(
                    "HTTP/1.1 {text}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(head.as_bytes());
                let _ = stream.write_all(body.as_bytes());
            }
        });
        (format!("http://{addr}/releases/latest"), hits)
    }

    fn now() -> u64 {
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs()
    }

    const RELEASE_099: &str = r#"{"tag_name":"v99.0.0","body":"Fixes\n\n## Install znicz 99.0.0\ncurl","html_url":"https://example.test/r"}"#;

    #[test]
    fn is_newer_compares_plain_releases() {
        assert!(is_newer("0.13.4", "0.14.0"));
        assert!(is_newer("0.13.4", "0.13.5"));
        assert!(!is_newer("0.13.4", "0.13.4"));
        assert!(!is_newer("0.14.0", "0.13.4"));
        assert!(!is_newer("0.13.4", "0.15.0-rc.1"));
        assert!(!is_newer("0.13.4", "soon"));
    }

    #[test]
    fn release_notes_stop_before_the_install_section() {
        let body = "Fixes a bug.\n\n## Install znicz 0.14.0\n\ncurl ... | sh";
        assert_eq!(release_notes(body), "Fixes a bug.");
        assert_eq!(release_notes("## Install znicz 0.14.0\ncurl"), NO_NOTES);
        assert_eq!(release_notes(""), NO_NOTES);
        assert_eq!(release_notes("  \n"), NO_NOTES);
    }

    #[test]
    fn version_from_tag_drops_the_v() {
        assert_eq!(version_from_tag("v0.14.0"), "0.14.0");
        assert_eq!(version_from_tag("0.14.0"), "0.14.0");
    }

    #[test]
    fn update_hint_points_a_checkout_at_cargo() {
        let hint = update_hint(Path::new("/home/x/znicz/target/release/znicz"));
        assert!(hint.contains("git pull && cargo build --release"), "{hint}");
    }

    #[test]
    fn update_hint_needs_cargo_output_dirs_not_any_target_dir() {
        let hint = update_hint(Path::new("/home/x/target/bin/znicz"));
        assert!(hint.contains("znicz-installer"), "{hint}");
        assert!(!hint.contains("cargo build"), "{hint}");
    }

    #[test]
    fn update_hint_points_an_installed_copy_at_the_installer() {
        let hint = update_hint(Path::new("/home/x/.local/bin/znicz"));
        assert!(hint.contains("znicz-installer"), "{hint}");
        assert!(!hint.contains("cargo build"), "{hint}");
    }

    #[test]
    fn check_state_is_due_after_a_day() {
        let now = 1_800_000_000;
        assert!(CheckState::default().due(now));
        let recent = CheckState {
            checked_at: now - 3600,
            ..Default::default()
        };
        assert!(!recent.due(now));
        let old = CheckState {
            checked_at: now - 25 * 3600,
            ..Default::default()
        };
        assert!(old.due(now));
    }

    #[test]
    fn check_state_round_trips_and_tolerates_bad_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("update-check.toml");
        assert_eq!(CheckState::load(&path), CheckState::default());

        let state = CheckState {
            checked_at: 42,
            latest: Some("0.14.0".into()),
            notes: Some("Fixes".into()),
            url: Some("https://example.test".into()),
            notified: Some("0.14.0".into()),
        };
        state.save(&path).unwrap();
        assert_eq!(CheckState::load(&path), state);

        std::fs::write(&path, "not = [toml").unwrap();
        assert_eq!(CheckState::load(&path), CheckState::default());
    }

    #[test]
    fn fetch_reads_tag_notes_and_url() {
        let (url, _) = serve(
            200,
            r#"{"tag_name":"v0.14.0","body":"Fixes\n\n## Install znicz 0.14.0\ncurl","html_url":"https://x"}"#,
        );
        assert_eq!(
            fetch_latest_at(&url),
            Ok(("0.14.0".into(), "Fixes".into(), "https://x".into()))
        );
    }

    #[test]
    fn fetch_accepts_a_null_body() {
        let (url, _) = serve(
            200,
            r#"{"tag_name":"v0.14.0","body":null,"html_url":"https://x"}"#,
        );
        let (_, notes, _) = fetch_latest_at(&url).unwrap();
        assert_eq!(notes, NO_NOTES);
    }

    #[test]
    fn fetch_fails_on_404_and_garbage() {
        let (url, _) = serve(404, "{}");
        assert!(fetch_latest_at(&url).is_err());
        let (url, _) = serve(200, "not json");
        assert!(fetch_latest_at(&url).is_err());
    }

    #[test]
    fn latest_fetches_when_due_and_saves_the_result() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("update-check.toml");
        let (url, hits) = serve(200, RELEASE_099);
        let updates = GithubUpdates::new(path.clone(), true, url);

        let release = updates.latest().expect("newer release");
        assert_eq!(release.version, "99.0.0");
        assert_eq!(release.url, "https://example.test/r");
        assert_eq!(*hits.lock().unwrap(), 1);
        assert!(CheckState::load(&path).checked_at > 0);
    }

    #[test]
    fn latest_uses_the_stored_release_within_a_day() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("update-check.toml");
        let stored = CheckState {
            checked_at: now(),
            latest: Some("99.0.0".into()),
            notes: Some("Stored".into()),
            url: Some("https://example.test/r".into()),
            notified: None,
        };
        stored.save(&path).unwrap();
        let (url, hits) = serve(404, "{}");
        let updates = GithubUpdates::new(path, true, url);

        assert_eq!(updates.latest().map(|r| r.version), Some("99.0.0".into()));
        assert_eq!(*hits.lock().unwrap(), 0);
    }

    #[test]
    fn latest_keeps_the_state_when_the_fetch_fails() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("update-check.toml");
        let (url, _) = serve(500, "{}");
        let updates = GithubUpdates::new(path.clone(), true, url);

        assert!(updates.latest().is_none());
        assert!(!path.exists());
    }

    #[test]
    fn latest_is_none_when_checks_are_off() {
        let dir = tempfile::tempdir().unwrap();
        let (url, hits) = serve(200, RELEASE_099);
        let updates = GithubUpdates::new(dir.path().join("update-check.toml"), false, url);

        assert!(updates.latest().is_none());
        assert_eq!(*hits.lock().unwrap(), 0);
    }

    #[test]
    fn notified_is_remembered_in_the_state_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("update-check.toml");
        let updates = GithubUpdates::new(path.clone(), true, API_URL.into());
        assert_eq!(updates.notified(), None);

        updates.set_notified("0.14.0");
        assert_eq!(updates.notified(), Some("0.14.0".into()));
        assert_eq!(CheckState::load(&path).notified, Some("0.14.0".into()));
    }
}
