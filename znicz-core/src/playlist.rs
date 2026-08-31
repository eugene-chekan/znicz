//! M3U / M3U8 playlists: a file of local paths for the queue.
//!
//! Comments and blanks are ignored. URLs and missing files are skipped and
//! counted. The engine is unchanged: callers send `QueueClear` / `QueueAdd` /
//! `QueuePlayIndex`.

use std::fs;
use std::path::{Path, PathBuf};

use crate::error::{Result, ZniczError};
use crate::player::commands::Command;
use crate::player::engine::PlayerHandle;

/// What a playlist file turned into.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LoadResult {
    pub paths: Vec<PathBuf>,
    /// URLs and missing files. Comments and blank lines are not counted.
    pub skipped: usize,
}

/// Read an M3U body. `base_dir` resolves relative paths.
pub fn parse(text: &str, base_dir: &Path) -> LoadResult {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut result = LoadResult::default();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.contains("://") {
            result.skipped += 1;
            continue;
        }
        let path = PathBuf::from(line);
        let path = if path.is_absolute() {
            path
        } else {
            base_dir.join(path)
        };
        if path.is_file() {
            result.paths.push(path);
        } else {
            result.skipped += 1;
        }
    }
    result
}

/// Read a playlist file from disk.
pub fn load_path(path: &Path) -> Result<LoadResult> {
    let text = fs::read_to_string(path)?;
    let base = path.parent().unwrap_or_else(|| Path::new("."));
    Ok(parse(&text, base))
}

/// UTF-8, no BOM, one absolute path per line.
pub fn write_text(paths: &[PathBuf]) -> String {
    let mut out = String::new();
    for path in paths {
        let absolute = path.canonicalize().unwrap_or_else(|_| path.clone());
        out.push_str(&absolute.to_string_lossy());
        out.push('\n');
    }
    out
}

pub fn write_path(path: &Path, paths: &[PathBuf]) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, write_text(paths))?;
    Ok(())
}

/// File name under the playlists folder: `evening` → `evening.m3u`.
pub fn sanitize_stem(name: &str) -> Result<String> {
    let name = name.trim();
    if name.is_empty() {
        return Err(ZniczError::Player("illegal playlist name".into()));
    }
    if name.contains('/') || name.contains('\\') || name.contains("..") {
        return Err(ZniczError::Player("illegal playlist name".into()));
    }
    let file = if name.ends_with(".m3u") || name.ends_with(".m3u8") {
        name.to_string()
    } else {
        format!("{name}.m3u")
    };
    let stem = file
        .strip_suffix(".m3u8")
        .or_else(|| file.strip_suffix(".m3u"))
        .unwrap_or(&file);
    if stem.is_empty() || stem == "." {
        return Err(ZniczError::Player("illegal playlist name".into()));
    }
    Ok(file)
}

fn playlist_stem(name: &str) -> Option<&str> {
    let lower = name.to_ascii_lowercase();
    if let Some(stem) = lower.strip_suffix(".m3u8") {
        return (!stem.is_empty()).then_some(&name[..stem.len()]);
    }
    if let Some(stem) = lower.strip_suffix(".m3u") {
        return (!stem.is_empty()).then_some(&name[..stem.len()]);
    }
    None
}

/// Stems in `dir`, sorted, without the `.m3u` / `.m3u8` suffix.
pub fn list_saved(dir: &Path) -> Vec<String> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .flatten()
        .filter_map(|entry| {
            if !entry.file_type().ok()?.is_file() {
                return None;
            }
            let name = entry.file_name();
            playlist_stem(&name.to_string_lossy()).map(str::to_string)
        })
        .collect();
    names.sort();
    names.dedup();
    names
}

/// `evening` → `dir/evening.m3u` or `.m3u8` if that is what exists.
pub fn saved_path(dir: &Path, name: &str) -> Option<PathBuf> {
    let file = sanitize_stem(name).ok()?;
    let direct = dir.join(&file);
    if direct.is_file() {
        return Some(direct);
    }
    let stem = file
        .strip_suffix(".m3u8")
        .or_else(|| file.strip_suffix(".m3u"))
        .unwrap_or(&file);
    for ext in [".m3u", ".m3u8"] {
        let path = dir.join(format!("{stem}{ext}"));
        if path.is_file() {
            return Some(path);
        }
    }
    None
}

pub fn m3u_paths(queue: &[crate::player::state::QueueItem]) -> Result<Vec<PathBuf>> {
    let paths: Vec<PathBuf> = queue
        .iter()
        .filter_map(|item| item.as_path().map(Path::to_path_buf))
        .collect();
    if paths.is_empty() {
        return Err(ZniczError::Player(
            "cannot save a radio queue as a playlist".into(),
        ));
    }
    Ok(paths)
}

/// Clear and play (`append == false`) or only append.
pub fn apply_to_player(player: &PlayerHandle, result: &LoadResult, append: bool) -> Result<()> {
    if result.paths.is_empty() {
        return Err(ZniczError::Player("playlist had no playable files".into()));
    }
    if !append {
        player.send_blocking(Command::QueueClear)?;
    }
    player.send_blocking(Command::QueueAdd(
        result
            .paths
            .iter()
            .cloned()
            .map(crate::player::state::QueueItem::file)
            .collect(),
    ))?;
    if !append {
        player.send_blocking(Command::QueuePlayIndex(0))?;
    }
    Ok(())
}

