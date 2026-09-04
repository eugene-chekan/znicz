//! Browsing a real library from the interface: albums, drilling in, searching.
//!
//! Fixtures are made with ffmpeg. Without it the tests report and pass, so the
//! suite still runs on a machine that has no encoder.

use std::path::{Path, PathBuf};
use std::process::Command as Shell;

use ratatui::backend::TestBackend;
use ratatui::Terminal;
use znicz_core::{spawn_player, AudioConfig};
use znicz_library::Library;
use znicz_tui::library_pane::Mode;
use znicz_tui::{views, App};

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

fn draw(app: &mut App) -> String {
    draw_size(app, 90, 24)
}

fn draw_size(app: &mut App, width: u16, height: u16) -> String {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("terminal");
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
fn artists_are_listed_then_albums_and_tracks_open() {
    if !ffmpeg_available() {
        eprintln!("ffmpeg not available, skipping");
        return;
    }

    let dir = fixture_dir("browse");
    let mut app = app_with_library(&dir);
    // Prefer paging so Enter drills levels in one list (narrow path of the design).
    app.tui.library_layout = znicz_tui::LibraryLayout::Columns;
    app.library
        .set_preferred_layout(znicz_tui::LibraryLayout::Columns);
    // Force paging by pretending a narrow list width.
    app.list_width = 50;

    let screen = draw_size(&mut app, 50, 24);
    assert!(
        screen.contains("Miles Davis") || screen.contains("Portishead"),
        "artists should be listed:\n{screen}"
    );
    assert!(
        screen.contains("Artists") || screen.contains("artists"),
        "paging header or summary:\n{screen}"
    );

    use znicz_tui::library_pane::EnterResult;
    assert_eq!(
        app.library.enter(40),
        EnterResult::Moved,
        "Enter should open the artist's albums"
    );
    assert_eq!(app.library.mode(), &Mode::Browse);

    let screen = draw_size(&mut app, 50, 24);
    // First artist alphabetically is Miles Davis → Kind of Blue
    assert!(
        screen.contains("Kind of Blue") || screen.contains("Dummy"),
        "albums should show:\n{screen}"
    );

    assert_eq!(app.library.enter(40), EnterResult::Moved);
    let screen = draw_size(&mut app, 50, 24);
    assert!(
        screen.contains("So What") || screen.contains("Mysterons"),
        "tracks should show:\n{screen}"
    );

    assert!(app.library.back());
    assert!(
        draw_size(&mut app, 50, 24).contains("Kind of Blue")
            || draw_size(&mut app, 50, 24).contains("Dummy")
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_search_shows_entity_hits_not_every_artist_track() {
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

    let screen = draw(&mut app);
    assert!(
        screen.contains("Miles Davis"),
        "artist hit should be shown:\n{screen}"
    );
    assert!(
        screen.contains("artist"),
        "artist cue on the row:\n{screen}"
    );
    assert!(
        !screen.contains("So What"),
        "artist match must not expand to every track:\n{screen}"
    );
    assert!(!screen.contains("Mysterons"), "non-matches should be gone");

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_title_search_still_lists_the_matching_track() {
    if !ffmpeg_available() {
        eprintln!("ffmpeg not available, skipping");
        return;
    }

    let dir = fixture_dir("search-title");
    let mut app = app_with_library(&dir);

    app.library.begin_search();
    for c in "So What".chars() {
        app.library.push_char(c);
    }
    let message = app.library.submit_search();
    assert!(message.contains("1 match"), "got: {message}");

    let screen = draw(&mut app);
    assert!(
        screen.contains("So What"),
        "title hit should be shown:\n{screen}"
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

    let screen = draw(&mut app);
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
    let mut app = app_with_library(&dir);
    app.list_width = 50;
    app.library
        .set_preferred_layout(znicz_tui::LibraryLayout::Columns);

    // Artists level: Miles Davis first (1 track), then Portishead (2).
    let tracks = app.library.selected_tracks(40);
    assert_eq!(
        tracks.len(),
        1,
        "selecting an artist should mean all its tracks"
    );

    assert_eq!(
        app.library.enter(40),
        znicz_tui::library_pane::EnterResult::Moved
    );
    // Albums for Miles → Kind of Blue (1 track)
    let tracks = app.library.selected_tracks(40);
    assert_eq!(tracks.len(), 1);

    // Everything listed at albums level for this artist.
    assert_eq!(app.library.listed_tracks(40).len(), 1);

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
    let mut app = App::with_library(player, Some(library));

    assert_eq!(app.library.mode(), &Mode::AllTracks);
    let screen = draw(&mut app);
    assert!(
        screen.contains("mystery"),
        "an untagged file must still be reachable:\n{screen}"
    );

    std::fs::remove_dir_all(&dir).ok();
}
