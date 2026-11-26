//! Edit `config.toml` in place: one key at a time, keeping the user's
//! comments, order and other keys.

use std::path::{Path, PathBuf};
use toml_edit::{Decor, DocumentMut, Item, RawString, Table, Value};

#[derive(Debug, Clone)]
pub struct ConfigFile {
    path: PathBuf,
}

impl ConfigFile {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn set_bool(&self, section: &str, key: &str, v: bool) -> Result<(), String> {
        self.set(section, key, Value::from(v))
    }

    pub fn set_int(&self, section: &str, key: &str, v: i64) -> Result<(), String> {
        self.set(section, key, Value::from(v))
    }

    pub fn set_str(&self, section: &str, key: &str, v: &str) -> Result<(), String> {
        self.set(section, key, Value::from(v))
    }

    /// The value of `key` in `[section]` as the file has it now. `None` when the
    /// file, the section or the key is missing.
    pub fn get(&self, section: &str, key: &str) -> Result<Option<Value>, String> {
        let Some(mut doc) = self.read_doc()? else {
            return Ok(None);
        };
        let parts: Vec<&str> = section.split('.').collect();
        Ok(existing_table(doc.as_table_mut(), &parts)
            .and_then(|table| table.get(key))
            .and_then(Item::as_value)
            .cloned())
    }

    /// Drop `key` from `[section]`. A missing file, section or key is a no-op:
    /// nothing is created and nothing is written.
    pub fn remove(&self, section: &str, key: &str) -> Result<(), String> {
        let Some(mut doc) = self.read_doc()? else {
            return Ok(());
        };
        let parts: Vec<&str> = section.split('.').collect();
        let Some(table) = existing_table(doc.as_table_mut(), &parts) else {
            return Ok(());
        };
        let keys: Vec<String> = table.iter().map(|(k, _)| k.to_string()).collect();
        let Some(at) = keys.iter().position(|k| k == key) else {
            return Ok(());
        };
        let next = keys.get(at + 1).cloned();
        let Some((removed, _)) = table.remove_entry(key) else {
            return Ok(());
        };
        // The comment lines above the key are its leading decor. Keep them by
        // moving them onto whatever follows the key.
        let leading = raw_text(removed.leaf_decor().prefix());
        if !leading.is_empty() {
            let placed = match next {
                Some(next) => place_before_entry(table, &next, &leading),
                None => false,
            };
            if !placed {
                let position = table.position();
                place_before_following_table(&mut doc, position, &leading);
            }
        }
        write_atomically(&self.path, &doc.to_string())
    }

    fn set(&self, section: &str, key: &str, new: Value) -> Result<(), String> {
        self.edit(section, |table| {
            match table.get_mut(key).and_then(Item::as_value_mut) {
                // Keep the decor so a trailing `# comment` survives the change.
                Some(existing) => {
                    let decor = existing.decor().clone();
                    *existing = new;
                    *existing.decor_mut() = decor;
                }
                None => {
                    table.insert(key, Item::Value(new));
                }
            }
        })
    }

    /// The parsed file, or `None` when it does not exist.
    fn read_doc(&self) -> Result<Option<DocumentMut>, String> {
        let text = match std::fs::read_to_string(&self.path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e.to_string()),
        };
        text.parse()
            .map(Some)
            .map_err(|e: toml_edit::TomlError| e.to_string())
    }

    fn edit(&self, section: &str, change: impl FnOnce(&mut Table)) -> Result<(), String> {
        let mut doc = self.read_doc()?.unwrap_or_default();
        let parts: Vec<&str> = section.split('.').collect();
        let table = create_table(doc.as_table_mut(), &parts, section)?;
        change(table);
        write_atomically(&self.path, &doc.to_string())
    }
}

/// The text of a decor prefix or suffix, empty when there is none.
fn raw_text(raw: Option<&RawString>) -> String {
    raw.and_then(RawString::as_str).unwrap_or("").to_string()
}

/// Put `lead` in front of the entry `next` of `table`. A key gets it in its
/// leading decor, a sub-table in its header prefix. Returns `false` when the
/// entry cannot take it (an array of tables, or no such entry).
fn place_before_entry(table: &mut Table, next: &str, lead: &str) -> bool {
    match table.get(next) {
        Some(Item::Value(_)) => {
            let Some(mut key) = table.key_mut(next) else {
                return false;
            };
            prepend_prefix(key.leaf_decor_mut(), lead);
            true
        }
        Some(Item::Table(_)) => {
            let Some(Item::Table(sub)) = table.get_mut(next) else {
                return false;
            };
            prepend_prefix(sub.decor_mut(), lead);
            true
        }
        _ => false,
    }
}

