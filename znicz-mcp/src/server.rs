use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::tool::Parameters;
use rmcp::model::{
    Annotated, Content, GetPromptResult, ListPromptsResult, ListResourcesResult, Prompt,
    PromptMessage, PromptMessageContent, PromptMessageRole, RawResource, ReadResourceRequestParam,
    ReadResourceResult, Resource, ResourceContents, ServerCapabilities, ServerInfo,
};
use rmcp::{ErrorData as McpError, ServerHandler, tool, tool_handler, tool_router};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use znicz_core::{
    AudioOutput, Command, IpcClient, LibraryHost, LibraryOp, LoadResult, PlaybackStatus,
    PlayerState, ScanSummary, SpotifyHost, SpotifyOp, SpotifyReply, SpotifyState, apply_to_player,
    copy_saved, list_saved, load_path, remove_saved, rename_saved, sanitize_stem, saved_path,
    wait_for_library, write_path,
};
use znicz_library::{Library, Track};

use crate::skills::SkillRegistry;
use crate::spotify_json;

/// The library is shared and needs exclusive access for writes, so it sits
/// behind a mutex. `None` means no library was configured.
type SharedLibrary = Option<Arc<Mutex<Library>>>;

#[derive(Clone)]
pub struct ZniczMcpServer {
    player: IpcClient,
    /// A second, independent connection for the Spotify ops that can be slow:
    /// a call can take about 40 s, and it must not hold up the player tools on
    /// `player`. Status reads use `player`, since they answer at once.
    spotify: IpcClient,
    skills: SkillRegistry,
    library: SharedLibrary,
    playlists_dir: PathBuf,
    stations_path: PathBuf,
    tool_router: ToolRouter<Self>,
}

impl ZniczMcpServer {
    /// Server without a music library. Library tools report that it is off.
    pub fn new(player: IpcClient, spotify: IpcClient, skills_dirs: Vec<PathBuf>) -> Self {
        Self::build(player, spotify, skills_dirs, None)
    }

    pub fn with_library(
        player: IpcClient,
        spotify: IpcClient,
        skills_dirs: Vec<PathBuf>,
        library: Library,
    ) -> Self {
        Self::build(
            player,
            spotify,
            skills_dirs,
            Some(Arc::new(Mutex::new(library))),
        )
    }

    fn build(
        player: IpcClient,
        spotify: IpcClient,
        skills_dirs: Vec<PathBuf>,
        library: SharedLibrary,
    ) -> Self {
        let bundled = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("skills");
        let mut dirs = vec![bundled];
        dirs.extend(skills_dirs);
        let skills = SkillRegistry::load(&dirs);

        Self {
            player,
            spotify,
            skills,
            library,
            playlists_dir: znicz_library::default_playlists_dir()
                .unwrap_or_else(|| temp_fallback("znicz-playlists")),
            stations_path: znicz_library::default_stations_path()
                .unwrap_or_else(|| temp_fallback("znicz-stations.toml")),
            tool_router: Self::tool_router(),
        }
    }

