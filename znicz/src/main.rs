use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use clap::{Parser, Subcommand};
use serde::Deserialize;
use znicz_core::wait_for_library;
use znicz_core::{
    AudioConfig, AudioOutput, ClientRole, Command, IpcClient, IpcServer, LibraryHost, LibraryOp,
    SESSION_SAVE_DEBOUNCE, ScanSummary, apply_to_player, copy_saved, list_saved, load_path,
    remove_saved, rename_saved, restore_session, save_session_from_player, saved_path,
    skipped_notice, spawn_player,
};
use znicz_library::Library;
use znicz_mcp::run_stdio;
use znicz_tui::app::Engine;
use znicz_tui::{App, ConfigFile, Resume, RunOutcome, TuiConfig, Updates};

mod daemon;
mod update;

/// The file written on first start and by `znicz config init`.
const DEFAULT_CONFIG: &str = include_str!("default-config.toml");

use daemon::{
    PLAYER_EXIT_TIMEOUT, acquire_player_lock, clear_stale_player_files, ensure_player, player_is_up,
};

#[derive(Debug, Parser)]
#[command(name = "znicz", version, about = "Audiophile TUI music player")]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,

    /// Audio files to play
    #[arg(value_name = "FILE")]
    files: Vec<PathBuf>,

    #[arg(long, help = "Select output device id")]
    device: Option<String>,

    #[arg(long, help = "List audio devices and exit")]
    list_devices: bool,

    #[arg(long, help = "Path to config file")]
    config: Option<PathBuf>,

    /// Set by a restart after an update: play from INDEX:SECONDS in the queue.
    #[arg(long, hide = true, value_name = "INDEX:SECONDS")]
    resume: Option<String>,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Run MCP server on stdio (attaches to `znicz player`)
    Mcp {
        #[arg(long, help = "Additional skills directory")]
        skills_dir: Vec<PathBuf>,
    },

    /// Shared playback engine (autostarted by TUI and MCP)
    Player {
        #[command(subcommand)]
        command: Option<PlayerCmd>,
    },

    /// Scan folders into the music library
    Scan {
        #[arg(value_name = "DIR", help = "Folders to scan")]
        dirs: Vec<PathBuf>,

        #[arg(long, help = "Drop entries whose files are gone")]
        prune: bool,
    },

    /// Search the music library
    Search {
        #[arg(value_name = "QUERY")]
        query: String,

        #[arg(long, default_value_t = 25, help = "Maximum results")]
        limit: usize,
    },

    /// List albums in the music library
    Albums,

    /// Load, list, or play M3U playlists
    Playlist {
        #[command(subcommand)]
        command: PlaylistCmd,
    },

    /// Saved Icecast stations
    Station {
        #[command(subcommand)]
        command: StationCmd,
    },

    /// Check for a newer release, or install it
    Update {
        #[arg(long, help = "Only check and print the release notes")]
        check: bool,
    },

    /// Write or locate config.toml
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },
}

#[derive(Debug, Subcommand)]
enum ConfigAction {
    /// Write the default config.toml (refuses to overwrite unless --force)
    Init {
        /// Overwrite the file; the old one is kept as config.toml.bak
        #[arg(long)]
        force: bool,
    },
    /// Print the config.toml path in use
    Path,
}

#[derive(Debug, Subcommand)]
enum PlayerCmd {
    /// Stop the player process
    Stop,
}

#[derive(Debug, Subcommand)]
enum PlaylistCmd {
    /// Print saved playlist names
    List,
    /// Load a playlist file and open the player
    Import {
        #[arg(value_name = "FILE")]
        file: PathBuf,
        /// Add to the queue instead of replacing it
        #[arg(long)]
        append: bool,
    },
    /// Save the queue (use the player: P then n, or MCP save_playlist)
    Save { name: String },
    /// Rename a saved playlist
    Rename { name: String, new_name: String },
    /// Copy a saved playlist to a new name
    Copy { name: String, new_name: String },
    /// Delete a saved playlist
    Remove { name: String },
    /// Load a saved playlist and open the player
    Play {
        name: String,
        /// Add to the queue instead of replacing it
        #[arg(long)]
        append: bool,
    },
}