fn prepend_prefix(decor: &mut Decor, lead: &str) {
    let existing = raw_text(decor.prefix());
    decor.set_prefix(format!("{lead}{existing}"));
}

/// Put `lead` above the table that follows the table at `position` in the
/// file, or at the end of the file when no table follows it.
fn place_before_following_table(doc: &mut DocumentMut, position: Option<usize>, lead: &str) {
    let Some(position) = position else {
        return append_to_document_end(doc, lead);
    };
    let mut best: Option<(usize, Vec<String>)> = None;
    find_following_table(doc.as_table(), position, &mut Vec::new(), &mut best);
    match best {
        Some((_, path)) => {
            let mut table = doc.as_table_mut();
            for name in &path {
                let Some(next) = table.get_mut(name).and_then(Item::as_table_mut) else {
                    return;
                };
                table = next;
            }
            prepend_prefix(table.decor_mut(), lead);
        }
        None => append_to_document_end(doc, lead),
    }
}

/// Records the path of the table with the smallest position after `after`.
fn find_following_table(
    table: &Table,
    after: usize,
    path: &mut Vec<String>,
    best: &mut Option<(usize, Vec<String>)>,
) {
    for (name, item) in table.iter() {
        let Item::Table(sub) = item else {
            continue;
        };
        if let Some(pos) = sub.position().filter(|p| *p > after) {
            if best.as_ref().is_none_or(|(b, _)| pos < *b) {
                let mut found = path.clone();
                found.push(name.to_string());
                *best = Some((pos, found));
            }
        }
        path.push(name.to_string());
        find_following_table(sub, after, path, best);
        path.pop();
    }
}

fn append_to_document_end(doc: &mut DocumentMut, lead: &str) {
    let existing = raw_text(Some(doc.trailing()));
    doc.set_trailing(format!("{lead}{existing}"));
}

/// The table at the dotted path `parts` (`["audio", "fade"]` for
/// `[audio.fade]`), or `None` when a part is missing or not a table.
fn existing_table<'a>(table: &'a mut Table, parts: &[&str]) -> Option<&'a mut Table> {
    match parts.split_first() {
        None => Some(table),
        Some((head, rest)) => existing_table(table.get_mut(head)?.as_table_mut()?, rest),
    }
}

/// The table at the dotted path `parts`, creating missing tables on the way.
/// A parent created only to hold a deeper table is implicit, so `[audio]` is
/// not written as an empty header just to hold `[audio.fade]`.
fn create_table<'a>(
    table: &'a mut Table,
    parts: &[&str],
    section: &str,
) -> Result<&'a mut Table, String> {
    let Some((head, rest)) = parts.split_first() else {
        return Ok(table);
    };
    let child = table.entry(head).or_insert_with(|| {
        let mut child = Table::new();
        child.set_implicit(!rest.is_empty());
        Item::Table(child)
    });
    let child = child
        .as_table_mut()
        .ok_or_else(|| format!("[{section}] is not a table"))?;
    create_table(child, rest, section)
}

/// Why `config.toml` was not used. The app then runs on the defaults.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigProblem {
    /// The text does not parse. `line` and `column` are 1-based.
    Parse {
        line: usize,
        column: usize,
        message: String,
    },
    /// The file exists but cannot be read.
    Unreadable { message: String },
}

impl ConfigProblem {
    /// "line 12, column 5" for a parse error; `None` when the file could not be read.
    pub fn location(&self) -> Option<String> {
        match self {
            Self::Parse { line, column, .. } => Some(format!("line {line}, column {column}")),
            Self::Unreadable { .. } => None,
        }
    }

    /// The error toast shown when the TUI starts.
    pub fn toast_text(&self) -> String {
        match self {
            Self::Parse {
                line,
                column,
                message,
            } => format!(
                "config.toml has an error (line {line}, column {column}): {message}. Using the defaults."
            ),
            Self::Unreadable { message } => {
                format!("config.toml could not be read: {message}. Using the defaults.")
            }
        }
    }
}

