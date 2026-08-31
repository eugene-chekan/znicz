//! Queue editing and playback modes, as driven by the TUI.
//!
//! These use paths that need not exist: adding, removing and reordering are
//! bookkeeping, so they can be checked without a sound card.

use std::path::Path;

use znicz_core::{
    spawn_player, AudioConfig, AudioOutput, Command, PlaybackStatus, QueueItem, RepeatMode,
};

fn skip_hardware_playback() -> bool {
    if std::env::var_os("CI").is_some() {
        eprintln!("CI: skipping hardware playback");
        return true;
    }
    let no_device = AudioOutput::list_devices()
        .map(|devices| devices.is_empty())
        .unwrap_or(true);
    if no_device {
        eprintln!("no audio output device, skipping");
    }
    no_device
}

fn write_silent_wav(path: &Path, sample_rate: u32, channels: u16, seconds: u32) {
    use std::io::Write;

    let frames = sample_rate * seconds;
    let bytes_per_frame = channels as u32 * 2;
    let data_size = frames * bytes_per_frame;
    let file_size = 36 + data_size;

    let mut file = std::fs::File::create(path).expect("create wav");
    file.write_all(b"RIFF").unwrap();
    file.write_all(&file_size.to_le_bytes()).unwrap();
    file.write_all(b"WAVE").unwrap();
    file.write_all(b"fmt ").unwrap();
    file.write_all(&16u32.to_le_bytes()).unwrap();
    file.write_all(&1u16.to_le_bytes()).unwrap();
    file.write_all(&channels.to_le_bytes()).unwrap();
    file.write_all(&sample_rate.to_le_bytes()).unwrap();
    file.write_all(&(sample_rate * bytes_per_frame).to_le_bytes())
        .unwrap();
    file.write_all(&(bytes_per_frame as u16).to_le_bytes())
        .unwrap();
    file.write_all(&16u16.to_le_bytes()).unwrap();
    file.write_all(b"data").unwrap();
    file.write_all(&data_size.to_le_bytes()).unwrap();

    let silence = vec![0u8; (bytes_per_frame * sample_rate) as usize];
    for _ in 0..seconds {
        file.write_all(&silence).unwrap();
    }
}

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

#[test]
fn appending_a_station_does_not_clear_or_start() {
    let (player, _thread) = spawn_player(AudioConfig::default());
    player
        .send_blocking(Command::QueueAdd(vec![QueueItem::file("/music/a.flac")]))
        .expect("seed");
    znicz_core::play_station(
        &player,
        &znicz_core::Station {
            name: "Live".into(),
            url: "http://127.0.0.1:1/s".into(),
        },
        true,
    )
    .expect("append");
    let state = player.state();
    assert_eq!(state.queue.len(), 2);
    assert_eq!(state.queue[0], QueueItem::file("/music/a.flac"));
    assert_eq!(
        state.queue[1],
        QueueItem::stream("Live", "http://127.0.0.1:1/s")
    );
    assert_eq!(state.status, PlaybackStatus::Stopped);
    assert_eq!(state.queue_position, 0);
}