#[derive(Debug, Subcommand)]
enum StationCmd {
    /// Print saved station names and URLs
    List,
    /// Add a station
    Add { name: String, url: String },
    /// Play a saved station and open the player
    Play {
        name: String,
        /// Add to the queue instead of replacing it and starting playback
        #[arg(long)]
        append: bool,
    },
    /// Remove a station
    Remove { name: String },
    /// Rename a station
    Rename { name: String, new_name: String },
    /// Change a station's stream URL
    Url { name: String, url: String },
    /// Copy a station to a new name (same URL)
    Copy { name: String, new_name: String },
    /// Set or clear a station's local cover file
    Art {
        name: String,
        path: Option<String>,
        #[arg(long)]
        clear: bool,
    },
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct Config {
    audio: AudioSection,
    mcp: McpSection,
    library: LibrarySection,
    player: PlayerSection,
    tui: TuiSection,
    covers: CoversSection,
    updates: UpdatesSection,
}

#[derive(Debug, Deserialize, Default)]
struct UpdatesSection {
    /// Check GitHub for a newer release once a day. Default on.
    check: Option<bool>,
}

#[derive(Debug, Deserialize, Default)]
struct CoversSection {
    /// Artist / title cover search for streams. Default on.
    search: Option<bool>,
}

#[derive(Debug, Deserialize, Default)]
struct LibrarySection {
    /// Where the database file lives. Defaults to the user data directory.
    path: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
struct TuiSection {
    show_cover: Option<bool>,
    cover_protocol: Option<String>,
    library_layout: Option<String>,
    marquee: Option<bool>,
    icons: Option<String>,
}

fn tui_config(config: &Config) -> TuiConfig {
    TuiConfig {
        show_cover: config.tui.show_cover.unwrap_or(true),
        cover_protocol: znicz_tui::CoverProtocol::parse(
            config.tui.cover_protocol.as_deref().unwrap_or("auto"),
        ),
        library_layout: znicz_tui::LibraryLayout::parse(
            config.tui.library_layout.as_deref().unwrap_or("columns"),
        ),
        marquee: config.tui.marquee.unwrap_or(true),
        cover_search: config.covers.search.unwrap_or(true),
        icons: znicz_tui::IconSet::parse(config.tui.icons.as_deref().unwrap_or("unicode")),
    }
}

#[derive(Debug, Deserialize)]
struct AudioSection {
    device: Option<String>,
    volume: Option<f32>,
    bit_perfect: Option<bool>,
}

impl Default for AudioSection {
    fn default() -> Self {
        Self {
            device: None,
            volume: Some(1.0),
            bit_perfect: Some(true),
        }
    }
}

#[derive(Debug, Deserialize, Default)]
struct PlayerSection {
    /// Seconds to stay up while Stopped with no UI. `0` never exits on idle.
    idle_secs: Option<u64>,
}

#[derive(Debug, Deserialize, Default)]
struct McpSection {
    skills_dirs: Vec<String>,
}

fn main() -> color_eyre::Result<()> {
    color_eyre::install()?;
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();
    update::remove_old_exe();

    let cli = Cli::parse();
    ensure_default_config(cli.config.as_deref());
    let (config, config_problem) = load_config(cli.config.as_deref());

    if cli.list_devices {
        list_devices()?;
        return Ok(());
    }

    let library_path = library_path(&config);
    let tui = tui_config(&config);
    let extras = TuiExtras {
        updates: update::state_path().map(|path| {
            Arc::new(update::GithubUpdates::new(
                path,
                config.updates.check.unwrap_or(true),
                update::API_URL.to_string(),
            )) as Arc<dyn Updates>
        }),
        resume: cli.resume.as_deref().and_then(parse_resume),
        config_file: cli.config.clone(),
        cli_device: cli.device.clone(),
        effective_config: config_path(cli.config.as_deref()).map(ConfigFile::new),
        config_problem,
    };
    let config_file = cli.config.clone();

    let audio_config = AudioConfig {
        device_id: cli.device.or(config.audio.device),
        volume: config.audio.volume.unwrap_or(1.0),
        bit_perfect: config.audio.bit_perfect.unwrap_or(true),
    };

    match cli.command {
        Some(Commands::Mcp { skills_dir }) => {
            let mut dirs: Vec<PathBuf> = config
                .mcp
                .skills_dirs
                .into_iter()
                .map(expand_path)
                .collect();
            dirs.extend(skills_dir);
            run_mcp(
                dirs,
                library_path,
                audio_config.device_id.as_deref(),
                config_file.as_deref(),
            )?;
        }
        Some(Commands::Player { command }) => match command {
            Some(PlayerCmd::Stop) => stop_player()?,
            None => run_player_daemon(
                audio_config,
                config.player.idle_secs.unwrap_or(900),
                library_path.clone(),
            )?,
        },
        Some(Commands::Scan { dirs, prune }) => {
            scan_library(
                library_path,
                &dirs,
                prune,
                audio_config.device_id.as_deref(),
                config_file.as_deref(),
            )?;
        }
        Some(Commands::Search { query, limit }) => {
            search_library(library_path, &query, limit)?;
        }
        Some(Commands::Albums) => {
            list_albums(library_path)?;
        }
        Some(Commands::Update { check }) => update::run_cli(check)?,
        Some(Commands::Config { action }) => {
            let path = config_path(cli.config.as_deref()).ok_or_else(|| {
                color_eyre::eyre::eyre!("cannot work out where to keep config.toml; pass --config")
            })?;
            match action {
                ConfigAction::Init { force } => {
                    let outcome =
                        init_config(&path, force).map_err(|e| color_eyre::eyre::eyre!("{e}"))?;
                    if let Some(backup) = outcome.backup {
                        println!("Backed up the old file to {}", backup.display());
                    }
                    println!("Wrote {}", path.display());
                }
                ConfigAction::Path => println!("{}", path.display()),
            }
        }
        Some(Commands::Playlist { command }) => match command {
            PlaylistCmd::List => list_playlists()?,
            PlaylistCmd::Save { name: _ } => {
                color_eyre::eyre::bail!(
                    "save the queue from the player (P then n, or MCP save_playlist)"
                );
            }
            PlaylistCmd::Import { file, append } => {
                load_playlist_and_run(
                    file,
                    append,
                    library_path,
                    audio_config.device_id.as_deref(),
                    config_file.as_deref(),
                    tui,
                    extras,
                )?;
            }
            PlaylistCmd::Play { name, append } => {
                let dir = playlists_dir()?;
                let path = saved_path(&dir, &name)
                    .ok_or_else(|| color_eyre::eyre::eyre!("no playlist named {name}"))?;
                load_playlist_and_run(
                    path,
                    append,
                    library_path,
                    audio_config.device_id.as_deref(),
                    config_file.as_deref(),
                    tui,
                    extras,
                )?;
            }
            PlaylistCmd::Rename { name, new_name } => {
                let dir = playlists_dir()?;
                let stem = rename_saved(&dir, &name, &new_name)?;
                println!("{stem}");
            }
            PlaylistCmd::Copy { name, new_name } => {
                let dir = playlists_dir()?;
                let stem = copy_saved(&dir, &name, &new_name)?;
                println!("{stem}");
            }
            PlaylistCmd::Remove { name } => {
                let dir = playlists_dir()?;
                remove_saved(&dir, &name)?;
            }
        },
        Some(Commands::Station { command }) => match command {
            StationCmd::List => list_stations()?,
            StationCmd::Add { name, url } => {
                mutate_stations(|s| znicz_core::add_station(s, &name, &url))?
            }
            StationCmd::Play { name, append } => {
                play_station_and_run(
                    &name,
                    append,
                    library_path,
                    audio_config.device_id.as_deref(),
                    config_file.as_deref(),
                    tui,
                    extras,
                )?;
            }
            StationCmd::Remove { name } => {
                mutate_stations(|s| znicz_core::remove_station(s, &name))?
            }
            StationCmd::Rename { name, new_name } => {
                mutate_stations(|s| znicz_core::rename_station(s, &name, &new_name))?
            }
            StationCmd::Url { name, url } => {
                mutate_stations(|s| znicz_core::set_station_url(s, &name, &url))?
            }
            StationCmd::Copy { name, new_name } => {
                mutate_stations(|s| znicz_core::copy_station(s, &name, &new_name))?
            }
            StationCmd::Art { name, path, clear } => {
                if clear {
                    mutate_stations(|s| znicz_core::set_station_art(s, &name, None))?
                } else {
                    let Some(path) = path else {
                        return Err(color_eyre::eyre::eyre!("pass a path or --clear"));
                    };
                    mutate_stations(|s| znicz_core::set_station_art(s, &name, Some(&path)))?
                }
            }
        },
        None => {
            run_tui(
                &cli.files,
                library_path,
                audio_config.device_id.as_deref(),
                config_file.as_deref(),
                tui,
                extras,
            )?;
        }
    }

    Ok(())
}

/// Where the library database lives: config first, then the default location.
fn library_path(config: &Config) -> Option<PathBuf> {
    config
        .library
        .path
        .clone()
        .map(expand_path)
        .or_else(znicz_library::default_database_path)
}

fn open_library(path: Option<PathBuf>) -> color_eyre::Result<Library> {
    let path = path.ok_or_else(|| {
        color_eyre::eyre::eyre!("cannot work out where to keep the library; set [library].path")
    })?;
    Ok(Library::open(&path)?)
}

/// Read the config file. A missing file is not a problem. A broken or
/// unreadable one gives the defaults, a warning, and the problem: either
/// "line L, column C: <toml message>" or "cannot read: <io error>".
fn load_config(path: Option<&Path>) -> (Config, Option<String>) {
    let Some(path) = config_path(path) else {
        return (Config::default(), None);
    };
    let parsed = match std::fs::read_to_string(&path) {
        Ok(text) => parse_config_text(&text),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return (Config::default(), None);
        }
        Err(e) => Err(format!("cannot read: {e}")),
    };
    match parsed {
        Ok(config) => (config, None),
        Err(message) => {
            tracing::warn!(path = %path.display(), "config.toml not used: {message}");
            (Config::default(), Some(message))
        }
    }
}

/// Parse config text. An error says where it is: "line L, column C: message".
/// The message is one line, since it is shown in a toast.
fn parse_config_text(text: &str) -> Result<Config, String> {
    toml::from_str::<Config>(text).map_err(|e| {
        let message = e.message().lines().collect::<Vec<_>>().join(", ");
        match e.span() {
            Some(span) => {
                let (line, column) = line_and_column(text, span.start);
                format!("line {line}, column {column}: {message}")
            }
            None => message,
        }
    })
}

/// The 1-based line and column (counted in characters) of byte offset `at`.
fn line_and_column(text: &str, at: usize) -> (usize, usize) {
    let before = text.get(..at.min(text.len())).unwrap_or(text);
    let line = before.matches('\n').count() + 1;
    let line_start = before.rfind('\n').map_or(0, |i| i + 1);
    let column = before[line_start..].chars().count() + 1;
    (line, column)
}

/// The config file in use: `--config`, or the default location.
fn config_path(cli_config: Option<&Path>) -> Option<PathBuf> {
    cli_config.map(PathBuf::from).or_else(default_config_path)
}

/// First start: write the default config when `--config` is not given and
/// the default file does not exist yet. Failures only log a warning.
fn ensure_default_config(cli_config: Option<&Path>) {
    if cli_config.is_some() {
        return;
    }
    let Some(path) = default_config_path() else {
        return;
    };
    match ensure_default_config_at(&path) {
        Ok(true) => tracing::info!("Created {} with the default settings", path.display()),
        Ok(false) => {}
        Err(e) => {
            tracing::warn!(path = %path.display(), "cannot write the default config: {e}");
        }
    }
}

/// Write the default config at `path` unless something already exists there
/// (a dangling symlink counts). Returns whether a file was written.
fn ensure_default_config_at(path: &Path) -> std::io::Result<bool> {
    if std::fs::symlink_metadata(path).is_ok() {
        return Ok(false);
    }
    znicz_tui::config_file::write_atomically(path, DEFAULT_CONFIG)
        .map_err(std::io::Error::other)?;
    Ok(true)
}

/// What `znicz config init` did beyond writing the file.
#[derive(Debug, PartialEq, Eq)]
struct InitOutcome {
    /// Where the previous file was copied, when `--force` replaced one.
    backup: Option<PathBuf>,
}

/// Write the default config at `path`. An existing file is refused unless
/// `force`, which first copies it to `<path>.bak`.
fn init_config(path: &Path, force: bool) -> Result<InitOutcome, String> {
    if std::fs::symlink_metadata(path).is_err() {
        znicz_tui::config_file::write_atomically(path, DEFAULT_CONFIG)?;
        return Ok(InitOutcome { backup: None });
    }
    if !force {
        return Err(format!(
            "{} already exists (use --force to overwrite)",
            path.display()
        ));
    }
    let backup = backup_path(path);
    std::fs::copy(path, &backup).map_err(|e| format!("cannot back up {}: {e}", path.display()))?;
    znicz_tui::config_file::write_atomically(path, DEFAULT_CONFIG)?;
    Ok(InitOutcome {
        backup: Some(backup),
    })
}

/// `<path>.bak`, next to the file it copies.
fn backup_path(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(".bak");
    PathBuf::from(name)
}

fn default_config_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("znicz").join("config.toml"))
}

