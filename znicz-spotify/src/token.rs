//! The OAuth token and `spotify_token.json`: written atomically, readable by
//! the owner only, and never printed.

use std::fmt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::Result;

/// Refresh this many seconds before the access token expires.
const REFRESH_MARGIN_SECS: u64 = 60;

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Token {
    pub access_token: String,
    pub refresh_token: String,
    /// Unix seconds.
    pub expires_at: u64,
    #[serde(default)]
    pub scope: String,
    /// The client ID the token was issued for. Empty for a token saved before
    /// this field existed: such a token matches any ID.
    #[serde(default)]
    pub client_id: String,
}

impl Token {
    pub(crate) fn needs_refresh(&self, now_secs: u64) -> bool {
        now_secs + REFRESH_MARGIN_SECS >= self.expires_at
    }

    /// Whether the token may be used with `client_id`: a legacy token (no
    /// client ID recorded) matches any ID.
    pub(crate) fn belongs_to(&self, client_id: &str) -> bool {
        self.client_id.is_empty() || self.client_id == client_id
    }
}

impl fmt::Debug for Token {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Token")
            .field("access_token", &"<redacted>")
            .field("refresh_token", &"<redacted>")
            .field("expires_at", &self.expires_at)
            .field("scope", &self.scope)
            .field("client_id", &self.client_id)
            .finish()
    }
}

/// `spotify_token.json` in the data folder, unless `ZNICZ_SPOTIFY_TOKEN_PATH` is set.
pub fn default_token_path() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("ZNICZ_SPOTIFY_TOKEN_PATH") {
        if !path.is_empty() {
            return Some(PathBuf::from(path));
        }
    }
    dirs::data_dir().map(|dir| dir.join("znicz").join("spotify_token.json"))
}

/// `None` when there is no file, or it cannot be parsed (logged once).
pub(crate) fn load(path: &Path) -> Result<Option<Token>> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    match serde_json::from_str(&text) {
        Ok(token) => Ok(Some(token)),
        Err(e) => {
            tracing::warn!(path = %path.display(), error = %e, "Spotify token file unreadable; logged out");
            Ok(None)
        }
    }
}

/// Write to a temporary file with mode `0600`, then rename it over `path`.
pub(crate) fn save(path: &Path, token: &Token) -> Result<()> {
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir)?;
    }
    let text = serde_json::to_string_pretty(token)
        .map_err(|e| crate::error::SpotifyError::Io(e.to_string()))?;
    let mut tmp_name = path.file_name().unwrap_or_default().to_os_string();
    tmp_name.push(".tmp");
    let tmp = path.with_file_name(tmp_name);
    // `mode(0o600)` applies only on create, so never reuse a `.tmp` left by a crash.
    let _ = std::fs::remove_file(&tmp);
    let result = write_private(&tmp, text.as_bytes()).and_then(|()| std::fs::rename(&tmp, path));
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    Ok(result?)
}

pub(crate) fn remove(path: &Path) -> Result<()> {
    match std::fs::remove_file(path) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.into()),
        _ => Ok(()),
    }
}

fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    file.write_all(bytes)?;
    file.sync_all()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn token() -> Token {
        Token {
            access_token: "secret-access".into(),
            refresh_token: "secret-refresh".into(),
            expires_at: 1_000,
            scope: "user-read-private".into(),
            client_id: "0123456789abcdef0123456789abcdef".into(),
        }
    }

    #[test]
    fn saves_and_loads() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("spotify_token.json");
        save(&path, &token()).unwrap();
        assert_eq!(load(&path).unwrap(), Some(token()));
    }

    #[cfg(unix)]
    #[test]
    fn the_file_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("spotify_token.json");
        save(&path, &token()).unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    #[test]
    fn a_missing_or_corrupt_file_means_logged_out() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("spotify_token.json");
        assert_eq!(load(&path).unwrap(), None);
        std::fs::write(&path, "{ not json").unwrap();
        assert_eq!(load(&path).unwrap(), None);
    }

    #[test]
    fn remove_ignores_a_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("spotify_token.json");
        remove(&path).unwrap();
        save(&path, &token()).unwrap();
        remove(&path).unwrap();
        assert!(!path.exists());
    }

    #[test]
    fn debug_hides_the_secrets() {
        let text = format!("{:?}", token());
        assert!(!text.contains("secret"), "{text}");
        assert!(text.contains("expires_at"), "{text}");
    }

    #[test]
    fn a_legacy_token_without_a_client_id_loads_and_matches_any_id() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("spotify_token.json");
        std::fs::write(
            &path,
            r#"{"access_token":"a","refresh_token":"r","expires_at":5}"#,
        )
        .unwrap();
        let loaded = load(&path).unwrap().unwrap();
        assert_eq!(loaded.client_id, "");
        assert!(loaded.belongs_to("anything"));
    }

    #[test]
    fn a_token_belongs_only_to_its_client_id() {
        let token = token();
        assert!(token.belongs_to("0123456789abcdef0123456789abcdef"));
        assert!(!token.belongs_to("fedcba9876543210fedcba9876543210"));
    }

    #[test]
    fn refreshes_a_minute_before_expiry() {
        let token = token();
        assert!(!token.needs_refresh(939));
        assert!(token.needs_refresh(940));
        assert!(token.needs_refresh(2_000));
    }
}