#[test]
fn removing_the_playing_row_starts_the_row_that_slid_in() {
    if skip_hardware_playback() {
        return;
    }
    let dir = std::env::temp_dir().join("znicz-remove-playing-next");
    std::fs::create_dir_all(&dir).unwrap();
    let a = dir.join("a.wav");
    let b = dir.join("b.wav");
    write_silent_wav(&a, 44_100, 2, 2);
    write_silent_wav(&b, 44_100, 2, 2);

    let (player, _thread) = spawn_player(AudioConfig::default());
    player
        .send_blocking(Command::QueueAdd(vec![
            QueueItem::file(&a),
            QueueItem::file(&b),
        ]))
        .unwrap();
    player
        .send_blocking(Command::QueuePlayIndex(0))
        .unwrap();
    assert_eq!(player.state().status, PlaybackStatus::Playing);

    player
        .send_blocking(Command::QueueRemove(0))
        .unwrap();

    let state = player.state();
    assert_eq!(state.queue.len(), 1);
    assert_eq!(state.queue_position, 0);
    assert_eq!(state.status, PlaybackStatus::Playing);
    assert_eq!(
        state.current_track.as_ref().and_then(|t| t.path.as_deref()),
        Some(b.as_path()),
        "pause/resume must control the replacement, not the deleted file"
    );

    player.send_blocking(Command::Stop).ok();
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn removing_the_last_playing_row_stops() {
    if skip_hardware_playback() {
        return;
    }
    let dir = std::env::temp_dir().join("znicz-remove-playing-last");
    std::fs::create_dir_all(&dir).unwrap();
    let a = dir.join("a.wav");
    let b = dir.join("b.wav");
    write_silent_wav(&a, 44_100, 2, 2);
    write_silent_wav(&b, 44_100, 2, 2);

    let (player, _thread) = spawn_player(AudioConfig::default());
    player
        .send_blocking(Command::QueueAdd(vec![
            QueueItem::file(&a),
            QueueItem::file(&b),
        ]))
        .unwrap();
    player
        .send_blocking(Command::QueuePlayIndex(1))
        .unwrap();

    player
        .send_blocking(Command::QueueRemove(1))
        .unwrap();

    let state = player.state();
    assert_eq!(state.queue.len(), 1);
    assert_eq!(state.queue[0], QueueItem::file(&a));
    assert_eq!(state.queue_position, 0, "playing index must stay in range");
    assert_eq!(state.status, PlaybackStatus::Stopped);
    assert!(
        state.current_track.is_none(),
        "the deleted file must not keep making sound"
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn removing_the_only_playing_row_leaves_an_empty_stopped_queue() {
    if skip_hardware_playback() {
        return;
    }
    let wav = std::env::temp_dir().join("znicz-remove-playing-only.wav");
    write_silent_wav(&wav, 44_100, 2, 2);

    let (player, _thread) = spawn_player(AudioConfig::default());
    player
        .send_blocking(Command::QueueAdd(vec![QueueItem::file(&wav)]))
        .unwrap();
    player
        .send_blocking(Command::QueuePlayIndex(0))
        .unwrap();
    player
        .send_blocking(Command::QueueRemove(0))
        .unwrap();

    let state = player.state();
    assert!(state.queue.is_empty());
    assert_eq!(state.queue_position, 0);
    assert_eq!(state.status, PlaybackStatus::Stopped);
    assert!(state.current_track.is_none());

    std::fs::remove_file(&wav).ok();
}

#[test]
fn removing_the_playing_row_while_paused_starts_the_replacement() {
    if skip_hardware_playback() {
        return;
    }
    let dir = std::env::temp_dir().join("znicz-remove-playing-paused");
    std::fs::create_dir_all(&dir).unwrap();
    let a = dir.join("a.wav");
    let b = dir.join("b.wav");
    write_silent_wav(&a, 44_100, 2, 2);
    write_silent_wav(&b, 44_100, 2, 2);

    let (player, _thread) = spawn_player(AudioConfig::default());
    player
        .send_blocking(Command::QueueAdd(vec![
            QueueItem::file(&a),
            QueueItem::file(&b),
        ]))
        .unwrap();
    player
        .send_blocking(Command::QueuePlayIndex(0))
        .unwrap();
    player.send_blocking(Command::Pause).unwrap();
    assert_eq!(player.state().status, PlaybackStatus::Paused);

    player
        .send_blocking(Command::QueueRemove(0))
        .unwrap();

    let state = player.state();
    assert_eq!(state.status, PlaybackStatus::Playing);
    assert_eq!(
        state.current_track.as_ref().and_then(|t| t.path.as_deref()),
        Some(b.as_path())
    );

    player.send_blocking(Command::Stop).ok();
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_dead_replacement_errors_and_stays_stopped() {
    if skip_hardware_playback() {
        return;
    }
    let a = std::env::temp_dir().join("znicz-remove-dead-a.wav");
    write_silent_wav(&a, 44_100, 2, 2);

    let (player, _thread) = spawn_player(AudioConfig::default());
    player
        .send_blocking(Command::QueueAdd(vec![
            QueueItem::file(&a),
            QueueItem::file("/music/missing-replacement.flac"),
        ]))
        .unwrap();
    player
        .send_blocking(Command::QueuePlayIndex(0))
        .unwrap();

    let err = player.send_blocking(Command::QueueRemove(0));
    assert!(err.is_err(), "a missing replacement must be reported");

    let state = player.state();
    assert_eq!(state.queue.len(), 1);
    assert_eq!(state.status, PlaybackStatus::Stopped);
    assert!(
        state.current_track.is_none(),
        "the deleted file must not keep playing"
    );

    std::fs::remove_file(&a).ok();
}

#[test]
fn next_from_a_stream_row_moves_to_the_following_file() {
    let (player, _thread) = spawn_player(AudioConfig::default());
    player
        .send_blocking(Command::QueueAdd(vec![
            QueueItem::stream("Live", "http://127.0.0.1:1/s"),
            QueueItem::file("/music/a.flac"),
        ]))
        .expect("seed");
    let result = player.send_blocking(Command::NextTrack);
    assert!(result.is_err(), "a missing file must be reported");
    assert_eq!(
        player.state().queue_position,
        0,
        "position advances only after play succeeds"
    );
    assert_eq!(player.state().queue.len(), 2);
}
