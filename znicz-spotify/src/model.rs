//! Read Web API JSON into the core types. Lenient: a missing field becomes a
//! default, and a row that is not a usable item (a removed track, a local
//! file in a playlist) is skipped.

use serde_json::Value;
use znicz_core::{
    SpotifyAccount, SpotifyDevice, SpotifyItem, SpotifyKind, SpotifyPage, SpotifyPlayback,
    SpotifyRepeat, SpotifyTrack,
};

/// Spotify album and playlist images go up to 640 px; the TUI covers are
/// never bigger.
const MAX_IMAGE_WIDTH: u64 = 640;

fn text(value: &Value) -> String {
    value.as_str().unwrap_or("").to_string()
}

fn names(list: &Value) -> Vec<String> {
    list.as_array()
        .map(|items| items.iter().map(|a| text(&a["name"])).collect())
        .unwrap_or_default()
}

/// The widest image no wider than 640 px; else the first one.
pub(crate) fn image(images: &Value) -> Option<String> {
    let list = images.as_array()?;
    let fitting = list
        .iter()
        .filter(|i| i["width"].as_u64().is_some_and(|w| w <= MAX_IMAGE_WIDTH))
        .max_by_key(|i| i["width"].as_u64().unwrap_or(0));
    fitting
        .or_else(|| list.first())
        .and_then(|i| i["url"].as_str())
        .map(str::to_string)
}

/// A track object. `None` for anything else (an episode, null).
pub(crate) fn track(value: &Value) -> Option<SpotifyTrack> {
    let kind = value["type"].as_str().unwrap_or("track");
    if !value.is_object() || kind != "track" {
        return None;
    }
    if value["is_local"].as_bool() == Some(true) {
        return None;
    }
    let uri = value["uri"].as_str()?;
    if uri.starts_with("spotify:local:") {
        return None;
    }
    let album = &value["album"];
    Some(SpotifyTrack {
        uri: uri.into(),
        title: text(&value["name"]),
        artists: names(&value["artists"]),
        album: text(&album["name"]),
        album_uri: text(&album["uri"]),
        year: album["release_date"]
            .as_str()
            .and_then(|d| d.get(..4))
            .and_then(|y| y.parse().ok()),
        track_number: value["track_number"].as_u64().map(|n| n as u32),
        duration_ms: value["duration_ms"].as_u64().unwrap_or(0),
        image_url: image(&album["images"]),
    })
}

/// A browse row from a track, album, artist or playlist object.
pub(crate) fn item(value: &Value) -> Option<SpotifyItem> {
    let kind = match value["type"].as_str() {
        Some("track") => SpotifyKind::Track,
        None if value.get("album").is_some() => SpotifyKind::Track,
        Some("album") => SpotifyKind::Album,
        Some("artist") => SpotifyKind::Artist,
        Some("playlist") => SpotifyKind::Playlist,
        _ => return None,
    };
    let uri = value["uri"].as_str()?.to_string();
    let name = text(&value["name"]);
    Some(match kind {
        SpotifyKind::Track => {
            let track = track(value)?;
            SpotifyItem {
                kind,
                uri,
                name,
                subtitle: track.artist_line(),
                image_url: track.image_url.clone(),
                track: Some(track),
                position: None,
            }
        }
        SpotifyKind::Album => SpotifyItem {
            kind,
            uri,
            name,
            subtitle: names(&value["artists"]).join(", "),
            image_url: image(&value["images"]),
            track: None,
            position: None,
        },
        SpotifyKind::Artist => SpotifyItem {
            kind,
            uri,
            name,
            subtitle: String::new(),
            image_url: image(&value["images"]),
            track: None,
            position: None,
        },
        SpotifyKind::Playlist => SpotifyItem {
            kind,
            uri,
            name,
            subtitle: text(&value["owner"]["display_name"]),
            image_url: image(&value["images"]),
            track: None,
            position: None,
        },
    })
}

