use std::path::PathBuf;

use clap::{Parser, Subcommand};
use serde::Deserialize;
use znicz_core::{
    apply_to_player, copy_saved, list_saved, load_path, remove_saved, rename_saved, saved_path,
    skipped_notice, spawn_player, AudioConfig, AudioOutput, Command,
};
use znicz_library::Library;
use znicz_mcp::run_stdio;
use znicz_tui::App;

#[derive(Debug, Parser)]
#[command(name = "znicz", about = "Audiophile TUI music player")]
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
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Run MCP server on stdio (headless player)
    Mcp {
        #[arg(long, help = "Additional skills directory")]
        skills_dir: Vec<PathBuf>,
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
    Play { name: String },
    /// Remove a station
    Remove { name: String },
    /// Rename a station
    Rename { name: String, new_name: String },
    /// Change a station's stream URL
    Url { name: String, url: String },
    /// Copy a station to a new name (same URL)
    Copy { name: String, new_name: String },
}

#[derive(Debug, Deserialize, Default)]
struct Config {
    audio: AudioSection,
    mcp: McpSection,
    library: LibrarySection,
}

#[derive(Debug, Deserialize, Default)]
struct LibrarySection {
    /// Where the database file lives. Defaults to the user data directory.
    path: Option<String>,
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
struct McpSection {
    skills_dirs: Vec<String>,
}

fn main() -> color_eyre::Result<()> {
    color_eyre::install()?;
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let cli = Cli::parse();
    let config = load_config(cli.config.as_deref());

    if cli.list_devices {
        list_devices()?;
        return Ok(());
    }

    let library_path = library_path(&config);

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
            run_mcp(audio_config, dirs, library_path)?;
        }
        Some(Commands::Scan { dirs, prune }) => {
            scan_library(library_path, &dirs, prune)?;
        }
        Some(Commands::Search { query, limit }) => {
            search_library(library_path, &query, limit)?;
        }
        Some(Commands::Albums) => {
            list_albums(library_path)?;
        }
        Some(Commands::Playlist { command }) => match command {
            PlaylistCmd::List => list_playlists()?,
            PlaylistCmd::Save { name: _ } => {
                color_eyre::eyre::bail!(
                    "save the queue from the player (P then n, or MCP save_playlist)"
                );
            }
            PlaylistCmd::Import { file, append } => {
                load_playlist_and_run(file, append, audio_config, library_path)?;
            }
            PlaylistCmd::Play { name, append } => {
                let dir = playlists_dir()?;
                let path = saved_path(&dir, &name)
                    .ok_or_else(|| color_eyre::eyre::eyre!("no playlist named {name}"))?;
                load_playlist_and_run(path, append, audio_config, library_path)?;
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
            StationCmd::Play { name } => {
                play_station_and_run(&name, audio_config, library_path)?;
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
        },
        None => {
            run_tui(audio_config, &cli.files, library_path)?;
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

fn load_config(path: Option<&std::path::Path>) -> Config {
    let path = path.map(PathBuf::from).or_else(default_config_path);
    if let Some(path) = path {
        if path.is_file() {
            if let Ok(text) = std::fs::read_to_string(&path) {
                if let Ok(cfg) = toml::from_str::<Config>(&text) {
                    return cfg;
                }
            }
        }
    }
    Config::default()
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

fn run_tui(
    audio_config: AudioConfig,
    files: &[PathBuf],
    library_path: Option<PathBuf>,
) -> color_eyre::Result<()> {
    let (player, _thread) = spawn_player(audio_config);

    if !files.is_empty() {
        let items: Vec<znicz_core::QueueItem> = files
            .iter()
            .map(|p| znicz_core::QueueItem::file(p.clone()))
            .collect();
        if items.len() == 1 {
            player.send(Command::Play(items[0].clone()))?;
        } else {
            player.send(Command::QueueAdd(items.clone()))?;
            player.send(Command::Play(items[0].clone()))?;
        }
    }

    run_tui_with_player(player, library_path, None)
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

fn play_station_and_run(
    name: &str,
    audio_config: AudioConfig,
    library_path: Option<PathBuf>,
) -> color_eyre::Result<()> {
    let path = stations_path()?;
    let stations = znicz_core::load_stations(&path)?;
    let station = znicz_core::find_station(&stations, name)
        .ok_or_else(|| color_eyre::eyre::eyre!("no station named {name}"))?
        .clone();
    let (player, _thread) = spawn_player(audio_config);
    znicz_core::play_station(&player, &station)?;
    run_tui_with_player(player, library_path, None)
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
    audio_config: AudioConfig,
    library_path: Option<PathBuf>,
) -> color_eyre::Result<()> {
    let result = load_path(&path)?;
    let (player, _thread) = spawn_player(audio_config);
    apply_to_player(&player, &result, append)?;
    run_tui_with_player(player, library_path, skipped_notice(&result))
}

fn run_tui_with_player(
    player: znicz_core::PlayerHandle,
    library_path: Option<PathBuf>,
    skip_notice: Option<String>,
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
    let mut app = App::with_library(player, library);
    if let Some(message) = skip_notice {
        app.toasts.warn(message);
    }
    let result = app.run();
    drop(log);

    result
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
    audio_config: AudioConfig,
    skills_dirs: Vec<PathBuf>,
    library_path: Option<PathBuf>,
) -> color_eyre::Result<()> {
    let (player, _thread) = spawn_player(audio_config);

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
) -> color_eyre::Result<()> {
    if dirs.is_empty() && !prune {
        println!("nothing to do: pass one or more folders, or --prune");
        return Ok(());
    }

    let mut library = open_library(library_path)?;

    for dir in dirs {
        println!("scanning {} ...", dir.display());
        let report = library.scan(dir)?;
        println!(
            "  {} files: {} added, {} updated, {} unchanged, {} failed",
            report.seen, report.added, report.updated, report.unchanged, report.failed
        );
    }

    if prune {
        let removed = library.remove_missing()?;
        println!("removed {removed} missing entries");
    }

    println!("library now holds {} tracks", library.track_count()?);
    Ok(())
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