    fn live_state(&self) -> Result<PlayerState, McpError> {
        self.player
            .state()
            .map_err(|e| McpError::internal_error(e.to_string(), None))
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

    /// Send a library job to the player; with `wait`, return what it did.
    /// Blocks while waiting, so callers run it off the async runtime.
    fn run_library_job(
        player: &IpcClient,
        op: LibraryOp,
        wait: bool,
    ) -> Result<Option<ScanSummary>, McpError> {
        let start = LibraryHost::status(player).scans_finished;
        let failure = op.clone();
        LibraryHost::apply(player, op)
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        if !wait {
            return Ok(None);
        }
        let status = wait_for_library(player, start + 1, None, |_| {})
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        let summary = status.last.unwrap_or_default();
        if let Some(error) = &summary.error {
            return Err(McpError::internal_error(failure.failure_text(error), None));
        }
        Ok(Some(summary))
    }

    /// `run_library_job` on a blocking thread, so the server keeps answering.
    async fn run_library_job_blocking(
        &self,
        op: LibraryOp,
        wait: bool,
    ) -> Result<Option<ScanSummary>, McpError> {
        let player = self.player.clone();
        tokio::task::spawn_blocking(move || Self::run_library_job(&player, op, wait))
            .await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?
    }

    /// One Spotify op on the Spotify connection, on a blocking thread: a
    /// Spotify call can take about 40 s, and the server must keep answering.
    async fn spotify_apply(&self, op: SpotifyOp) -> Result<SpotifyReply, McpError> {
        let spotify = self.spotify.clone();
        tokio::task::spawn_blocking(move || SpotifyHost::apply(&spotify, op))
            .await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?
            .map_err(spotify_json::error_of)
    }

    /// The Spotify snapshot. It sends `Touch` first, so the player polls fast
    /// for the next minute, as it does after any Spotify call. Both go over
    /// the main connection, so a slow Spotify call on `spotify` does not delay them.
    async fn spotify_status_state(&self) -> Result<SpotifyState, McpError> {
        let player = self.player.clone();
        tokio::task::spawn_blocking(move || {
            SpotifyHost::apply(&player, SpotifyOp::Touch)?;
            SpotifyHost::try_state(&player)
        })
        .await
        .map_err(|e| McpError::internal_error(e.to_string(), None))?
        .map_err(spotify_json::error_of)
    }

    fn spotify_status_result(
        state: &SpotifyState,
    ) -> Result<rmcp::model::CallToolResult, McpError> {
        Self::json_result(&spotify_json::status_json(state, spotify_json::now_ms()))
    }

    /// Run one Spotify op and return its reply as the tool result.
    async fn spotify_reply(&self, op: SpotifyOp) -> Result<rmcp::model::CallToolResult, McpError> {
        let reply = self.spotify_apply(op).await?;
        Self::json_result(&reply)
    }

    fn json_result(value: &impl Serialize) -> Result<rmcp::model::CallToolResult, McpError> {
        let json = serde_json::to_string_pretty(value)
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        Ok(rmcp::model::CallToolResult::success(vec![Content::text(
            json,
        )]))
    }

    fn state_json(&self) -> Result<String, McpError> {
        serde_json::to_string_pretty(&self.live_state()?)
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
        self.player
            .send_blocking(command)
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        self.ok_state()
    }

    fn apply_playlist(
        &self,
        result: LoadResult,
        append: bool,
    ) -> Result<rmcp::model::CallToolResult, McpError> {
        apply_to_player(&self.player, &result, append)
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        Self::json_result(&serde_json::json!({
            "loaded": result.items.len(),
            "skipped": result.skipped,
            "state": self.live_state()?,
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
struct QueueRemoveParams {
    /// 0-based queue index, same as TUI `d` / `Command::QueueRemove`.
    index: usize,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct QueueMoveParams {
    /// 0-based index of the row to move.
    from: usize,
    /// 0-based index it should end up at (as the queue is before the move).
    to: usize,
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
    /// Wait for the scan to finish (default) or return as soon as it is queued.
    #[serde(default)]
    wait: Option<bool>,
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
struct StationPlayParams {
    name: String,
    #[serde(default)]
    append: bool,
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

#[derive(Debug, Deserialize, JsonSchema)]
struct StationArtParams {
    name: String,
    #[serde(default)]
    path: Option<String>,
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

    #[tool(description = "Add file paths and http(s) stream URLs to the playback queue")]
    fn queue_add(
        &self,
        Parameters(params): Parameters<QueueAddParams>,
    ) -> Result<rmcp::model::CallToolResult, McpError> {
        let items = params
            .paths
            .into_iter()
            .map(znicz_core::QueueItem::file_or_http_url)
            .collect();
        self.apply(Command::QueueAdd(items))
    }

    #[tool(description = "Remove one queue row by 0-based index")]
    fn queue_remove(
        &self,
        Parameters(params): Parameters<QueueRemoveParams>,
    ) -> Result<rmcp::model::CallToolResult, McpError> {
        self.apply(Command::QueueRemove(params.index))
    }

    #[tool(
        description = "Move one queue row to another 0-based index; the playing track keeps playing"
    )]
    fn queue_move(
        &self,
        Parameters(params): Parameters<QueueMoveParams>,
    ) -> Result<rmcp::model::CallToolResult, McpError> {
        self.apply(Command::QueueMove {
            from: params.from,
            to: params.to,
        })
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

    #[tool(
        description = "Scan a folder into the music library (through the player, which remembers the folder)"
    )]
    async fn scan_library(
        &self,
        Parameters(params): Parameters<ScanParams>,
    ) -> Result<rmcp::model::CallToolResult, McpError> {
        let path = std::path::absolute(expand_home(&params.path, dirs::home_dir().as_deref()))
            .ok()
            .filter(|p| p.is_dir())
            .ok_or_else(|| {
                McpError::invalid_params(format!("not a folder: {}", params.path), None)
            })?;
        let summary = self
            .run_library_job_blocking(LibraryOp::AddFolder { path }, params.wait.unwrap_or(true))
            .await?;
        match summary {
            None => Self::json_result(&serde_json::json!({ "queued": true })),
            Some(s) => Self::json_result(&serde_json::json!({
                "seen": s.seen,
                "added": s.added,
                "updated": s.updated,
                "unchanged": s.unchanged,
                "failed": s.failed,
            })),
        }
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

    #[tool(description = "Drop library entries whose files no longer exist")]
    async fn library_prune(&self) -> Result<rmcp::model::CallToolResult, McpError> {
        let summary = self
            .run_library_job_blocking(LibraryOp::Prune, true)
            .await?
            .unwrap_or_default();
        Self::json_result(&serde_json::json!({ "removed": summary.removed }))
    }

    #[tool(
        description = "Remembered music folders, the running scan's progress, and the last result"
    )]
    fn library_status(&self) -> Result<rmcp::model::CallToolResult, McpError> {
        Self::json_result(&LibraryHost::status(&self.player))
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
        let queue = self.live_state()?.queue;
        if queue.is_empty() {
            return Err(McpError::invalid_params("queue is empty", None));
        }
        let name = sanitize_stem(&params.name).map_err(Self::map_io)?;
        write_path(&self.playlists_dir.join(&name), &queue).map_err(Self::map_io)?;
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

    #[tool(
        description = "Play a saved radio station by name. append true adds without starting; default clears the queue and plays"
    )]
    fn play_station(
        &self,
        Parameters(params): Parameters<StationPlayParams>,
    ) -> Result<rmcp::model::CallToolResult, McpError> {
        let stations = znicz_core::load_stations(&self.stations_path).map_err(Self::map_io)?;
        let station = znicz_core::find_station(&stations, &params.name)
            .cloned()
            .ok_or_else(|| {
                McpError::invalid_params(format!("no station named {:?}", params.name), None)
            })?;
        znicz_core::play_station(&self.player, &station, params.append)
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
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

    #[tool(description = "Set a station's local cover image path; omit path to clear")]
    fn set_station_art(
        &self,
        Parameters(params): Parameters<StationArtParams>,
    ) -> Result<rmcp::model::CallToolResult, McpError> {
        self.mutate_stations(|stations| {
            znicz_core::set_station_art(stations, &params.name, params.path.as_deref())
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

    #[tool(
        description = "Spotify status: configured, login, account, Premium, device, what is playing and the last error. Call this first"
    )]
    async fn spotify_status(&self) -> Result<rmcp::model::CallToolResult, McpError> {
        let state = self.spotify_status_state().await?;
        Self::spotify_status_result(&state)
    }

    #[tool(
        description = "Start a Spotify login. Returns authorize_url: give it to the user, who approves it in a browser"
    )]
    async fn spotify_login_begin(&self) -> Result<rmcp::model::CallToolResult, McpError> {
        match self.spotify_apply(SpotifyOp::BeginLogin).await? {
            SpotifyReply::AuthorizeUrl {
                url,
                redirect_uri,
                listening,
            } => Self::json_result(&serde_json::json!({
                "authorize_url": url,
                "redirect_uri": redirect_uri,
                "listening": listening,
                "next": "Give authorize_url to the user. If listening is true, the browser can redirect to redirect_uri and the login finishes by itself. Otherwise, or if that fails, the user pastes the address the browser lands on into spotify_login_complete.",
            })),
            _ => Err(spotify_json::unexpected_reply()),
        }
    }

    #[tool(
        description = "Finish a Spotify login with the address the browser landed on (the paste route)"
    )]
    async fn spotify_login_complete(
        &self,
        Parameters(params): Parameters<spotify_json::SpotifyLoginCompleteParams>,
    ) -> Result<rmcp::model::CallToolResult, McpError> {
        self.spotify_apply(SpotifyOp::CompleteLogin {
            redirect_url: params.redirect_url,
        })
        .await?;
        let state = self.spotify_status_state().await?;
        Self::spotify_status_result(&state)
    }

    #[tool(description = "Log out of Spotify and delete the stored login")]
    async fn spotify_logout(&self) -> Result<rmcp::model::CallToolResult, McpError> {
        self.spotify_apply(SpotifyOp::Logout).await?;
        let state = self.spotify_status_state().await?;
        Self::spotify_status_result(&state)
    }

    #[tool(
        description = "Search Spotify for tracks, albums, artists and playlists. Spotify returns at most 10 per kind; to page, search one kind at a time and use offset"
    )]
    async fn spotify_search(
        &self,
        Parameters(params): Parameters<spotify_json::SpotifySearchParams>,
    ) -> Result<rmcp::model::CallToolResult, McpError> {
        self.spotify_reply(spotify_json::search_op(params)).await
    }

