//! Saved radio stations: a TOML file of name + URL.

use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::{Result, ZniczError};
use crate::player::commands::Command;
use crate::player::engine::PlayerHandle;
use crate::player::state::QueueItem;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Station {
    pub name: String,
    pub url: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct StationsFile {
    #[serde(default)]
    station: Vec<Station>,
}

pub fn load(path: &Path) -> Result<Vec<Station>> {
    if !path.is_file() {
        return Ok(Vec::new());
    }
    let text = fs::read_to_string(path)?;
    let file: StationsFile = toml::from_str(&text)
        .map_err(|e| ZniczError::Player(format!("stations.toml: {e}")))?;
    Ok(file.station)
}

pub fn save(path: &Path, stations: &[Station]) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let file = StationsFile {
        station: stations.to_vec(),
    };
    let text = toml::to_string_pretty(&file)
        .map_err(|e| ZniczError::Player(format!("stations.toml: {e}")))?;
    fs::write(path, text)?;
    Ok(())
}

pub fn validate_name(name: &str) -> Result<String> {
    let name = name.trim();
    if name.is_empty() {
        return Err(ZniczError::Player("illegal station name".into()));
    }
    if name.contains('/') || name.contains('\\') || name.contains("..") {
        return Err(ZniczError::Player("illegal station name".into()));
    }
    Ok(name.to_string())
}

pub fn validate_url(url: &str) -> Result<String> {
    let url = url.trim();
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err(ZniczError::Player(
            "station URL must be http:// or https://".into(),
        ));
    }
    Ok(url.to_string())
}

pub fn find<'a>(stations: &'a [Station], name: &str) -> Option<&'a Station> {
    stations.iter().find(|s| s.name == name)
}

pub fn add(stations: &mut Vec<Station>, name: &str, url: &str) -> Result<()> {
    let name = validate_name(name)?;
    let url = validate_url(url)?;
    if stations.iter().any(|s| s.name == name) {
        return Err(ZniczError::Player(format!(
            "station {name:?} already exists"
        )));
    }
    stations.push(Station { name, url });
    Ok(())
}

pub fn remove(stations: &mut Vec<Station>, name: &str) -> Result<()> {
    let Some(index) = stations.iter().position(|s| s.name == name) else {
        return Err(ZniczError::Player(format!("no station named {name}")));
    };
    stations.remove(index);
    Ok(())
}

pub fn rename(stations: &mut Vec<Station>, name: &str, new_name: &str) -> Result<()> {
    let new_name = validate_name(new_name)?;
    if stations.iter().any(|s| s.name == new_name && s.name != name) {
        return Err(ZniczError::Player(format!(
            "station {new_name:?} already exists"
        )));
    }
    let station = stations
        .iter_mut()
        .find(|s| s.name == name)
        .ok_or_else(|| ZniczError::Player(format!("no station named {name}")))?;
    station.name = new_name;
    Ok(())
}

pub fn set_url(stations: &mut Vec<Station>, name: &str, url: &str) -> Result<()> {
    let url = validate_url(url)?;
    let station = stations
        .iter_mut()
        .find(|s| s.name == name)
        .ok_or_else(|| ZniczError::Player(format!("no station named {name}")))?;
    station.url = url;
    Ok(())
}

pub fn play_station(player: &PlayerHandle, station: &Station) -> Result<()> {
    player.send_blocking(Command::QueueClear)?;
    player.send_blocking(Command::QueueAdd(vec![QueueItem::stream(
        station.name.clone(),
        station.url.clone(),
    )]))?;
    player.send_blocking(Command::QueuePlayIndex(0))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT: AtomicU64 = AtomicU64::new(0);

    fn tmp() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "znicz-stations-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&dir).unwrap();
        dir.join("stations.toml")
    }

    #[test]
    fn missing_file_is_an_empty_list() {
        let path = tmp();
        let _ = fs::remove_file(&path);
        assert!(load(&path).unwrap().is_empty());
    }

    #[test]
    fn round_trip_keeps_file_order() {
        let path = tmp();
        let stations = vec![
            Station {
                name: "One".into(),
                url: "https://example.com/one".into(),
            },
            Station {
                name: "Two".into(),
                url: "http://example.com/two".into(),
            },
        ];
        save(&path, &stations).unwrap();
        assert_eq!(load(&path).unwrap(), stations);
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains("[[station]]"));
        assert!(text.contains("name = \"One\""));
    }

    #[test]
    fn add_rejects_duplicate_names() {
        let mut stations = Vec::new();
        add(&mut stations, " Example ", "https://example.com/a").unwrap();
        let err = add(&mut stations, "Example", "https://example.com/b").unwrap_err();
        assert!(err.to_string().contains("already exists"));
        assert_eq!(stations.len(), 1);
        assert_eq!(stations[0].name, "Example");
    }

    #[test]
    fn names_reject_empty_slash_dotdot() {
        for name in ["", "  ", "a/b", "a\\b", "..", "foo..bar"] {
            assert!(validate_name(name).is_err(), "{name:?}");
        }
        assert_eq!(validate_name("  BBC  ").unwrap(), "BBC");
    }

    #[test]
    fn urls_must_be_http_or_https() {
        assert!(validate_url("ftp://x").is_err());
        assert!(validate_url("example.com/stream").is_err());
        assert!(validate_url("").is_err());
        assert_eq!(
            validate_url("  https://ex.com/s  ").unwrap(),
            "https://ex.com/s"
        );
        assert!(validate_url("http://ex.com/s").is_ok());
    }

    #[test]
    fn rename_collision_is_an_error() {
        let mut stations = vec![
            Station {
                name: "A".into(),
                url: "https://a".into(),
            },
            Station {
                name: "B".into(),
                url: "https://b".into(),
            },
        ];
        assert!(rename(&mut stations, "A", "B").is_err());
        rename(&mut stations, "A", "C").unwrap();
        assert_eq!(stations[0].name, "C");
        assert_eq!(stations[0].url, "https://a");
    }

    #[test]
    fn remove_and_set_url_by_name() {
        let mut stations = vec![Station {
            name: "A".into(),
            url: "https://a".into(),
        }];
        set_url(&mut stations, "A", "https://b").unwrap();
        assert_eq!(stations[0].url, "https://b");
        remove(&mut stations, "A").unwrap();
        assert!(stations.is_empty());
        assert!(remove(&mut stations, "A").is_err());
    }
}