/// Write `text` next to `path`, then rename it over `path`. Creates the parent
/// directory when it is missing.
///
/// When `path` is a symlink (a dotfiles link, say), the write goes to the
/// resolved target so the link is kept rather than replaced by a regular file.
pub fn write_atomically(path: &Path, text: &str) -> Result<(), String> {
    let is_link = std::fs::symlink_metadata(path).is_ok_and(|m| m.is_symlink());
    let target = if is_link {
        std::fs::canonicalize(path).map_err(|e| e.to_string())?
    } else {
        path.to_path_buf()
    };
    if let Some(dir) = target.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let mut tmp_name = target.file_name().unwrap_or_default().to_os_string();
    tmp_name.push(".tmp");
    let tmp = target.with_file_name(tmp_name);
    let result = (|| {
        std::fs::write(&tmp, text)?;
        // The temp file starts with the default mode; keep the user's mode.
        if let Ok(meta) = std::fs::metadata(&target) {
            std::fs::set_permissions(&tmp, meta.permissions())?;
        }
        std::fs::rename(&tmp, &target)
    })();
    result.map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        e.to_string()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn file_in(dir: &TempDir, name: &str, text: &str) -> ConfigFile {
        let path = dir.path().join(name);
        std::fs::write(&path, text).unwrap();
        ConfigFile::new(path)
    }

    fn read(config: &ConfigFile) -> String {
        std::fs::read_to_string(config.path()).unwrap()
    }

    #[test]
    fn set_keeps_comments_and_other_keys() {
        let dir = TempDir::new().unwrap();
        let config = file_in(
            &dir,
            "config.toml",
            "# mine\n[tui]\nmarquee = true # scroll\nicons = \"emoji\"\n",
        );

        config.set_bool("tui", "marquee", false).unwrap();

        let text = read(&config);
        assert!(text.contains("# mine"), "{text}");
        assert!(text.contains("icons = \"emoji\""), "{text}");
        assert!(text.contains("marquee = false"), "{text}");
        assert!(text.contains("# scroll"), "{text}");
    }

    #[test]
    fn set_creates_file_dir_and_table() {
        let dir = TempDir::new().unwrap();
        let config = ConfigFile::new(dir.path().join("sub").join("config.toml"));

        config.set_int("tui", "cover_size", 75).unwrap();

        let value: toml::Value = toml::from_str(&read(&config)).unwrap();
        assert_eq!(value["tui"]["cover_size"].as_integer(), Some(75));
    }

    #[test]
    fn remove_drops_only_the_key() {
        let dir = TempDir::new().unwrap();
        let config = file_in(
            &dir,
            "config.toml",
            "[audio]\ndevice = \"x\"\nvolume = 1.0\n",
        );

        config.remove("audio", "device").unwrap();

        let text = read(&config);
        assert!(!text.contains("device"), "{text}");
        assert!(text.contains("volume = 1.0"), "{text}");
        assert!(text.contains("[audio]"), "{text}");
        config.remove("audio", "device").unwrap();
    }

    #[test]
    fn set_replaces_a_value_of_another_type() {
        let dir = TempDir::new().unwrap();
        let config = file_in(&dir, "config.toml", "[tui]\nmarquee = \"yes\"\n");

        config.set_bool("tui", "marquee", false).unwrap();

        let value: toml::Value = toml::from_str(&read(&config)).unwrap();
        assert_eq!(value["tui"]["marquee"].as_bool(), Some(false));
    }

    #[test]
    fn no_temp_file_left_behind() {
        let dir = TempDir::new().unwrap();
        let config = file_in(&dir, "config.toml", "[tui]\n");

        config.set_bool("tui", "show_cover", true).unwrap();

        let names: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        assert_eq!(names, vec!["config.toml".to_string()]);
    }

    #[test]
    fn remove_on_missing_file_creates_nothing() {
        let dir = TempDir::new().unwrap();
        let config = ConfigFile::new(dir.path().join("config.toml"));

        config.remove("audio", "device").unwrap();

        assert!(!config.path().exists());
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    #[test]
    fn remove_with_missing_section_or_key_writes_nothing() {
        let dir = TempDir::new().unwrap();
        let original = "# keep\n[tui]\nmarquee = true\n";
        let config = file_in(&dir, "config.toml", original);

        config.remove("audio", "device").unwrap();
        config.remove("tui", "device").unwrap();

        assert_eq!(read(&config), original);
    }

    #[cfg(unix)]
    #[test]
    fn set_through_a_symlink_keeps_the_link() {
        let dir = TempDir::new().unwrap();
        let real_dir = dir.path().join("real");
        std::fs::create_dir(&real_dir).unwrap();
        let target = real_dir.join("config.toml");
        std::fs::write(&target, "[tui]\nmarquee = true\n").unwrap();
        let link = dir.path().join("link.toml");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        let config = ConfigFile::new(&link);

        config.set_bool("tui", "marquee", false).unwrap();

        assert!(std::fs::symlink_metadata(&link).unwrap().is_symlink());
        let value: toml::Value =
            toml::from_str(&std::fs::read_to_string(&target).unwrap()).unwrap();
        assert_eq!(value["tui"]["marquee"].as_bool(), Some(false));
        let names: Vec<_> = std::fs::read_dir(&real_dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        assert_eq!(names, vec!["config.toml".to_string()]);
    }

    #[cfg(unix)]
    #[test]
    fn set_keeps_the_file_mode() {
        use std::os::unix::fs::PermissionsExt;
        let dir = TempDir::new().unwrap();
        let config = file_in(&dir, "config.toml", "[tui]\nmarquee = true\n");
        std::fs::set_permissions(config.path(), std::fs::Permissions::from_mode(0o600)).unwrap();

        config.set_bool("tui", "marquee", false).unwrap();

        let mode = std::fs::metadata(config.path())
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    #[test]
    fn a_dotted_section_is_a_nested_table() {
        let dir = TempDir::new().unwrap();
        let config = file_in(&dir, "config.toml", "[audio]\n# keep me\nvolume = 1.0\n");
        config.set_int("audio.fade", "play_ms", 200).unwrap();
        let text = read(&config);
        assert!(text.contains("# keep me"));
        assert!(text.contains("[audio.fade]"), "{text}");
        assert!(text.contains("play_ms = 200"), "{text}");
        let parsed: toml::Table = text.parse().unwrap();
        assert_eq!(parsed["audio"]["fade"]["play_ms"].as_integer(), Some(200));
    }

    #[test]
    fn remove_finds_a_nested_key() {
        let dir = TempDir::new().unwrap();
        let config = file_in(
            &dir,
            "config.toml",
            "[audio.fade]\nplay_ms = 200\nstop_ms = 300\n",
        );
        config.remove("audio.fade", "play_ms").unwrap();
        let text = read(&config);
        assert!(!text.contains("play_ms"));
        assert!(text.contains("stop_ms = 300"));
    }

    /// The `[spotify]` block from a real config where comments sit next to values.
    const SPOTIFY_BLOCK: &str = "[spotify]\n\
client_id = \"889562359a9049ed8970bd1565425e56\"\n\
# redirect_port = 8898        # redirect URI http://127.0.0.1:8898/callback\n\
device = \"EugeniusThink\"     # preferred device when none is active\n\
# open_browser = \"auto\"       # auto | always | never\n";

    #[test]
    fn set_keeps_the_trailing_comment_and_every_comment_line() {
        let dir = TempDir::new().unwrap();
        let config = file_in(&dir, "config.toml", SPOTIFY_BLOCK);

        config.set_str("spotify", "device", "Other").unwrap();

        let text = read(&config);
        assert!(text.contains("device = \"Other\""), "{text}");
        assert!(
            text.contains("# preferred device when none is active"),
            "{text}"
        );
        assert!(
            text.contains("# redirect_port = 8898        # redirect URI"),
            "{text}"
        );
        assert!(text.contains("# open_browser = \"auto\""), "{text}");
        assert!(!text.contains("EugeniusThink"), "{text}");
    }

    #[test]
    fn remove_keeps_the_comment_lines_above_the_key() {
        let dir = TempDir::new().unwrap();
        let config = file_in(&dir, "config.toml", SPOTIFY_BLOCK);

        config.remove("spotify", "device").unwrap();

        let text = read(&config);
        assert!(!text.contains("device"), "{text}");
        assert!(
            text.contains("# redirect_port = 8898        # redirect URI"),
            "{text}"
        );
        assert!(text.contains("# open_browser = \"auto\""), "{text}");
        assert!(text.contains("client_id = "), "{text}");
    }

    #[test]
    fn removing_the_last_key_of_a_table_keeps_its_leading_comments() {
        let dir = TempDir::new().unwrap();
        let config = file_in(
            &dir,
            "config.toml",
            "[tui]\nmarquee = true\n# show_cover = false\nicons = \"emoji\"\n[audio]\nvolume = 1.0\n",
        );

        config.remove("tui", "icons").unwrap();

        let text = read(&config);
        assert!(!text.contains("icons"), "{text}");
        assert!(text.contains("# show_cover = false"), "{text}");
        assert!(text.contains("marquee = true"), "{text}");
        let parsed: toml::Table = text.parse().unwrap();
        assert_eq!(parsed["audio"]["volume"].as_float(), Some(1.0));
    }

    #[test]
    fn set_adds_a_missing_key_to_a_commented_table() {
        let dir = TempDir::new().unwrap();
        let config = file_in(&dir, "config.toml", SPOTIFY_BLOCK);

        config.set_str("spotify", "playlist", "Mix").unwrap();

        let text = read(&config);
        assert!(text.contains("playlist = \"Mix\""), "{text}");
        assert!(
            text.contains("# preferred device when none is active"),
            "{text}"
        );
        let parsed: toml::Table = text.parse().unwrap();
        assert_eq!(parsed["spotify"]["playlist"].as_str(), Some("Mix"));
    }

    #[test]
    fn broken_file_is_not_written() {
        let dir = TempDir::new().unwrap();
        let config = file_in(&dir, "config.toml", "[tui\n");

        assert!(config.set_bool("tui", "marquee", false).is_err());
        assert_eq!(read(&config), "[tui\n");
    }
}
