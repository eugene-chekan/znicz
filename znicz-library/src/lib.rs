//! The music library: a searchable index of local files.
//!
//! Scanning walks folders, reads tags with `znicz-core`, and stores one row per
//! track in SQLite. Nothing here plays audio; the library only answers
//! questions like "which tracks mention Miles Davis" or "what is on this
//! album".

mod error;
mod scan;
mod store;
mod track;

pub use error::{LibraryError, Result};
pub use scan::ScanReport;
pub use store::Library;
pub use track::{AlbumSummary, Track};

/// Where the library database lives by default.
pub fn default_database_path() -> Option<std::path::PathBuf> {
    dirs_data_dir().map(|dir| dir.join("znicz").join("library.db"))
}

/// Data directory without pulling in an extra dependency here.
fn dirs_data_dir() -> Option<std::path::PathBuf> {
    if let Some(dir) = std::env::var_os("XDG_DATA_HOME") {
        if !dir.is_empty() {
            return Some(std::path::PathBuf::from(dir));
        }
    }
    if let Some(home) = std::env::var_os("HOME") {
        if !home.is_empty() {
            return Some(std::path::PathBuf::from(home).join(".local").join("share"));
        }
    }
    // Windows
    std::env::var_os("APPDATA").map(std::path::PathBuf::from)
}
