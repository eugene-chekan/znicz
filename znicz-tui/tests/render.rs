//! Rendering tests against an in-memory terminal.
//!
//! A TUI fails in two ways the type system cannot catch: it panics because a
//! width calculation went negative, or it silently draws nothing useful. These
//! tests draw real frames at awkward sizes and read the result back.

use std::path::PathBuf;
use std::time::Duration;

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use znicz_core::{AudioConfig, Command, PlayerHandle, spawn_player};
use znicz_tui::app::Pane;
use znicz_tui::meta::Entry;
use znicz_tui::{App, views};

/// Sizes worth checking: a default terminal, a wide one, a cramped one, and a
/// window small enough that panes have to be dropped.
const SIZES: &[(u16, u16)] = &[
    (80, 24),
    (120, 40),
    (200, 60),
    (60, 18),
    (40, 12),
    (30, 8),
    (20, 5),
    (10, 3),
];

fn player() -> PlayerHandle {
    let (player, _thread) = spawn_player(AudioConfig::default());
    // The thread handle is dropped on purpose: the engine outlives the test.
    player
}

fn draw(app: &App, width: u16, height: u16) -> String {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("test terminal");
    let state = app.state();
    terminal
        .draw(|frame| views::render(frame, app, &state))
        .expect("draw");
    dump(&terminal)
}

/// The rendered screen as text, one line per terminal row.
fn dump(terminal: &Terminal<TestBackend>) -> String {
    let buffer = terminal.backend().buffer();
    let area = buffer.area();
    let mut out = String::new();
    for y in 0..area.height {
        for x in 0..area.width {
            if let Some(cell) = buffer.cell((x, y)) {
                out.push_str(cell.symbol());
            }
        }
        out.push('\n');
    }
    out
}

#[test]
fn every_pane_draws_at_every_size() {
    let mut app = App::with_library(player(), None);
    app.player
        .send_blocking(Command::QueueAdd(vec![
            PathBuf::from("/music/one.flac"),
            PathBuf::from("/music/two.flac"),
        ]))
        .expect("queue add");

    for pane in Pane::ALL {
        app.pane = pane;
        for &(width, height) in SIZES {
            // A panic here is the failure; the assert just uses the result.
            let screen = draw(&app, width, height);
            assert_eq!(
                screen.lines().count(),
                height as usize,
                "{pane:?} at {width}x{height} should fill the window"
            );
        }
    }
}

#[test]
fn the_help_overlay_draws_at_every_size() {
    let mut app = App::with_library(player(), None);
    app.show_help = true;

    for &(width, height) in SIZES {
        let screen = draw(&app, width, height);
        assert_eq!(screen.lines().count(), height as usize);
    }

    // On a normal terminal the keys should actually be legible.
    let screen = draw(&app, 100, 40);
    assert!(screen.contains("Keys"), "the overlay needs its title");
    assert!(screen.contains("play / pause"), "bindings should be listed");
    assert!(screen.contains("search the library"));
}

#[test]
fn an_idle_player_says_so_rather_than_showing_blanks() {
    let app = App::with_library(player(), None);
    let screen = draw(&app, 90, 24);

    assert!(screen.contains("Nothing playing"));
    assert!(
        screen.contains("Queue is empty"),
        "the empty queue should guide the user"
    );
    assert!(
        screen.contains("stopped"),
        "the status bar should show the state"
    );
}

#[test]
fn queue_rows_show_tags_once_they_are_known() {
    let mut app = App::with_library(player(), None);
    let path = PathBuf::from("/music/kashmir.flac");

    app.meta.insert(
        path.clone(),
        Entry {
            title: "Kashmir".into(),
            artist: Some("Led Zeppelin".into()),
            album: Some("Physical Graffiti".into()),
            duration: Some(Duration::from_secs(508)),
        },
    );
    app.player
        .send_blocking(Command::QueueAdd(vec![path]))
        .expect("queue add");
    app.pane = Pane::Queue;

    let screen = draw(&app, 90, 24);
    assert!(
        screen.contains("Led Zeppelin — Kashmir"),
        "the queue should read as music, not as file names:\n{screen}"
    );
    assert!(screen.contains("8:28"), "track length should be shown");
}

#[test]
fn a_queue_row_without_tags_falls_back_to_the_file_name() {
    let mut app = App::with_library(player(), None);
    app.player
        .send_blocking(Command::QueueAdd(vec![PathBuf::from(
            "/music/04 - mystery.flac",
        )]))
        .expect("queue add");
    app.pane = Pane::Queue;

    let screen = draw(&app, 90, 24);
    assert!(
        screen.contains("04 - mystery"),
        "a row must never be blank while tags are loading:\n{screen}"
    );
}

#[test]
fn the_library_pane_explains_how_to_fill_it() {
    let mut app = App::with_library(player(), None);
    app.pane = Pane::Library;

    let screen = draw(&app, 90, 24);
    assert!(
        screen.contains("scan"),
        "an empty library should point at the scan command:\n{screen}"
    );
}

#[test]
fn the_search_prompt_shows_what_is_being_typed() {
    let mut app = App::with_library(player(), None);
    app.pane = Pane::Library;
    app.library.begin_search();
    for c in "zeppelin".chars() {
        app.library.push_char(c);
    }

    let screen = draw(&app, 90, 24);
    assert!(screen.contains("search:"), "the prompt should be visible");
    assert!(
        screen.contains("zeppelin"),
        "typed text should be echoed:\n{screen}"
    );
}

#[test]
fn messages_replace_the_hint_line_when_present() {
    let mut app = App::with_library(player(), None);

    let screen = draw(&app, 90, 24);
    assert!(
        screen.contains("? help"),
        "hints show when there is no message"
    );

    app.toasts.error("could not open device");
    let screen = draw(&app, 90, 24);
    assert!(
        screen.contains("could not open device"),
        "errors must be visible on screen, not just in the log:\n{screen}"
    );
}

#[test]
fn the_focused_pane_is_the_one_shown() {
    let mut app = App::with_library(player(), None);

    app.pane = Pane::Devices;
    let screen = draw(&app, 90, 24);
    assert!(screen.contains("Devices"));

    app.pane = Pane::Library;
    let screen = draw(&app, 90, 24);
    assert!(screen.contains("Library"));
}

#[test]
fn a_very_long_title_is_cut_rather_than_wrapped() {
    let app = App::with_library(player(), None);
    let path = PathBuf::from("/music/long.flac");
    app.meta.insert(
        path.clone(),
        Entry {
            title: "A".repeat(400),
            artist: Some("B".repeat(400)),
            album: None,
            duration: None,
        },
    );
    app.player
        .send_blocking(Command::QueueAdd(vec![path]))
        .expect("queue add");

    let screen = draw(&app, 60, 20);
    for line in screen.lines() {
        assert!(
            line.chars().count() <= 60,
            "a long title must not push past the window: {} chars",
            line.chars().count()
        );
    }
}