fn expand_path(path: String) -> PathBuf {
    if path.starts_with("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(path.trim_start_matches("~/"));
        }
    }
    PathBuf::from(path)
}

fn list_devices() -> color_eyre::Result<()> {
    let devices = AudioOutput::list_devices()?;
    for device in devices {
        let mark = if device.is_default { "*" } else { " " };
        println!("{mark} {}  ({})", device.name, device.id);
    }
    Ok(())
}

fn ipc_path() -> color_eyre::Result<PathBuf> {
    znicz_library::default_ipc_path().ok_or_else(|| {
        color_eyre::eyre::eyre!("cannot work out where to keep ipc.toml; set ZNICZ_IPC_PATH")
    })
}

fn player_lock_path() -> color_eyre::Result<PathBuf> {
    znicz_library::default_player_lock_path().ok_or_else(|| {
        color_eyre::eyre::eyre!("cannot work out where to keep player.lock; set ZNICZ_IPC_PATH")
    })
}

fn connect_ui(device: Option<&str>, config: Option<&Path>) -> color_eyre::Result<IpcClient> {
    connect_client(ClientRole::Ui, device, config)
}

fn connect_agent(device: Option<&str>, config: Option<&Path>) -> color_eyre::Result<IpcClient> {
    connect_client(ClientRole::Agent, device, config)
}

