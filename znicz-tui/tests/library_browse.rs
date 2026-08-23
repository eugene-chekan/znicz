//! Browsing a real library from the interface: albums, drilling in, searching.
//!
//! Fixtures are made with ffmpeg. Without it the tests report and pass, so the
//! suite still runs on a machine that has no encoder.

use std::path::{Path, PathBuf};
use std::process::Command as Shell;

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use znicz_core::{AudioConfig, spawn_player};
use znicz_library::Library;
use znicz_tui::library_pane::Mode;
use znicz_tui::{App, views};

fn ffmpeg_available() -> bool {
    Shell::new("ffmpeg")
        .arg("-version")
        .output()
        .map(|out| out.status.success())
        .unwrap_or(false)
}

fn write_track(path: &Path, title: &str, artist: &str, album: &str, track: u32) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("create fixture dir");
    }
    let status = Shell::new("ffmpeg")
        .args(["-y", "-loglevel", "error"])
        .args(["-f", "lavfi", "-i", "sine=frequency=440:duration=1"])
        .args(["-ac", "2", "-ar", "44100"])
        .args(["-metadata", &format!("title={title}")])
        .args(["-metadata", &format!("artist={artist}")])
        .args(["-metadata", &format!("album={album}")])
        .args(["-metadata", &format!("track={track}")])
        .args(["-c:a", "flac"])
        .arg(path)
        .status()
        .expect("run ffmpeg");
    assert!(status.success(), "ffmpeg failed for {}", path.display());
}

fn fixture_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("znicz-tui-{name}"));
    std::fs::remove_dir_all(&dir).ok();
    std::fs::create_dir_all(&dir).expect("create dir");
    dir
}

/// An app whose library holds two albums by two artists.
fn app_with_library(dir: &Path) -> App {
    write_track(
        &dir.join("01 Mysterons.flac"),
        "Mysterons",
        "Portishead",
        "Dummy",
        1,
    );
    write_track(
        &dir.join("02 Sour Times.flac"),
        "Sour Times",
        "Portishead",
        "Dummy",
        2,
    );
    write_track(
        &dir.join("03 So What.flac"),
        "So What",
        "Miles Davis",
        "Kind of Blue",
        1,
    );

    let mut library = Library::open_in_memory().expect("open library");
    library.scan(dir).expect("scan");

    let (player, _thread) = spawn_player(AudioConfig::default());
    App::with_library(player, Some(library))
}

fn draw(app: &App) -> String {
    let mut terminal = Terminal::new(TestBackend::new(90, 24)).expect("terminal");
    let state = app.state();
    terminal
        .draw(|frame| views::render(frame, app, &state))
        .expect("draw");

    let buffer = terminal.backend().buffer();
    let mut out = String::new();
    for y in 0..buffer.area().height {
        for x in 0..buffer.area().width {
            if let Some(cell) = buffer.cell((x, y)) {
                out.push_str(cell.symbol());
            }
        }
        out.push('\n');
    }
    out
}

#[test]
fn albums_are_listed_then_opened() {
    if !ffmpeg_available() {
        eprintln!("ffmpeg not available, skipping");
        return;
    }

    let dir = fixture_dir("browse");
    let mut app = app_with_library(&dir);

    let screen = draw(&app);
    assert!(
        screen.contains("Dummy"),
        "albums should be listed:\n{screen}"
    );
    assert!(screen.contains("Kind of Blue"));
    assert!(
        screen.contains("Portishead"),
        "the album artist belongs on the row"
    );
    assert!(screen.contains("2 albums"));

    // Open "Dummy": it sorts first, so the cursor is already on it.
    assert!(app.library.enter(), "Enter should open the album");
    assert_eq!(app.library.mode(), &Mode::Album("Dummy".to_string()));

    let screen = draw(&app);
    assert!(screen.contains("Mysterons"), "album tracks:\n{screen}");
    assert!(screen.contains("Sour Times"));
    assert!(
        !screen.contains("So What"),
        "the other album must not leak in"
    );

    // And back out again.
    assert!(app.library.back());
    assert_eq!(app.library.mode(), &Mode::Albums);
    assert!(draw(&app).contains("Kind of Blue"));

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_search_narrows_the_list_to_matches() {
    if !ffmpeg_available() {
        eprintln!("ffmpeg not available, skipping");
        return;
    }

    let dir = fixture_dir("search");
    let mut app = app_with_library(&dir);

    app.library.begin_search();
    for c in "miles".chars() {
        app.library.push_char(c);
    }
    let message = app.library.submit_search();
    assert!(message.contains("1 match"), "got: {message}");

    let screen = draw(&app);
    assert!(
        screen.contains("So What"),
        "the match should be shown:\n{screen}"
    );
    assert!(!screen.contains("Mysterons"), "non-matches should be gone");

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_search_with_no_matches_says_so_on_screen() {
    if !ffmpeg_available() {
        eprintln!("ffmpeg not available, skipping");
        return;
    }

    let dir = fixture_dir("nomatch");
    let mut app = app_with_library(&dir);

    app.library.begin_search();
    for c in "zzzz".chars() {
        app.library.push_char(c);
    }
    app.library.submit_search();

    let screen = draw(&app);
    assert!(
        screen.contains("nothing matched"),
        "an empty result must be explained:\n{screen}"
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn adding_an_album_queues_all_of_its_tracks() {
    if !ffmpeg_available() {
        eprintln!("ffmpeg not available, skipping");
        return;
    }

    let dir = fixture_dir("enqueue");
    let app = app_with_library(&dir);

    // The cursor sits on "Dummy", which holds two tracks.
    let tracks = app.library.selected_tracks();
    assert_eq!(
        tracks.len(),
        2,
        "selecting an album should mean all its tracks"
    );

    // Everything listed means every track in the library.
    assert_eq!(app.library.listed_tracks().len(), 3);

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_library_with_no_album_tags_falls_back_to_a_track_list() {
    if !ffmpeg_available() {
        eprintln!("ffmpeg not available, skipping");
        return;
    }

    let dir = fixture_dir("untagged");
    let path = dir.join("mystery.flac");
    let status = Shell::new("ffmpeg")
        .args(["-y", "-loglevel", "error"])
        .args(["-f", "lavfi", "-i", "sine=frequency=440:duration=1"])
        .args(["-map_metadata", "-1"])
        .args(["-c:a", "flac"])
        .arg(&path)
        .status()
        .expect("run ffmpeg");
    assert!(status.success());

    let mut library = Library::open_in_memory().expect("open library");
    library.scan(&dir).expect("scan");

    let (player, _thread) = spawn_player(AudioConfig::default());
    let app = App::with_library(player, Some(library));

    assert_eq!(app.library.mode(), &Mode::AllTracks);
    let screen = draw(&app);
    assert!(
        screen.contains("mystery"),
        "an untagged file must still be reachable:\n{screen}"
    );

    std::fs::remove_dir_all(&dir).ok();
}