/// Warn text when some rows were URLs or missing files.
pub fn skipped_notice(result: &LoadResult) -> Option<String> {
    if result.skipped == 0 {
        None
    } else {
        Some(format!(
            "{} tracks, {} skipped",
            result.paths.len(),
            result.skipped
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT: AtomicU64 = AtomicU64::new(0);

    fn tmp() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "znicz-playlist-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn touch(dir: &Path, name: &str) -> PathBuf {
        let path = dir.join(name);
        fs::write(&path, b"x").unwrap();
        path
    }

    #[test]
    fn comments_and_blank_lines_are_not_skipped_counts() {
        let dir = tmp();
        let a = touch(&dir, "a.flac");
        let text = format!("#EXTM3U\n\n#EXTINF:123,Title\n{}\n", a.display());
        let result = parse(&text, &dir);
        assert_eq!(result.paths, vec![a]);
        assert_eq!(result.skipped, 0);
    }

    #[test]
    fn relative_paths_resolve_against_the_playlist_directory() {
        let dir = tmp();
        let a = touch(&dir, "a.flac");
        let result = parse("a.flac\n", &dir);
        assert_eq!(result.paths, vec![a]);
    }

    #[test]
    fn a_bom_is_stripped() {
        let dir = tmp();
        let a = touch(&dir, "a.flac");
        let mut text = String::from("\u{feff}");
        text.push_str(&format!("{}\n", a.display()));
        let result = parse(&text, &dir);
        assert_eq!(result.paths, vec![a]);
    }

    #[test]
    fn urls_and_missing_files_count_as_skipped() {
        let dir = tmp();
        let a = touch(&dir, "a.flac");
        let text = format!("http://example.com/x.mp3\nmissing.flac\n{}\n", a.display());
        let result = parse(&text, &dir);
        assert_eq!(result.paths, vec![a]);
        assert_eq!(result.skipped, 2);
    }

    #[test]
    fn empty_result_when_nothing_playable() {
        let result = parse("# only a comment\nhttp://x\n", &tmp());
        assert!(result.paths.is_empty());
        assert_eq!(result.skipped, 1);
    }

    #[test]
    fn write_then_parse_round_trips_absolute_paths() {
        let dir = tmp();
        let a = touch(&dir, "a.flac");
        let b = touch(&dir, "b.flac");
        let text = write_text(&[a.clone(), b.clone()]);
        assert!(!text.contains('\u{feff}'));
        let result = parse(&text, &dir);
        // `write_text` writes `canonicalize()` output. On Windows that adds a
        // `\\?\` prefix and may expand 8.3 names (`RUNNER~1`), so the strings
        // are not the temp paths we started with. The files are the same.
        assert_eq!(
            result.paths,
            vec![a.canonicalize().unwrap(), b.canonicalize().unwrap()]
        );
        assert_eq!(result.skipped, 0);
    }

    #[test]
    fn sanitize_stem_rejects_path_bits() {
        assert!(sanitize_stem("evening").is_ok());
        assert_eq!(sanitize_stem("evening.m3u").unwrap(), "evening.m3u");
        assert_eq!(sanitize_stem("  weekend  ").unwrap(), "weekend.m3u");
        assert!(sanitize_stem("").is_err());
        assert!(sanitize_stem(".").is_err());
        assert!(sanitize_stem(".m3u").is_err());
        assert!(sanitize_stem("a/b").is_err());
        assert!(sanitize_stem("..").is_err());
        assert!(sanitize_stem("a\\b").is_err());
    }

    #[test]
    fn list_saved_is_sorted_stems_only() {
        let dir = tmp();
        fs::write(dir.join("b.m3u"), "").unwrap();
        fs::write(dir.join("a.m3u8"), "").unwrap();
        fs::write(dir.join("Evening.M3U"), "").unwrap();
        fs::create_dir(dir.join("folder.m3u")).unwrap();
        fs::write(dir.join("ignore.txt"), "").unwrap();
        assert_eq!(
            list_saved(&dir),
            vec!["Evening".to_string(), "a".to_string(), "b".to_string()]
        );
    }

    #[test]
    fn skipped_notice_is_none_when_every_row_loaded() {
        assert_eq!(
            skipped_notice(&LoadResult {
                paths: vec![PathBuf::from("a.flac")],
                skipped: 0
            }),
            None
        );
        assert_eq!(
            skipped_notice(&LoadResult {
                paths: vec![PathBuf::from("a.flac")],
                skipped: 2
            })
            .as_deref(),
            Some("1 tracks, 2 skipped")
        );
    }

    #[test]
    fn empty_apply_leaves_the_queue_alone() {
        let dir = tmp();
        let a = touch(&dir, "a.flac");
        let (player, _thread) = crate::spawn_player(crate::AudioConfig::default());
        player
            .send_blocking(Command::QueueAdd(vec![crate::player::state::QueueItem::file(a.clone())]))
            .unwrap();
        let err = apply_to_player(
            &player,
            &LoadResult {
                paths: Vec::new(),
                skipped: 1,
            },
            false,
        );
        assert!(err.is_err());
        assert_eq!(player.state().queue, vec![crate::player::state::QueueItem::file(a)]);
    }
}
