//! One method per Web API endpoint Znicz uses. Each call is a single HTTP
//! request (two for album tracks) with the caller's access token.

use serde_json::{Value, json};
use znicz_core::{
    SpotifyAccount, SpotifyDevice, SpotifyKind, SpotifyPage, SpotifyPlayback, SpotifyRepeat,
    SpotifyTrack,
};

use crate::error::{Result, SpotifyError};
use crate::http::{Body, Http, Method, check};
use crate::model;

/// Spotify caps search at 10 results per type for Development Mode apps.
const SEARCH_LIMIT_MAX: u32 = 10;

pub(crate) struct Api<'a> {
    pub http: &'a Http,
    /// `https://api.spotify.com/v1`, or a test server.
    pub base: &'a str,
    pub token: &'a str,
}

/// `spotify:album:ID` → (Album, ID). Only albums, playlists, artists and tracks.
pub(crate) fn split_uri(uri: &str) -> Option<(SpotifyKind, &str)> {
    let mut parts = uri.split(':');
    if parts.next()? != "spotify" {
        return None;
    }
    let kind = match parts.next()? {
        "album" => SpotifyKind::Album,
        "playlist" => SpotifyKind::Playlist,
        "artist" => SpotifyKind::Artist,
        "track" => SpotifyKind::Track,
        _ => return None,
    };
    let id = parts
        .next()
        .filter(|id| !id.is_empty() && id.bytes().all(|b| b.is_ascii_alphanumeric()))?;
    if parts.next().is_some() {
        return None;
    }
    Some((kind, id))
}

fn device_query(device_id: Option<&str>) -> Vec<(&str, &str)> {
    device_id
        .map(|id| vec![("device_id", id)])
        .unwrap_or_default()
}