fn connect_client(
    role: ClientRole,
    device: Option<&str>,
    config: Option<&Path>,
) -> color_eyre::Result<IpcClient> {
    let ipc = ipc_path()?;
    let lock = player_lock_path()?;
    ensure_player(&ipc, &lock, device, config)?;
    let device = device.map(str::to_owned);
    let config = config.map(Path::to_path_buf);
    let ipc_for_ensure = ipc.clone();
    IpcClient::connect_with_ensure(ipc, role, move || {
        ensure_player(&ipc_for_ensure, &lock, device.as_deref(), config.as_deref())
            .map_err(|e| znicz_core::ZniczError::Player(e.to_string()))
    })
    .map_err(|e| color_eyre::eyre::eyre!("{e}"))
}

fn stop_player() -> color_eyre::Result<()> {
    let ipc = ipc_path()?;
    if let Ok(client) = IpcClient::connect(&ipc, ClientRole::Agent) {
        client
            .shutdown()
            .map_err(|e| color_eyre::eyre::eyre!("{e}"))?;
    }
    Ok(())
}

fn run_player_daemon(
    audio_config: AudioConfig,
    idle_secs: u64,
    library_path: Option<PathBuf>,
) -> color_eyre::Result<()> {
    let ipc = ipc_path()?;
    let lock_path = player_lock_path()?;
    clear_stale_player_files(&lock_path, &ipc);
    let Some(_lock) = acquire_player_lock(&lock_path, &ipc)? else {
        return Ok(());
    };
    if player_is_up(&ipc) {
        return Ok(());
    }

    let (player, _thread) = spawn_player(audio_config);
    if let Err(e) = restore_session(&player, &session_path()?, true) {
        tracing::warn!("session restore: {e}");
    }
    let idle = if idle_secs == 0 {
        std::time::Duration::ZERO
    } else {
        std::time::Duration::from_secs(idle_secs)
    };
    // A broken library must not stop playback; library writes then report it.
    let library: Option<Arc<dyn LibraryHost>> =
        library_path.and_then(|path| match znicz_library::LibraryService::open(&path) {
            Ok(service) => Some(Arc::new(service) as _),
            Err(e) => {
                tracing::warn!(path = %path.display(), error = %e, "library service unavailable");
                None
            }
        });
    let mut server = IpcServer::start_with_session_and_library(
        player.clone(),
        ipc,
        idle,
        session_path()?,
        SESSION_SAVE_DEBOUNCE,
        library,
    )
    .map_err(|e| color_eyre::eyre::eyre!("{e}"))?;
    install_player_signals(server.stop_flag());
    server.wait();
    if let Err(e) = save_session_from_player(&player, &session_path()?) {
        tracing::warn!("session.toml: {e}");
    }
    Ok(())
}

