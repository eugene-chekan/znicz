//! The library lists when they are longer than the screen: what is drawn and
//! which row is highlighted once the selection is near the end.
//!
//! No ffmpeg needed: the library is filled straight from track rows.

use std::collections::BTreeSet;
use std::path::PathBuf;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::style::Color;
use znicz_core::{AudioConfig, spawn_player};
use znicz_library::Library;
use znicz_tui::{App, LibraryLayout, theme, views};

const ARTISTS: usize = 120;
const WIDTH: u16 = 160;
const HEIGHT: u16 = 30;

/// A library with `ARTISTS` artists, one track each, named `Artist 000`..`Artist 119`.
fn app_with_many_artists(layout: LibraryLayout) -> App {
    let mut library = Library::open_in_memory().expect("open library");
    for i in 0..ARTISTS {
        library
            .upsert_track_for_test(
                &PathBuf::from(format!("/music/artist-{i:03}/song.flac")),
                "Song",
                Some(format!("Artist {i:03}")),
                Some(format!("Album {i:03}")),
                None,
            )
            .expect("insert track");
    }
    let (player, _thread) = spawn_player(AudioConfig::default());
    let mut app = App::with_library(player, Some(library));
    app.tui.library_layout = layout;
    app.library.set_preferred_layout(layout);
    app
}

fn press(app: &mut App, code: KeyCode) {
    app.on_key(KeyEvent::new(code, KeyModifiers::NONE));
}

/// One string per screen row, and whether any cell of that row is drawn as the selection.
fn draw_rows(app: &mut App) -> Vec<(String, bool)> {
    let mut terminal = Terminal::new(TestBackend::new(WIDTH, HEIGHT)).expect("terminal");
    let state = app.state();
    terminal
        .draw(|frame| views::render(frame, app, &state))
        .expect("draw");
    let buffer = terminal.backend().buffer();
    let selected = theme::selected();
    (0..HEIGHT)
        .map(|y| {
            let mut text = String::new();
            let mut highlighted = false;
            for x in 0..WIDTH {
                let cell = buffer.cell((x, y)).expect("cell");
                text.push_str(cell.symbol());
                if cell.bg == selected.bg.unwrap_or(Color::Reset)
                    && cell.fg == selected.fg.unwrap_or(Color::Reset)
                {
                    highlighted = true;
                }
            }
            (text, highlighted)
        })
        .collect()
}

/// The `Artist NNN` numbers drawn anywhere on screen.
fn artist_numbers(rows: &[(String, bool)]) -> BTreeSet<usize> {
    let mut numbers = BTreeSet::new();
    for (text, _) in rows {
        let mut rest = text.as_str();
        while let Some(at) = rest.find("Artist ") {
            let digits: String = rest[at + 7..].chars().take(3).collect();
            if let Ok(n) = digits.parse::<usize>() {
                numbers.insert(n);
            }
            rest = &rest[at + 7..];
        }
    }
    numbers
}

/// The rows drawn as the selection, trimmed.
fn highlighted(rows: &[(String, bool)]) -> Vec<String> {
    rows.iter()
        .filter(|(_, on)| *on)
        .map(|(text, _)| text.trim().to_string())
        .collect()
}

/// Checks the frame after `G` has moved the selection to the last artist.
fn assert_last_artist_drawn(rows: &[(String, bool)]) {
    let numbers = artist_numbers(rows);
    assert!(numbers.len() >= 10, "a screenful of artists: {numbers:?}");
    assert_eq!(numbers.last(), Some(&(ARTISTS - 1)));
    assert!(!numbers.contains(&0), "the top has scrolled away");
    let count = numbers.len();
    assert_eq!(
        numbers.iter().copied().collect::<Vec<_>>(),
        (ARTISTS - count..ARTISTS).collect::<Vec<_>>(),
        "the drawn artists are one unbroken run ending at the selection"
    );

    let on = highlighted(rows);
    assert_eq!(on.len(), 1, "one highlighted row: {on:?}");
    assert!(on[0].contains("Artist 119"), "highlight: {on:?}");
}

#[test]
fn a_long_tree_with_the_selection_at_the_end_draws_the_last_rows() {
    let mut app = app_with_many_artists(LibraryLayout::Tree);

    let rows = draw_rows(&mut app);
    assert!(rows.iter().any(|(t, _)| t.contains("Artist 000")));
    assert!(!rows.iter().any(|(t, _)| t.contains("Artist 119")));

    press(&mut app, KeyCode::Char('G'));
    assert_last_artist_drawn(&draw_rows(&mut app));
}

#[test]
fn a_long_column_with_the_selection_at_the_end_draws_the_last_rows() {
    let mut app = app_with_many_artists(LibraryLayout::Columns);

    press(&mut app, KeyCode::Char('G'));
    assert_last_artist_drawn(&draw_rows(&mut app));
}
