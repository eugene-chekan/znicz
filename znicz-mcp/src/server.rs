use std::path::PathBuf;

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

use znicz_core::{AudioOutput, Command, PlaybackStatus, PlayerHandle, PlayerState};

use crate::skills::SkillRegistry;

#[derive(Clone)]
pub struct ZniczMcpServer {
    player: PlayerHandle,
    skills: SkillRegistry,
    tool_router: ToolRouter<Self>,
}

impl ZniczMcpServer {
    pub fn new(player: PlayerHandle, skills_dirs: Vec<PathBuf>) -> Self {
        let bundled = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("skills");
        let mut dirs = vec![bundled];
        dirs.extend(skills_dirs);
        let skills = SkillRegistry::load(&dirs);

        Self {
            player,
            skills,
            tool_router: Self::tool_router(),
        }
    }

    fn state_json(&self) -> Result<String, McpError> {
        serde_json::to_string_pretty(&self.player.state())
            .map_err(|e| McpError::internal_error(e.to_string(), None))
    }

    fn ok_state(&self) -> Result<rmcp::model::CallToolResult, McpError> {
        Ok(rmcp::model::CallToolResult::success(vec![
            Content::text(self.state_json()?),
        ]))
    }

    fn not_implemented(feature: &str) -> McpError {
        McpError::internal_error(format!("{feature} is not implemented yet"), None)
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

#[tool_router]
impl ZniczMcpServer {
    #[tool(description = "Play a local audio file")]
    fn play(&self, Parameters(params): Parameters<PlayParams>) -> Result<rmcp::model::CallToolResult, McpError> {
        map_player_err(self.player.send(Command::Play(params.path.into())))?;
        self.ok_state()
    }

    #[tool(description = "Pause playback")]
    fn pause(&self) -> Result<rmcp::model::CallToolResult, McpError> {
        map_player_err(self.player.send(Command::Pause))?;
        self.ok_state()
    }

    #[tool(description = "Resume playback")]
    fn resume(&self) -> Result<rmcp::model::CallToolResult, McpError> {
        map_player_err(self.player.send(Command::Resume))?;
        self.ok_state()
    }

    #[tool(description = "Stop playback")]
    fn stop(&self) -> Result<rmcp::model::CallToolResult, McpError> {
        map_player_err(self.player.send(Command::Stop))?;
        self.ok_state()
    }

    #[tool(description = "Seek to position in seconds")]
    fn seek(&self, Parameters(params): Parameters<SeekParams>) -> Result<rmcp::model::CallToolResult, McpError> {
        let pos = std::time::Duration::from_secs_f64(params.seconds.max(0.0));
        map_player_err(self.player.send(Command::Seek(pos)))?;
        self.ok_state()
    }

    #[tool(description = "Set volume between 0.0 and 1.0")]
    fn set_volume(
        &self,
        Parameters(params): Parameters<VolumeParams>,
    ) -> Result<rmcp::model::CallToolResult, McpError> {
        map_player_err(self.player.send(Command::SetVolume(params.volume)))?;
        self.ok_state()
    }

    #[tool(description = "Play next track in queue")]
    fn next_track(&self) -> Result<rmcp::model::CallToolResult, McpError> {
        map_player_err(self.player.send(Command::NextTrack))?;
        self.ok_state()
    }

    #[tool(description = "Play previous track in queue")]
    fn previous_track(&self) -> Result<rmcp::model::CallToolResult, McpError> {
        map_player_err(self.player.send(Command::PreviousTrack))?;
        self.ok_state()
    }

    #[tool(description = "Add paths to the playback queue")]
    fn queue_add(
        &self,
        Parameters(params): Parameters<QueueAddParams>,
    ) -> Result<rmcp::model::CallToolResult, McpError> {
        let paths = params.paths.into_iter().map(PathBuf::from).collect();
        map_player_err(self.player.send(Command::QueueAdd(paths)))?;
        self.ok_state()
    }

    #[tool(description = "Clear the playback queue")]
    fn queue_clear(&self) -> Result<rmcp::model::CallToolResult, McpError> {
        map_player_err(self.player.send(Command::QueueClear))?;
        self.ok_state()
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
        let devices = AudioOutput::list_devices().map_err(|e| McpError::internal_error(e.to_string(), None))?;
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
        Ok(rmcp::model::CallToolResult::success(vec![Content::text(json)]))
    }

    #[tool(description = "Select audio output device by id")]
    fn set_device(
        &self,
        Parameters(params): Parameters<DeviceParams>,
    ) -> Result<rmcp::model::CallToolResult, McpError> {
        map_player_err(self.player.send(Command::SetDevice(params.device_id)))?;
        self.ok_state()
    }

    #[tool(description = "List bundled Agent Skills (SEP-2640 index)")]
    fn skills_list(&self) -> Result<rmcp::model::CallToolResult, McpError> {
        let json = self.skills.index_json();
        Ok(rmcp::model::CallToolResult::success(vec![Content::text(json)]))
    }

    #[tool(description = "Search music library (Phase 2)")]
    fn search_library(&self) -> Result<rmcp::model::CallToolResult, McpError> {
        Err(Self::not_implemented("search_library"))
    }

    #[tool(description = "Get track metadata from library (Phase 2)")]
    fn get_track(&self) -> Result<rmcp::model::CallToolResult, McpError> {
        Err(Self::not_implemented("get_track"))
    }

    #[tool(description = "Browse album in library (Phase 2)")]
    fn browse_album(&self) -> Result<rmcp::model::CallToolResult, McpError> {
        Err(Self::not_implemented("browse_album"))
    }

    #[tool(description = "Import playlist file (Phase 3)")]
    fn import_playlist(&self) -> Result<rmcp::model::CallToolResult, McpError> {
        Err(Self::not_implemented("import_playlist"))
    }

    #[tool(description = "Save current queue as playlist (Phase 3)")]
    fn save_playlist(&self) -> Result<rmcp::model::CallToolResult, McpError> {
        Err(Self::not_implemented("save_playlist"))
    }

    #[tool(description = "Play a saved playlist (Phase 3)")]
    fn play_playlist(&self) -> Result<rmcp::model::CallToolResult, McpError> {
        Err(Self::not_implemented("play_playlist"))
    }

    #[tool(description = "Add radio station (Phase 4)")]
    fn add_radio_station(&self) -> Result<rmcp::model::CallToolResult, McpError> {
        Err(Self::not_implemented("add_radio_station"))
    }

    #[tool(description = "List radio stations (Phase 4)")]
    fn list_stations(&self) -> Result<rmcp::model::CallToolResult, McpError> {
        Err(Self::not_implemented("list_stations"))
    }

    #[tool(description = "Play radio station (Phase 4)")]
    fn play_station(&self) -> Result<rmcp::model::CallToolResult, McpError> {
        Err(Self::not_implemented("play_station"))
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
            make_resource(
                "skill://index.json",
                "skills-index",
                "Agent Skills discovery index",
            ),
        ];

        for file in self.skills.all_resources() {
            resources.push(make_resource(
                &file.uri,
                &file.uri,
                "Skill resource file",
            ));
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

    fn list_prompts(
        &self,
        _request: Option<rmcp::model::PaginatedRequestParam>,
        _context: rmcp::service::RequestContext<rmcp::RoleServer>,
    ) -> impl std::future::Future<Output = Result<ListPromptsResult, McpError>> + Send + '_ {
        async move {
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
