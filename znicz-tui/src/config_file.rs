//! Edit `config.toml` in place: one key at a time, keeping the user's
//! comments, order and other keys.

use std::path::{Path, PathBuf};
use toml_edit::{DocumentMut, Item, Table, Value};

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

    pub fn remove(&self, section: &str, key: &str) -> Result<(), String> {
        self.edit(section, |table| {
            table.remove(key);
        })
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

    fn edit(&self, section: &str, change: impl FnOnce(&mut Table)) -> Result<(), String> {
        let text = match std::fs::read_to_string(&self.path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(e) => return Err(e.to_string()),
        };
        let mut doc: DocumentMut = text
            .parse()
            .map_err(|e: toml_edit::TomlError| e.to_string())?;
        let table = doc
            .entry(section)
            .or_insert(Item::Table(Table::new()))
            .as_table_mut()
            .ok_or_else(|| format!("[{section}] is not a table"))?;
        change(table);
        write_atomically(&self.path, &doc.to_string())
    }
}

/// Write `text` next to `path`, then rename it over `path`. Creates the parent
/// directory when it is missing.
pub fn write_atomically(path: &Path, text: &str) -> Result<(), String> {
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let tmp = path.with_extension("toml.tmp");
    std::fs::write(&tmp, text).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, path).map_err(|e| {
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
    fn broken_file_is_not_written() {
        let dir = TempDir::new().unwrap();
        let config = file_in(&dir, "config.toml", "[tui\n");

        assert!(config.set_bool("tui", "marquee", false).is_err());
        assert_eq!(read(&config), "[tui\n");
    }
}