/// A paging object. Saved tracks and albums wrap each row as
/// `{"track": …}` / `{"album": …}`; playlists as `{"track": …}` or `{"item": …}`.
pub(crate) fn page(value: &Value) -> SpotifyPage {
    let offset = value["offset"].as_u64().unwrap_or(0) as u32;
    let rows = value["items"].as_array();
    // A row's position counts every raw row, the skipped ones too: Spotify's
    // context offsets count them, and so does the next page's offset.
    let items = rows
        .map(|rows| {
            rows.iter()
                .enumerate()
                .filter_map(|(index, row)| {
                    // Wrapper rows (`{"added_at": …, "track": …}`) have no `type`;
                    // a track object has an `album` key, so check `type` first.
                    let found = if row.get("type").is_some() {
                        item(row)
                    } else {
                        ["track", "album", "item"]
                            .iter()
                            .find_map(|key| item(&row[*key]))
                    };
                    found.map(|found| SpotifyItem {
                        position: Some(offset + index as u32),
                        ..found
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    SpotifyPage {
        items,
        offset,
        total: value["total"].as_u64().unwrap_or(0) as u32,
        next_offset: rows.map(|rows| offset + rows.len() as u32),
    }
}

/// `/search` groups results by kind; keep the order the caller asked for.
pub(crate) fn search(value: &Value, kinds: &[SpotifyKind], offset: u32) -> SpotifyPage {
    let pages: Vec<SpotifyPage> = kinds
        .iter()
        .map(|kind| page(&value[format!("{}s", kind.as_api())]))
        .collect();
    // Each kind pages on its own, so the largest per-kind total is what a caller can page through.
    let total = pages.iter().map(|p| p.total).max().unwrap_or(0);
    SpotifyPage {
        items: pages.into_iter().flat_map(|p| p.items).collect(),
        offset,
        total,
        next_offset: None,
    }
}

pub(crate) fn device(value: &Value) -> Option<SpotifyDevice> {
    Some(SpotifyDevice {
        id: value["id"].as_str()?.to_string(),
        name: text(&value["name"]),
        kind: text(&value["type"]),
        is_active: value["is_active"].as_bool().unwrap_or(false),
        volume_percent: value["volume_percent"].as_u64().map(|v| v.min(100) as u8),
    })
}

pub(crate) fn devices(value: &Value) -> Vec<SpotifyDevice> {
    value["devices"]
        .as_array()
        .map(|list| list.iter().filter_map(device).collect())
        .unwrap_or_default()
}

/// `/me/player`. `None` when nothing is playing (the API answers 204).
pub(crate) fn playback(value: &Value, now_ms: u64) -> Option<SpotifyPlayback> {
    if !value.is_object() {
        return None;
    }
    let item = track(&value["item"]);
    let kind = value["currently_playing_type"].as_str().unwrap_or("track");
    Some(SpotifyPlayback {
        unsupported: item.is_none() && (kind != "track" || !value["item"].is_null()),
        item,
        is_playing: value["is_playing"].as_bool().unwrap_or(false),
        progress_ms: value["progress_ms"].as_u64().unwrap_or(0),
        read_at_ms: now_ms,
        repeat: SpotifyRepeat::from_api(value["repeat_state"].as_str().unwrap_or("off")),
        shuffle: value["shuffle_state"].as_bool().unwrap_or(false),
        device: device(&value["device"]),
        context_uri: value["context"]["uri"].as_str().map(str::to_string),
    })
}

pub(crate) fn account(value: &Value) -> SpotifyAccount {
    let user = value["display_name"]
        .as_str()
        .filter(|name| !name.is_empty())
        .or_else(|| value["id"].as_str())
        .unwrap_or("")
        .to_string();
    SpotifyAccount {
        user,
        premium: value["product"].as_str().map(|p| p == "premium"),
    }
}

pub(crate) fn queue(value: &Value) -> (Option<SpotifyTrack>, Vec<SpotifyTrack>) {
    let next = value["queue"]
        .as_array()
        .map(|list| list.iter().filter_map(track).collect())
        .unwrap_or_default();
    (track(&value["currently_playing"]), next)
}

#[cfg(test)]
mod tests {
    use super::*;

    const TRACK: &str = r#"{
        "type": "track", "uri": "spotify:track:t1", "name": "So What",
        "duration_ms": 562000, "track_number": 1,
        "artists": [{"name": "Miles Davis"}, {"name": "John Coltrane"}],
        "album": {
            "uri": "spotify:album:a1", "name": "Kind of Blue", "release_date": "1959-08-17",
            "images": [
                {"url": "https://i/1280", "width": 1280, "height": 1280},
                {"url": "https://i/640", "width": 640, "height": 640},
                {"url": "https://i/64", "width": 64, "height": 64}
            ]
        }
    }"#;

    fn json(text: &str) -> Value {
        serde_json::from_str(text).unwrap()
    }

    #[test]
    fn reads_a_track() {
        let track = track(&json(TRACK)).unwrap();
        assert_eq!(track.uri, "spotify:track:t1");
        assert_eq!(track.title, "So What");
        assert_eq!(track.artists, vec!["Miles Davis", "John Coltrane"]);
        assert_eq!(track.album, "Kind of Blue");
        assert_eq!(track.album_uri, "spotify:album:a1");
        assert_eq!(track.year, Some(1959));
        assert_eq!(track.track_number, Some(1));
        assert_eq!(track.duration_ms, 562_000);
        assert_eq!(track.image_url.as_deref(), Some("https://i/640"));
    }

    #[test]
    fn picks_the_first_image_when_widths_are_unknown() {
        let images = json(r#"[{"url": "https://a", "width": null}, {"url": "https://b"}]"#);
        assert_eq!(image(&images).as_deref(), Some("https://a"));
        assert_eq!(image(&json("[]")), None);
    }

    #[test]
    fn reads_playback() {
        let text = format!(
            r#"{{
                "device": {{"id": "d1", "name": "Kitchen", "type": "Speaker", "is_active": true, "volume_percent": 70}},
                "repeat_state": "context", "shuffle_state": true, "is_playing": true,
                "progress_ms": 62000, "currently_playing_type": "track",
                "context": {{"uri": "spotify:album:a1"}},
                "item": {TRACK}
            }}"#
        );
        let playback = playback(&json(&text), 5_000).unwrap();
        assert!(playback.is_playing);
        assert!(!playback.unsupported);
        assert_eq!(playback.progress_ms, 62_000);
        assert_eq!(playback.read_at_ms, 5_000);
        assert_eq!(playback.repeat, SpotifyRepeat::Context);
        assert!(playback.shuffle);
        assert_eq!(playback.device.unwrap().name, "Kitchen");
        assert_eq!(playback.context_uri.as_deref(), Some("spotify:album:a1"));
        assert_eq!(playback.item.unwrap().title, "So What");
    }

    #[test]
    fn an_episode_is_unsupported_and_nothing_playing_is_none() {
        let episode = json(
            r#"{"is_playing": true, "progress_ms": 1, "currently_playing_type": "episode", "item": null}"#,
        );
        let playback = playback(&episode, 0).unwrap();
        assert!(playback.unsupported);
        assert_eq!(playback.item, None);
        assert_eq!(super::playback(&Value::Null, 0), None);
    }

    #[test]
    fn reads_devices_and_skips_restricted_ones() {
        let devices = devices(&json(
            r#"{"devices": [
                {"id": "d1", "name": "Laptop", "type": "Computer", "is_active": false, "volume_percent": 50},
                {"id": null, "name": "Restricted", "type": "Speaker", "is_active": false}
            ]}"#,
        ));
        assert_eq!(
            devices,
            vec![SpotifyDevice {
                id: "d1".into(),
                name: "Laptop".into(),
                kind: "Computer".into(),
                is_active: false,
                volume_percent: Some(50),
            }]
        );
    }

    #[test]
    fn reads_the_account() {
        let account = account(&json(
            r#"{"id": "u1", "display_name": "Eugene", "product": "premium"}"#,
        ));
        assert_eq!(
            account,
            SpotifyAccount {
                user: "Eugene".into(),
                premium: Some(true)
            }
        );
        let free = super::account(&json(
            r#"{"id": "u2", "display_name": null, "product": "free"}"#,
        ));
        assert_eq!(
            free,
            SpotifyAccount {
                user: "u2".into(),
                premium: Some(false)
            }
        );
        let unknown = super::account(&json(r#"{"id": "u3", "display_name": "X"}"#));
        assert_eq!(unknown.premium, None, "Spotify no longer reports the plan");
    }

    #[test]
    fn reads_saved_tracks_and_skips_empty_rows() {
        let text = format!(
            r#"{{"offset": 50, "total": 120, "items": [
                {{"added_at": "x", "track": {TRACK}}},
                {{"added_at": "y", "track": null}}
            ]}}"#
        );
        let page = page(&json(&text));
        assert_eq!(page.offset, 50);
        assert_eq!(page.total, 120);
        assert_eq!(page.items.len(), 1);
        let item = &page.items[0];
        assert_eq!(item.kind, SpotifyKind::Track);
        assert_eq!(item.name, "So What");
        assert_eq!(item.subtitle, "Miles Davis, John Coltrane");
        assert!(item.track.is_some());
    }

    #[test]
    fn skips_local_files_in_a_playlist() {
        let text = format!(
            r#"{{"offset": 0, "total": 2, "items": [
                {{"added_at": "x", "is_local": false, "track": {TRACK}}},
                {{"added_at": "y", "is_local": true, "track": {{"type": "track", "uri": "spotify:local:a:b:c:1", "name": "Mine", "is_local": true, "duration_ms": 1, "artists": [], "album": {{"name": "", "images": []}}}}}}
            ]}}"#
        );
        let page = page(&json(&text));
        assert_eq!(page.items.len(), 1);
    }

    #[test]
    fn a_playlist_page_numbers_rows_by_their_full_position_and_points_past_them() {
        let local = r#"{"type": "track", "uri": "spotify:local:a:b:c:1", "name": "Mine", "is_local": true, "duration_ms": 1, "artists": [], "album": {"name": "", "images": []}}"#;
        let text = format!(
            r#"{{"offset": 100, "total": 200, "items": [
                {{"added_at": "x", "track": {TRACK}}},
                {{"added_at": "y", "is_local": true, "track": {local}}},
                {{"added_at": "z", "track": {TRACK}}}
            ]}}"#
        );
        let page = page(&json(&text));
        assert_eq!(page.items.len(), 2, "the local row is skipped");
        assert_eq!(page.items[0].position, Some(100));
        assert_eq!(
            page.items[1].position,
            Some(102),
            "the skipped row still counts"
        );
        assert_eq!(page.next_offset, Some(103), "three raw rows were returned");
        assert!(page.has_more());
    }

    #[test]
    fn wrapper_row_uses_the_first_key_with_a_usable_item() {
        let text = format!(
            r#"{{"offset": 0, "total": 2, "items": [
                {{"added_at": "x", "track": null, "item": {TRACK}}},
                {{"added_at": "y", "track": null}}
            ]}}"#
        );
        let page = page(&json(&text));
        assert_eq!(page.items.len(), 1);
    }

    #[test]
    fn reads_albums_playlists_and_artists() {
        let page = page(&json(
            r#"{"offset": 0, "total": 3, "items": [
                {"album": {"type": "album", "uri": "spotify:album:a1", "name": "Kind of Blue",
                           "artists": [{"name": "Miles Davis"}], "images": []}},
                {"type": "playlist", "uri": "spotify:playlist:p1", "name": "Jazz",
                 "owner": {"display_name": "Eugene"}, "images": null},
                {"type": "artist", "uri": "spotify:artist:r1", "name": "Miles Davis", "images": []}
            ]}"#,
        ));
        let kinds: Vec<_> = page.items.iter().map(|i| i.kind).collect();
        assert_eq!(
            kinds,
            vec![
                SpotifyKind::Album,
                SpotifyKind::Playlist,
                SpotifyKind::Artist
            ]
        );
        assert_eq!(page.items[0].subtitle, "Miles Davis");
        assert_eq!(page.items[1].subtitle, "Eugene");
        assert_eq!(page.items[2].subtitle, "");
    }

    #[test]
    fn search_results_keep_the_requested_kind_order() {
        let text = format!(
            r#"{{
                "tracks": {{"items": [{TRACK}], "total": 1}},
                "albums": {{"items": [{{"type": "album", "uri": "spotify:album:a1", "name": "Kind of Blue", "artists": [], "images": []}}], "total": 1}}
            }}"#
        );
        let page = search(&json(&text), &[SpotifyKind::Album, SpotifyKind::Track], 0);
        let kinds: Vec<_> = page.items.iter().map(|i| i.kind).collect();
        assert_eq!(kinds, vec![SpotifyKind::Album, SpotifyKind::Track]);
        assert_eq!(
            page.total, 1,
            "the largest per-kind total, not the row count"
        );
    }

    #[test]
    fn search_total_is_the_largest_kind_total_and_keeps_the_offset() {
        let text = r#"{
            "tracks": {"items": [], "total": 37},
            "albums": {"items": [], "total": 5}
        }"#;
        let page = search(&json(text), &[SpotifyKind::Album, SpotifyKind::Track], 20);
        assert_eq!(page.total, 37);
        assert_eq!(page.offset, 20);
    }

    #[test]
    fn reads_the_queue() {
        let text = format!(
            r#"{{"currently_playing": {TRACK}, "queue": [{TRACK}, {{"type": "episode"}}]}}"#
        );
        let (current, next) = queue(&json(&text));
        assert_eq!(current.unwrap().title, "So What");
        assert_eq!(next.len(), 1);
    }
}
