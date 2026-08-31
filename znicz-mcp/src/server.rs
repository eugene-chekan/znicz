use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::tool::Parameters;
use rmcp::model::{
    Annotated, Content, GetPromptResult, ListPromptsResult, ListResourcesResult, Prompt,
    PromptMessage, PromptMessageContent, PromptMessageRole, RawResource, ReadResourceRequestParam,
    ReadResourceResult, Resource, ResourceContents, ServerCapabilities, ServerInfo,
};
use rmcp::{tool, tool_handler, tool_router, ErrorData as McpError, ServerHandler};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use znicz_core::{
    apply_to_player, copy_saved, list_saved, load_path, remove_saved, rename_saved, sanitize_stem,
    saved_path, write_path, AudioOutput, Command, LoadResult, PlaybackStatus, PlayerHandle,
    PlayerState,
};
use znicz_library::{Library, Track};

use crate::skills::SkillRegistry;

/// The library is shared and needs exclusive access for writes, so it sits
/// behind a mutex. `None` means no library was configured.
type SharedLibrary = Option<Arc<Mutex<Library>>>;

#[derive(Clone)]
pub struct ZniczMcpServer {
    player: PlayerHandle,
    skills: SkillRegistry,
    library: SharedLibrary,
    playlists_dir: PathBuf,
    stations_path: PathBuf,
    tool_router: ToolRouter<Self>,
}

impl ZniczMcpServer {
    /// Server without a music library. Library tools report that it is off.
    pub fn new(player: PlayerHandle, skills_dirs: Vec<PathBuf>) -> Self {
        Self::build(player, skills_dirs, None)
    }

    pub fn with_library(player: PlayerHandle, skills_dirs: Vec<PathBuf>, library: Library) -> Self {
        Self::build(player, skills_dirs, Some(Arc::new(Mutex::new(library))))
    }

    fn build(player: PlayerHandle, skills_dirs: Vec<PathBuf>, library: SharedLibrary) -> Self {
        let bundled = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("skills");
        let mut dirs = vec![bundled];
        dirs.extend(skills_dirs);
        let skills = SkillRegistry::load(&dirs);

        Self {
            player,
            skills,
            library,
            playlists_dir: znicz_library::default_playlists_dir()
                .unwrap_or_else(|| std::env::temp_dir().join("znicz-playlists")),
            stations_path: znicz_library::default_stations_path()
                .unwrap_or_else(|| std::env::temp_dir().join("znicz-stations.toml")),
            tool_router: Self::tool_router(),
        }
    }

    /// Run something against the library, or explain that there is none.
    fn with_library_locked<T>(
        &self,
        action: impl FnOnce(&mut Library) -> znicz_library::Result<T>,
    ) -> Result<T, McpError> {
        let Some(library) = self.library.as_ref() else {
            return Err(McpError::internal_error(
                "no music library configured; set [library].path in config.toml or run `znicz scan <dir>`",
                None,
            ));
        };

        let mut guard = library
            .lock()
            .map_err(|_| McpError::internal_error("library lock poisoned", None))?;

        action(&mut guard).map_err(|e| McpError::internal_error(e.to_string(), None))
    }

    fn json_result(value: &impl Serialize) -> Result<rmcp::model::CallToolResult, McpError> {
        let json = serde_json::to_string_pretty(value)
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        Ok(rmcp::model::CallToolResult::success(vec![Content::text(
            json,
        )]))
    }

    fn state_json(&self) -> Result<String, McpError> {
        serde_json::to_string_pretty(&self.player.state())
            .map_err(|e| McpError::internal_error(e.to_string(), None))
    }

    fn ok_state(&self) -> Result<rmcp::model::CallToolResult, McpError> {
        Ok(rmcp::model::CallToolResult::success(vec![Content::text(
            self.state_json()?,
        )]))
    }

    /// Apply a command, then report the state it produced.
    ///
    /// Waiting for the engine matters: reading state straight after queuing a
    /// command returns the previous snapshot, so callers cannot tell whether
    /// the command worked.
    fn apply(&self, command: Command) -> Result<rmcp::model::CallToolResult, McpError> {
        map_player_err(self.player.send_blocking(command))?;
        self.ok_state()
    }

    fn apply_playlist(
        &self,
        result: LoadResult,
        append: bool,
    ) -> Result<rmcp::model::CallToolResult, McpError> {
        map_player_err(apply_to_player(&self.player, &result, append))?;
        Self::json_result(&serde_json::json!({
            "loaded": result.paths.len(),
            "skipped": result.skipped,
            "state": self.player.state(),
        }))
    }

    fn map_io(err: znicz_core::ZniczError) -> McpError {
        McpError::internal_error(err.to_string(), None)
    }

    fn not_implemented(feature: &str) -> McpError {
        McpError::internal_error(format!("{feature} is not implemented yet"), None)
    }

