//! Queue editing and playback modes, as driven by the TUI.
//!
//! These use paths that need not exist: adding, removing and reordering are
//! bookkeeping, so they can be checked without a sound card.

use znicz_core::{spawn_player, AudioConfig, Command, PlaybackStatus, RepeatMode};

fn paths(count: usize) -> Vec<znicz_core::QueueItem> {
    (0..count)
        .map(|i| znicz_core::QueueItem::file(format!("/music/track-{i}.flac")))
        .collect()
}

#[test]
fn removing_an_entry_closes_the_gap() {
    let (player, _thread) = spawn_player(AudioConfig::default());
    player
        .send_blocking(Command::QueueAdd(paths(3)))
        .expect("queue add");

    player
        .send_blocking(Command::QueueRemove(1))
        .expect("queue remove");

    let queue = player.state().queue;
    assert_eq!(queue.len(), 2);
    assert_eq!(queue[0], znicz_core::QueueItem::file("/music/track-0.flac"));
    assert_eq!(
        queue[1],
        znicz_core::QueueItem::file("/music/track-2.flac"),
        "the entry after the removed one should move up"
    );
}

#[test]
fn removing_past_the_end_is_ignored() {
    let (player, _thread) = spawn_player(AudioConfig::default());
    player
        .send_blocking(Command::QueueAdd(paths(2)))
        .expect("queue add");

    player
        .send_blocking(Command::QueueRemove(99))
        .expect("out of range remove should not be an error");

    assert_eq!(player.state().queue.len(), 2, "queue must be untouched");
}

#[test]
fn removing_from_an_empty_queue_is_harmless() {
    let (player, _thread) = spawn_player(AudioConfig::default());
    player
        .send_blocking(Command::QueueRemove(0))
        .expect("removing from nothing should not fail");
    assert!(player.state().queue.is_empty());
}

#[test]
fn repeat_and_shuffle_are_remembered() {
    let (player, _thread) = spawn_player(AudioConfig::default());

    let state = player.state();
    assert_eq!(state.repeat, RepeatMode::Off, "repeat starts off");
    assert!(!state.shuffle, "shuffle starts off");

    player
        .send_blocking(Command::SetRepeat(RepeatMode::All))
        .expect("set repeat");
    player
        .send_blocking(Command::SetShuffle(true))
        .expect("set shuffle");

    let state = player.state();
    assert_eq!(state.repeat, RepeatMode::All);
    assert!(state.shuffle);
}

#[test]
fn muting_keeps_the_volume_setting() {
    let (player, _thread) = spawn_player(AudioConfig::default());

    player
        .send_blocking(Command::SetVolume(0.6))
        .expect("set volume");
    player.send_blocking(Command::SetMuted(true)).expect("mute");

    let state = player.state();
    assert!(state.muted);
    assert!(
        (state.volume - 0.6).abs() < f32::EPSILON,
        "unmuting must restore 0.6, so the setting has to survive; got {}",
        state.volume
    );
    assert_eq!(
        state.effective_volume(),
        0.0,
        "nothing should reach the device"
    );

    player
        .send_blocking(Command::SetMuted(false))
        .expect("unmute");
    assert_eq!(player.state().effective_volume(), 0.6);
}

#[test]
fn playing_a_missing_queue_entry_reports_the_failure() {
    let (player, _thread) = spawn_player(AudioConfig::default());
    player
        .send_blocking(Command::QueueAdd(paths(2)))
        .expect("queue add");

    // The TUI turns this error into a visible message rather than doing nothing.
    let result = player.send_blocking(Command::QueuePlayIndex(1));
    assert!(result.is_err(), "a missing file must be reported");
}

#[test]
fn playing_an_index_past_the_end_does_nothing() {
    let (player, _thread) = spawn_player(AudioConfig::default());
    player
        .send_blocking(Command::QueueAdd(paths(2)))
        .expect("queue add");

    player
        .send_blocking(Command::QueuePlayIndex(50))
        .expect("out of range index should be ignored, not an error");

    assert_eq!(player.state().status, PlaybackStatus::Stopped);
}

#[test]
fn a_stopped_player_reports_no_output_stream() {
    let (player, _thread) = spawn_player(AudioConfig::default());
    assert!(
        player.state().output.is_none(),
        "there is no signal path before playback starts"
    );
}
