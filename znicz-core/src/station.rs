//! Saved radio stations: a TOML file of name + URL.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{Result, ZniczError};
use crate::player::commands::Command;
use crate::player::engine::PlayerOps;
use crate::player::state::QueueItem;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Station {
    pub name: String,
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub art: Option<PathBuf>,
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
    let file: StationsFile =
        toml::from_str(&text).map_err(|e| ZniczError::Player(format!("stations.toml: {e}")))?;
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
    stations.push(Station {
        name,
        url,
        art: None,
    });
    Ok(())
}

pub fn remove(stations: &mut Vec<Station>, name: &str) -> Result<()> {
    let Some(index) = stations.iter().position(|s| s.name == name) else {
        return Err(ZniczError::Player(format!("no station named {name}")));
    };
    stations.remove(index);
    Ok(())
}

pub fn rename(stations: &mut [Station], name: &str, new_name: &str) -> Result<()> {
    let new_name = validate_name(new_name)?;
    if stations
        .iter()
        .any(|s| s.name == new_name && s.name != name)
    {
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

pub fn set_url(stations: &mut [Station], name: &str, url: &str) -> Result<()> {
    let url = validate_url(url)?;
    let station = stations
        .iter_mut()
        .find(|s| s.name == name)
        .ok_or_else(|| ZniczError::Player(format!("no station named {name}")))?;
    station.url = url;
    Ok(())
}

fn expand_tilde(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Ok(home) = std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE")) {
            return PathBuf::from(home).join(rest);
        }
    }
    PathBuf::from(path)
}

pub fn set_art(stations: &mut [Station], name: &str, art: Option<&str>) -> Result<()> {
    let station = stations
        .iter_mut()
        .find(|s| s.name == name)
        .ok_or_else(|| ZniczError::Player(format!("no station named {name}")))?;
    let Some(raw) = art.map(str::trim).filter(|s| !s.is_empty()) else {
        station.art = None;
        return Ok(());
    };
    if raw.starts_with("http://") || raw.starts_with("https://") {
        return Err(ZniczError::Player(
            "station art must be a local image file".into(),
        ));
    }
    let path = expand_tilde(raw);
    if !path.is_file() {
        return Err(ZniczError::Player(format!(
            "station art not found: {}",
            path.display()
        )));
    }
    station.art = Some(
        path.canonicalize()
            .map_err(|e| ZniczError::Player(format!("station art: {e}")))?,
    );
    Ok(())
}

pub fn copy(stations: &mut Vec<Station>, name: &str, new_name: &str) -> Result<()> {
    let src = find(stations, name)
        .ok_or_else(|| ZniczError::Player(format!("no station named {name}")))?
        .clone();
    add(stations, new_name, &src.url)?;
    if let Some(last) = stations.last_mut() {
        last.art = src.art;
    }
    Ok(())
}

pub fn update(stations: &mut [Station], name: &str, new_name: &str, url: &str) -> Result<()> {
    let new_name = validate_name(new_name)?;
    let url = validate_url(url)?;
    if stations
        .iter()
        .any(|s| s.name == new_name && s.name != name)
    {
        return Err(ZniczError::Player(format!(
            "station {new_name:?} already exists"
        )));
    }
    let station = stations
        .iter_mut()
        .find(|s| s.name == name)
        .ok_or_else(|| ZniczError::Player(format!("no station named {name}")))?;
    station.name = new_name;
    station.url = url;
    Ok(())
}

pub fn play_station(player: &dyn PlayerOps, station: &Station, append: bool) -> Result<()> {
    if !append {
        player.send_blocking(Command::QueueClear)?;
    }
    player.send_blocking(Command::QueueAdd(vec![QueueItem::stream(
        station.name.clone(),
        station.url.clone(),
    )]))?;
    if !append {
        player.send_blocking(Command::QueuePlayIndex(0))?;
    }
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
                art: None,
            },
            Station {
                name: "Two".into(),
                url: "http://example.com/two".into(),
                art: None,
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
                art: None,
            },
            Station {
                name: "B".into(),
                url: "https://b".into(),
                art: None,
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
            art: None,
        }];
        set_url(&mut stations, "A", "https://b").unwrap();
        assert_eq!(stations[0].url, "https://b");
        remove(&mut stations, "A").unwrap();
        assert!(stations.is_empty());
        assert!(remove(&mut stations, "A").is_err());
    }

    #[test]
    fn copy_clones_the_url_under_a_new_name() {
        let mut stations = vec![Station {
            name: "A".into(),
            url: "https://a".into(),
            art: None,
        }];
        copy(&mut stations, "A", "B").unwrap();
        assert_eq!(stations.len(), 2);
        assert_eq!(stations[1].name, "B");
        assert_eq!(stations[1].url, "https://a");
        assert!(copy(&mut stations, "A", "B").is_err());
    }

    #[test]
    fn update_changes_name_and_url_together() {
        let mut stations = vec![
            Station {
                name: "A".into(),
                url: "https://a".into(),
                art: None,
            },
            Station {
                name: "B".into(),
                url: "https://b".into(),
                art: None,
            },
        ];
        update(&mut stations, "A", "C", "https://c").unwrap();
        assert_eq!(stations[0].name, "C");
        assert_eq!(stations[0].url, "https://c");
        assert!(update(&mut stations, "C", "B", "https://x").is_err());
    }

    #[test]
    fn art_round_trips_and_copy_keeps_the_path() {
        let png = tmp();
        let img = png.parent().unwrap().join("logo.png");
        fs::write(&img, b"not-a-real-decode-here").unwrap();
        let mut stations = vec![Station {
            name: "A".into(),
            url: "https://a".into(),
            art: None,
        }];
        set_art(&mut stations, "A", Some(img.to_str().unwrap())).unwrap();
        assert_eq!(
            stations[0].art.as_deref(),
            Some(img.canonicalize().unwrap().as_path())
        );
        copy(&mut stations, "A", "B").unwrap();
        assert_eq!(stations[1].art, stations[0].art);
        set_art(&mut stations, "A", None).unwrap();
        assert!(stations[0].art.is_none());
        assert!(stations[1].art.is_some());
    }

    #[test]
    fn art_rejects_http_and_missing_file() {
        let mut stations = vec![Station {
            name: "A".into(),
            url: "https://a".into(),
            art: None,
        }];
        assert!(set_art(&mut stations, "A", Some("https://x/a.png")).is_err());
        assert!(set_art(&mut stations, "A", Some("/definitely/missing/cover.png")).is_err());
        assert!(stations[0].art.is_none());
    }
}