impl Api<'_> {
    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base)
    }

    /// GET and parse; an empty body (204) is `Value::Null`.
    fn get(&self, path: &str, query: &[(&str, &str)]) -> Result<Value> {
        let reply = check(self.http.send(
            Method::Get,
            &self.url(path),
            query,
            Some(self.token),
            Body::None,
        )?)?;
        if reply.body.trim().is_empty() {
            return Ok(Value::Null);
        }
        serde_json::from_str(&reply.body).map_err(|e| SpotifyError::Parse(e.to_string()))
    }

    fn send(
        &self,
        method: Method,
        path: &str,
        query: &[(&str, &str)],
        body: Option<&Value>,
    ) -> Result<()> {
        let body = match body {
            Some(value) => Body::Json(value),
            None => Body::None,
        };
        check(
            self.http
                .send(method, &self.url(path), query, Some(self.token), body)?,
        )?;
        Ok(())
    }

    pub(crate) fn me(&self) -> Result<SpotifyAccount> {
        Ok(model::account(&self.get("/me", &[])?))
    }

    pub(crate) fn playback(&self, now_ms: u64) -> Result<Option<SpotifyPlayback>> {
        Ok(model::playback(&self.get("/me/player", &[])?, now_ms))
    }

    pub(crate) fn devices(&self) -> Result<Vec<SpotifyDevice>> {
        Ok(model::devices(&self.get("/me/player/devices", &[])?))
    }

    pub(crate) fn queue(&self) -> Result<(Option<SpotifyTrack>, Vec<SpotifyTrack>)> {
        Ok(model::queue(&self.get("/me/player/queue", &[])?))
    }

    pub(crate) fn search(
        &self,
        query: &str,
        kinds: &[SpotifyKind],
        limit: u32,
    ) -> Result<SpotifyPage> {
        let types: Vec<&str> = kinds.iter().map(|k| k.as_api()).collect();
        let types = types.join(",");
        let limit = limit.clamp(1, SEARCH_LIMIT_MAX).to_string();
        let json = self.get(
            "/search",
            &[("q", query), ("type", &types), ("limit", &limit)],
        )?;
        Ok(model::search(&json, kinds))
    }

    fn paged(&self, path: &str, offset: u32, limit: u32) -> Result<SpotifyPage> {
        let offset = offset.to_string();
        let limit = limit.clamp(1, 50).to_string();
        Ok(model::page(
            &self.get(path, &[("offset", &offset), ("limit", &limit)])?,
        ))
    }

    pub(crate) fn playlists(&self, offset: u32, limit: u32) -> Result<SpotifyPage> {
        self.paged("/me/playlists", offset, limit)
    }

    pub(crate) fn liked_songs(&self, offset: u32, limit: u32) -> Result<SpotifyPage> {
        self.paged("/me/tracks", offset, limit)
    }

    pub(crate) fn albums(&self, offset: u32, limit: u32) -> Result<SpotifyPage> {
        self.paged("/me/albums", offset, limit)
    }

    /// An album's or a playlist's tracks. Artists have no track list any more (see `artist_albums`).
    pub(crate) fn tracks(&self, uri: &str, offset: u32, limit: u32) -> Result<SpotifyPage> {
        let unknown = || SpotifyError::Parse(format!("{uri} is not an album, playlist or artist"));
        let (kind, id) = split_uri(uri).ok_or_else(unknown)?;
        match kind {
            SpotifyKind::Album => {
                // Album track rows carry no album; fill it from the album itself.
                let album = self.get(&format!("/albums/{id}"), &[])?;
                let mut page = self.paged(&format!("/albums/{id}/tracks"), offset, limit);
                if let Ok(page) = &mut page {
                    fill_album(page, &album);
                }
                page
            }
            SpotifyKind::Playlist => {
                match self.paged(&format!("/playlists/{id}/items"), offset, limit) {
                    Err(SpotifyError::Http {
                        status: 403 | 404, ..
                    }) => Err(SpotifyError::Unavailable(
                        "Spotify only lists the tracks of playlists you own or collaborate on; Enter still plays it"
                            .into(),
                    )),
                    other => other,
                }
            }
            SpotifyKind::Artist => Err(SpotifyError::Unavailable(
                "Spotify no longer lists an artist's top tracks; open their albums".into(),
            )),
            SpotifyKind::Track => Err(unknown()),
        }
    }

    pub(crate) fn artist_albums(&self, uri: &str, offset: u32, limit: u32) -> Result<SpotifyPage> {
        match split_uri(uri) {
            Some((SpotifyKind::Artist, id)) => {
                let offset = offset.to_string();
                let limit = limit.clamp(1, 50).to_string();
                let json = self.get(
                    &format!("/artists/{id}/albums"),
                    &[
                        ("include_groups", "album,single"),
                        ("offset", &offset),
                        ("limit", &limit),
                    ],
                )?;
                Ok(model::page(&json))
            }
            _ => Err(SpotifyError::Parse(format!("{uri} is not an artist"))),
        }
    }

    pub(crate) fn play(
        &self,
        device_id: Option<&str>,
        context_uri: Option<&str>,
        uris: &[String],
        offset: Option<u32>,
    ) -> Result<()> {
        let mut body = serde_json::Map::new();
        if let Some(context) = context_uri {
            body.insert("context_uri".into(), json!(context));
        } else if !uris.is_empty() {
            body.insert("uris".into(), json!(uris));
        }
        if let (false, Some(position)) = (body.is_empty(), offset) {
            body.insert("offset".into(), json!({ "position": position }));
        }
        let payload = Value::Object(body);
        let payload = payload
            .as_object()
            .is_some_and(|b| !b.is_empty())
            .then_some(&payload);
        self.send(
            Method::Put,
            "/me/player/play",
            &device_query(device_id),
            payload,
        )
    }

    pub(crate) fn pause(&self, device_id: Option<&str>) -> Result<()> {
        self.send(
            Method::Put,
            "/me/player/pause",
            &device_query(device_id),
            None,
        )
    }

    pub(crate) fn next(&self, device_id: Option<&str>) -> Result<()> {
        self.send(
            Method::Post,
            "/me/player/next",
            &device_query(device_id),
            None,
        )
    }

    pub(crate) fn previous(&self, device_id: Option<&str>) -> Result<()> {
        self.send(
            Method::Post,
            "/me/player/previous",
            &device_query(device_id),
            None,
        )
    }

    pub(crate) fn seek(&self, device_id: Option<&str>, position_ms: u64) -> Result<()> {
        let position = position_ms.to_string();
        let mut query = vec![("position_ms", position.as_str())];
        query.extend(device_query(device_id));
        self.send(Method::Put, "/me/player/seek", &query, None)
    }

    /// `volume` is 0.0–1.0.
    pub(crate) fn volume(&self, device_id: Option<&str>, volume: f32) -> Result<()> {
        let percent = ((volume.clamp(0.0, 1.0) * 100.0).round() as u8).to_string();
        let mut query = vec![("volume_percent", percent.as_str())];
        query.extend(device_query(device_id));
        self.send(Method::Put, "/me/player/volume", &query, None)
    }

    pub(crate) fn repeat(&self, device_id: Option<&str>, mode: SpotifyRepeat) -> Result<()> {
        let mut query = vec![("state", mode.as_api())];
        query.extend(device_query(device_id));
        self.send(Method::Put, "/me/player/repeat", &query, None)
    }

    pub(crate) fn shuffle(&self, device_id: Option<&str>, on: bool) -> Result<()> {
        let mut query = vec![("state", if on { "true" } else { "false" })];
        query.extend(device_query(device_id));
        self.send(Method::Put, "/me/player/shuffle", &query, None)
    }

    pub(crate) fn queue_add(&self, device_id: Option<&str>, uri: &str) -> Result<()> {
        let mut query = vec![("uri", uri)];
        query.extend(device_query(device_id));
        self.send(Method::Post, "/me/player/queue", &query, None)
    }

    pub(crate) fn transfer(&self, device_id: &str, play: bool) -> Result<()> {
        let body = json!({ "device_ids": [device_id], "play": play });
        self.send(Method::Put, "/me/player", &[], Some(&body))
    }
}

