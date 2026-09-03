//! Mouse: select-only clicks, wheel, click-outside, queue toggle.

use crossterm::event::{
    KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::layout::Rect;
use znicz_core::{spawn_player, AudioConfig, PlaybackStatus, PlayerHandle};
use znicz_library::AlbumSummary;
use znicz_tui::hit::{HitMap, ListHit};
use znicz_tui::{App, Focus, Modal};

fn player() -> PlayerHandle {
    let (player, _thread) = spawn_player(AudioConfig::default());
    player
}

fn new_app() -> App {
    App::with_library(player(), None)
}

fn left_click(column: u16, row: u16) -> MouseEvent {
    MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column,
        row,
        modifiers: KeyModifiers::NONE,
    }
}

fn albums(n: usize) -> Vec<AlbumSummary> {
    (0..n)
        .map(|i| AlbumSummary {
            album: format!("Album {i}"),
            album_artist: Some("Artist".into()),
            year: None,
            track_count: 1,
            total_secs: None,
        })
        .collect()
}

fn library_hits(len: usize) -> HitMap {
    HitMap {
        library: Some(ListHit {
            inner: Rect::new(1, 1, 40, 10),
            offset: 0,
            len,
        }),
        library_pane: Some(Rect::new(0, 0, 80, 20)),
        queue_toggle: Some(Rect::new(79, 0, 1, 20)),
        ..HitMap::default()
    }
}

#[test]
fn a_library_click_moves_the_cursor_and_does_not_play() {
    let mut app = new_app();
    app.library.inject_albums_for_test(albums(5));
    app.hits = library_hits(5);
    assert_eq!(app.library.selected_index(), Some(0));

    app.on_mouse(left_click(2, 3));

    assert_eq!(app.library.selected_index(), Some(2));
    assert_eq!(app.focus, Focus::Library);
    assert_eq!(app.player.state().status, PlaybackStatus::Stopped);
    assert!(!app.library.is_empty());
    match app.library.mode() {
        znicz_tui::library_pane::Mode::Albums => {}
        other => panic!("must not open an album, got {other:?}"),
    }
}

#[test]
fn a_click_below_the_last_library_row_does_nothing() {
    let mut app = new_app();
    app.library.inject_albums_for_test(albums(2));
    app.hits = library_hits(2);
    app.on_mouse(left_click(2, 5));
    assert_eq!(app.library.selected_index(), Some(0));
}
