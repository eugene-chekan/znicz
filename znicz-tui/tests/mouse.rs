//! Mouse: select-only clicks, wheel, click-outside, queue toggle.

use crossterm::event::{
    KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::layout::Rect;
use znicz_core::{spawn_player, AudioConfig, PlaybackStatus, PlayerHandle};
use znicz_library::AlbumSummary;
use znicz_tui::hit::{HitMap, ListHit};
use znicz_core::Command;
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

fn queue(app: &mut App, count: usize) {
    let items: Vec<znicz_core::QueueItem> = (0..count)
        .map(|i| znicz_core::QueueItem::file(format!("/music/track-{i}.flac")))
        .collect();
    app.player
        .send_blocking(Command::QueueAdd(items))
        .expect("queue add");
}

fn contains(rect: Rect, column: u16, row: u16) -> bool {
    rect.contains(ratatui::layout::Position { x: column, y: row })
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

#[test]
fn a_queue_click_selects_the_row_and_focuses_the_queue() {
    let mut app = new_app();
    queue(&mut app, 4);
    app.queue_open = true;
    app.focus = Focus::Library;
    app.hits.queue = Some(ListHit {
        inner: Rect::new(60, 1, 38, 10),
        offset: 0,
        len: 4,
    });
    app.on_mouse(left_click(62, 3));
    assert_eq!(app.queue_cursor.selected(4), Some(2));
    assert_eq!(app.focus, Focus::Queue);
    assert_eq!(app.player.state().queue_position, 0);
}

#[test]
fn the_right_border_opens_the_queue_when_it_is_closed() {
    let mut app = new_app();
    app.list_width = 100;
    app.hits.queue_toggle = Some(Rect::new(79, 0, 1, 20));
    assert!(!app.queue_open);
    app.on_mouse(left_click(79, 4));
    assert!(app.queue_open);
    assert_eq!(app.focus, Focus::Queue);
}

#[test]
fn a_click_on_the_library_closes_an_overlay_queue() {
    let mut app = new_app();
    app.list_width = 100;
    app.queue_open = true;
    app.focus = Focus::Queue;
    app.hits.library_pane = Some(Rect::new(0, 0, 59, 20));
    app.hits.queue = Some(ListHit {
        inner: Rect::new(60, 1, 38, 18),
        offset: 0,
        len: 0,
    });
    app.on_mouse(left_click(10, 5));
    assert!(!app.queue_open);
    assert_eq!(app.focus, Focus::Library);
}

#[test]
fn the_right_border_closes_a_queue_sheet() {
    let mut app = new_app();
    app.list_width = 81;
    app.queue_open = true;
    app.focus = Focus::Queue;
    app.hits.queue_toggle = Some(Rect::new(80, 0, 1, 20));
    app.hits.queue = Some(ListHit {
        inner: Rect::new(1, 1, 79, 18),
        offset: 0,
        len: 0,
    });
    app.on_mouse(left_click(80, 2));
    assert!(!app.queue_open);
    assert_eq!(app.focus, Focus::Library);
}

#[test]
fn a_click_outside_help_closes_it() {
    let mut app = new_app();
    app.modal = Modal::Help;
    app.hits.overlay = Some(Rect::new(10, 4, 60, 16));
    app.on_mouse(left_click(0, 0));
    assert_eq!(app.modal, Modal::None);
}

#[test]
fn a_click_inside_help_does_not_close_it() {
    let mut app = new_app();
    app.modal = Modal::Help;
    app.hits.overlay = Some(Rect::new(10, 4, 60, 16));
    app.on_mouse(left_click(12, 6));
    assert_eq!(app.modal, Modal::Help);
}

#[test]
fn a_click_outside_inspector_closes_it() {
    let mut app = new_app();
    app.modal = Modal::Inspector;
    app.hits.overlay = Some(Rect::new(20, 4, 40, 12));
    app.on_mouse(left_click(0, 0));
    assert_eq!(app.modal, Modal::None);
}

#[test]
fn a_devices_row_click_moves_the_cursor_without_applying() {
    let mut app = new_app();
    app.modal = Modal::Devices;
    app.devices = vec![
        znicz_core::AudioDeviceInfo {
            id: "a".into(),
            name: "A".into(),
            is_default: true,
        },
        znicz_core::AudioDeviceInfo {
            id: "b".into(),
            name: "B".into(),
            is_default: false,
        },
    ];
    app.hits.overlay = Some(Rect::new(10, 4, 40, 12));
    app.hits.overlay_list = Some(ListHit {
        inner: Rect::new(11, 5, 38, 10),
        offset: 0,
        len: 2,
    });
    let before = app.player.state().device_id.clone();
    app.on_mouse(left_click(12, 6));
    assert_eq!(app.device_cursor.selected(2), Some(1));
    assert_eq!(app.modal, Modal::Devices);
    assert_eq!(app.player.state().device_id, before);
}

#[test]
fn a_click_outside_devices_closes_the_overlay() {
    let mut app = new_app();
    app.modal = Modal::Devices;
    app.hits.overlay = Some(Rect::new(10, 4, 40, 12));
    app.on_mouse(left_click(0, 0));
    assert_eq!(app.modal, Modal::None);
}