    #[tool(description = "The user's Spotify playlists, a page at a time")]
    async fn spotify_playlists(
        &self,
        Parameters(params): Parameters<spotify_json::SpotifyPagedParams>,
    ) -> Result<rmcp::model::CallToolResult, McpError> {
        let (offset, limit) = spotify_json::page(params.offset, params.limit);
        self.spotify_reply(SpotifyOp::Playlists { offset, limit })
            .await
    }

    #[tool(description = "The user's liked songs, a page at a time")]
    async fn spotify_liked_songs(
        &self,
        Parameters(params): Parameters<spotify_json::SpotifyPagedParams>,
    ) -> Result<rmcp::model::CallToolResult, McpError> {
        let (offset, limit) = spotify_json::page(params.offset, params.limit);
        self.spotify_reply(SpotifyOp::LikedSongs { offset, limit })
            .await
    }

    #[tool(description = "The albums the user has saved, a page at a time")]
    async fn spotify_albums(
        &self,
        Parameters(params): Parameters<spotify_json::SpotifyPagedParams>,
    ) -> Result<rmcp::model::CallToolResult, McpError> {
        let (offset, limit) = spotify_json::page(params.offset, params.limit);
        self.spotify_reply(SpotifyOp::Albums { offset, limit })
            .await
    }

