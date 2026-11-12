//! The words the TUI shows for a library scan.

use std::path::Path;

use znicz_core::{ScanProgress, ScanSummary};

use crate::folder_complete::tilde;
use crate::format;
use crate::toast::Level;

/// `1240` → `1,240`.
pub fn grouped(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// `⟳ scanning ~/Music/Jazz · 1,240 files · 37 new · 2 updated`, with the folder
/// cut from the left so the counts stay visible in `width` columns.
pub fn progress_text(progress: &ScanProgress, home: Option<&Path>, width: usize) -> String {
    let folder = match home {
        Some(home) => tilde(&progress.folder, home),
        None => progress.folder.display().to_string(),
    };
    let head = "⟳ scanning ";
    let tail = format!(
        " · {} files · {} new · {} updated",
        grouped(progress.seen),
        grouped(progress.added),
        grouped(progress.updated)
    );
    let room = width.saturating_sub(head.chars().count() + tail.chars().count());
    format::truncate(&format!("{head}{}{tail}", cut_left(&folder, room)), width)
}

fn cut_left(text: &str, width: usize) -> String {
    let count = text.chars().count();
    if count <= width {
        return text.to_string();
    }
    if width == 0 {
        return String::new();
    }
    let keep: String = text.chars().skip(count - (width - 1)).collect();
    format!("…{keep}")
}

/// The message posted when a job finishes.
pub fn finish_message(summary: &ScanSummary, home: Option<&Path>) -> (Level, String) {
    if let Some(folder) = &summary.removed_folder {
        let shown = match home {
            Some(home) => tilde(folder, home),
            None => folder.display().to_string(),
        };
        return match &summary.error {
            Some(error) => (
                Level::Error,
                format!("could not remove the folder: {error}"),
            ),
            None => (
                Level::Success,
                format!("Removed {shown} ({} tracks)", grouped(summary.removed)),
            ),
        };
    }
    if let Some(error) = &summary.error {
        return (Level::Error, format!("scan failed: {error}"));
    }
    let mut text = format!(
        "scan done: {} new, {} updated, {} failed",
        grouped(summary.added),
        grouped(summary.updated),
        grouped(summary.failed)
    );
    if summary.removed > 0 {
        text.push_str(&format!(", {} removed", grouped(summary.removed)));
    }
    if summary.skipped.is_empty() {
        return (Level::Success, text);
    }
    let skipped: Vec<String> = summary
        .skipped
        .iter()
        .map(|p| match home {
            Some(home) => tilde(p, home),
            None => p.display().to_string(),
        })
        .collect();
    text.push_str(&format!(" · skipped missing {}", skipped.join(", ")));
    (Level::Warn, text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn progress() -> ScanProgress {
        ScanProgress {
            folder: PathBuf::from("/home/u/Music/Jazz"),
            seen: 1240,
            added: 37,
            updated: 2,
        }
    }

    #[test]
    fn numbers_are_grouped_in_thousands() {
        assert_eq!(grouped(7), "7");
        assert_eq!(grouped(1240), "1,240");
        assert_eq!(grouped(1_234_567), "1,234,567");
    }

    #[test]
    fn progress_reads_naturally_when_it_fits() {
        assert_eq!(
            progress_text(&progress(), Some(Path::new("/home/u")), 80),
            "⟳ scanning ~/Music/Jazz · 1,240 files · 37 new · 2 updated"
        );
    }

    #[test]
    fn a_long_folder_is_cut_from_the_left() {
        let text = progress_text(&progress(), Some(Path::new("/home/u")), 50);
        assert_eq!(text, "⟳ scanning …azz · 1,240 files · 37 new · 2 updated");
        assert_eq!(text.chars().count(), 50);
    }

    #[test]
    fn finish_messages_follow_the_outcome() {
        let home = Some(Path::new("/home/u"));
        let ok = ScanSummary {
            added: 412,
            updated: 3,
            ..ScanSummary::default()
        };
        assert_eq!(
            finish_message(&ok, home),
            (
                Level::Success,
                "scan done: 412 new, 3 updated, 0 failed".into()
            )
        );
        let pruned = ScanSummary {
            removed: 5,
            ..ScanSummary::default()
        };
        assert!(finish_message(&pruned, home).1.ends_with(", 5 removed"));
        let skipped = ScanSummary {
            added: 1,
            skipped: vec![PathBuf::from("/home/u/nas/music")],
            ..ScanSummary::default()
        };
        assert_eq!(
            finish_message(&skipped, home),
            (
                Level::Warn,
                "scan done: 1 new, 0 updated, 0 failed · skipped missing ~/nas/music".into()
            )
        );
        let failed = ScanSummary {
            error: Some("disk gone".into()),
            ..ScanSummary::default()
        };
        assert_eq!(
            finish_message(&failed, home),
            (Level::Error, "scan failed: disk gone".into())
        );
    }

    #[test]
    fn a_removed_folder_names_the_folder_and_its_tracks() {
        let home = Some(Path::new("/home/u"));
        let removed = ScanSummary {
            removed: 1240,
            removed_folder: Some(PathBuf::from("/home/u/Music/Jazz")),
            ..ScanSummary::default()
        };
        assert_eq!(
            finish_message(&removed, home),
            (Level::Success, "Removed ~/Music/Jazz (1,240 tracks)".into())
        );
        let failed = ScanSummary {
            error: Some("disk gone".into()),
            removed_folder: Some(PathBuf::from("/home/u/Music/Jazz")),
            ..ScanSummary::default()
        };
        assert_eq!(
            finish_message(&failed, home),
            (
                Level::Error,
                "could not remove the folder: disk gone".into()
            )
        );
    }
}