#[cfg(unix)]
fn install_player_signals(stop: Arc<AtomicBool>) {
    static STOP: std::sync::OnceLock<Arc<AtomicBool>> = std::sync::OnceLock::new();
    let _ = STOP.set(stop);

    unsafe extern "C" fn on_signal(_: libc::c_int) {
        if let Some(stop) = STOP.get() {
            stop.store(true, Ordering::SeqCst);
        }
    }

    unsafe {
        libc::signal(libc::SIGTERM, on_signal as *const () as usize);
        libc::signal(libc::SIGINT, on_signal as *const () as usize);
    }
}

#[cfg(not(unix))]
fn install_player_signals(_stop: Arc<AtomicBool>) {}

fn run_tui(
    files: &[PathBuf],
    library_path: Option<PathBuf>,
    device: Option<&str>,
    config: Option<&Path>,
    tui: TuiConfig,
    extras: TuiExtras,
) -> color_eyre::Result<()> {
    let player = connect_ui(device, config)?;

    if !files.is_empty() {
        let items: Vec<znicz_core::QueueItem> = files
            .iter()
            .map(|p| znicz_core::QueueItem::file(p.clone()))
            .collect();
        player
            .send_blocking(Command::QueueClear)
            .map_err(|e| color_eyre::eyre::eyre!("{e}"))?;
        if items.len() == 1 {
            player
                .send_blocking(Command::Play(items[0].clone()))
                .map_err(|e| color_eyre::eyre::eyre!("{e}"))?;
        } else {
            player
                .send_blocking(Command::ReplaceQueue {
                    items: items.clone(),
                    position: 0,
                })
                .map_err(|e| color_eyre::eyre::eyre!("{e}"))?;
            player
                .send_blocking(Command::QueuePlayIndex(0))
                .map_err(|e| color_eyre::eyre::eyre!("{e}"))?;
        }
    }

    run_tui_with_client(player, library_path, None, tui, extras)
}

fn playlists_dir() -> color_eyre::Result<PathBuf> {
    znicz_library::default_playlists_dir().ok_or_else(|| {
        color_eyre::eyre::eyre!("cannot work out where to keep playlists; set ZNICZ_PLAYLISTS_DIR")
    })
}

fn stations_path() -> color_eyre::Result<PathBuf> {
    znicz_library::default_stations_path().ok_or_else(|| {
        color_eyre::eyre::eyre!("cannot work out where to keep stations; set ZNICZ_STATIONS_PATH")
    })
}

fn session_path() -> color_eyre::Result<PathBuf> {
    znicz_library::default_session_path().ok_or_else(|| {
        color_eyre::eyre::eyre!("cannot work out where to keep the session; set ZNICZ_SESSION_PATH")
    })
}

fn play_station_and_run(
    name: &str,
    append: bool,
    library_path: Option<PathBuf>,
    device: Option<&str>,
    config: Option<&Path>,
    tui: TuiConfig,
    extras: TuiExtras,
) -> color_eyre::Result<()> {
    let path = stations_path()?;
    let stations = znicz_core::load_stations(&path)?;
    let station = znicz_core::find_station(&stations, name)
        .ok_or_else(|| color_eyre::eyre::eyre!("no station named {name}"))?
        .clone();
    let player = connect_ui(device, config)?;
    znicz_core::play_station(&player, &station, append)
        .map_err(|e| color_eyre::eyre::eyre!("{e}"))?;
    run_tui_with_client(player, library_path, None, tui, extras)
}

fn list_stations() -> color_eyre::Result<()> {
    let path = stations_path()?;
    for station in znicz_core::load_stations(&path)? {
        println!("{} — {}", station.name, station.url);
    }
    Ok(())
}

fn mutate_stations(
    op: impl FnOnce(&mut Vec<znicz_core::Station>) -> znicz_core::Result<()>,
) -> color_eyre::Result<()> {
    let path = stations_path()?;
    let mut stations = znicz_core::load_stations(&path)?;
    op(&mut stations)?;
    znicz_core::save_stations(&path, &stations)?;
    Ok(())
}

fn list_playlists() -> color_eyre::Result<()> {
    let dir = playlists_dir()?;
    for name in list_saved(&dir) {
        println!("{name}");
    }
    Ok(())
}

fn load_playlist_and_run(
    path: PathBuf,
    append: bool,
    library_path: Option<PathBuf>,
    device: Option<&str>,
    config: Option<&Path>,
    tui: TuiConfig,
    extras: TuiExtras,
) -> color_eyre::Result<()> {
    let result = load_path(&path)?;
    let player = connect_ui(device, config)?;
    apply_to_player(&player, &result, append).map_err(|e| color_eyre::eyre::eyre!("{e}"))?;
    run_tui_with_client(player, library_path, skipped_notice(&result), tui, extras)
}

/// What a TUI run gets besides the playback target: the update checker and,
/// after a restart, where to pick playback back up.
struct TuiExtras {
    updates: Option<Arc<dyn Updates>>,
    resume: Option<Resume>,
    /// `--config` as given on this run, passed on to a restarted binary.
    config_file: Option<PathBuf>,
    /// `--device` as given on this run (not the config file's), for a restart.
    cli_device: Option<String>,
    /// The config file Settings edits: `--config`, or the default path.
    effective_config: Option<ConfigFile>,
    /// Why the config file was not used, shown as a toast when the TUI starts.
    config_problem: Option<String>,
}

