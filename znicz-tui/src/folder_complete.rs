//! Typing a folder path: `~` handling and `Tab` completion against the disk.

use std::path::{Path, PathBuf};

/// The result of completing what was typed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Completion {
    /// The new prompt text.
    pub text: String,
    /// Folder names that matched, sorted (empty when none did).
    pub matches: Vec<String>,
}

/// `~` and `~/…` become paths under `home`; anything else is taken as is.
pub fn expand(input: &str, home: &Path) -> PathBuf {
    if input == "~" {
        home.to_path_buf()
    } else if let Some(rest) = input.strip_prefix("~/") {
        home.join(rest)
    } else {
        PathBuf::from(input)
    }
}

/// `path` with the home folder shown as `~`.
pub fn tilde(path: &Path, home: &Path) -> String {
    match path.strip_prefix(home) {
        Ok(rest) if rest.as_os_str().is_empty() => "~".to_string(),
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => path.display().to_string(),
    }
}

/// Complete the last path segment against the folders in its parent.
///
/// One match completes it with a trailing `/`; several complete their common
/// prefix. Hidden folders only match a segment that starts with `.`.
pub fn complete(input: &str, home: &Path) -> Completion {
    let cut = input
        .rfind(['/', std::path::MAIN_SEPARATOR])
        .map_or(0, |i| i + 1);
    let (parent_text, partial) = input.split_at(cut);
    let parent = if parent_text.is_empty() {
        PathBuf::from(".")
    } else {
        expand(parent_text, home)
    };
    let mut names: Vec<String> = std::fs::read_dir(&parent)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|entry| entry.path().is_dir())
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|name| name.starts_with(partial))
        .filter(|name| partial.starts_with('.') || !name.starts_with('.'))
        .collect();
    names.sort();
    let text = match names.as_slice() {
        [] => input.to_string(),
        [one] => format!("{parent_text}{one}/"),
        many => format!("{parent_text}{}", common_prefix(many)),
    };
    Completion {
        text,
        matches: names,
    }
}

fn common_prefix(names: &[String]) -> String {
    let first = &names[0];
    let mut len = first.chars().count();
    for name in &names[1..] {
        len = len.min(
            first
                .chars()
                .zip(name.chars())
                .take_while(|(a, b)| a == b)
                .count(),
        );
    }
    first.chars().take(len).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A small folder tree; the `TempDir` must stay alive while it is used.
    fn tree() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        for sub in [
            "Music/Classical",
            "Music/Classic Rock",
            "Music/Jazz",
            "Music/.hidden",
        ] {
            std::fs::create_dir_all(root.join(sub)).unwrap();
        }
        std::fs::write(root.join("Music/Clash.txt"), b"a file").unwrap();
        (dir, root)
    }

    #[test]
    fn tilde_expands_against_home_and_back() {
        let home = Path::new("/home/u");
        assert_eq!(expand("~", home), PathBuf::from("/home/u"));
        assert_eq!(expand("~/Music", home), PathBuf::from("/home/u/Music"));
        assert_eq!(expand("/srv/m", home), PathBuf::from("/srv/m"));
        assert_eq!(tilde(Path::new("/home/u/Music"), home), "~/Music");
        assert_eq!(tilde(Path::new("/srv/m"), home), "/srv/m");
    }

    #[test]
    fn a_single_match_completes_with_a_slash() {
        let (_dir, home) = tree();
        let done = complete("~/Music/Ja", &home);
        assert_eq!(done.text, "~/Music/Jazz/");
        assert_eq!(done.matches, ["Jazz"]);
    }

    #[test]
    fn several_matches_complete_their_common_prefix_and_skip_files() {
        let (_dir, home) = tree();
        let done = complete("~/Music/Cl", &home);
        assert_eq!(done.text, "~/Music/Classic");
        assert_eq!(done.matches, ["Classic Rock", "Classical"]);
    }

    #[test]
    fn hidden_folders_need_a_dot() {
        let (_dir, home) = tree();
        assert!(!complete("~/Music/", &home)
            .matches
            .contains(&".hidden".to_string()));
        assert_eq!(complete("~/Music/.h", &home).text, "~/Music/.hidden/");
    }

    #[test]
    fn no_match_leaves_the_text_alone() {
        let (_dir, home) = tree();
        let done = complete("~/Music/Zz", &home);
        assert_eq!(done.text, "~/Music/Zz");
        assert!(done.matches.is_empty());
    }
}
