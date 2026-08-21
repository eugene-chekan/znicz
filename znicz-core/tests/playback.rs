use std::path::Path;
use std::time::{Duration, Instant};

use znicz_core::{AudioConfig, AudioOutput, Command, PlayerEvent, spawn_player};

/// Skip audio tests on machines without an output device (headless CI).
fn has_output_device() -> bool {
    AudioOutput::list_devices()
        .map(|devices| !devices.is_empty())
        .unwrap_or(false)
}

#[test]
fn play_local_wav_starts_playback() {
    if !has_output_device() {
        eprintln!("no audio output device, skipping");
        return;
    }

    let wav = std::env::temp_dir().join("znicz-start-test.wav");
    write_silent_wav(&wav, 44_100, 2, 8);

    let (player, _thread) = spawn_player(AudioConfig::default());
    player
        .send(Command::Play(wav.clone()))
        .expect("play command");

    for _ in 0..50 {
        if player.state().current_track.is_some() {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }

    let state = player.state();
    assert!(
        state.current_track.is_some(),
        "expected a track to start; status={:?}",
        state.status
    );

    player.send(Command::Stop).ok();
    std::fs::remove_file(wav).ok();
}

/// Regression test for the fast-forward bug.
///
/// The ring buffer holds about two seconds. Once it is full, a decoded packet
/// only fits partly. The old code threw the rest away, so the track raced to
/// the end and sounded like 2x-3x speed. A correct player needs roughly real
/// time to play a file.
#[test]
fn track_plays_for_its_real_duration() {
    if !has_output_device() {
        eprintln!("no audio output device, skipping");
        return;
    }

    let seconds = 6;
    let wav = std::env::temp_dir().join("znicz-duration-test.wav");
    write_silent_wav(&wav, 44_100, 2, seconds);

    let (player, _thread) = spawn_player(AudioConfig::default());
    let started = Instant::now();
    player
        .send(Command::Play(wav.clone()))
        .expect("play command");

    let mut ended_after = None;
    while started.elapsed() < Duration::from_secs(seconds as u64 + 6) {
        if player
            .drain_events()
            .iter()
            .any(|e| matches!(e, PlayerEvent::TrackEnded))
        {
            ended_after = Some(started.elapsed());
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }

    player.send(Command::Stop).ok();
    std::fs::remove_file(wav).ok();

    let ended_after = ended_after.expect("track never reported TrackEnded");

    // Allow startup latency on both sides, but a dropped-sample bug ends the
    // 6 second file in about 2-3 seconds, which this catches.
    assert!(
        ended_after >= Duration::from_secs_f64(seconds as f64 - 1.0),
        "track ended after {:.2}s but the file is {seconds}s long: samples are being dropped",
        ended_after.as_secs_f64()
    );
    assert!(
        ended_after <= Duration::from_secs_f64(seconds as f64 + 3.0),
        "track took {:.2}s for a {seconds}s file: playback is too slow",
        ended_after.as_secs_f64()
    );
}

/// Write a silent PCM WAV file.
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
    file.write_all(&1u16.to_le_bytes()).unwrap(); // PCM
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