fn run_tui_with_client(
    player: IpcClient,
    library_path: Option<PathBuf>,
    skip_notice: Option<String>,
    tui: TuiConfig,
    extras: TuiExtras,
) -> color_eyre::Result<()> {
    // No library is not an error: the browser pane explains how to build one.
    let library = match open_library(library_path) {
        Ok(library) => Some(library),
        Err(e) => {
            tracing::warn!("library unavailable: {e}");
            None
        }
    };

    // ALSA and other C libraries write warnings straight to stderr, which in a
    // full-screen interface means drawing over it. Send stderr to a file for as
    // long as the TUI owns the terminal; nothing is lost, it just moves.
    let log = stderr::redirect_to_log();
    let mut app = App::with_remote(player, library);
    app.tui = tui;
    app.library.set_preferred_layout(app.tui.library_layout);
    if let Some(updates) = extras.updates {
        app = app.with_updates(updates);
    }
    if let Some(file) = extras.effective_config {
        app = app.with_config_file(file);
    }
    if let Some(message) = extras.config_problem {
        app = app.with_config_problem(message);
    }
    if let Some(message) = skip_notice {
        app.toasts.warn(message);
    }
    if let Some(version) = app
        .player
        .player_version()
        .filter(|v| update::is_newer(v, env!("CARGO_PKG_VERSION")))
    {
        app.toasts.info(format!(
            "Player runs {version} · it updates when it next restarts"
        ));
    }
    if let Some(resume) = extras.resume {
        apply_resume(&app.player, resume);
    }
    let restart_config = extras.config_file;
    let restart_device = extras.cli_device;
    let outcome = app.run();
    drop(log);

    match outcome? {
        RunOutcome::Quit => Ok(()),
        RunOutcome::Restart { resume } => restart(restart_args(
            restart_config.as_deref(),
            restart_device.as_deref(),
            resume,
        )),
    }
}

/// After a restart: wait for the restored queue, then play where we were.
fn apply_resume(player: &Engine, resume: Resume) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while player.state().queue.len() <= resume.index {
        if Instant::now() >= deadline {
            tracing::warn!("resume: queue not restored in time");
            return;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let _ = player.send_blocking(Command::QueuePlayIndex(resume.index));
    if resume.seconds > 0 {
        let _ = player.send_blocking(Command::Seek(Duration::from_secs(resume.seconds)));
    }
}

/// `--resume INDEX:SECONDS`. Anything else is ignored with a warning.
pub(crate) fn parse_resume(s: &str) -> Option<Resume> {
    let parsed = s.split_once(':').and_then(|(index, seconds)| {
        Some(Resume {
            index: index.parse().ok()?,
            seconds: seconds.parse().ok()?,
        })
    });
    if parsed.is_none() {
        tracing::warn!("ignoring malformed --resume {s:?}");
    }
    parsed
}

/// The arguments for the restarted binary: the global options only, so the
/// start-up action (files, playlist, station) does not run again.
pub(crate) fn restart_args(
    config: Option<&Path>,
    device: Option<&str>,
    resume: Option<Resume>,
) -> Vec<OsString> {
    let mut out = Vec::new();
    if let Some(config) = config {
        out.push(OsString::from("--config"));
        out.push(config.as_os_str().to_os_string());
    }
    if let Some(device) = device {
        out.push(OsString::from("--device"));
        out.push(OsString::from(device));
    }
    if let Some(resume) = resume {
        out.push(OsString::from("--resume"));
        out.push(OsString::from(format!(
            "{}:{}",
            resume.index, resume.seconds
        )));
    }
    out
}

/// Start the updated binary. Unix replaces this process (returns only on
/// failure); Windows runs it as a child and exits with its status.
fn restart(args: Vec<OsString>) -> color_eyre::Result<()> {
    // The TUI may have just asked the player to stop; let it release its lock
    // so the new player can start right away. Best effort: no paths, no wait.
    if let (Ok(ipc), Ok(lock)) = (ipc_path(), player_lock_path()) {
        daemon::wait_for_player_exit(&ipc, &lock, PLAYER_EXIT_TIMEOUT);
    }
    let exe = daemon::live_exe_path(std::env::current_exe()?);
    let mut cmd = std::process::Command::new(exe);
    cmd.args(args);
    #[cfg(unix)]
    {
        // Replace this process with the new binary (no shell involved);
        // this only returns when starting it failed.
        use std::os::unix::process::CommandExt;
        Err(cmd.exec().into())
    }
    #[cfg(not(unix))]
    {
        let status = cmd.status()?;
        std::process::exit(status.code().unwrap_or(0));
    }
}

/// Keeping stderr out of the interface.
mod stderr {
    use std::path::PathBuf;

    /// Where the diverted output goes.
    pub fn log_path() -> PathBuf {
        dirs::cache_dir()
            .map(|dir| dir.join("znicz"))
            .unwrap_or_else(std::env::temp_dir)
            .join("znicz-session.log")
    }

    /// Point stderr at the log file. Restored when the returned value is dropped.
    #[cfg(unix)]
    pub fn redirect_to_log() -> Option<Redirect> {
        let path = log_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).ok()?;
        }
        let file = std::fs::File::create(&path).ok()?;
        Redirect::new(&file)
    }

    #[cfg(not(unix))]
    pub fn redirect_to_log() -> Option<Redirect> {
        // WASAPI does not chatter the way ALSA does, so leave stderr alone.
        None
    }

    #[cfg(unix)]
    pub struct Redirect {
        saved: std::os::fd::OwnedFd,
    }

    #[cfg(unix)]
    impl Redirect {
        fn new(file: &std::fs::File) -> Option<Self> {
            use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};

            // SAFETY: dup and dup2 on the process's own stderr. The saved
            // descriptor is owned from here on and closed by OwnedFd.
            unsafe {
                let saved = libc::dup(libc::STDERR_FILENO);
                if saved < 0 {
                    return None;
                }
                let saved = OwnedFd::from_raw_fd(saved);

                if libc::dup2(file.as_raw_fd(), libc::STDERR_FILENO) < 0 {
                    return None;
                }
                Some(Self { saved })
            }
        }
    }

    #[cfg(unix)]
    impl Drop for Redirect {
        fn drop(&mut self) {
            use std::os::fd::AsRawFd;

            // SAFETY: putting the original stderr back where it was.
            unsafe {
                libc::dup2(self.saved.as_raw_fd(), libc::STDERR_FILENO);
            }
        }
    }

    #[cfg(not(unix))]
    pub struct Redirect;
}

