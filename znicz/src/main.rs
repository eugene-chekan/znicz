use std::path::PathBuf;

use clap::{Parser, Subcommand};
use serde::Deserialize;
use znicz_core::{AudioConfig, AudioOutput, Command, spawn_player};
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
        None => {
            run_tui(audio_config, &cli.files)?;
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

fn run_tui(audio_config: AudioConfig, files: &[PathBuf]) -> color_eyre::Result<()> {
    let (player, _thread) = spawn_player(audio_config);

    if !files.is_empty() {
        let paths: Vec<PathBuf> = files.to_vec();
        if paths.len() == 1 {
            player.send(Command::Play(paths[0].clone()))?;
        } else {
            player.send(Command::QueueAdd(paths))?;
            player.send(Command::Play(files[0].clone()))?;
        }
    }

    let mut app = App::new(player);
    app.run()?;
    Ok(())
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
        let year = album
            .year
            .map(|y| format!(" ({y})"))
            .unwrap_or_default();
        println!(
            "{} — {}{}  [{} {}]",
            artist,
            album.album,
            year,
            album.track_count,
            if album.track_count == 1 { "track" } else { "tracks" }
        );
    }
    println!("{} album(s)", albums.len());
    Ok(())
}
