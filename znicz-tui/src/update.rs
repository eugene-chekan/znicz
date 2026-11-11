//! What the TUI needs from the updater; the binary provides the real one.

/// A release newer than this build.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    pub version: String,
    /// Release notes, already trimmed of the installer section.
    pub notes: String,
    pub url: String,
}

pub trait Updates: Send + Sync {
    /// The newer release, from the daily check (may use the network; called
    /// on a background thread). `None` when up to date or unknown.
    fn latest(&self) -> Option<Release>;
    /// Whether this binary is the installer's copy and can replace itself.
    fn can_install(&self) -> bool;
    /// Install `version` (blocking; called on a background thread).
    fn install(&self, version: &str) -> Result<(), String>;
    /// How to update when `can_install` is false.
    fn hint(&self) -> String;
    /// The version shown last in a toast, and recording a new one.
    fn notified(&self) -> Option<String>;
    fn set_notified(&self, version: &str);
}
