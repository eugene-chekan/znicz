use std::path::Path;
use std::time::{Duration, Instant};

use znicz_core::{spawn_player, AudioConfig, AudioOutput, Command, PlayerEvent};

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

/// Tags must reach the player state, not just the file name.
#[test]
fn track_info_carries_tags() {
    let dir = std::env::temp_dir().join("znicz-tags-test");
    std::fs::create_dir_all(&dir).expect("create dir");
    let flac = dir.join("01-track.flac");

    let made = std::process::Command::new("ffmpeg")
        .args(["-y", "-loglevel", "error"])
        .args(["-f", "lavfi", "-i", "sine=frequency=440:duration=1"])
        .args(["-metadata", "title=Real Title"])
        .args(["-metadata", "artist=Real Artist"])
        .args(["-metadata", "album=Real Album"])
        .args(["-c:a", "flac"])
        .arg(&flac)
        .status()
        .map(|status| status.success())
        .unwrap_or(false);

    if !made {
        eprintln!("ffmpeg not available, skipping");
        std::fs::remove_dir_all(&dir).ok();
        return;
    }

    let info = znicz_core::probe_track(&flac).expect("probe track");

    assert_eq!(info.title, "Real Title", "title should come from tags");
    assert_eq!(info.artist(), Some("Real Artist"));
    assert_eq!(info.album(), Some("Real Album"));
    assert_eq!(
        info.artist_album().as_deref(),
        Some("Real Artist — Real Album")
    );

    std::fs::remove_dir_all(&dir).ok();
}

/// Files without tags still get a usable name.
#[test]
fn untagged_file_falls_back_to_file_name() {
    let wav = std::env::temp_dir().join("znicz-untagged.wav");
    write_silent_wav(&wav, 44_100, 2, 1);

    let info = znicz_core::probe_track(&wav).expect("probe track");

    assert_eq!(info.title, "znicz-untagged");
    assert_eq!(info.artist(), None);
    assert_eq!(info.artist_album(), None);
    assert_eq!(
        info.codec, "WAV",
        "the format must be a name, not a codec id; got {}",
        info.codec
    );
    assert!(
        !info.codec.starts_with("0x"),
        "hex codec ids are not for the UI"
    );
    let line = info.format_description();
    assert!(
        !line.contains("unknown"),
        "missing pieces should be omitted; got {line}"
    );
    assert!(
        line.contains("kbps"),
        "WAV has a known uncompressed bitrate; got {line}"
    );

    std::fs::remove_file(&wav).ok();
}

/// MP3 is the case that used to print `0x1003` and `unknown depth`.
#[test]
fn mp3_probe_uses_a_real_format_name() {
    let dir = std::env::temp_dir().join("znicz-mp3-probe");
    std::fs::create_dir_all(&dir).ok();
    let mp3 = dir.join("track.mp3");

    let made = std::process::Command::new("ffmpeg")
        .args(["-y", "-loglevel", "error"])
        .args(["-f", "lavfi", "-i", "sine=frequency=440:duration=1"])
        .args(["-ac", "2", "-ar", "44100"])
        .args(["-c:a", "libmp3lame", "-b:a", "192k"])
        .arg(&mp3)
        .status()
        .map(|status| status.success())
        .unwrap_or(false);

    if !made {
        eprintln!("ffmpeg/libmp3lame not available, skipping");
        std::fs::remove_dir_all(&dir).ok();
        return;
    }

    let info = znicz_core::probe_track(&mp3).expect("probe track");
    assert_eq!(info.codec, "MP3", "got {}", info.codec);
    assert_eq!(info.bits_per_sample, None, "MP3 has no PCM bit depth");
    let line = info.format_description();
    assert!(
        !line.contains("unknown") && !line.contains("0x"),
        "got {line}"
    );
    assert!(
        line.contains("kbps") || info.bitrate_kbps.is_some(),
        "a CBR MP3 should report a bitrate; got {line}"
    );

    std::fs::remove_dir_all(&dir).ok();
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
