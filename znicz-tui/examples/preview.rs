//! Print the interface to stdout instead of taking over the terminal.
//!
//! Useful for checking the layout without a sound card or a music library:
//!
//! ```text
//! cargo run -p znicz-tui --example preview
//! cargo run -p znicz-tui --example preview -- 120 40
//! ```

use std::path::PathBuf;
use std::time::Duration;

use ratatui::backend::TestBackend;
use ratatui::Terminal;
use znicz_core::{
    spawn_player, AudioConfig, Command, OutputInfo, PlaybackStatus, PlayerState, TrackInfo,
    TrackTags,
};
use znicz_tui::meta::Entry;
use znicz_tui::{Focus, Modal, views, App};

fn main() {
    let mut args = std::env::args().skip(1);
    let width: u16 = args.next().and_then(|a| a.parse().ok()).unwrap_or(96);
    let height: u16 = args.next().and_then(|a| a.parse().ok()).unwrap_or(28);

    let (player, _thread) = spawn_player(AudioConfig::default());
    let mut app = App::with_library(player, None);

    let queue = demo_queue();
    app.player
        .send_blocking(Command::QueueAdd(
            queue.iter().map(|(p, _)| p.clone()).collect(),
        ))
        .expect("queue add");
    for (path, entry) in &queue {
        app.meta.insert(path.clone(), entry.clone());
    }

    app.queue_open = true;
    app.focus = Focus::Queue;
    show(
        "Queue — bit perfect",
        &mut app,
        &playing_state(&queue, true),
        width,
        height,
    );

    app.toasts.error("device refused 96 kHz, resampling");
    show(
        "Queue — resampled, with an error message",
        &mut app,
        &playing_state(&queue, false),
        width,
        height,
    );

    app.queue_open = false;
    app.focus = Focus::Library;
    app.modal = Modal::None;
    show(
        "Library — no library yet",
        &mut app,
        &PlayerState::default(),
        width,
        height,
    );

    app.modal = Modal::Devices;
    show(
        "Devices",
        &mut app,
        &playing_state(&queue, true),
        width,
        height,
    );

    app.modal = Modal::None;
    app.queue_open = true;
    app.focus = Focus::Queue;
    app.modal = Modal::Help;
    show(
        "Help overlay",
        &mut app,
        &playing_state(&queue, true),
        width,
        height,
    );
    app.modal = Modal::None;

    show(
        "Small window (48x14)",
        &mut app,
        &playing_state(&queue, true),
        48,
        14,
    );
}

fn show(label: &str, app: &mut App, state: &PlayerState, width: u16, height: u16) {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("terminal");
    terminal
        .draw(|frame| views::render(frame, app, state))
        .expect("draw");

    println!("\n{label}");
    println!("┌{}┐", "─".repeat(width as usize));
    let buffer = terminal.backend().buffer();
    for y in 0..buffer.area().height {
        let mut line = String::new();
        for x in 0..buffer.area().width {
            if let Some(cell) = buffer.cell((x, y)) {
                line.push_str(cell.symbol());
            }
        }
        println!("│{line}│");
    }
    println!("└{}┘", "─".repeat(width as usize));
}

fn demo_queue() -> Vec<(PathBuf, Entry)> {
    [
        ("Kashmir", "Led Zeppelin", "Physical Graffiti", 508),
        (
            "In My Time of Dying",
            "Led Zeppelin",
            "Physical Graffiti",
            664,
        ),
        ("Ten Years Gone", "Led Zeppelin", "Physical Graffiti", 393),
        ("The Rain Song", "Led Zeppelin", "Houses of the Holy", 458),
    ]
    .into_iter()
    .enumerate()
    .map(|(i, (title, artist, album, secs))| {
        (
            PathBuf::from(format!("/music/{i}-{title}.flac")),
            Entry {
                title: title.to_string(),
                artist: Some(artist.to_string()),
                album: Some(album.to_string()),
                duration: Some(Duration::from_secs(secs)),
            },
        )
    })
    .collect()
}

fn playing_state(queue: &[(PathBuf, Entry)], bit_perfect: bool) -> PlayerState {
    let (path, entry) = &queue[1];

    PlayerState {
        status: PlaybackStatus::Playing,
        current_track: Some(TrackInfo {
            path: path.clone(),
            title: entry.title.clone(),
            codec: "FLAC".to_string(),
            sample_rate: 96_000,
            channels: 2,
            bits_per_sample: Some(24),
            bitrate_kbps: Some(2882),
            duration: entry.duration,
            tags: TrackTags {
                title: Some(entry.title.clone()),
                artist: entry.artist.clone(),
                album: entry.album.clone(),
                ..TrackTags::default()
            },
        }),
        position: Duration::from_secs(212),
        volume: 0.7,
        muted: false,
        device_id: None,
        device_name: Some("Topping E30 II".to_string()),
        output: Some(OutputInfo {
            sample_rate: if bit_perfect { 96_000 } else { 48_000 },
            channels: 2,
            sample_format: "f32".to_string(),
            bit_perfect,
        }),
        queue: queue.iter().map(|(p, _)| p.clone()).collect(),
        queue_position: 1,
        repeat: znicz_core::RepeatMode::All,
        shuffle: true,
    }
}
