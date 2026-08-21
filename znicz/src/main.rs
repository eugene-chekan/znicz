use std::path::PathBuf;

use clap::{Parser, Subcommand};
use serde::Deserialize;
use znicz_core::{AudioConfig, AudioOutput, Command, spawn_player};
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
}

#[derive(Debug, Deserialize, Default)]
struct Config {
    audio: AudioSection,
    mcp: McpSection,
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
            run_mcp(audio_config, dirs)?;
        }
        None => {
            run_tui(audio_config, &cli.files)?;
        }
    }

    Ok(())
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

fn run_mcp(audio_config: AudioConfig, skills_dirs: Vec<PathBuf>) -> color_eyre::Result<()> {
    let (player, _thread) = spawn_player(audio_config);
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?
        .block_on(run_stdio(player, skills_dirs))
        .map_err(|e| color_eyre::eyre::eyre!("{e}"))?;
    Ok(())
}
