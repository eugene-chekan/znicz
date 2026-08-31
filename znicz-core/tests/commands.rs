//! Command acknowledgement behaviour (Issue #1).
//!
//! A caller that waits for a command must see the result of that command, not
//! the state from before it ran.

use std::time::Duration;

use znicz_core::{spawn_player, AudioConfig, Command};

#[test]
fn blocking_send_applies_before_returning() {
    let (player, _thread) = spawn_player(AudioConfig::default());

    player
        .send_blocking(Command::SetVolume(0.3))
        .expect("volume command");

    // No sleep, no polling: the state must already be correct.
    let state = player.state();
    assert!(
        (state.volume - 0.3).abs() < f32::EPSILON,
        "expected volume 0.3 right after the call, got {}",
        state.volume
    );
}

#[test]
fn blocking_send_reports_failure_to_the_caller() {
    let (player, _thread) = spawn_player(AudioConfig::default());

    let missing = std::env::temp_dir().join("znicz-does-not-exist.flac");
    let result = player.send_blocking(Command::Play(missing));

    assert!(
        result.is_err(),
        "playing a missing file must return an error, got {result:?}"
    );
}

#[test]
fn queue_commands_are_visible_immediately() {
    let (player, _thread) = spawn_player(AudioConfig::default());

    let paths = vec![
        std::path::PathBuf::from("/music/a.flac"),
        std::path::PathBuf::from("/music/b.flac"),
    ];
    player
        .send_blocking(Command::QueueAdd(paths))
        .expect("queue add");

    assert_eq!(player.state().queue.len(), 2, "queue should already hold 2");

    player
        .send_blocking(Command::QueueClear)
        .expect("queue clear");

    assert!(player.state().queue.is_empty(), "queue should be empty");
}

#[test]
fn fire_and_forget_send_still_works() {
    let (player, _thread) = spawn_player(AudioConfig::default());

    player.send(Command::SetVolume(0.5)).expect("send");

    // This path makes no promise about timing, so allow a moment.
    for _ in 0..50 {
        if (player.state().volume - 0.5).abs() < f32::EPSILON {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("volume never reached 0.5, got {}", player.state().volume);
}