    fn mutate_stations(
        &self,
        op: impl FnOnce(&mut Vec<znicz_core::Station>) -> znicz_core::Result<()>,
    ) -> Result<rmcp::model::CallToolResult, McpError> {
        let mut stations = znicz_core::load_stations(&self.stations_path).map_err(Self::map_io)?;
        op(&mut stations).map_err(Self::map_io)?;
        znicz_core::save_stations(&self.stations_path, &stations).map_err(Self::map_io)?;
        Self::json_result(&serde_json::json!({ "stations": stations }))
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
struct PlayParams {
    path: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct SeekParams {
    seconds: f64,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct VolumeParams {
    volume: f32,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct QueueAddParams {
    paths: Vec<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct DeviceParams {
    device_id: String,
}

#[derive(Debug, Serialize, JsonSchema)]
struct DeviceInfo {
    id: String,
    name: String,
    is_default: bool,
}

#[derive(Debug, Serialize, JsonSchema)]
struct DevicesResult {
    devices: Vec<DeviceInfo>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct ScanParams {
    /// Folder to walk. Subfolders are included.
    path: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct SearchParams {
    /// Free text matched against title, artist and album.
    query: String,
    /// Maximum rows to return (default 50).
    limit: Option<usize>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct TrackParams {
    /// Path to the audio file.
    path: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct AlbumParams {
    /// Album title, matched without case sensitivity.
    album: String,
}

/// One track, shaped for an agent to read.
#[derive(Debug, Serialize, JsonSchema)]
struct TrackView {
    path: String,
    title: String,
    artist: Option<String>,
    album: Option<String>,
    album_artist: Option<String>,
    genre: Option<String>,
    year: Option<u32>,
    track_number: Option<u32>,
    disc_number: Option<u32>,
    sample_rate: Option<u32>,
    channels: Option<u16>,
    bits_per_sample: Option<u32>,
    duration_secs: Option<f64>,
    /// False when the answer came from reading the file instead of the index.
    in_library: bool,
}

impl From<&Track> for TrackView {
    fn from(track: &Track) -> Self {
        Self {
            path: track.path.to_string_lossy().into_owned(),
            title: track.title.clone(),
            artist: track.artist.clone(),
            album: track.album.clone(),
            album_artist: track.album_artist.clone(),
            genre: track.genre.clone(),
            year: track.year,
            track_number: track.track_number,
            disc_number: track.disc_number,
            sample_rate: track.sample_rate,
            channels: track.channels,
            bits_per_sample: track.bits_per_sample,
            duration_secs: track.duration_secs,
            in_library: true,
        }
    }
}

impl TrackView {
    /// Read a file that was never scanned.
    fn from_file(path: &Path) -> Self {
        let metadata = znicz_core::read_metadata(path);
        let tags = metadata.tags;
        let properties = metadata.properties;

        Self {
            path: path.to_string_lossy().into_owned(),
            title: tags
                .title
                .clone()
                .unwrap_or_else(|| znicz_core::title_from_path(path)),
            artist: tags.artist,
            album: tags.album,
            album_artist: tags.album_artist,
            genre: tags.genre,
            year: tags.year,
            track_number: tags.track_number,
            disc_number: tags.disc_number,
            sample_rate: properties.sample_rate,
            channels: properties.channels,
            bits_per_sample: properties.bits_per_sample,
            duration_secs: properties.duration.map(|d| d.as_secs_f64()),
            in_library: false,
        }
    }
}

#[derive(Debug, Serialize, JsonSchema)]
struct SearchResult {
    query: String,
    count: usize,
    tracks: Vec<TrackView>,
}

#[derive(Debug, Serialize, JsonSchema)]
struct AlbumResult {
    album: String,
    count: usize,
    tracks: Vec<TrackView>,
}

#[derive(Debug, Serialize, JsonSchema)]
struct LibraryStats {
    tracks: u64,
    albums: u64,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct ImportPlaylistParams {
    path: String,
    /// Add to the queue instead of replacing it and starting playback.
    #[serde(default)]
    append: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct PlayPlaylistParams {
    name: String,
    #[serde(default)]
    append: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct SavePlaylistParams {
    name: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct RenamePlaylistParams {
    name: String,
    new_name: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct PlaylistNameParams {
    name: String,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
struct StationAddParams {
    name: String,
    url: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct StationNameParams {
    name: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct StationRenameParams {
    name: String,
    new_name: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct StationUrlParams {
    name: String,
    url: String,
}

#[tool_router]
impl ZniczMcpServer {
    #[tool(description = "Play a local audio file")]
    fn play(
        &self,
        Parameters(params): Parameters<PlayParams>,
    ) -> Result<rmcp::model::CallToolResult, McpError> {
        self.apply(Command::Play(znicz_core::QueueItem::file(params.path)))
    }

    #[tool(description = "Pause playback")]
    fn pause(&self) -> Result<rmcp::model::CallToolResult, McpError> {
        self.apply(Command::Pause)
    }

    #[tool(description = "Resume playback")]
    fn resume(&self) -> Result<rmcp::model::CallToolResult, McpError> {
        self.apply(Command::Resume)
    }

    #[tool(description = "Stop playback")]
    fn stop(&self) -> Result<rmcp::model::CallToolResult, McpError> {
        self.apply(Command::Stop)
    }

    #[tool(description = "Seek to position in seconds")]
    fn seek(
        &self,
        Parameters(params): Parameters<SeekParams>,
    ) -> Result<rmcp::model::CallToolResult, McpError> {
        let pos = std::time::Duration::from_secs_f64(params.seconds.max(0.0));
        self.apply(Command::Seek(pos))
    }

    #[tool(description = "Set volume between 0.0 and 1.0")]
    fn set_volume(
        &self,
        Parameters(params): Parameters<VolumeParams>,
    ) -> Result<rmcp::model::CallToolResult, McpError> {
        self.apply(Command::SetVolume(params.volume))
    }

    #[tool(description = "Play next track in queue")]
    fn next_track(&self) -> Result<rmcp::model::CallToolResult, McpError> {
        self.apply(Command::NextTrack)
    }

    #[tool(description = "Play previous track in queue")]
    fn previous_track(&self) -> Result<rmcp::model::CallToolResult, McpError> {
        self.apply(Command::PreviousTrack)
    }

    #[tool(description = "Add paths to the playback queue")]
    fn queue_add(
        &self,
        Parameters(params): Parameters<QueueAddParams>,
    ) -> Result<rmcp::model::CallToolResult, McpError> {
        let items = params
            .paths
            .into_iter()
            .map(znicz_core::QueueItem::file)
            .collect();
        self.apply(Command::QueueAdd(items))
    }

    #[tool(description = "Clear the playback queue")]
    fn queue_clear(&self) -> Result<rmcp::model::CallToolResult, McpError> {
        self.apply(Command::QueueClear)
    }

    #[tool(description = "Get current queue and player state")]
    fn queue_get(&self) -> Result<rmcp::model::CallToolResult, McpError> {
        self.ok_state()
    }

    #[tool(description = "Get full player state snapshot")]
    fn get_player_state(&self) -> Result<rmcp::model::CallToolResult, McpError> {
        self.ok_state()
    }

    #[tool(description = "List available audio output devices")]
    fn list_devices(&self) -> Result<rmcp::model::CallToolResult, McpError> {
        let devices = AudioOutput::list_devices()
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        let mapped: Vec<DeviceInfo> = devices
            .into_iter()
            .map(|d| DeviceInfo {
                id: d.id,
                name: d.name,
                is_default: d.is_default,
            })
            .collect();
        let json = serde_json::to_string_pretty(&DevicesResult { devices: mapped })
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        Ok(rmcp::model::CallToolResult::success(vec![Content::text(
            json,
        )]))
    }

    #[tool(description = "Select audio output device by id")]
    fn set_device(
        &self,
        Parameters(params): Parameters<DeviceParams>,
    ) -> Result<rmcp::model::CallToolResult, McpError> {
        self.apply(Command::SetDevice(params.device_id))
    }

    #[tool(description = "List bundled Agent Skills (SEP-2640 index)")]
    fn skills_list(&self) -> Result<rmcp::model::CallToolResult, McpError> {
        let json = self.skills.index_json();
        Ok(rmcp::model::CallToolResult::success(vec![Content::text(
            json,
        )]))
    }

    #[tool(description = "Scan a folder into the music library")]
    fn scan_library(
        &self,
        Parameters(params): Parameters<ScanParams>,
    ) -> Result<rmcp::model::CallToolResult, McpError> {
        let root = PathBuf::from(params.path);
        let report = self.with_library_locked(|library| library.scan(&root))?;
        Self::json_result(&report)
    }

    #[tool(description = "Search the music library by title, artist or album")]
    fn search_library(
        &self,
        Parameters(params): Parameters<SearchParams>,
    ) -> Result<rmcp::model::CallToolResult, McpError> {
        let limit = params.limit.unwrap_or(50).clamp(1, 500);
        let tracks = self.with_library_locked(|library| library.search(&params.query, limit))?;

        Self::json_result(&SearchResult {
            query: params.query,
            count: tracks.len(),
            tracks: tracks.iter().map(TrackView::from).collect(),
        })
    }

    #[tool(description = "Get metadata for one track by file path")]
    fn get_track(
        &self,
        Parameters(params): Parameters<TrackParams>,
    ) -> Result<rmcp::model::CallToolResult, McpError> {
        let path = PathBuf::from(&params.path);

        // Prefer the indexed row, but still answer for files never scanned.
        let indexed = match self.library.as_ref() {
            Some(library) => library
                .lock()
                .map_err(|_| McpError::internal_error("library lock poisoned", None))?
                .get_by_path(&path)
                .map_err(|e| McpError::internal_error(e.to_string(), None))?,
            None => None,
        };

        if let Some(track) = indexed {
            return Self::json_result(&TrackView::from(&track));
        }

        if !path.is_file() {
            return Err(McpError::invalid_params(
                format!(
                    "{} is not in the library and not a readable file",
                    params.path
                ),
                None,
            ));
        }

        Self::json_result(&TrackView::from_file(&path))
    }

    #[tool(description = "List the tracks of an album, in track order")]
    fn browse_album(
        &self,
        Parameters(params): Parameters<AlbumParams>,
    ) -> Result<rmcp::model::CallToolResult, McpError> {
        let tracks = self.with_library_locked(|library| library.browse_album(&params.album))?;

        if tracks.is_empty() {
            return Err(McpError::invalid_params(
                format!("no album named {:?} in the library", params.album),
                None,
            ));
        }

        Self::json_result(&AlbumResult {
            album: params.album,
            count: tracks.len(),
            tracks: tracks.iter().map(TrackView::from).collect(),
        })
    }

    #[tool(description = "List all albums in the music library")]
    fn list_albums(&self) -> Result<rmcp::model::CallToolResult, McpError> {
        let albums = self.with_library_locked(|library| library.albums())?;
        Self::json_result(&albums)
    }

    #[tool(description = "Library size and health")]
    fn library_stats(&self) -> Result<rmcp::model::CallToolResult, McpError> {
        let (tracks, albums) = self.with_library_locked(|library| {
            let tracks = library.track_count()?;
            let albums = library.albums()?.len();
            Ok((tracks, albums))
        })?;

        Self::json_result(&LibraryStats {
            tracks,
            albums: albums as u64,
        })
    }

    #[tool(description = "Remove library entries whose files are gone")]
    fn library_prune(&self) -> Result<rmcp::model::CallToolResult, McpError> {
        let removed = self.with_library_locked(|library| library.remove_missing())?;
        Self::json_result(&serde_json::json!({ "removed": removed }))
    }

    #[tool(description = "List saved M3U playlists")]
    fn list_playlists(&self) -> Result<rmcp::model::CallToolResult, McpError> {
        Self::json_result(&serde_json::json!({
            "playlists": list_saved(&self.playlists_dir),
        }))
    }

    #[tool(description = "Import an M3U playlist file into the queue")]
    fn import_playlist(
        &self,
        Parameters(params): Parameters<ImportPlaylistParams>,
    ) -> Result<rmcp::model::CallToolResult, McpError> {
        let result = load_path(Path::new(&params.path)).map_err(Self::map_io)?;
        self.apply_playlist(result, params.append)
    }

    #[tool(description = "Save the current queue as an M3U playlist")]
    fn save_playlist(
        &self,
        Parameters(params): Parameters<SavePlaylistParams>,
    ) -> Result<rmcp::model::CallToolResult, McpError> {
        let queue = self.player.state().queue;
        if queue.is_empty() {
            return Err(McpError::invalid_params("queue is empty", None));
        }
        let name = sanitize_stem(&params.name).map_err(Self::map_io)?;
        let paths = znicz_core::m3u_paths(&queue).map_err(Self::map_io)?;
        write_path(&self.playlists_dir.join(&name), &paths).map_err(Self::map_io)?;
        Self::json_result(&serde_json::json!({ "saved": name }))
    }

    #[tool(description = "Play a saved M3U playlist by name")]
    fn play_playlist(
        &self,
        Parameters(params): Parameters<PlayPlaylistParams>,
    ) -> Result<rmcp::model::CallToolResult, McpError> {
        let path = saved_path(&self.playlists_dir, &params.name).ok_or_else(|| {
            McpError::invalid_params(format!("no playlist named {:?}", params.name), None)
        })?;
        let result = load_path(&path).map_err(Self::map_io)?;
        self.apply_playlist(result, params.append)
    }

    #[tool(description = "Rename a saved M3U playlist")]
    fn rename_playlist(
        &self,
        Parameters(params): Parameters<RenamePlaylistParams>,
    ) -> Result<rmcp::model::CallToolResult, McpError> {
        let stem = rename_saved(&self.playlists_dir, &params.name, &params.new_name)
            .map_err(Self::map_io)?;
        Self::json_result(&serde_json::json!({ "renamed": stem }))
    }

    #[tool(description = "Copy a saved M3U playlist to a new name")]
    fn copy_playlist(
        &self,
        Parameters(params): Parameters<RenamePlaylistParams>,
    ) -> Result<rmcp::model::CallToolResult, McpError> {
        let stem = copy_saved(&self.playlists_dir, &params.name, &params.new_name)
            .map_err(Self::map_io)?;
        Self::json_result(&serde_json::json!({ "copied": stem }))
    }

    #[tool(description = "Delete a saved M3U playlist")]
    fn remove_playlist(
        &self,
        Parameters(params): Parameters<PlaylistNameParams>,
    ) -> Result<rmcp::model::CallToolResult, McpError> {
        remove_saved(&self.playlists_dir, &params.name).map_err(Self::map_io)?;
        Self::json_result(&serde_json::json!({ "removed": params.name }))
    }

    #[tool(description = "Add a radio station (name + HTTP URL)")]
    fn add_radio_station(
        &self,
        Parameters(params): Parameters<StationAddParams>,
    ) -> Result<rmcp::model::CallToolResult, McpError> {
        self.mutate_stations(|stations| {
            znicz_core::add_station(stations, &params.name, &params.url)
        })
    }

    #[tool(description = "List saved radio stations")]
    fn list_stations(&self) -> Result<rmcp::model::CallToolResult, McpError> {
        let stations = znicz_core::load_stations(&self.stations_path).map_err(Self::map_io)?;
        Self::json_result(&serde_json::json!({ "stations": stations }))
    }

    #[tool(description = "Play a saved radio station by name (clears the queue)")]
    fn play_station(
        &self,
        Parameters(params): Parameters<StationNameParams>,
    ) -> Result<rmcp::model::CallToolResult, McpError> {
        let stations = znicz_core::load_stations(&self.stations_path).map_err(Self::map_io)?;
        let station = znicz_core::find_station(&stations, &params.name)
            .cloned()
            .ok_or_else(|| {
                McpError::invalid_params(format!("no station named {:?}", params.name), None)
            })?;
        map_player_err(znicz_core::play_station(&self.player, &station))?;
        self.ok_state()
    }

    #[tool(description = "Remove a saved radio station")]
    fn remove_radio_station(
        &self,
        Parameters(params): Parameters<StationNameParams>,
    ) -> Result<rmcp::model::CallToolResult, McpError> {
        self.mutate_stations(|stations| znicz_core::remove_station(stations, &params.name))
    }

    #[tool(description = "Rename a saved radio station")]
    fn rename_radio_station(
        &self,
        Parameters(params): Parameters<StationRenameParams>,
    ) -> Result<rmcp::model::CallToolResult, McpError> {
        self.mutate_stations(|stations| {
            znicz_core::rename_station(stations, &params.name, &params.new_name)
        })
    }

    #[tool(description = "Change a saved radio station URL")]
    fn set_station_url(
        &self,
        Parameters(params): Parameters<StationUrlParams>,
    ) -> Result<rmcp::model::CallToolResult, McpError> {
        self.mutate_stations(|stations| {
            znicz_core::set_station_url(stations, &params.name, &params.url)
        })
    }

    #[tool(description = "Copy a saved radio station to a new name (same URL)")]
    fn copy_radio_station(
        &self,
        Parameters(params): Parameters<StationRenameParams>,
    ) -> Result<rmcp::model::CallToolResult, McpError> {
        self.mutate_stations(|stations| {
            znicz_core::copy_station(stations, &params.name, &params.new_name)
        })
    }

    #[tool(description = "Enrich metadata via MusicBrainz (Phase 6)")]
    fn enrich_metadata(&self) -> Result<rmcp::model::CallToolResult, McpError> {
        Err(Self::not_implemented("enrich_metadata"))
    }
}

#[tool_handler]
impl ServerHandler for ZniczMcpServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            instructions: Some(
                "Znicz audiophile music player. Use playback tools for transport control. \
                 Read znicz:// resources for live state. Use skills_list and skill:// resources for workflows."
                    .into(),
            ),
            capabilities: ServerCapabilities::builder()
                .enable_tools()
                .enable_resources()
                .enable_prompts()
                .build(),
            ..Default::default()
        }
    }

    fn list_resources(
        &self,
        _request: Option<rmcp::model::PaginatedRequestParam>,
        _context: rmcp::service::RequestContext<rmcp::RoleServer>,
    ) -> impl std::future::Future<Output = Result<ListResourcesResult, McpError>> + Send + '_ {
        let mut resources = vec![
            make_resource("znicz://now-playing", "now-playing", "Current track"),
            make_resource("znicz://queue", "queue", "Playback queue"),
            make_resource("znicz://player/status", "player-status", "Player status"),
            make_resource("znicz://devices", "devices", "Audio devices"),
            make_resource("znicz://config", "config", "Sanitized config"),
            make_resource("znicz://stations", "stations", "Saved radio stations"),
            make_resource(
                "skill://index.json",
                "skills-index",
                "Agent Skills discovery index",
            ),
        ];

        for file in self.skills.all_resources() {
            resources.push(make_resource(&file.uri, &file.uri, "Skill resource file"));
        }

        async move {
            Ok(ListResourcesResult {
                resources,
                ..Default::default()
            })
        }
    }

    fn read_resource(
        &self,
        request: ReadResourceRequestParam,
        _context: rmcp::service::RequestContext<rmcp::RoleServer>,
    ) -> impl std::future::Future<Output = Result<ReadResourceResult, McpError>> + Send + '_ {
        let uri = request.uri.clone();
        let state = self.player.state();
        let skills = self.skills.clone();
        let stations_path = self.stations_path.clone();

        async move {
            if uri == "skill://index.json" {
                return Ok(ReadResourceResult {
                    contents: vec![ResourceContents::TextResourceContents {
                        uri: uri.clone(),
                        mime_type: Some("application/json".into()),
                        text: skills.index_json(),
                    }],
                });
            }

            if let Some(file) = skills.get_file(&uri) {
                let text = std::fs::read_to_string(&file.path)
                    .map_err(|e| McpError::internal_error(e.to_string(), None))?;
                return Ok(ReadResourceResult {
                    contents: vec![ResourceContents::TextResourceContents {
                        uri,
                        mime_type: Some(file.mime_type.clone()),
                        text,
                    }],
                });
            }

            let json = match uri.as_str() {
                "znicz://now-playing" => serde_json::to_string_pretty(&state.current_track)
                    .map_err(|e| McpError::internal_error(e.to_string(), None))?,
                "znicz://queue" => serde_json::to_string_pretty(&state.queue)
                    .map_err(|e| McpError::internal_error(e.to_string(), None))?,
                "znicz://player/status" => {
                    serde_json::to_string_pretty(&PlayerStatusView::from(&state))
                        .map_err(|e| McpError::internal_error(e.to_string(), None))?
                }
                "znicz://devices" => {
                    let devices = AudioOutput::list_devices()
                        .map_err(|e| McpError::internal_error(e.to_string(), None))?;
                    serde_json::to_string_pretty(&devices)
                        .map_err(|e| McpError::internal_error(e.to_string(), None))?
                }
                "znicz://config" => serde_json::to_string_pretty(&ConfigView::default())
                    .map_err(|e| McpError::internal_error(e.to_string(), None))?,
                "znicz://stations" => {
                    let stations =
                        znicz_core::load_stations(&stations_path).map_err(Self::map_io)?;
                    serde_json::to_string_pretty(&stations)
                        .map_err(|e| McpError::internal_error(e.to_string(), None))?
                }
                _ => return Err(McpError::resource_not_found(uri, None)),
            };

            Ok(ReadResourceResult {
                contents: vec![ResourceContents::TextResourceContents {
                    uri,
                    mime_type: Some("application/json".into()),
                    text: json,
                }],
            })
        }
    }

    async fn list_prompts(
        &self,
        _request: Option<rmcp::model::PaginatedRequestParam>,
        _context: rmcp::service::RequestContext<rmcp::RoleServer>,
    ) -> Result<ListPromptsResult, McpError> {
        Ok(ListPromptsResult {
            prompts: vec![
                prompt_meta(
                    "audiophile-setup",
                    "Configure bit-perfect playback and device selection",
                ),
                prompt_meta(
                    "playback-session",
                    "Start a listening session with queue guidance",
                ),
                prompt_meta("explain-format", "Interpret current track codec and format"),
                prompt_meta("build-queue", "Build a multi-track queue"),
            ],
            ..Default::default()
        })
    }

    fn get_prompt(
        &self,
        request: rmcp::model::GetPromptRequestParam,
        _context: rmcp::service::RequestContext<rmcp::RoleServer>,
    ) -> impl std::future::Future<Output = Result<GetPromptResult, McpError>> + Send + '_ {
        let name = request.name.clone();
        let state = self.player.state();

        async move {
            let text = match name.as_str() {
                "audiophile-setup" => {
                    "Walk through bit-perfect setup: list_devices, match sample rate, set_device."
                        .to_string()
                }
                "playback-session" => {
                    "Start playback: queue_add paths, play first track, monitor get_player_state."
                        .to_string()
                }
                "explain-format" => {
                    if let Some(track) = &state.current_track {
                        format!(
                            "Current track: {} — {}",
                            track.title,
                            track.format_description()
                        )
                    } else {
                        "No track playing. Use play tool with a file path.".to_string()
                    }
                }
                "build-queue" => {
                    "Gather track paths, call queue_add, then play or next_track as needed."
                        .to_string()
                }
                _ => return Err(McpError::invalid_params("unknown prompt", None)),
            };

            Ok(GetPromptResult {
                description: None,
                messages: vec![PromptMessage {
                    role: PromptMessageRole::User,
                    content: PromptMessageContent::Text { text },
                }],
            })
        }
    }
}

#[derive(Serialize)]
struct PlayerStatusView {
    status: PlaybackStatus,
    position_secs: f64,
    volume: f32,
    device_id: Option<String>,
}

impl From<&PlayerState> for PlayerStatusView {
    fn from(state: &PlayerState) -> Self {
        Self {
            status: state.status,
            position_secs: state.position.as_secs_f64(),
            volume: state.volume,
            device_id: state.device_id.clone(),
        }
    }
}

#[derive(Serialize, Default)]
struct ConfigView {
    bit_perfect: bool,
    volume: f32,
}

fn make_resource(uri: &str, name: &str, description: &str) -> Resource {
    Annotated::new(
        RawResource {
            uri: uri.into(),
            name: name.into(),
            description: Some(description.into()),
            mime_type: Some("application/json".into()),
            size: None,
        },
        None,
    )
}

fn prompt_meta(name: &str, description: &str) -> Prompt {
    Prompt {
        name: name.into(),
        description: Some(description.into()),
        arguments: None,
    }
}

fn map_player_err(result: znicz_core::Result<()>) -> Result<(), McpError> {
    result.map_err(|e| McpError::internal_error(e.to_string(), None))
}

#[cfg(test)]
mod tests {
    use super::*;
    use znicz_core::{spawn_player, AudioConfig};

    /// Serialises `ZNICZ_STATIONS_PATH` writes against other tests that read env
    /// while constructing a server (`default_playlists_dir` / `default_stations_path`).
    static STATIONS_ENV: Mutex<()> = Mutex::new(());

    fn env_lock() -> std::sync::MutexGuard<'static, ()> {
        STATIONS_ENV
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn server() -> ZniczMcpServer {
        let (player, _thread) = spawn_player(AudioConfig::default());
        // The engine thread outlives the test; the handle keeps it alive.
        std::mem::forget(_thread);
        let _lock = env_lock();
        ZniczMcpServer::new(player, Vec::new())
    }

    fn server_with_library() -> ZniczMcpServer {
        let (player, _thread) = spawn_player(AudioConfig::default());
        std::mem::forget(_thread);
        let library = Library::open_in_memory().expect("in-memory library");
        let _lock = env_lock();
        ZniczMcpServer::with_library(player, Vec::new(), library)
    }

    fn result_text(result: &rmcp::model::CallToolResult) -> String {
        result
            .content
            .iter()
            .filter_map(|content| content.as_text().map(|text| text.text.clone()))
            .collect()
    }

    /// Issue #1: the snapshot a tool returns must show the change it made.
    #[test]
    fn set_volume_returns_the_new_volume() {
        let server = server();

        let result = server
            .set_volume(Parameters(VolumeParams { volume: 0.3 }))
            .expect("set_volume");

        let state: serde_json::Value =
            serde_json::from_str(&result_text(&result)).expect("state json");
        let volume = state["volume"].as_f64().expect("volume field");

        assert!(
            (volume - 0.3).abs() < 1e-6,
            "tool reported volume {volume}, expected 0.3"
        );
    }

    #[test]
    fn queue_add_returns_the_new_queue() {
        let server = server();

        let result = server
            .queue_add(Parameters(QueueAddParams {
                paths: vec!["/music/a.flac".into(), "/music/b.flac".into()],
            }))
            .expect("queue_add");

        let state: serde_json::Value =
            serde_json::from_str(&result_text(&result)).expect("state json");
        let queue = state["queue"].as_array().expect("queue field");

        assert_eq!(queue.len(), 2, "tool reported queue {queue:?}");
    }

    /// A failing command must surface as an error, not a silent stale snapshot.
    #[test]
    fn play_reports_a_missing_file() {
        let server = server();
        let missing = std::env::temp_dir().join("znicz-mcp-missing.flac");

        let result = server.play(Parameters(PlayParams {
            path: missing.to_string_lossy().into_owned(),
        }));

        assert!(result.is_err(), "expected an error, got {result:?}");
    }

    #[test]
    fn library_tools_explain_when_no_library_is_configured() {
        let server = server();

        let result = server.search_library(Parameters(SearchParams {
            query: "anything".into(),
            limit: None,
        }));

        let error = result.expect_err("should refuse without a library");
        assert!(
            error.to_string().contains("no music library"),
            "unhelpful error: {error}"
        );
    }

    #[test]
    fn empty_library_reports_zero_stats() {
        let server = server_with_library();

        let result = server.library_stats().expect("library_stats");
        let stats: serde_json::Value =
            serde_json::from_str(&result_text(&result)).expect("stats json");

        assert_eq!(stats["tracks"], 0);
        assert_eq!(stats["albums"], 0);
    }

    #[test]
    fn searching_an_empty_library_returns_no_tracks() {
        let server = server_with_library();

        let result = server
            .search_library(Parameters(SearchParams {
                query: "portishead".into(),
                limit: None,
            }))
            .expect("search");

        let payload: serde_json::Value =
            serde_json::from_str(&result_text(&result)).expect("search json");

        assert_eq!(payload["count"], 0);
        assert!(payload["tracks"].as_array().unwrap().is_empty());
    }

    #[test]
    fn browsing_an_unknown_album_is_an_error() {
        let server = server_with_library();

        let result = server.browse_album(Parameters(AlbumParams {
            album: "Nothing Here".into(),
        }));

        assert!(result.is_err(), "expected an error, got {result:?}");
    }

    /// get_track works for files that were never scanned.
    #[test]
    fn get_track_reads_a_file_outside_the_library() {
        let server = server_with_library();
        let path = std::env::temp_dir().join("znicz-mcp-get-track.flac");

        // A real tagged file is not needed: any readable file exercises the
        // fallback path, and an unreadable one must still not panic.
        std::fs::write(&path, b"not really audio").expect("write file");

        let result = server
            .get_track(Parameters(TrackParams {
                path: path.to_string_lossy().into_owned(),
            }))
            .expect("get_track");

        let view: serde_json::Value =
            serde_json::from_str(&result_text(&result)).expect("track json");

        assert_eq!(view["in_library"], false);
        assert_eq!(view["title"], "znicz-mcp-get-track");

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn get_track_rejects_a_path_that_does_not_exist() {
        let server = server_with_library();

        let result = server.get_track(Parameters(TrackParams {
            path: "/definitely/not/here.flac".into(),
        }));

        assert!(result.is_err(), "expected an error, got {result:?}");
    }

    #[test]
    fn scan_indexes_and_then_search_finds_the_track() {
        let server = server_with_library();

        // Build a tiny folder with one real FLAC, if ffmpeg is around.
        let dir = std::env::temp_dir().join("znicz-mcp-scan");
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).expect("create dir");

        let track = dir.join("track.flac");
        let made = std::process::Command::new("ffmpeg")
            .args(["-y", "-loglevel", "error"])
            .args(["-f", "lavfi", "-i", "sine=frequency=440:duration=1"])
            .args(["-metadata", "title=Test Title"])
            .args(["-metadata", "artist=Test Artist"])
            .args(["-metadata", "album=Test Album"])
            .args(["-c:a", "flac"])
            .arg(&track)
            .status()
            .map(|status| status.success())
            .unwrap_or(false);

        if !made {
            eprintln!("ffmpeg not available, skipping");
            std::fs::remove_dir_all(&dir).ok();
            return;
        }

        let scan = server
            .scan_library(Parameters(ScanParams {
                path: dir.to_string_lossy().into_owned(),
            }))
            .expect("scan_library");
        let report: serde_json::Value =
            serde_json::from_str(&result_text(&scan)).expect("scan json");
        assert_eq!(report["added"], 1, "report={report}");

        let found = server
            .search_library(Parameters(SearchParams {
                query: "Test Artist".into(),
                limit: None,
            }))
            .expect("search_library");
        let payload: serde_json::Value =
            serde_json::from_str(&result_text(&found)).expect("search json");

        assert_eq!(payload["count"], 1, "payload={payload}");
        assert_eq!(payload["tracks"][0]["title"], "Test Title");
        assert_eq!(payload["tracks"][0]["in_library"], true);

        let album = server
            .browse_album(Parameters(AlbumParams {
                album: "test album".into(), // case-insensitive
            }))
            .expect("browse_album");
        let album_payload: serde_json::Value =
            serde_json::from_str(&result_text(&album)).expect("album json");
        assert_eq!(album_payload["count"], 1);

        std::fs::remove_dir_all(&dir).ok();
    }

    fn playlist_server() -> (ZniczMcpServer, PathBuf) {
        let dir = std::env::temp_dir().join(format!(
            "znicz-mcp-playlists-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).expect("playlists dir");
        let mut server = server();
        server.playlists_dir = dir.clone();
        (server, dir)
    }

    fn touch_track(dir: &Path, name: &str) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, b"x").unwrap();
        path
    }

    fn ffmpeg_flac(dir: &Path, name: &str) -> Option<PathBuf> {
        let path = dir.join(name);
        let made = std::process::Command::new("ffmpeg")
            .args(["-y", "-loglevel", "error"])
            .args(["-f", "lavfi", "-i", "sine=frequency=440:duration=1"])
            .args(["-ac", "2", "-ar", "44100"])
            .args(["-c:a", "flac"])
            .arg(&path)
            .status()
            .map(|status| status.success())
            .unwrap_or(false);
        made.then_some(path)
    }

    #[test]
    fn save_playlist_errors_when_the_queue_is_empty() {
        let (server, dir) = playlist_server();
        let result = server.save_playlist(Parameters(SavePlaylistParams {
            name: "evening".into(),
        }));
        assert!(result.is_err(), "expected an error, got {result:?}");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn list_and_play_playlist_append_keeps_the_existing_queue() {
        let (server, dir) = playlist_server();
        let existing = touch_track(&dir, "old.flac");
        let added = touch_track(&dir, "new.flac");
        std::fs::write(dir.join("evening.m3u"), format!("{}\n", added.display())).unwrap();

        server
            .queue_add(Parameters(QueueAddParams {
                paths: vec![existing.to_string_lossy().into_owned()],
            }))
            .expect("seed queue");

        let listed = server.list_playlists().expect("list_playlists");
        let names: serde_json::Value =
            serde_json::from_str(&result_text(&listed)).expect("list json");
        assert_eq!(names["playlists"][0], "evening");

        let result = server
            .play_playlist(Parameters(PlayPlaylistParams {
                name: "evening".into(),
                append: true,
            }))
            .expect("play append");
        let payload: serde_json::Value =
            serde_json::from_str(&result_text(&result)).expect("play json");
        assert_eq!(payload["loaded"], 1);
        assert_eq!(payload["skipped"], 0);
        assert_eq!(payload["state"]["queue"].as_array().unwrap().len(), 2);
        assert_eq!(payload["state"]["status"], "Stopped");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn play_playlist_starts_playback_when_ffmpeg_can_make_a_file() {
        let (server, dir) = playlist_server();
        let Some(track) = ffmpeg_flac(&dir, "real.flac") else {
            eprintln!("ffmpeg not available, skipping playback assert");
            std::fs::remove_dir_all(&dir).ok();
            return;
        };
        std::fs::write(dir.join("evening.m3u"), format!("{}\n", track.display())).unwrap();

        let result = server
            .play_playlist(Parameters(PlayPlaylistParams {
                name: "evening".into(),
                append: false,
            }))
            .expect("play");
        let payload: serde_json::Value =
            serde_json::from_str(&result_text(&result)).expect("play json");
        assert_eq!(payload["loaded"], 1);
        assert_eq!(payload["state"]["status"], "Playing");
        assert_eq!(payload["state"]["queue"].as_array().unwrap().len(), 1);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn import_playlist_errors_when_nothing_is_playable() {
        let (server, dir) = playlist_server();
        let path = dir.join("empty.m3u");
        std::fs::write(&path, "# only a comment\nhttp://x\n").unwrap();

        let result = server.import_playlist(Parameters(ImportPlaylistParams {
            path: path.to_string_lossy().into_owned(),
            append: false,
        }));
        assert!(result.is_err(), "expected an error, got {result:?}");
        assert!(server.player.state().queue.is_empty());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn save_playlist_writes_the_queue() {
        let (server, dir) = playlist_server();
        let track = touch_track(&dir, "keep.flac");
        server
            .queue_add(Parameters(QueueAddParams {
                paths: vec![track.to_string_lossy().into_owned()],
            }))
            .expect("queue");

        let result = server
            .save_playlist(Parameters(SavePlaylistParams {
                name: "evening".into(),
            }))
            .expect("save");
        let payload: serde_json::Value =
            serde_json::from_str(&result_text(&result)).expect("save json");
        assert_eq!(payload["saved"], "evening.m3u");
        assert!(dir.join("evening.m3u").is_file());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn rename_playlist_moves_the_file() {
        let (server, dir) = playlist_server();
        std::fs::write(dir.join("evening.m3u"), "x\n").unwrap();
        let result = server
            .rename_playlist(Parameters(RenamePlaylistParams {
                name: "evening".into(),
                new_name: "night".into(),
            }))
            .expect("rename");
        let payload: serde_json::Value =
            serde_json::from_str(&result_text(&result)).expect("rename json");
        assert_eq!(payload["renamed"], "night");
        assert!(dir.join("night.m3u").is_file());
        assert!(!dir.join("evening.m3u").is_file());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn copy_playlist_duplicates_the_file() {
        let (server, dir) = playlist_server();
        std::fs::write(dir.join("evening.m3u"), "x\n").unwrap();
        let result = server
            .copy_playlist(Parameters(RenamePlaylistParams {
                name: "evening".into(),
                new_name: "night".into(),
            }))
            .expect("copy");
        let payload: serde_json::Value =
            serde_json::from_str(&result_text(&result)).expect("copy json");
        assert_eq!(payload["copied"], "night");
        assert!(dir.join("evening.m3u").is_file());
        assert!(dir.join("night.m3u").is_file());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn remove_playlist_deletes_the_file() {
        let (server, dir) = playlist_server();
        std::fs::write(dir.join("evening.m3u"), "x\n").unwrap();
        let result = server
            .remove_playlist(Parameters(PlaylistNameParams {
                name: "evening".into(),
            }))
            .expect("remove");
        let payload: serde_json::Value =
            serde_json::from_str(&result_text(&result)).expect("remove json");
        assert_eq!(payload["removed"], "evening");
        assert!(!dir.join("evening.m3u").is_file());

        std::fs::remove_dir_all(&dir).ok();
    }

    /// Path is captured at `ZniczMcpServer::new`; set `ZNICZ_STATIONS_PATH` first.
    fn station_server() -> (ZniczMcpServer, PathBuf) {
        let path = std::env::temp_dir().join(format!(
            "znicz-mcp-stations-{}-{}.toml",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let (player, _thread) = spawn_player(AudioConfig::default());
        std::mem::forget(_thread);
        let _lock = env_lock();
        // SAFETY: `STATIONS_ENV` serialises env mutation against other server constructors.
        unsafe {
            std::env::set_var("ZNICZ_STATIONS_PATH", &path);
        }
        let server = ZniczMcpServer::new(player, Vec::new());
        unsafe {
            std::env::remove_var("ZNICZ_STATIONS_PATH");
        }
        (server, path)
    }

    #[test]
    fn add_and_list_stations_round_trip() {
        let (server, path) = station_server();
        server
            .add_radio_station(Parameters(StationAddParams {
                name: "Example".into(),
                url: "https://example.com/stream".into(),
            }))
            .unwrap();
        let listed = server.list_stations().unwrap();
        let text = result_text(&listed);
        assert!(text.contains("Example"));
        assert!(text.contains("https://example.com/stream"));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn play_station_clears_the_queue_then_errors_on_a_dead_url() {
        let (server, path) = station_server();
        server
            .add_radio_station(Parameters(StationAddParams {
                name: "Dead".into(),
                url: "http://127.0.0.1:1/nope".into(),
            }))
            .unwrap();
        let err = server
            .play_station(Parameters(StationNameParams {
                name: "Dead".into(),
            }))
            .unwrap_err();
        assert!(!err.to_string().contains("not implemented"));
        let queue = server.player.state().queue;
        assert_eq!(queue.len(), 1);
        assert!(queue[0].is_stream());
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn duplicate_station_name_is_an_error() {
        let (server, path) = station_server();
        let params = StationAddParams {
            name: "Example".into(),
            url: "https://example.com/a".into(),
        };
        server
            .add_radio_station(Parameters(params.clone()))
            .unwrap();
        assert!(server
            .add_radio_station(Parameters(StationAddParams {
                name: "Example".into(),
                url: "https://example.com/b".into(),
            }))
            .is_err());
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn remove_rename_and_set_url_persist() {
        let (server, path) = station_server();
        server
            .add_radio_station(Parameters(StationAddParams {
                name: "Example".into(),
                url: "https://example.com/a".into(),
            }))
            .unwrap();
        server
            .rename_radio_station(Parameters(StationRenameParams {
                name: "Example".into(),
                new_name: "Renamed".into(),
            }))
            .unwrap();
        server
            .set_station_url(Parameters(StationUrlParams {
                name: "Renamed".into(),
                url: "https://example.com/b".into(),
            }))
            .unwrap();
        let listed = result_text(&server.list_stations().unwrap());
        assert!(listed.contains("Renamed"));
        assert!(listed.contains("https://example.com/b"));
        assert!(!listed.contains("Example"));
        server
            .remove_radio_station(Parameters(StationNameParams {
                name: "Renamed".into(),
            }))
            .unwrap();
        let listed = result_text(&server.list_stations().unwrap());
        assert!(!listed.contains("Renamed"));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn copy_radio_station_clones_the_url() {
        let (server, path) = station_server();
        server
            .add_radio_station(Parameters(StationAddParams {
                name: "Example".into(),
                url: "https://example.com/a".into(),
            }))
            .unwrap();
        server
            .copy_radio_station(Parameters(StationRenameParams {
                name: "Example".into(),
                new_name: "Example2".into(),
            }))
            .unwrap();
        let listed = result_text(&server.list_stations().unwrap());
        assert!(listed.contains("Example"));
        assert!(listed.contains("Example2"));
        let _ = std::fs::remove_file(path);
    }
}
