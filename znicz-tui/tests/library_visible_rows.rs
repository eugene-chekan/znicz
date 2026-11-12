//! The library lists when they are longer than the screen: what is drawn and
//! which row is highlighted once the selection is near the end.
//!
//! No ffmpeg needed: the library is filled straight from track rows.

use std::collections::BTreeSet;
use std::path::PathBuf;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
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

/// Track rows titled `Tune 000`..`Tune 119`, by distinct artists.
fn app_with_many_tracks() -> App {
    let mut library = Library::open_in_memory().expect("open library");
    for i in 0..ARTISTS {
        library
            .upsert_track_for_test(
                &PathBuf::from(format!("/music/tune-{i:03}.flac")),
                &format!("Tune {i:03}"),
                Some(format!("Band {i:03}")),
                Some(format!("Record {i:03}")),
                None,
            )
            .expect("insert track");
    }
    let (player, _thread) = spawn_player(AudioConfig::default());
    App::with_library(player, Some(library))
}

/// One string per screen row, and whether any cell of that row is drawn as the selection.
fn draw_rows(app: &mut App) -> Vec<(String, bool)> {
    draw_rows_at(app, WIDTH)
}

fn draw_rows_at(app: &mut App, width: u16) -> Vec<(String, bool)> {
    let mut terminal = Terminal::new(TestBackend::new(width, HEIGHT)).expect("terminal");
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
            for x in 0..width {
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

#[test]
fn a_narrow_paged_list_with_the_selection_at_the_end_draws_the_last_rows() {
    let mut app = app_with_many_artists(LibraryLayout::Columns);
    // Narrow enough that the library pages instead of showing columns.
    app.list_width = 40;

    press(&mut app, KeyCode::Char('G'));
    assert_last_artist_drawn(&draw_rows_at(&mut app, 40));
}

#[test]
fn a_long_search_with_the_selection_at_the_end_draws_the_last_rows() {
    let mut app = app_with_many_tracks();
    app.library.begin_search();
    for c in "tune".chars() {
        app.library.push_char(c);
    }
    app.library.submit_search();

    press(&mut app, KeyCode::Char('G'));
    let rows = draw_rows(&mut app);
    let last = match app.library.search_hits().last() {
        Some(znicz_library::SearchHit::Track(track)) => track.title.clone(),
        other => panic!("expected a track hit last, got {other:?}"),
    };
    let on = highlighted(&rows);
    assert_eq!(on.len(), 1, "one highlighted row: {on:?}");
    assert!(on[0].contains(&last), "highlight {on:?} should show {last}");
    assert!(
        rows.iter().any(|(t, _)| t.contains(&last)),
        "the last hit is drawn"
    );
}

#[test]
fn going_up_one_row_keeps_the_scrolled_window() {
    let mut app = app_with_many_artists(LibraryLayout::Tree);
    press(&mut app, KeyCode::Char('G'));
    let before = draw_rows(&mut app);
    let last_at = before
        .iter()
        .position(|(t, _)| t.contains("▶ Artist 119"))
        .expect("last artist drawn");

    press(&mut app, KeyCode::Char('k'));
    let rows = draw_rows(&mut app);
    assert_eq!(
        rows.iter().position(|(t, _)| t.contains("▶ Artist 119")),
        Some(last_at),
        "the window did not move"
    );
    let on = highlighted(&rows);
    assert_eq!(on.len(), 1, "one highlighted row: {on:?}");
    assert!(on[0].contains("Artist 118"), "highlight: {on:?}");
}

#[test]
fn a_click_on_the_third_visible_row_selects_that_absolute_row() {
    let mut app = app_with_many_artists(LibraryLayout::Tree);
    press(&mut app, KeyCode::Char('G'));
    let rows = draw_rows(&mut app);
    let inner = app.hits.library.expect("library hit").inner;
    let first = artist_numbers(&rows)
        .into_iter()
        .next()
        .expect("drawn artists");
    let clicked = first + 2;

    app.on_mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: inner.x + 1,
        row: inner.y + 2,
        modifiers: KeyModifiers::NONE,
    });
    let rows = draw_rows(&mut app);
    let on = highlighted(&rows);
    assert_eq!(on.len(), 1, "one highlighted row: {on:?}");
    assert!(
        on[0].contains(&format!("Artist {clicked:03}")),
        "clicked row {clicked} should be selected, got {on:?}"
    );
}
