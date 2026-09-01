use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};

use znicz_core::{
    play_station, spawn_player, AudioConfig, AudioOutput, Command, PlaybackStatus, PlayerEvent,
    QueueItem, Station, TrackInfo,
};

fn serve_html() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut buf = [0u8; 2048];
        let _ = stream.read(&mut buf);
        let body = b"<html>nope</html>";
        let header = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        let _ = stream.write_all(header.as_bytes());
        let _ = stream.write_all(body);
    });
    format!("http://{addr}/x")
}

fn silent_wav_bytes(sample_rate: u32, channels: u16, frames: u32) -> Vec<u8> {
    let bytes_per_frame = u32::from(channels) * 2;
    let data_size = frames * bytes_per_frame;
    let file_size = 36 + data_size;
    let mut buf = Vec::new();
    buf.extend_from_slice(b"RIFF");
    buf.extend_from_slice(&file_size.to_le_bytes());
    buf.extend_from_slice(b"WAVE");
    buf.extend_from_slice(b"fmt ");
    buf.extend_from_slice(&16u32.to_le_bytes());
    buf.extend_from_slice(&1u16.to_le_bytes());
    buf.extend_from_slice(&channels.to_le_bytes());
    buf.extend_from_slice(&sample_rate.to_le_bytes());
    buf.extend_from_slice(&(sample_rate * bytes_per_frame).to_le_bytes());
    buf.extend_from_slice(&(bytes_per_frame as u16).to_le_bytes());
    buf.extend_from_slice(&16u16.to_le_bytes());
    buf.extend_from_slice(b"data");
    buf.extend_from_slice(&data_size.to_le_bytes());
    buf.extend(vec![0u8; data_size as usize]);
    buf
}

fn serve_once(body: Vec<u8>, content_type: &str) -> String {
    serve_once_with(body, content_type, None)
}

fn serve_once_with(body: Vec<u8>, content_type: &str, icy_metaint: Option<usize>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let icy = icy_metaint
        .map(|n| format!("icy-metaint: {n}\r\n"))
        .unwrap_or_default();
    let header = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\n{icy}Content-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut buf = [0u8; 2048];
        let _ = stream.read(&mut buf);
        let _ = stream.write_all(header.as_bytes());
        let _ = stream.write_all(&body);
    });
    format!("http://{addr}/stream")
}

fn icy_block(payload: &str) -> Vec<u8> {
    let mut bytes = payload.as_bytes().to_vec();
    let padded = bytes.len().div_ceil(16) * 16;
    bytes.resize(padded, 0);
    let mut out = vec![(padded / 16) as u8];
    out.extend(bytes);
    out
}

/// Decode-only: no cpal, no device. Skip only when ffmpeg cannot make the MP3.
#[test]
fn audio_decoder_decodes_a_loopback_mp3_stream() {
    let mp3 = std::env::temp_dir().join("znicz-stream-decode-test.mp3");
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
        let _ = std::fs::remove_file(&mp3);
        return;
    }

    let buf = std::fs::read(&mp3).unwrap();
    let _ = std::fs::remove_file(&mp3);
    decode_loopback_body(buf, "audio/mpeg");
}

/// Same decode-only path with a WAV fixture so the test still runs without ffmpeg.
#[test]
fn audio_decoder_decodes_a_loopback_wav_stream() {
    decode_loopback_body(silent_wav_bytes(44_100, 2, 256), "audio/wav");
}

fn decode_loopback_body(body: Vec<u8>, content_type: &str) {
    let url = serve_once(body, content_type);
    let source = znicz_core::audio::http::HttpStreamSource::new("TestStream", url);
    let (mut decoder, _info) =
        znicz_core::audio::source::AudioDecoder::open(&source).expect("open decoder");
    let next = decoder.decode_next().expect("decode_next error");
    assert!(next.is_some(), "expected at least one decoded packet");
}

#[test]
fn audio_decoder_measures_coded_bitrate_on_a_wav_stream() {
    let url = serve_once(silent_wav_bytes(44_100, 2, 44_100), "audio/wav");
    let source = znicz_core::audio::http::HttpStreamSource::new("TestStream", url);
    let (mut decoder, _) =
        znicz_core::audio::source::AudioDecoder::open(&source).expect("open decoder");
    while decoder.measured_bitrate_kbps().is_none() {
        match decoder.decode_next().expect("decode") {
            Some(_) => {}
            None => break,
        }
    }
    let kbps = decoder
        .measured_bitrate_kbps()
        .expect("enough PCM to measure bitrate");
    assert!(
        (1200..=1600).contains(&kbps),
        "16-bit stereo 44.1 kHz is 1411 kbps; got {kbps}"
    );
}

#[test]
fn playing_a_non_audio_url_returns_an_error() {
    let (player, _thread) = spawn_player(AudioConfig::default());
    let url = serve_html();
    let result = player.send_blocking(Command::Play(QueueItem::stream("Bad", url)));
    assert!(result.is_err(), "got {result:?}");
}

#[test]
fn seek_is_refused_when_the_queue_row_is_a_stream() {
    let (player, _thread) = spawn_player(AudioConfig::default());
    player
        .send_blocking(Command::QueueAdd(vec![QueueItem::stream(
            "Live",
            "http://127.0.0.1:1/stream",
        )]))
        .unwrap();
    let err = player
        .send_blocking(Command::Seek(Duration::from_secs(1)))
        .unwrap_err();
    assert!(err.to_string().contains("radio cannot seek"), "{err}");
}

