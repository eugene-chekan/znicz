use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread;
use std::time::Duration;

use znicz_core::{spawn_player, AudioConfig, Command, QueueItem};

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
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let header = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
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
