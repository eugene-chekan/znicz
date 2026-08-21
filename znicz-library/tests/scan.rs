//! Library scanning and querying against real tagged files.
//!
//! The fixtures are generated with ffmpeg. When ffmpeg is missing the tests
//! report that and pass, so the suite still works on a bare machine.

use std::path::{Path, PathBuf};
use std::process::Command;

use znicz_library::Library;

fn ffmpeg_available() -> bool {
    Command::new("ffmpeg")
        .arg("-version")
        .output()
        .map(|out| out.status.success())
        .unwrap_or(false)
}

/// Write a short tagged FLAC file.
fn write_track(path: &Path, title: &str, artist: &str, album: &str, track: u32) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("create fixture dir");
    }

    let status = Command::new("ffmpeg")
        .args(["-y", "-loglevel", "error"])
        .args(["-f", "lavfi", "-i", "sine=frequency=440:duration=1"])
        .args(["-ac", "2", "-ar", "44100"])
        .args(["-metadata", &format!("title={title}")])
        .args(["-metadata", &format!("artist={artist}")])
        .args(["-metadata", &format!("album={album}")])
        .args(["-metadata", &format!("track={track}")])
        .args(["-metadata", "date=1994"])
        .args(["-c:a", "flac"])
        .arg(path)
        .status()
        .expect("run ffmpeg");

    assert!(status.success(), "ffmpeg failed for {}", path.display());
}

fn fixture_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("znicz-library-{name}"));
    std::fs::remove_dir_all(&dir).ok();
    std::fs::create_dir_all(&dir).expect("create dir");
    dir
}

#[test]
fn scan_indexes_tagged_files() {
    if !ffmpeg_available() {
        eprintln!("ffmpeg not available, skipping");
        return;
    }

    let dir = fixture_dir("scan");
    write_track(&dir.join("Dummy/01 Mysterons.flac"), "Mysterons", "Portishead", "Dummy", 1);
    write_track(&dir.join("Dummy/02 Sour Times.flac"), "Sour Times", "Portishead", "Dummy", 2);
    write_track(&dir.join("Blue/01 So What.flac"), "So What", "Miles Davis", "Kind of Blue", 1);
    // Not audio: must be ignored.
    std::fs::write(dir.join("Dummy/cover.jpg"), b"not audio").unwrap();

    let mut library = Library::open_in_memory().expect("open library");
    let report = library.scan(&dir).expect("scan");

    assert_eq!(report.seen, 3, "should see 3 audio files, report={report:?}");
    assert_eq!(report.added, 3, "should add 3 tracks, report={report:?}");
    assert_eq!(library.track_count().unwrap(), 3);

    // Tags, not file names.
    let found = library.search("Portishead", 10).expect("search");
    assert_eq!(found.len(), 2, "expected 2 Portishead tracks, got {found:?}");
    assert_eq!(found[0].artist.as_deref(), Some("Portishead"));
    assert_eq!(found[0].album.as_deref(), Some("Dummy"));
    assert_eq!(found[0].year, Some(1994));

    // Album browse comes back in track order.
    let album = library.browse_album("Dummy").expect("browse");
    let titles: Vec<&str> = album.iter().map(|t| t.title.as_str()).collect();
    assert_eq!(titles, vec!["Mysterons", "Sour Times"]);

    // Technical details were read too.
    assert_eq!(album[0].sample_rate, Some(44_100));
    assert_eq!(album[0].channels, Some(2));
    assert!(album[0].duration_secs.unwrap_or(0.0) > 0.5);

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn rescan_skips_unchanged_files() {
    if !ffmpeg_available() {
        eprintln!("ffmpeg not available, skipping");
        return;
    }

    let dir = fixture_dir("rescan");
    write_track(&dir.join("a.flac"), "A", "Artist", "Album", 1);

    let mut library = Library::open_in_memory().expect("open library");
    let first = library.scan(&dir).expect("first scan");
    assert_eq!(first.added, 1);

    let second = library.scan(&dir).expect("second scan");
    assert_eq!(second.added, 0, "nothing new should be added");
    assert_eq!(second.unchanged, 1, "the file should be skipped");
    assert_eq!(library.track_count().unwrap(), 1, "no duplicate rows");

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn albums_are_grouped() {
    if !ffmpeg_available() {
        eprintln!("ffmpeg not available, skipping");
        return;
    }

    let dir = fixture_dir("albums");
    write_track(&dir.join("a1.flac"), "A1", "Artist One", "First", 1);
    write_track(&dir.join("a2.flac"), "A2", "Artist One", "First", 2);
    write_track(&dir.join("b1.flac"), "B1", "Artist Two", "Second", 1);

    let mut library = Library::open_in_memory().expect("open library");
    library.scan(&dir).expect("scan");

    let albums = library.albums().expect("albums");
    assert_eq!(albums.len(), 2, "expected 2 albums, got {albums:?}");

    let first = albums.iter().find(|a| a.album == "First").expect("First");
    assert_eq!(first.track_count, 2);

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn missing_files_are_removed() {
    if !ffmpeg_available() {
        eprintln!("ffmpeg not available, skipping");
        return;
    }

    let dir = fixture_dir("missing");
    let track = dir.join("gone.flac");
    write_track(&track, "Gone", "Artist", "Album", 1);

    let mut library = Library::open_in_memory().expect("open library");
    library.scan(&dir).expect("scan");
    assert_eq!(library.track_count().unwrap(), 1);

    std::fs::remove_file(&track).unwrap();
    let removed = library.remove_missing().expect("prune");

    assert_eq!(removed, 1);
    assert_eq!(library.track_count().unwrap(), 0);

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn scanning_a_file_path_is_rejected() {
    let dir = fixture_dir("notadir");
    let file = dir.join("plain.txt");
    std::fs::write(&file, b"hello").unwrap();

    let mut library = Library::open_in_memory().expect("open library");
    assert!(library.scan(&file).is_err(), "a file is not a folder");

    std::fs::remove_dir_all(&dir).ok();
}
