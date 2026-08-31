//! Playback timing check.
//!
//! Plays a file and compares the reported position with the wall clock. Drift
//! means the audio is running fast or slow, which is what you hear as
//! fast-forward or slow motion.
//!
//! Usage: cargo run --release -p znicz-core --example timing -- <file>

use std::time::{Duration, Instant};

use znicz_core::{spawn_player, AudioConfig, Command, PlaybackStatus, PlayerEvent};

fn main() {
    tracing_subscriber::fmt::init();

    let Some(path) = std::env::args().nth(1) else {
        eprintln!("usage: timing <audio file>");
        std::process::exit(2);
    };

    let (player, _thread) = spawn_player(AudioConfig::default());
    let started = Instant::now();
    player
        .send(Command::Play(path.into()))
        .expect("send play command");

    let mut track_duration = None;
    let mut last_report = Instant::now();

    loop {
        for event in player.drain_events() {
            match event {
                PlayerEvent::TrackStarted(info) => {
                    track_duration = info.duration;
                    println!(
                        "track: {} Hz, {} ch, duration {:?}",
                        info.sample_rate, info.channels, info.duration
                    );
                }
                PlayerEvent::TrackEnded => {
                    let elapsed = started.elapsed().as_secs_f64();
                    println!("\nended after {elapsed:.2}s of wall clock");
                    if let Some(duration) = track_duration {
                        let expected = duration.as_secs_f64();
                        let ratio = expected / elapsed;
                        println!("file duration {expected:.2}s, speed factor {ratio:.3}x");
                        if ratio > 1.05 {
                            println!("PROBLEM: playback ran fast (samples are being lost)");
                        } else if ratio < 0.95 {
                            println!("PROBLEM: playback ran slow");
                        } else {
                            println!("OK: playback ran at real time");
                        }
                    }
                    return;
                }
                PlayerEvent::Error(message) => {
                    eprintln!("error: {message}");
                    return;
                }
                _ => {}
            }
        }

        if last_report.elapsed() > Duration::from_secs(1) {
            last_report = Instant::now();
            let state = player.state();
            let wall = started.elapsed().as_secs_f64();
            let position = state.position.as_secs_f64();
            println!(
                "wall {wall:6.2}s | position {position:6.2}s | drift {:+.2}s",
                position - wall
            );
            if state.status == PlaybackStatus::Stopped && wall > 2.0 {
                println!("stopped unexpectedly");
                return;
            }
        }

        std::thread::sleep(Duration::from_millis(20));
    }
}
