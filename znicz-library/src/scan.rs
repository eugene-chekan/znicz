use std::path::Path;
use std::time::UNIX_EPOCH;

use serde::{Deserialize, Serialize};
use walkdir::WalkDir;
use znicz_core::{is_audio_file, read_metadata, title_from_path};

use crate::error::{LibraryError, Result};
use crate::store::Library;

/// How often the progress callback runs, in audio files seen.
const PROGRESS_EVERY: usize = 100;

/// What a scan did.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScanReport {
    /// Audio files seen on disk.
    pub seen: usize,
    /// Rows inserted.
    pub added: usize,
    /// Rows refreshed because the file changed.
    pub updated: usize,
    /// Files skipped because they had not changed since the last scan.
    pub unchanged: usize,
    /// Files we could not read at all.
    pub failed: usize,
}

impl Library {
    /// Walk a folder and index every audio file inside it.
    ///
    /// Files that have not changed since the last scan are left alone, so
    /// rescanning a large library is cheap.
    pub fn scan(&mut self, root: &Path) -> Result<ScanReport> {
        self.scan_with_progress(root, |_| {})
    }

    /// `scan`, calling `progress` at the start, every `PROGRESS_EVERY` audio
    /// files, and at the end.
    pub fn scan_with_progress(
        &mut self,
        root: &Path,
        mut progress: impl FnMut(&ScanReport),
    ) -> Result<ScanReport> {
        if !root.is_dir() {
            return Err(LibraryError::NotADirectory(root.display().to_string()));
        }

        let mut report = ScanReport::default();
        progress(&report);

        for entry in WalkDir::new(root).follow_links(false) {
            let entry = match entry {
                Ok(entry) => entry,
                Err(e) => {
                    tracing::debug!(error = %e, "skipping unreadable entry");
                    report.failed += 1;
                    continue;
                }
            };

            if !entry.file_type().is_file() || !is_audio_file(entry.path()) {
                continue;
            }

            report.seen += 1;
            match self.index_file(entry.path())? {
                Indexed::Added => report.added += 1,
                Indexed::Updated => report.updated += 1,
                Indexed::Unchanged => report.unchanged += 1,
                Indexed::Failed => report.failed += 1,
            }
            if report.seen % PROGRESS_EVERY == 0 {
                progress(&report);
            }
        }

        tracing::info!(
            root = %root.display(),
            seen = report.seen,
            added = report.added,
            updated = report.updated,
            unchanged = report.unchanged,
            failed = report.failed,
            "library scan finished"
        );

        progress(&report);
        Ok(report)
    }

    /// Read one file's metadata and store it.
    fn index_file(&mut self, path: &Path) -> Result<Indexed> {
        let modified = file_modified_secs(path);

        // Unchanged files keep their row: reading tags is the slow part.
        if let (Some(modified), Some(stored)) = (modified, self.modified_secs(path)?) {
            if modified == stored {
                return Ok(Indexed::Unchanged);
            }
        }

        let existed = self.get_by_path(path)?.is_some();
        let metadata = read_metadata(path);

        if metadata.tags.is_empty() && metadata.properties == Default::default() {
            // Nothing readable at all: likely not really an audio file.
            tracing::debug!(path = %path.display(), "no metadata, skipping");
            return Ok(Indexed::Failed);
        }

        let tags = metadata.tags;
        let properties = metadata.properties;
        let title = tags.title.clone().unwrap_or_else(|| title_from_path(path));
        let codec = path
            .extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| ext.to_ascii_lowercase());

        self.upsert_track(
            path,
            &title,
            tags.artist,
            tags.album,
            tags.album_artist,
            tags.genre,
            tags.year,
            tags.track_number,
            tags.disc_number,
            codec,
            properties.sample_rate,
            properties.channels,
            properties.bits_per_sample,
            properties.duration.map(|d| d.as_secs_f64()),
            modified,
        )?;

        Ok(if existed {
            Indexed::Updated
        } else {
            Indexed::Added
        })
    }
}

enum Indexed {
    Added,
    Updated,
    Unchanged,
    Failed,
}

/// File modification time in whole seconds, when the platform reports it.
fn file_modified_secs(path: &Path) -> Option<i64> {
    std::fs::metadata(path)
        .ok()?
        .modified()
        .ok()?
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|d| d.as_secs() as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_reports_growing_counts() {
        let dir = tempfile::tempdir().unwrap();
        // Audio by extension, unreadable inside: seen and failed, never indexed.
        for i in 0..250 {
            std::fs::write(dir.path().join(format!("t{i}.flac")), b"not audio").unwrap();
        }
        let mut library = Library::open_in_memory().unwrap();
        let mut seen = Vec::new();
        let report = library
            .scan_with_progress(dir.path(), |r| seen.push(r.seen))
            .unwrap();
        assert_eq!(report.seen, 250);
        assert_eq!(seen.first(), Some(&0), "reports at the start");
        assert_eq!(seen.last(), Some(&250), "and at the end");
        assert!(seen.contains(&100) && seen.contains(&200), "{seen:?}");
        assert!(seen.windows(2).all(|w| w[0] <= w[1]), "{seen:?}");
    }
}