    #[tool(
        description = "Tracks of an album, or of a playlist the user owns or collaborates on. Pass its uri. Other playlists can still be played, but their tracks cannot be listed"
    )]
    async fn spotify_tracks(
        &self,
        Parameters(params): Parameters<spotify_json::SpotifyUriPagedParams>,
    ) -> Result<rmcp::model::CallToolResult, McpError> {
        let (offset, limit) = spotify_json::page(params.offset, params.limit);
        self.spotify_reply(SpotifyOp::Tracks {
            uri: params.uri,
            offset,
            limit,
        })
        .await
    }

    #[tool(description = "An artist's albums. Pass the artist's uri")]
    async fn spotify_artist_albums(
        &self,
        Parameters(params): Parameters<spotify_json::SpotifyUriPagedParams>,
    ) -> Result<rmcp::model::CallToolResult, McpError> {
        let (offset, limit) = spotify_json::page(params.offset, params.limit);
        self.spotify_reply(SpotifyOp::ArtistAlbums {
            uri: params.uri,
            offset,
            limit,
        })
        .await
    }

    #[tool(
        description = "Play on Spotify. uri plays one track; context_uri plays an album or playlist from offset; giving neither resumes. Starting Spotify pauses Znicz's own playback"
    )]
    async fn spotify_play(
        &self,
        Parameters(params): Parameters<spotify_json::SpotifyPlayParams>,
    ) -> Result<rmcp::model::CallToolResult, McpError> {
        let op = spotify_json::play_op(params)?;
        self.spotify_reply(op).await
    }

    #[tool(description = "Pause Spotify. Znicz's playback is not affected")]
    async fn spotify_pause(&self) -> Result<rmcp::model::CallToolResult, McpError> {
        self.spotify_reply(SpotifyOp::Pause).await
    }

    #[tool(description = "Next track on Spotify")]
    async fn spotify_next(&self) -> Result<rmcp::model::CallToolResult, McpError> {
        self.spotify_reply(SpotifyOp::Next).await
    }

    #[tool(description = "Previous track on Spotify")]
    async fn spotify_previous(&self) -> Result<rmcp::model::CallToolResult, McpError> {
        self.spotify_reply(SpotifyOp::Previous).await
    }

    #[tool(description = "Seek within the Spotify track, in milliseconds")]
    async fn spotify_seek(
        &self,
        Parameters(params): Parameters<spotify_json::SpotifySeekParams>,
    ) -> Result<rmcp::model::CallToolResult, McpError> {
        self.spotify_reply(SpotifyOp::Seek {
            position_ms: params.position_ms,
        })
        .await
    }

    #[tool(description = "Spotify volume between 0.0 and 1.0, the same scale as set_volume")]
    async fn spotify_set_volume(
        &self,
        Parameters(params): Parameters<spotify_json::SpotifyVolumeParams>,
    ) -> Result<rmcp::model::CallToolResult, McpError> {
        let volume = spotify_json::volume(params.volume)?;
        self.spotify_reply(SpotifyOp::SetVolume { volume }).await
    }

    #[tool(description = "Spotify repeat mode: off, track or context")]
    async fn spotify_set_repeat(
        &self,
        Parameters(params): Parameters<spotify_json::SpotifyRepeatParams>,
    ) -> Result<rmcp::model::CallToolResult, McpError> {
        self.spotify_reply(SpotifyOp::SetRepeat {
            mode: params.mode.into(),
        })
        .await
    }

    #[tool(description = "Turn Spotify shuffle on or off")]
    async fn spotify_set_shuffle(
        &self,
        Parameters(params): Parameters<spotify_json::SpotifyShuffleParams>,
    ) -> Result<rmcp::model::CallToolResult, McpError> {
        self.spotify_reply(SpotifyOp::SetShuffle { on: params.on })
            .await
    }

    #[tool(description = "Add a track to the Spotify queue")]
    async fn spotify_queue_add(
        &self,
        Parameters(params): Parameters<spotify_json::SpotifyQueueAddParams>,
    ) -> Result<rmcp::model::CallToolResult, McpError> {
        self.spotify_reply(SpotifyOp::QueueAdd { uri: params.uri })
            .await
    }

    #[tool(description = "Spotify devices the account can see, with their ids")]
    async fn spotify_devices(&self) -> Result<rmcp::model::CallToolResult, McpError> {
        self.spotify_reply(SpotifyOp::Devices).await
    }

    #[tool(
        description = "Move Spotify playback to a device from spotify_devices. play: true starts it there"
    )]
    async fn spotify_transfer(
        &self,
        Parameters(params): Parameters<spotify_json::SpotifyTransferParams>,
    ) -> Result<rmcp::model::CallToolResult, McpError> {
        self.spotify_reply(SpotifyOp::Transfer {
            device_id: params.device_id,
            play: params.play.unwrap_or(false),
        })
        .await
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
                "znicz://spotify/now-playing",
                "spotify-now-playing",
                "Spotify login, account, playback and devices",
            ),
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
        let state = self.live_state();
        let player = self.player.clone();
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

            if uri == "znicz://spotify/now-playing" {
                let snapshot = tokio::task::spawn_blocking(move || SpotifyHost::try_state(&player))
                    .await
                    .map_err(|e| McpError::internal_error(e.to_string(), None))?
                    .map_err(spotify_json::error_of)?;
                let text = serde_json::to_string_pretty(&snapshot)
                    .map_err(|e| McpError::internal_error(e.to_string(), None))?;
                return Ok(ReadResourceResult {
                    contents: vec![ResourceContents::TextResourceContents {
                        uri,
                        mime_type: Some("application/json".into()),
                        text,
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
                "znicz://now-playing" => serde_json::to_string_pretty(&state?.current_track)
                    .map_err(|e| McpError::internal_error(e.to_string(), None))?,
                "znicz://queue" => serde_json::to_string_pretty(&state?.queue)
                    .map_err(|e| McpError::internal_error(e.to_string(), None))?,
                "znicz://player/status" => {
                    serde_json::to_string_pretty(&PlayerStatusView::from(&state?))
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
        let state = self.live_state();

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
                    if let Some(track) = &state?.current_track {
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

/// Expand a leading `~` or `~/` against `home`; anything else is left alone.
fn expand_home(path: &str, home: Option<&Path>) -> PathBuf {
    match (home, path.strip_prefix('~')) {
        (Some(home), Some("")) => home.to_path_buf(),
        (Some(home), Some(rest)) if rest.starts_with('/') => home.join(&rest[1..]),
        _ => PathBuf::from(path),
    }
}

/// Where to keep data when the user data directory cannot be found: the
/// temp dir, which the system may empty, so say so in the log.
fn temp_fallback(name: &str) -> PathBuf {
    let path = std::env::temp_dir().join(name);
    tracing::warn!(path = %path.display(), "no user data directory; using a temporary path");
    path
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_leading_tilde_expands_to_home() {
        let home = Path::new("/home/me");
        assert_eq!(
            super::expand_home("~/Music", Some(home)),
            Path::new("/home/me/Music")
        );
        assert_eq!(super::expand_home("~", Some(home)), home);
        assert_eq!(super::expand_home("~x/y", Some(home)), Path::new("~x/y"));
        assert_eq!(
            super::expand_home("/a/~/b", Some(home)),
            Path::new("/a/~/b")
        );
        assert_eq!(super::expand_home("~/Music", None), Path::new("~/Music"));
    }

    use super::*;
    use std::ops::{Deref, DerefMut};
    use std::time::Duration;
    use znicz_core::{AudioConfig, ClientRole, IpcServer, PlaybackStatus, spawn_player};
    use znicz_core::{SpotifyAccount, SpotifyErrorKind, SpotifyLogin, SpotifyPage, ZniczError};

    /// Serialises `ZNICZ_STATIONS_PATH` writes against
    /// other tests that read env while constructing a server.
    static STATIONS_ENV: Mutex<()> = Mutex::new(());

    fn env_lock() -> std::sync::MutexGuard<'static, ()> {
        STATIONS_ENV
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    struct Harness {
        server: ZniczMcpServer,
        host: znicz_core::PlayerHandle,
        _ipc: IpcServer,
        // Holds the folder with the advertise file, removed with the test.
        _dir: tempfile::TempDir,
    }

    impl Deref for Harness {
        type Target = ZniczMcpServer;
        fn deref(&self) -> &ZniczMcpServer {
            &self.server
        }
    }

    impl DerefMut for Harness {
        fn deref_mut(&mut self) -> &mut ZniczMcpServer {
            &mut self.server
        }
    }

    fn hosted_player(dir: &Path) -> (znicz_core::PlayerHandle, IpcServer, IpcClient) {
        let path = dir.join("ipc.toml");
        let (host, thread) = spawn_player(AudioConfig::default());
        std::mem::forget(thread);
        let ipc = IpcServer::start(host.clone(), &path).expect("start ipc");
        let client = IpcClient::connect(&path, ClientRole::Agent).expect("connect agent");
        (host, ipc, client)
    }

    /// A second, independent connection to the player at `dir`, as `znicz mcp`
    /// opens for Spotify calls.
    fn second_client(dir: &Path) -> IpcClient {
        IpcClient::connect(dir.join("ipc.toml"), ClientRole::Agent)
            .expect("connect the spotify agent")
    }

    /// Records every op it gets. It refuses playback as a free account would,
    /// and answers the account and login calls with fixed values.
    #[derive(Default)]
    struct FakeSpotify {
        ops: Mutex<Vec<SpotifyOp>>,
        /// How long `Next` takes, as a slow Spotify call does.
        next_delay: Duration,
    }

    impl SpotifyHost for FakeSpotify {
        fn apply(&self, op: SpotifyOp) -> znicz_core::Result<SpotifyReply> {
            self.ops.lock().unwrap().push(op.clone());
            match op {
                SpotifyOp::Play { .. } => Err(ZniczError::Spotify {
                    kind: SpotifyErrorKind::PremiumRequired,
                    message: "needs Spotify Premium".into(),
                }),
                SpotifyOp::Search { .. } => Ok(SpotifyReply::Page {
                    page: SpotifyPage::default(),
                }),
                SpotifyOp::BeginLogin => Ok(SpotifyReply::AuthorizeUrl {
                    url: "https://accounts.spotify.com/authorize?fake".into(),
                    redirect_uri: "http://127.0.0.1:8898/callback".into(),
                    listening: true,
                }),
                SpotifyOp::Next => {
                    std::thread::sleep(self.next_delay);
                    Ok(SpotifyReply::Done)
                }
                _ => Ok(SpotifyReply::Done),
            }
        }

        fn state(&self) -> SpotifyState {
            SpotifyState::default()
        }

        fn try_state(&self) -> znicz_core::Result<SpotifyState> {
            Ok(SpotifyState {
                login: SpotifyLogin::LoggedIn,
                account: Some(SpotifyAccount {
                    user: "eve".into(),
                    premium: Some(true),
                }),
                ..SpotifyState::default()
            })
        }
    }

    /// A player that hosts `FakeSpotify`, and an MCP server with two
    /// connections to it. The fake is returned to read what it was sent.
    /// The temp dir comes first: a `let` pattern drops its bindings in reverse
    /// order, so the dir is dropped last, after the server it backs.
    fn spotify_server() -> (
        tempfile::TempDir,
        IpcServer,
        ZniczMcpServer,
        Arc<FakeSpotify>,
    ) {
        spotify_server_with(FakeSpotify::default())
    }

    fn spotify_server_with(
        fake: FakeSpotify,
    ) -> (
        tempfile::TempDir,
        IpcServer,
        ZniczMcpServer,
        Arc<FakeSpotify>,
    ) {
        let _lock = env_lock();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ipc.toml");
        let (player, thread) = spawn_player(AudioConfig::default());
        std::mem::forget(thread);
        let fake = Arc::new(fake);
        let ipc = IpcServer::start_with_session_and_library(
            player,
            &path,
            Duration::ZERO,
            dir.path().join("session.toml"),
            Duration::ZERO,
            None,
            Some(fake.clone() as Arc<dyn SpotifyHost>),
        )
        .expect("start ipc");
        let player_client = IpcClient::connect(&path, ClientRole::Agent).expect("connect");
        let spotify_client = second_client(dir.path());
        let server = ZniczMcpServer::new(player_client, spotify_client, Vec::new());
        (dir, ipc, server, fake)
    }

    fn json_of(result: &rmcp::model::CallToolResult) -> serde_json::Value {
        serde_json::from_str(&result_text(result)).expect("tool result is JSON")
    }

    /// A slow Spotify call on the Spotify connection must not hold up the
    /// status read or a player read: each has its own connection.
    #[tokio::test(flavor = "multi_thread")]
    async fn status_and_player_reads_do_not_wait_behind_a_slow_spotify_call() {
        let (_dir, _ipc, server, _fake) = spotify_server_with(FakeSpotify {
            next_delay: Duration::from_secs(2),
            ..FakeSpotify::default()
        });
        let slow = server.clone();
        let next = tokio::spawn(async move { slow.spotify_next().await });
        std::thread::sleep(Duration::from_millis(100));

        let started = std::time::Instant::now();
        let status = server.spotify_status().await.expect("status");
        let waited = started.elapsed();
        assert!(waited < Duration::from_secs(1), "status waited {waited:?}");
        assert_eq!(json_of(&status)["login"], "logged_in");

        let started = std::time::Instant::now();
        server.get_player_state().expect("player state");
        let waited = started.elapsed();
        assert!(
            waited < Duration::from_secs(1),
            "player state waited {waited:?}"
        );

        next.await.expect("join").expect("next");
    }

    #[tokio::test]
    async fn spotify_status_reads_the_snapshot_and_marks_spotify_in_use() {
        let (_dir, _ipc, server, fake) = spotify_server();
        let result = server.spotify_status().await.expect("status");
        let json = json_of(&result);
        assert_eq!(json["configured"], true);
        assert_eq!(json["login"], "logged_in");
        assert_eq!(json["user"], "eve");
        assert_eq!(json["premium"], true);
        assert!(
            fake.ops.lock().unwrap().contains(&SpotifyOp::Touch),
            "a status call marks Spotify use"
        );
    }

    #[tokio::test]
    async fn login_begin_hands_back_the_url_and_what_to_do_with_it() {
        let (_dir, _ipc, server, _fake) = spotify_server();
        let result = server.spotify_login_begin().await.expect("begin");
        let json = json_of(&result);
        assert_eq!(
            json["authorize_url"],
            "https://accounts.spotify.com/authorize?fake"
        );
        assert_eq!(json["redirect_uri"], "http://127.0.0.1:8898/callback");
        assert_eq!(json["listening"], true);
        assert!(json["next"].as_str().unwrap().contains("authorize_url"));
    }

    #[tokio::test]
    async fn login_complete_and_logout_send_their_ops_and_return_the_status() {
        let (_dir, _ipc, server, fake) = spotify_server();
        let result = server
            .spotify_login_complete(Parameters(spotify_json::SpotifyLoginCompleteParams {
                redirect_url: "http://127.0.0.1:8898/callback?code=x&state=y".into(),
            }))
            .await
            .expect("complete");
        assert_eq!(json_of(&result)["login"], "logged_in");
        server.spotify_logout().await.expect("logout");
        let ops = fake.ops.lock().unwrap();
        assert!(matches!(
            ops.iter().find(|op| matches!(op, SpotifyOp::CompleteLogin { .. })),
            Some(SpotifyOp::CompleteLogin { redirect_url })
                if redirect_url.contains("code=x")
        ));
        assert!(ops.contains(&SpotifyOp::Logout));
    }

    #[tokio::test]
    async fn search_sends_one_op_with_every_kind_and_at_most_ten_results() {
        let (_dir, _ipc, server, fake) = spotify_server();
        let result = server
            .spotify_search(Parameters(spotify_json::SpotifySearchParams {
                query: "blue".into(),
                types: None,
                limit: Some(99),
                offset: None,
            }))
            .await
            .expect("search");
        assert_eq!(json_of(&result)["reply"], "page");
        let ops = fake.ops.lock().unwrap();
        assert!(
            matches!(ops.last(), Some(SpotifyOp::Search { limit: 10, .. })),
            "{ops:?}"
        );
    }

    #[tokio::test]
    async fn tracks_pass_the_uri_and_clamp_the_page() {
        let (_dir, _ipc, server, fake) = spotify_server();
        server
            .spotify_tracks(Parameters(spotify_json::SpotifyUriPagedParams {
                uri: "spotify:album:1".into(),
                offset: None,
                limit: Some(500),
            }))
            .await
            .expect("tracks");
        let ops = fake.ops.lock().unwrap();
        assert!(
            matches!(
                ops.last(),
                Some(SpotifyOp::Tracks { uri, limit: 50, .. }) if uri.as_str() == "spotify:album:1"
            ),
            "{ops:?}"
        );
    }

    #[tokio::test]
    async fn a_premium_refusal_comes_back_with_its_kind() {
        let (_dir, _ipc, server, _fake) = spotify_server();
        let err = server
            .spotify_play(Parameters(spotify_json::SpotifyPlayParams {
                uri: Some("spotify:track:1".into()),
                context_uri: None,
                offset: None,
            }))
            .await
            .expect_err("a free account cannot play");
        assert_eq!(err.message.as_ref(), "needs Spotify Premium");
        assert_eq!(
            err.data,
            Some(serde_json::json!({ "kind": "premium_required" }))
        );
    }

    #[tokio::test]
    async fn volume_out_of_range_is_refused_before_it_is_sent() {
        let (_dir, _ipc, server, fake) = spotify_server();
        let err = server
            .spotify_set_volume(Parameters(spotify_json::SpotifyVolumeParams {
                volume: 1.5,
            }))
            .await
            .expect_err("out of range");
        assert_eq!(err.code, rmcp::model::ErrorCode::INVALID_PARAMS);
        assert!(fake.ops.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn transfer_without_play_only_moves_the_device() {
        let (_dir, _ipc, server, fake) = spotify_server();
        server
            .spotify_transfer(Parameters(spotify_json::SpotifyTransferParams {
                device_id: "dev1".into(),
                play: None,
            }))
            .await
            .expect("transfer");
        let ops = fake.ops.lock().unwrap();
        assert!(
            matches!(
                ops.last(),
                Some(SpotifyOp::Transfer { device_id, play: false })
                    if device_id.as_str() == "dev1"
            ),
            "{ops:?}"
        );
    }

    #[test]
    fn the_server_takes_a_spotify_connection_of_its_own() {
        let dir = tempfile::tempdir().unwrap();
        let (_host, _ipc, client) = hosted_player(dir.path());
        let spotify = second_client(dir.path());
        let server = ZniczMcpServer::new(client, spotify, Vec::new());
        assert!(server.live_state().is_ok());
    }

    /// A player that hosts a library service over a temp database, plus an MCP
    /// server reading the same database.
    /// `(dir, host, server)`: bound in this order, the dir is dropped last,
    /// after the host and the server that use it.
    fn hosted_with_library() -> (tempfile::TempDir, IpcServer, ZniczMcpServer) {
        let root = tempfile::tempdir().unwrap();
        let db = root.path().join("library.db");
        let advertise = root.path().join("ipc.toml");
        let (player, _thread) = znicz_core::spawn_player(znicz_core::AudioConfig::default());
        let service = std::sync::Arc::new(znicz_library::LibraryService::open(&db).unwrap());
        let host = IpcServer::start_with_library(player, &advertise, Duration::ZERO, service)
            .expect("start ipc");
        let client = IpcClient::connect(&advertise, ClientRole::Agent).expect("connect");
        let server = ZniczMcpServer::with_library(
            client,
            second_client(root.path()),
            Vec::new(),
            Library::open(&db).unwrap(),
        );
        (root, host, server)
    }

    #[tokio::test]
    async fn scan_library_without_waiting_reports_queued() {
        let (root, _host, server) = hosted_with_library();
        let music = root.path().join("music");
        std::fs::create_dir_all(&music).unwrap();
        let result = server
            .scan_library(Parameters(ScanParams {
                path: music.to_string_lossy().into_owned(),
                wait: Some(false),
            }))
            .await
            .expect("scan");
        let json: serde_json::Value = serde_json::from_str(&result_text(&result)).unwrap();
        assert_eq!(json["queued"], true);
    }

    #[tokio::test]
    async fn library_status_lists_remembered_folders() {
        let (root, _host, server) = hosted_with_library();
        let music = std::fs::canonicalize({
            let m = root.path().join("music");
            std::fs::create_dir_all(&m).unwrap();
            m
        })
        .unwrap();
        server
            .scan_library(Parameters(ScanParams {
                path: music.to_string_lossy().into_owned(),
                wait: None,
            }))
            .await
            .expect("scan");
        let status = server.library_status().expect("status");
        let json: serde_json::Value = serde_json::from_str(&result_text(&status)).unwrap();
        assert_eq!(json["folders"][0], music.to_string_lossy().as_ref());
    }

    #[tokio::test]
    async fn library_prune_reports_removed_entries() {
        let (root, _host, server) = hosted_with_library();
        Library::open(&root.path().join("library.db"))
            .unwrap()
            .upsert_track_for_test(
                std::path::Path::new("/nowhere/x.flac"),
                "t",
                None,
                None,
                None,
            )
            .unwrap();
        let result = server.library_prune().await.expect("prune");
        let json: serde_json::Value = serde_json::from_str(&result_text(&result)).unwrap();
        assert_eq!(json["removed"], 1);
    }

    fn server() -> Harness {
        let _lock = env_lock();
        let dir = tempfile::tempdir().unwrap();
        let (host, ipc, client) = hosted_player(dir.path());
        let spotify = second_client(dir.path());
        Harness {
            server: ZniczMcpServer::new(client, spotify, Vec::new()),
            host,
            _ipc: ipc,
            _dir: dir,
        }
    }

    fn server_with_library() -> Harness {
        let library = Library::open_in_memory().expect("in-memory library");
        let _lock = env_lock();
        let dir = tempfile::tempdir().unwrap();
        let (host, ipc, client) = hosted_player(dir.path());
        let spotify = second_client(dir.path());
        Harness {
            server: ZniczMcpServer::with_library(client, spotify, Vec::new(), library),
            host,
            _ipc: ipc,
            _dir: dir,
        }
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
    fn set_volume_changes_the_host_engine() {
        let server = server();
        server
            .set_volume(Parameters(VolumeParams { volume: 0.3 }))
            .expect("set_volume");
        assert!(
            (server.host.state().volume - 0.3).abs() < 1e-6,
            "host volume {}",
            server.host.state().volume
        );
    }

    #[test]
    fn get_player_state_reads_the_host() {
        let server = server();
        server
            .host
            .send_blocking(Command::SetVolume(0.4))
            .expect("host volume");
        let result = server.get_player_state().expect("state");
        let state: serde_json::Value =
            serde_json::from_str(&result_text(&result)).expect("state json");
        let volume = state["volume"].as_f64().expect("volume field");
        assert!(
            (volume - 0.4).abs() < 1e-6,
            "MCP reported volume {volume}, expected the host 0.4"
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

    #[test]
    fn queue_add_treats_http_urls_as_stream_rows() {
        let server = server();

        let result = server
            .queue_add(Parameters(QueueAddParams {
                paths: vec![
                    "/music/a.flac".into(),
                    "HTTPS://example.com/live".into(),
                    "ftp://example.com/x".into(),
                ],
            }))
            .expect("queue_add");

        let state: serde_json::Value =
            serde_json::from_str(&result_text(&result)).expect("state json");
        let queue = state["queue"].as_array().expect("queue field");
        assert_eq!(queue.len(), 3, "tool reported queue {queue:?}");
        assert_eq!(queue[0]["kind"], "file");
        assert_eq!(queue[1]["kind"], "stream");
        assert_eq!(queue[1]["url"], "HTTPS://example.com/live");
        assert_eq!(queue[1]["name"], "HTTPS://example.com/live");
        assert_eq!(
            queue[2]["kind"], "file",
            "non-http(s) URLs stay file rows; got {queue:?}"
        );
    }

    #[test]
    fn queue_remove_drops_one_row_and_returns_the_new_queue() {
        let server = server();
        server
            .queue_add(Parameters(QueueAddParams {
                paths: vec!["/music/a.flac".into(), "/music/b.flac".into()],
            }))
            .expect("queue_add");

        let result = server
            .queue_remove(Parameters(QueueRemoveParams { index: 0 }))
            .expect("queue_remove");

        let state: serde_json::Value =
            serde_json::from_str(&result_text(&result)).expect("state json");
        let queue = state["queue"].as_array().expect("queue field");
        assert_eq!(queue.len(), 1, "tool reported queue {queue:?}");
        assert!(
            queue[0]["path"].as_str().unwrap().ends_with("b.flac"),
            "row 0 should slide up; got {queue:?}"
        );
    }

    #[test]
    fn queue_move_reorders_and_returns_the_new_queue() {
        let server = server();
        server
            .queue_add(Parameters(QueueAddParams {
                paths: vec![
                    "/music/a.flac".into(),
                    "/music/b.flac".into(),
                    "/music/c.flac".into(),
                ],
            }))
            .expect("queue_add");
        let result = server
            .queue_move(Parameters(QueueMoveParams { from: 2, to: 0 }))
            .expect("queue_move");
        let state: serde_json::Value = serde_json::from_str(&result_text(&result)).unwrap();
        let queue = state["queue"].as_array().expect("queue");
        assert!(
            queue[0]["path"].as_str().unwrap().ends_with("c.flac"),
            "{queue:?}"
        );
        assert!(
            queue[2]["path"].as_str().unwrap().ends_with("b.flac"),
            "{queue:?}"
        );
    }

    #[test]
    fn queue_move_out_of_range_is_an_error() {
        let server = server();
        server
            .queue_add(Parameters(QueueAddParams {
                paths: vec!["/music/a.flac".into()],
            }))
            .expect("queue_add");
        assert!(
            server
                .queue_move(Parameters(QueueMoveParams { from: 0, to: 3 }))
                .is_err()
        );
    }

    /// A failing command must surface as an error, not a silent stale snapshot.
    #[test]
    fn play_reports_a_missing_file() {
        let server = server();
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("znicz-mcp-missing.flac");

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
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("znicz-mcp-get-track.flac");

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
    }

    #[test]
    fn get_track_rejects_a_path_that_does_not_exist() {
        let server = server_with_library();

        let result = server.get_track(Parameters(TrackParams {
            path: "/definitely/not/here.flac".into(),
        }));

        assert!(result.is_err(), "expected an error, got {result:?}");
    }

    #[tokio::test]
    async fn scan_indexes_and_then_search_finds_the_track() {
        let (root, _host, server) = hosted_with_library();

        // Build a tiny folder with one real FLAC, if ffmpeg is around.
        let dir = root.path().join("music");
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
            return;
        }

        let scan = server
            .scan_library(Parameters(ScanParams {
                path: dir.to_string_lossy().into_owned(),
                wait: None,
            }))
            .await
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
    }

    fn playlist_server() -> (Harness, tempfile::TempDir) {
        let dir = tempfile::tempdir().expect("playlists dir");
        let mut server = server();
        server.playlists_dir = dir.path().to_path_buf();
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
        let (server, _dir) = playlist_server();
        let result = server.save_playlist(Parameters(SavePlaylistParams {
            name: "evening".into(),
        }));
        assert!(result.is_err(), "expected an error, got {result:?}");
    }

    #[test]
    fn list_and_play_playlist_append_keeps_the_existing_queue() {
        let (server, dir) = playlist_server();
        let existing = touch_track(dir.path(), "old.flac");
        let added = touch_track(dir.path(), "new.flac");
        std::fs::write(
            dir.path().join("evening.m3u"),
            format!("{}\n", added.display()),
        )
        .unwrap();

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
    }

    #[test]
    fn play_playlist_starts_playback_when_ffmpeg_can_make_a_file() {
        let (server, dir) = playlist_server();
        let Some(track) = ffmpeg_flac(dir.path(), "real.flac") else {
            eprintln!("ffmpeg not available, skipping playback assert");
            return;
        };
        std::fs::write(
            dir.path().join("evening.m3u"),
            format!("{}\n", track.display()),
        )
        .unwrap();

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
    }

    #[test]
    fn import_playlist_errors_when_nothing_is_playable() {
        let (server, dir) = playlist_server();
        let path = dir.path().join("empty.m3u");
        std::fs::write(&path, "# only a comment\nftp://x\n").unwrap();

        let result = server.import_playlist(Parameters(ImportPlaylistParams {
            path: path.to_string_lossy().into_owned(),
            append: false,
        }));
        assert!(result.is_err(), "expected an error, got {result:?}");
        assert!(server.player.state().expect("state").queue.is_empty());
    }

    #[test]
    fn import_playlist_appends_an_http_line() {
        let (server, dir) = playlist_server();
        let path = dir.path().join("radio.m3u");
        std::fs::write(&path, "#EXTINF:-1,Live\nhttp://127.0.0.1:1/s\n").unwrap();

        let result = server
            .import_playlist(Parameters(ImportPlaylistParams {
                path: path.to_string_lossy().into_owned(),
                append: true,
            }))
            .expect("import stream line");
        let payload: serde_json::Value =
            serde_json::from_str(&result_text(&result)).expect("import json");
        assert_eq!(payload["loaded"], 1);
        assert_eq!(payload["skipped"], 0);
        let queue = server.player.state().expect("state").queue;
        assert_eq!(queue.len(), 1);
        assert_eq!(
            queue[0],
            znicz_core::QueueItem::stream("Live", "http://127.0.0.1:1/s")
        );
        assert_eq!(
            server.player.state().expect("state").status,
            PlaybackStatus::Stopped
        );
    }

    #[test]
    fn save_playlist_writes_the_queue() {
        let (server, dir) = playlist_server();
        let track = touch_track(dir.path(), "keep.flac");
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
        assert!(dir.path().join("evening.m3u").is_file());
    }

    #[test]
    fn rename_playlist_moves_the_file() {
        let (server, dir) = playlist_server();
        std::fs::write(dir.path().join("evening.m3u"), "x\n").unwrap();
        let result = server
            .rename_playlist(Parameters(RenamePlaylistParams {
                name: "evening".into(),
                new_name: "night".into(),
            }))
            .expect("rename");
        let payload: serde_json::Value =
            serde_json::from_str(&result_text(&result)).expect("rename json");
        assert_eq!(payload["renamed"], "night");
        assert!(dir.path().join("night.m3u").is_file());
        assert!(!dir.path().join("evening.m3u").is_file());
    }

    #[test]
    fn copy_playlist_duplicates_the_file() {
        let (server, dir) = playlist_server();
        std::fs::write(dir.path().join("evening.m3u"), "x\n").unwrap();
        let result = server
            .copy_playlist(Parameters(RenamePlaylistParams {
                name: "evening".into(),
                new_name: "night".into(),
            }))
            .expect("copy");
        let payload: serde_json::Value =
            serde_json::from_str(&result_text(&result)).expect("copy json");
        assert_eq!(payload["copied"], "night");
        assert!(dir.path().join("evening.m3u").is_file());
        assert!(dir.path().join("night.m3u").is_file());
    }

    #[test]
    fn remove_playlist_deletes_the_file() {
        let (server, dir) = playlist_server();
        std::fs::write(dir.path().join("evening.m3u"), "x\n").unwrap();
        let result = server
            .remove_playlist(Parameters(PlaylistNameParams {
                name: "evening".into(),
            }))
            .expect("remove");
        let payload: serde_json::Value =
            serde_json::from_str(&result_text(&result)).expect("remove json");
        assert_eq!(payload["removed"], "evening");
        assert!(!dir.path().join("evening.m3u").is_file());
    }

    /// Path is captured at `ZniczMcpServer::new`; set `ZNICZ_STATIONS_PATH` first.
    fn station_server() -> (Harness, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("stations.toml");
        let _lock = env_lock();
        // SAFETY: `STATIONS_ENV` serialises env mutation against other server constructors.
        unsafe {
            std::env::set_var("ZNICZ_STATIONS_PATH", &path);
        }
        let (host, ipc, client) = hosted_player(dir.path());
        let server = ZniczMcpServer::new(client, second_client(dir.path()), Vec::new());
        unsafe {
            std::env::remove_var("ZNICZ_STATIONS_PATH");
        }
        (
            Harness {
                server,
                host,
                _ipc: ipc,
                _dir: dir,
            },
            path,
        )
    }

    #[test]
    fn add_and_list_stations_round_trip() {
        let (server, _path) = station_server();
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
    }

    #[test]
    fn set_station_art_round_trip_and_clear() {
        let (server, path) = station_server();
        let img = path.parent().unwrap().join("cover.png");
        std::fs::write(&img, b"png").unwrap();
        server
            .add_radio_station(Parameters(StationAddParams {
                name: "Example".into(),
                url: "https://example.com/stream".into(),
            }))
            .unwrap();
        server
            .set_station_art(Parameters(StationArtParams {
                name: "Example".into(),
                path: Some(img.to_string_lossy().into_owned()),
            }))
            .unwrap();
        let listed = result_text(&server.list_stations().unwrap());
        assert!(listed.contains("cover.png"), "{listed}");
        server
            .set_station_art(Parameters(StationArtParams {
                name: "Example".into(),
                path: None,
            }))
            .unwrap();
        let listed = result_text(&server.list_stations().unwrap());
        assert!(!listed.contains("cover.png"), "{listed}");
    }

    #[test]
    fn play_station_clears_the_queue_then_errors_on_a_dead_url() {
        let (server, _path) = station_server();
        server
            .add_radio_station(Parameters(StationAddParams {
                name: "Dead".into(),
                url: "http://127.0.0.1:1/nope".into(),
            }))
            .unwrap();
        let err = server
            .play_station(Parameters(StationPlayParams {
                name: "Dead".into(),
                append: false,
            }))
            .unwrap_err();
        assert!(!err.to_string().contains("not implemented"));
        let queue = server.player.state().expect("state").queue;
        assert_eq!(queue.len(), 1);
        assert!(queue[0].is_stream());
    }

    #[test]
    fn play_station_append_keeps_the_existing_queue() {
        let (server, _path) = station_server();
        server
            .queue_add(Parameters(QueueAddParams {
                paths: vec!["/music/a.flac".into()],
            }))
            .unwrap();
        server
            .add_radio_station(Parameters(StationAddParams {
                name: "Example".into(),
                url: "http://127.0.0.1:1/stream".into(),
            }))
            .unwrap();
        server
            .play_station(Parameters(StationPlayParams {
                name: "Example".into(),
                append: true,
            }))
            .expect("append");
        let queue = server.player.state().expect("state").queue;
        assert_eq!(queue.len(), 2);
        assert_eq!(queue[0], znicz_core::QueueItem::file("/music/a.flac"));
        assert!(queue[1].is_stream());
        assert_eq!(
            server.player.state().expect("state").status,
            PlaybackStatus::Stopped
        );
    }

    #[test]
    fn save_playlist_writes_a_station_row() {
        let (server, dir) = playlist_server();
        server
            .queue_add(Parameters(QueueAddParams {
                paths: vec!["/music/a.flac".into()],
            }))
            .unwrap();
        server
            .player
            .send_blocking(Command::QueueAdd(vec![znicz_core::QueueItem::stream(
                "Live",
                "http://127.0.0.1:1/s",
            )]))
            .unwrap();
        server
            .save_playlist(Parameters(SavePlaylistParams {
                name: "evening".into(),
            }))
            .expect("save mixed queue");
        let body = std::fs::read_to_string(dir.path().join("evening.m3u")).unwrap();
        assert!(body.contains("#EXTINF:-1,Live"), "{body}");
        assert!(body.contains("http://127.0.0.1:1/s"), "{body}");
        assert!(
            body.contains("/music/a.flac") || body.contains("a.flac"),
            "{body}"
        );
    }

    #[test]
    fn duplicate_station_name_is_an_error() {
        let (server, _path) = station_server();
        let params = StationAddParams {
            name: "Example".into(),
            url: "https://example.com/a".into(),
        };
        server
            .add_radio_station(Parameters(params.clone()))
            .unwrap();
        assert!(
            server
                .add_radio_station(Parameters(StationAddParams {
                    name: "Example".into(),
                    url: "https://example.com/b".into(),
                }))
                .is_err()
        );
    }

    #[test]
    fn remove_rename_and_set_url_persist() {
        let (server, _path) = station_server();
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
    }

    #[test]
    fn copy_radio_station_clones_the_url() {
        let (server, _path) = station_server();
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
    }
}