fn run_mcp(
    skills_dirs: Vec<PathBuf>,
    library_path: Option<PathBuf>,
    device: Option<&str>,
    config: Option<&Path>,
) -> color_eyre::Result<()> {
    let player = connect_agent(device, config)?;

    // A broken library must not stop the player tools from working.
    let library = library_path.and_then(|path| match Library::open(&path) {
        Ok(library) => Some(library),
        Err(e) => {
            tracing::warn!(path = %path.display(), error = %e, "library unavailable");
            None
        }
    });

    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?
        .block_on(run_stdio(player, skills_dirs, library))
        .map_err(|e| color_eyre::eyre::eyre!("{e}"))?;
    Ok(())
}

fn scan_library(
    library_path: Option<PathBuf>,
    dirs: &[PathBuf],
    prune: bool,
    device: Option<&str>,
    config: Option<&Path>,
) -> color_eyre::Result<()> {
    if dirs.is_empty() && !prune {
        println!("nothing to do: pass one or more folders, or --prune");
        return Ok(());
    }

    // Check every folder before contacting the player, so a typo scans nothing.
    let folders = dirs
        .iter()
        .map(|dir| {
            std::path::absolute(dir)
                .ok()
                .filter(|p| p.is_dir())
                .ok_or_else(|| color_eyre::eyre::eyre!("not a folder: {}", dir.display()))
        })
        .collect::<color_eyre::Result<Vec<_>>>()?;

    let player = connect_agent(device, config)?;
    let live = std::io::IsTerminal::is_terminal(&std::io::stdout());

    for folder in folders {
        println!("scanning {} ...", folder.display());
        let report = run_library_job(&player, LibraryOp::AddFolder { path: folder }, live)?;
        println!(
            "  {} files: {} added, {} updated, {} unchanged, {} failed",
            report.seen, report.added, report.updated, report.unchanged, report.failed
        );
    }

    if prune {
        let report = run_library_job(&player, LibraryOp::Prune, live)?;
        println!("removed {} missing entries", report.removed);
    }

    let library = open_library(library_path)?;
    println!("library now holds {} tracks", library.track_count()?);
    Ok(())
}

/// Send one library job to the player and wait for it, drawing progress on a terminal.
fn run_library_job(
    player: &IpcClient,
    op: LibraryOp,
    live: bool,
) -> color_eyre::Result<ScanSummary> {
    let start = player.status().scans_finished;
    let failure = op.clone();
    player
        .apply(op)
        .map_err(|e| color_eyre::eyre::eyre!("{e}"))?;
    let status = wait_for_library(player, start + 1, None, |status| {
        if let (true, Some(p)) = (live, status.scanning.as_ref()) {
            print!(
                "\r  {} files · {} new · {} updated",
                p.seen, p.added, p.updated
            );
            let _ = std::io::Write::flush(&mut std::io::stdout());
        }
    })
    .map_err(|e| color_eyre::eyre::eyre!("{e}"))?;
    if live {
        print!("\r\x1b[2K");
    }
    let summary = status.last.unwrap_or_default();
    if let Some(error) = &summary.error {
        color_eyre::eyre::bail!("{}", failure.failure_text(error));
    }
    Ok(summary)
}

fn search_library(
    library_path: Option<PathBuf>,
    query: &str,
    limit: usize,
) -> color_eyre::Result<()> {
    let library = open_library(library_path)?;
    let tracks = library.search(query, limit)?;

    if tracks.is_empty() {
        println!("no matches for {query:?}");
        return Ok(());
    }

    for track in &tracks {
        let detail = track.artist_album().unwrap_or_else(|| "—".to_string());
        println!("{}  ({})", track.title, detail);
        println!("    {}", track.path.display());
    }
    println!("{} match(es)", tracks.len());
    Ok(())
}