fn skip_hardware_playback() -> bool {
    if std::env::var_os("CI").is_some() {
        eprintln!("CI: skipping hardware playback");
        return true;
    }
    let no_device = AudioOutput::list_devices()
        .map(|devices| devices.is_empty())
        .unwrap_or(true);
    if no_device {
        eprintln!("no audio output device, skipping hardware playback");
    }
    no_device
}

fn write_silent_wav(path: &Path) {
    std::fs::write(path, silent_wav_bytes(44_100, 2, 44_100)).expect("write wav");
}

fn file_track(path: &Path) -> TrackInfo {
    TrackInfo {
        path: Some(path.to_path_buf()),
        url: None,
        title: "old-file".into(),
        codec: "WAV".into(),
        sample_rate: 44_100,
        channels: 2,
        bits_per_sample: Some(16),
        bitrate_kbps: Some(1411),
        duration: None,
        tags: Default::default(),
    }
}

/// Failed station play must not leave the previous file as now-playing.
#[test]
fn failed_station_play_does_not_keep_the_old_file() {
    let wav = std::env::temp_dir().join("znicz-failed-station-old.wav");
    write_silent_wav(&wav);

    let (player, _thread) = spawn_player(AudioConfig::default());
    player
        .send_blocking(Command::QueueAdd(vec![QueueItem::file(wav.clone())]))
        .unwrap();

    let played = if skip_hardware_playback() {
        false
    } else {
        match player.send_blocking(Command::Play(QueueItem::file(wav.clone()))) {
            Ok(()) => true,
            Err(e) => {
                eprintln!("could not play local wav, bookkeeping only: {e}");
                false
            }
        }
    };

    if played {
        let state = player.state();
        assert_eq!(state.status, PlaybackStatus::Playing);
        assert_eq!(
            state.current_track.as_ref().and_then(|t| t.path.as_ref()),
            Some(&wav)
        );
    } else {
        let state = player.state_arc();
        let mut state = state.write().unwrap();
        state.current_track = Some(file_track(&wav));
        state.status = PlaybackStatus::Playing;
    }

    let url = serve_html();
    let result = play_station(
        &player,
        &Station {
            name: "Bad".into(),
            url,
        },
        false,
    );
    assert!(result.is_err(), "got {result:?}");

    let state = player.state();
    assert_eq!(state.queue.len(), 1);
    assert!(state.queue[0].is_stream(), "queue should be the stream row");
    match &state.current_track {
        None => {}
        Some(track) => {
            assert_ne!(
                track.path.as_ref(),
                Some(&wav),
                "current_track must not still be the old file"
            );
        }
    }

    if played {
        assert_ne!(
            state.status,
            PlaybackStatus::Playing,
            "speakers must not keep playing the old file"
        );
    }

    std::fs::remove_file(wav).ok();
}

/// A stream body that dies after probe must stop, not sit in Playing with no decoder.
#[test]
fn dropped_stream_body_stops_playback() {
    if skip_hardware_playback() {
        return;
    }

    let mut body = silent_wav_bytes(44_100, 2, 44_100);
    body.truncate(44 + 4096);
    let url = serve_once(body, "audio/wav");

    let (player, _thread) = spawn_player(AudioConfig::default());
    if let Err(e) = player.send_blocking(Command::Play(QueueItem::stream("Drop", url))) {
        eprintln!("could not start truncated stream, skipping: {e}");
        return;
    }

    let started = Instant::now();
    let mut saw_error = false;
    loop {
        saw_error |= player
            .drain_events()
            .iter()
            .any(|e| matches!(e, PlayerEvent::Error(_)));
        let state = player.state();
        if saw_error && state.status == PlaybackStatus::Stopped && state.output.is_none() {
            return;
        }
        if started.elapsed() > Duration::from_secs(5) {
            panic!(
                "expected Error then Stopped with no output; saw_error={saw_error} status={:?} output={:?}",
                state.status, state.output
            );
        }
        thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn icy_stream_title_reaches_player_state() {
    if skip_hardware_playback() {
        return;
    }

    let wav = silent_wav_bytes(44_100, 2, 44_100);
    let mut body = wav[..44].to_vec();
    body.extend_from_slice(&icy_block("StreamTitle='Song';"));
    body.extend_from_slice(&wav[44..]);
    let url = serve_once_with(body, "audio/wav", Some(44));

    let (player, _thread) = spawn_player(AudioConfig::default());
    if let Err(e) = player.send_blocking(Command::Play(QueueItem::stream("Station", url))) {
        eprintln!("could not start icy stream, skipping: {e}");
        return;
    }

    let started = Instant::now();
    loop {
        let state = player.state();
        let title_ok = state.current_track.as_ref().is_some_and(|track| {
            track.title == "Song" && track.tags.title.as_deref() == Some("Song")
        });
        if title_ok {
            match state.queue.get(state.queue_position) {
                Some(QueueItem::Stream { name, .. }) => assert_eq!(name, "Station"),
                other => panic!("queue row should stay the station, got {other:?}"),
            }
            return;
        }
        if started.elapsed() > Duration::from_secs(5) {
            panic!(
                "expected StreamTitle on now-playing; title={:?} tags.title={:?}",
                state.current_track.as_ref().map(|t| t.title.as_str()),
                state
                    .current_track
                    .as_ref()
                    .and_then(|t| t.tags.title.clone())
            );
        }
        thread::sleep(Duration::from_millis(20));
    }
}
