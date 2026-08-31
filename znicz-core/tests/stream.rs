use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread;
use std::time::Duration;

use znicz_core::{
    spawn_player, AudioConfig, Command, QueueItem,
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
            "https://example.com/s",
        )]))
        .unwrap();
    let err = player
        .send_blocking(Command::Seek(Duration::from_secs(1)))
        .unwrap_err();
    assert!(err.to_string().contains("radio cannot seek"), "{err}");
}