/// Copy the album's name, URI, year and image into each track row.
fn fill_album(page: &mut SpotifyPage, album: &Value) {
    let name = album["name"].as_str().unwrap_or("").to_string();
    let uri = album["uri"].as_str().unwrap_or("").to_string();
    let year = album["release_date"]
        .as_str()
        .and_then(|d| d.get(..4))
        .and_then(|y| y.parse().ok());
    let image = model::image(&album["images"]);
    for item in &mut page.items {
        item.image_url = image.clone();
        if let Some(track) = &mut item.track {
            track.album = name.clone();
            track.album_uri = uri.clone();
            track.year = year;
            track.image_url = image.clone();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::test_server::{Response, serve};

    fn api_for<'a>(http: &'a Http, base: &'a str) -> Api<'a> {
        Api {
            http,
            base,
            token: "tok",
        }
    }

    #[test]
    fn splits_uris() {
        assert_eq!(
            split_uri("spotify:album:abc"),
            Some((SpotifyKind::Album, "abc"))
        );
        assert_eq!(
            split_uri("spotify:playlist:p"),
            Some((SpotifyKind::Playlist, "p"))
        );
        assert_eq!(
            split_uri("spotify:artist:r"),
            Some((SpotifyKind::Artist, "r"))
        );
        assert_eq!(split_uri("spotify:show:s"), None);
        assert_eq!(split_uri("nonsense"), None);
        assert_eq!(split_uri("spotify:album:../me/player"), None);
        assert_eq!(split_uri("spotify:album:abc?x=1"), None);
        assert_eq!(split_uri("spotify:playlist:a/b"), None);
        assert_eq!(split_uri("spotify:album:abc:extra"), None);
    }

    #[test]
    fn play_sends_context_and_offset() {
        let (origin, seen) = serve(|_| Response::empty(204));
        let http = Http::new();
        let base = format!("{origin}/v1");
        api_for(&http, &base)
            .play(Some("d1"), Some("spotify:album:a1"), &[], Some(3))
            .unwrap();
        api_for(&http, &base)
            .play(None, None, &["spotify:track:t1".to_string()], Some(0))
            .unwrap();
        api_for(&http, &base).play(None, None, &[], None).unwrap();
        let seen = seen.lock().unwrap();
        assert_eq!(seen[0].method, "PUT");
        assert_eq!(seen[0].path(), "/v1/me/player/play");
        assert_eq!(seen[0].query("device_id").as_deref(), Some("d1"));
        let body: Value = serde_json::from_str(&seen[0].body).unwrap();
        assert_eq!(
            body,
            json!({"context_uri": "spotify:album:a1", "offset": {"position": 3}})
        );
        let body: Value = serde_json::from_str(&seen[1].body).unwrap();
        assert_eq!(
            body,
            json!({"uris": ["spotify:track:t1"], "offset": {"position": 0}})
        );
        assert_eq!(seen[2].body, "", "a resume sends no body");
    }

    #[test]
    fn transport_calls_use_the_documented_endpoints() {
        let (origin, seen) = serve(|_| Response::empty(204));
        let http = Http::new();
        let base = format!("{origin}/v1");
        let api = api_for(&http, &base);
        api.pause(None).unwrap();
        api.next(None).unwrap();
        api.previous(None).unwrap();
        api.seek(None, 61_000).unwrap();
        api.volume(None, 0.5).unwrap();
        api.repeat(None, SpotifyRepeat::Context).unwrap();
        api.shuffle(None, true).unwrap();
        api.queue_add(None, "spotify:track:t1").unwrap();
        api.transfer("d2", true).unwrap();
        let seen = seen.lock().unwrap();
        let lines: Vec<String> = seen
            .iter()
            .map(|r| format!("{} {}", r.method, r.path()))
            .collect();
        assert_eq!(
            lines,
            vec![
                "PUT /v1/me/player/pause",
                "POST /v1/me/player/next",
                "POST /v1/me/player/previous",
                "PUT /v1/me/player/seek",
                "PUT /v1/me/player/volume",
                "PUT /v1/me/player/repeat",
                "PUT /v1/me/player/shuffle",
                "POST /v1/me/player/queue",
                "PUT /v1/me/player",
            ]
        );
        assert_eq!(seen[3].query("position_ms").as_deref(), Some("61000"));
        assert_eq!(seen[4].query("volume_percent").as_deref(), Some("50"));
        assert_eq!(seen[5].query("state").as_deref(), Some("context"));
        assert_eq!(seen[6].query("state").as_deref(), Some("true"));
        assert_eq!(seen[7].query("uri").as_deref(), Some("spotify:track:t1"));
        let body: Value = serde_json::from_str(&seen[8].body).unwrap();
        assert_eq!(body, json!({"device_ids": ["d2"], "play": true}));
        assert_eq!(seen[0].header("authorization"), Some("Bearer tok"));
    }

    #[test]
    fn nothing_playing_is_none() {
        let (origin, _) = serve(|_| Response::empty(204));
        let http = Http::new();
        let base = format!("{origin}/v1");
        assert_eq!(api_for(&http, &base).playback(0).unwrap(), None);
    }

    #[test]
    fn album_tracks_get_the_albums_details() {
        let (origin, _) = serve(|request| match request.path() {
            "/v1/albums/a1" => Response::json(
                200,
                r#"{"uri": "spotify:album:a1", "name": "Kind of Blue", "release_date": "1959",
                    "images": [{"url": "https://i/640", "width": 640}]}"#,
            ),
            "/v1/albums/a1/tracks" => Response::json(
                200,
                r#"{"offset": 0, "total": 1, "items": [
                    {"type": "track", "uri": "spotify:track:t1", "name": "So What",
                     "duration_ms": 1, "track_number": 1, "artists": [{"name": "Miles Davis"}]}
                ]}"#,
            ),
            _ => Response::empty(404),
        });
        let http = Http::new();
        let base = format!("{origin}/v1");
        let page = api_for(&http, &base)
            .tracks("spotify:album:a1", 0, 50)
            .unwrap();
        let track = page.items[0].track.clone().unwrap();
        assert_eq!(track.album, "Kind of Blue");
        assert_eq!(track.year, Some(1959));
        assert_eq!(track.image_url.as_deref(), Some("https://i/640"));
        assert_eq!(page.items[0].image_url.as_deref(), Some("https://i/640"));
    }

    #[test]
    fn an_unknown_uri_is_refused() {
        let http = Http::new();
        let error = api_for(&http, "http://127.0.0.1:1")
            .tracks("spotify:show:s", 0, 50)
            .unwrap_err();
        assert!(matches!(error, SpotifyError::Parse(_)), "{error:?}");
    }

    #[test]
    fn playlist_tracks_use_the_items_endpoint() {
        let (origin, seen) = serve(|_| {
            Response::json(
                200,
                r#"{"offset":0,"total":1,"items":[{"added_at":"x","track":true,"item":{"type":"track","uri":"spotify:track:t1","name":"So What","duration_ms":1,"artists":[],"album":{"name":"Kind of Blue","images":[]}}}]}"#,
            )
        });
        let http = Http::new();
        let base = format!("{origin}/v1");
        let page = api_for(&http, &base)
            .tracks("spotify:playlist:p1", 0, 50)
            .unwrap();
        assert_eq!(page.items.len(), 1);
        assert_eq!(page.items[0].name, "So What");
        let seen = seen.lock().unwrap();
        assert_eq!(seen[0].path(), "/v1/playlists/p1/items");
    }

    #[test]
    fn a_playlist_you_do_not_own_is_unavailable() {
        let (origin, _) =
            serve(|_| Response::json(403, r#"{"error":{"status":403,"message":"Forbidden"}}"#));
        let http = Http::new();
        let base = format!("{origin}/v1");
        let error = api_for(&http, &base)
            .tracks("spotify:playlist:p1", 0, 50)
            .unwrap_err();
        assert!(matches!(error, SpotifyError::Unavailable(_)), "{error:?}");
    }

    #[test]
    fn artists_have_no_track_list() {
        let http = Http::new();
        let error = api_for(&http, "http://127.0.0.1:1")
            .tracks("spotify:artist:r1", 0, 50)
            .unwrap_err();
        assert!(matches!(error, SpotifyError::Unavailable(_)), "{error:?}");
    }

    #[test]
    fn search_asks_for_at_most_ten() {
        let (origin, seen) = serve(|_| Response::json(200, "{}"));
        let http = Http::new();
        let base = format!("{origin}/v1");
        api_for(&http, &base)
            .search("x", &[SpotifyKind::Track], 50)
            .unwrap();
        let seen = seen.lock().unwrap();
        assert_eq!(seen[0].query("limit").as_deref(), Some("10"));
    }
}