fn list_albums(library_path: Option<PathBuf>) -> color_eyre::Result<()> {
    let library = open_library(library_path)?;
    let albums = library.albums()?;

    if albums.is_empty() {
        println!("library is empty; run `znicz scan <dir>` first");
        return Ok(());
    }

    for album in &albums {
        let artist = album.album_artist.as_deref().unwrap_or("Unknown artist");
        let year = album.year.map(|y| format!(" ({y})")).unwrap_or_default();
        println!(
            "{} — {}{}  [{} {}]",
            artist,
            album.album,
            year,
            album.track_count,
            if album.track_count == 1 {
                "track"
            } else {
                "tracks"
            }
        );
    }
    println!("{} album(s)", albums.len());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;
    use tempfile::TempDir;

    #[test]
    fn version_is_wired() {
        let rendered = Cli::command().render_version();
        assert!(
            rendered.contains(env!("CARGO_PKG_VERSION")),
            "expected the workspace version in {rendered:?}"
        );
    }

    fn os(args: &[&str]) -> Vec<OsString> {
        args.iter().map(OsString::from).collect()
    }

    #[test]
    fn parse_resume_reads_index_and_seconds() {
        assert_eq!(
            parse_resume("3:125"),
            Some(Resume {
                index: 3,
                seconds: 125
            })
        );
        assert_eq!(
            parse_resume("0:0"),
            Some(Resume {
                index: 0,
                seconds: 0
            })
        );
    }

    #[test]
    fn parse_resume_rejects_malformed_values() {
        assert_eq!(parse_resume("x"), None);
        assert_eq!(parse_resume("3:"), None);
        assert_eq!(parse_resume(":5"), None);
    }

    #[test]
    fn restart_args_carry_only_the_global_options() {
        let resume = Resume {
            index: 3,
            seconds: 7,
        };
        assert_eq!(
            restart_args(Some(Path::new("p.toml")), Some("d"), Some(resume)),
            os(&["--config", "p.toml", "--device", "d", "--resume", "3:7"])
        );
    }

    #[test]
    fn restart_args_are_empty_without_options() {
        assert!(restart_args(None, None, None).is_empty());
    }

    #[test]
    fn default_template_parses_to_the_defaults() {
        let parsed: Config = toml::from_str(DEFAULT_CONFIG).unwrap();
        let defaults = Config::default();
        assert_eq!(tui_config(&parsed), tui_config(&defaults));
        assert_eq!(parsed.audio.device, None);
        assert_eq!(
            parsed.audio.volume.unwrap_or(1.0),
            defaults.audio.volume.unwrap_or(1.0)
        );
        assert_eq!(
            parsed.audio.bit_perfect.unwrap_or(true),
            defaults.audio.bit_perfect.unwrap_or(true)
        );
        assert_eq!(parsed.player.idle_secs.unwrap_or(900), 900);
        assert!(parsed.covers.search.unwrap_or(true));
        assert!(parsed.updates.check.unwrap_or(true));
        assert!(parsed.mcp.skills_dirs.is_empty());
        assert_eq!(parsed.library.path, None);
    }

    #[test]
    fn parse_error_names_the_line_and_column() {
        let err = parse_config_text("[audio]\nvolume = [\n").unwrap_err();
        assert_eq!(err, "line 3, column 1: invalid array, expected `]`");
    }

    #[test]
    fn good_text_parses_without_a_problem() {
        let config = parse_config_text("[tui]\nmarquee = false\n").unwrap();
        assert!(!tui_config(&config).marquee);
    }

    #[test]
    fn line_and_column_count_characters_from_one() {
        assert_eq!(line_and_column("abc", 0), (1, 1));
        assert_eq!(line_and_column("ab\ncd", 4), (2, 2));
        assert_eq!(line_and_column("é\nx", 3), (2, 1));
    }

    #[test]
    fn load_config_reports_a_broken_file_and_uses_defaults() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "[tui\nmarquee = true\n").unwrap();

        let (config, problem) = load_config(Some(&path));

        let problem = problem.expect("a problem for a broken file");
        assert!(problem.starts_with("line 1, column "), "{problem}");
        assert!(tui_config(&config).marquee);
    }

    #[test]
    fn load_config_is_quiet_for_good_and_missing_files() {
        let dir = TempDir::new().unwrap();
        let good = dir.path().join("good.toml");
        std::fs::write(&good, "[tui]\nmarquee = false\n").unwrap();

        let (config, problem) = load_config(Some(&good));
        assert!(problem.is_none());
        assert!(!tui_config(&config).marquee);

        let (_, problem) = load_config(Some(&dir.path().join("missing.toml")));
        assert!(problem.is_none());
    }

    #[test]
    fn load_config_reports_an_unreadable_file() {
        let dir = TempDir::new().unwrap();
        let (_, problem) = load_config(Some(dir.path()));
        let message = problem.expect("a problem");
        assert!(message.starts_with("cannot read"), "{message}");
    }

    #[test]
    fn first_start_writes_the_template_and_keeps_an_existing_file() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("znicz").join("config.toml");

        assert!(ensure_default_config_at(&path).unwrap());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), DEFAULT_CONFIG);

        std::fs::write(&path, "[tui]\nmarquee = false\n").unwrap();
        assert!(!ensure_default_config_at(&path).unwrap());
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "[tui]\nmarquee = false\n"
        );
    }

    #[test]
    fn config_init_writes_when_missing() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("config.toml");

        let outcome = init_config(&path, false).unwrap();

        assert_eq!(outcome, InitOutcome { backup: None });
        assert_eq!(std::fs::read_to_string(&path).unwrap(), DEFAULT_CONFIG);
    }

    #[test]
    fn config_init_refuses_to_overwrite_without_force() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "mine\n").unwrap();

        let err = init_config(&path, false).unwrap_err();

        assert!(
            err.contains("already exists (use --force to overwrite)"),
            "{err}"
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "mine\n");
        assert!(!backup_path(&path).exists());
    }

    #[test]
    fn config_init_force_backs_up_the_old_file_first() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "mine\n").unwrap();

        let outcome = init_config(&path, true).unwrap();

        assert_eq!(
            outcome,
            InitOutcome {
                backup: Some(dir.path().join("config.toml.bak"))
            }
        );
        assert_eq!(
            std::fs::read_to_string(backup_path(&path)).unwrap(),
            "mine\n"
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), DEFAULT_CONFIG);
    }
}
