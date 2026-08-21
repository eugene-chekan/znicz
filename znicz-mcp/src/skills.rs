use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use walkdir::WalkDir;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillEntry {
    pub name: String,
    pub description: String,
    pub uri: String,
}

#[derive(Debug, Clone)]
pub struct SkillFile {
    pub uri: String,
    pub path: PathBuf,
    pub mime_type: String,
}

#[derive(Clone)]
pub struct SkillRegistry {
    files: HashMap<String, SkillFile>,
    entries: Vec<SkillEntry>,
}

impl SkillRegistry {
    pub fn load(dirs: &[PathBuf]) -> Self {
        let mut registry = Self {
            files: HashMap::new(),
            entries: Vec::new(),
        };

        for dir in dirs {
            if dir.is_dir() {
                registry.scan_dir(dir);
            }
        }

        registry.entries.sort_by(|a, b| a.name.cmp(&b.name));
        registry
    }

    fn scan_dir(&mut self, root: &Path) {
        for entry in WalkDir::new(root)
            .min_depth(1)
            .max_depth(3)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }

            let rel = path
                .strip_prefix(root)
                .unwrap_or(path)
                .to_string_lossy()
                .replace('\\', "/");

            let skill_name = rel.split('/').next().unwrap_or("skill").to_string();
            let file_part = rel
                .strip_prefix(&skill_name)
                .unwrap_or(rel.as_str())
                .trim_start_matches('/');

            let uri = if file_part.is_empty() || file_part == "SKILL.md" {
                format!("skill://{skill_name}/SKILL.md")
            } else {
                format!("skill://{skill_name}/{file_part}")
            };

            if path.file_name() == Some(std::ffi::OsStr::new("SKILL.md")) {
                if let Some((name, description)) = parse_skill_frontmatter(path) {
                    if !self.entries.iter().any(|e| e.name == name) {
                        self.entries.push(SkillEntry {
                            name,
                            description,
                            uri: uri.clone(),
                        });
                    }
                }
            }

            let mime_type = mime_for_path(path);
            self.files.insert(
                uri.clone(),
                SkillFile {
                    uri,
                    path: path.to_path_buf(),
                    mime_type,
                },
            );
        }
    }

    pub fn entries(&self) -> &[SkillEntry] {
        &self.entries
    }

    pub fn get_file(&self, uri: &str) -> Option<&SkillFile> {
        self.files.get(uri)
    }

    pub fn all_resources(&self) -> Vec<&SkillFile> {
        self.files.values().collect()
    }

    pub fn index_json(&self) -> String {
        serde_json::to_string_pretty(self.entries()).unwrap_or_else(|_| "[]".to_string())
    }
}

fn parse_skill_frontmatter(path: &Path) -> Option<(String, String)> {
    let content = std::fs::read_to_string(path).ok()?;
    if !content.starts_with("---") {
        return None;
    }
    let end = content[3..].find("---")?;
    let frontmatter = &content[3..3 + end];
    let mut name = None;
    let mut description = None;

    for line in frontmatter.lines() {
        if let Some((key, value)) = line.split_once(':') {
            match key.trim() {
                "name" => name = Some(value.trim().to_string()),
                "description" => description = Some(value.trim().to_string()),
                _ => {}
            }
        }
    }

    match (name, description) {
        (Some(n), Some(d)) => Some((n, d)),
        _ => None,
    }
}

fn mime_for_path(path: &Path) -> String {
    match path.extension().and_then(|e| e.to_str()) {
        Some("md") => "text/markdown".to_string(),
        Some("json") => "application/json".to_string(),
        Some("py") => "text/x-python".to_string(),
        Some("sh") => "text/x-shellscript".to_string(),
        _ => "application/octet-stream".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn bundled_skills_loaded() {
        let skills_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("skills");
        let registry = SkillRegistry::load(&[skills_dir]);
        assert!(registry.entries().len() >= 5);
    }

    #[test]
    fn parses_frontmatter() {
        let dir = std::env::temp_dir().join("znicz-skill-test");
        std::fs::create_dir_all(&dir).ok();
        let skill_dir = dir.join("test-skill");
        std::fs::create_dir_all(&skill_dir).ok();
        std::fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: test-skill\ndescription: A test skill\n---\n# Test\n",
        )
        .ok();

        let registry = SkillRegistry::load(std::slice::from_ref(&dir));
        assert!(registry.entries().iter().any(|e| e.name == "test-skill"));

        std::fs::remove_dir_all(&dir).ok();
    }
}
