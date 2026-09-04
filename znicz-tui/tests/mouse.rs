//! Mouse: select-only clicks, wheel, click-outside, queue toggle.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::backend::TestBackend;
use ratatui::layout::Rect;
use ratatui::Terminal;
use znicz_core::Command;
use znicz_core::{spawn_player, AudioConfig, PlaybackStatus, PlayerHandle};
use znicz_library::AlbumSummary;
use znicz_tui::hit::{FooterHit, HitMap, ListHit};
use znicz_tui::views;
use znicz_tui::{App, Focus, Modal, PlaylistPrompt};

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

fn wheel(up: bool) -> MouseEvent {
    MouseEvent {
        kind: if up {
            MouseEventKind::ScrollUp
        } else {
            MouseEventKind::ScrollDown
        },
        column: 2,
        row: 2,
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
fn a_drawn_library_row_is_clickable() {
    let mut app = new_app();
    app.library.inject_albums_for_test(albums(5));
    let state = app.player.state();
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).expect("backend");
    terminal
        .draw(|frame| views::render(frame, &mut app, &state))
        .expect("draw");
    let inner = app.hits.library.expect("library hit after draw");
    let col = inner.inner.x + 1;
    let row = inner.inner.y + 2;
    app.on_mouse(left_click(col, row));
    assert_eq!(app.library.selected_index(), Some(2));
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
fn a_library_click_under_an_open_queue_does_not_close_it() {
    let mut app = new_app();
    queue(&mut app, 2);
    app.queue_open = true;
    app.focus = Focus::Queue;
    app.hits = HitMap {
        library: Some(ListHit {
            inner: Rect::new(1, 1, 40, 10),
            offset: 0,
            len: 0,
        }),
        library_pane: Some(Rect::new(0, 0, 50, 20)),
        queue: Some(ListHit {
            inner: Rect::new(51, 1, 28, 10),
            offset: 0,
            len: 2,
        }),
        queue_toggle: Some(Rect::new(49, 0, 1, 20)),
        ..HitMap::default()
    };
    app.on_mouse(left_click(2, 5));
    assert!(app.queue_open);
}

#[test]
fn a_toggle_column_click_does_not_close_an_open_queue() {
    let mut app = new_app();
    app.queue_open = true;
    app.focus = Focus::Queue;
    app.hits.queue_toggle = Some(Rect::new(79, 0, 1, 20));
    app.hits.library_pane = Some(Rect::new(0, 0, 80, 20));
    app.on_mouse(left_click(79, 4));
    assert!(app.queue_open);
}

#[test]
fn a_click_outside_help_does_not_close_it() {
    let mut app = new_app();
    app.modal = Modal::Help;
    app.hits.overlay = Some(Rect::new(10, 4, 60, 16));
    app.on_mouse(left_click(0, 0));
    assert_eq!(app.modal, Modal::Help);
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
fn a_click_outside_inspector_does_not_close_it() {
    let mut app = new_app();
    app.modal = Modal::Inspector;
    app.hits.overlay = Some(Rect::new(20, 4, 40, 12));
    app.on_mouse(left_click(0, 0));
    assert_eq!(app.modal, Modal::Inspector);
}

#[test]
fn a_drawn_overlay_exposes_a_close_hit() {
    let mut app = new_app();
    app.modal = Modal::Help;
    let state = app.player.state();
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).expect("backend");
    terminal
        .draw(|frame| views::render(frame, &mut app, &state))
        .expect("draw");
    let close = app.hits.close.expect("close hit after draw");
    app.on_mouse(left_click(close.x, close.y));
    assert_eq!(app.modal, Modal::None);
}

#[test]
fn a_drawn_queue_exposes_a_close_hit() {
    let mut app = new_app();
    queue(&mut app, 1);
    app.queue_open = true;
    let state = app.player.state();
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).expect("backend");
    terminal
        .draw(|frame| views::render(frame, &mut app, &state))
        .expect("draw");
    let close = app.hits.close.expect("queue close hit");
    app.on_mouse(left_click(close.x, close.y));
    assert!(!app.queue_open);
}

#[test]
fn a_click_on_close_dismisses_help() {
    let mut app = new_app();
    app.modal = Modal::Help;
    app.hits.overlay = Some(Rect::new(10, 4, 60, 16));
    app.hits.close = Some(Rect::new(68, 4, 1, 1));
    app.on_mouse(left_click(68, 4));
    assert_eq!(app.modal, Modal::None);
}

#[test]
fn a_click_on_close_closes_the_queue_drawer() {
    let mut app = new_app();
    app.queue_open = true;
    app.focus = Focus::Queue;
    app.hits.close = Some(Rect::new(78, 0, 1, 1));
    app.on_mouse(left_click(78, 0));
    assert!(!app.queue_open);
    assert_eq!(app.focus, Focus::Library);
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
fn a_click_outside_devices_does_not_close_the_overlay() {
    let mut app = new_app();
    app.modal = Modal::Devices;
    app.hits.overlay = Some(Rect::new(10, 4, 40, 12));
    app.on_mouse(left_click(0, 0));
    assert_eq!(app.modal, Modal::Devices);
}

#[test]
fn a_click_outside_search_cancels_the_prompt() {
    let mut app = new_app();
    app.library.inject_albums_for_test(albums(3));
    app.library.begin_search();
    app.hits.search_prompt = Some(Rect::new(0, 0, 80, 1));
    app.hits.library = Some(ListHit {
        inner: Rect::new(1, 2, 40, 10),
        offset: 0,
        len: 3,
    });
    app.on_mouse(left_click(2, 5));
    assert!(!app.library.is_typing());
    assert_eq!(app.library.selected_index(), Some(0));
}

#[test]
fn a_click_on_the_search_line_does_not_type_or_select() {
    let mut app = new_app();
    app.library.begin_search();
    app.hits.search_prompt = Some(Rect::new(0, 0, 80, 1));
    app.on_mouse(left_click(4, 0));
    assert!(app.library.is_typing());
}

#[test]
fn a_click_outside_a_playlist_form_cancels_the_form_and_keeps_the_overlay() {
    let mut app = new_app();
    app.modal = Modal::Playlists;
    app.playlist_prompt = Some(PlaylistPrompt::Save(
        znicz_tui::line_edit::LineEdit::from_text("x"),
    ));
    app.hits.overlay = Some(Rect::new(10, 4, 40, 12));
    app.on_mouse(left_click(0, 0));
    assert!(app.playlist_prompt.is_none());
    assert_eq!(app.modal, Modal::Playlists);
}

#[test]
fn wheel_down_steps_the_library() {
    let mut app = new_app();
    app.library.inject_albums_for_test(albums(5));
    app.on_mouse(wheel(false));
    assert_eq!(app.library.selected_index(), Some(1));
}

#[test]
fn wheel_up_wraps_like_k() {
    let mut app = new_app();
    app.library.inject_albums_for_test(albums(5));
    app.on_mouse(wheel(true));
    assert_eq!(app.library.selected_index(), Some(4));
}

#[test]
fn wheel_steps_a_list_overlay_not_the_library() {
    let mut app = new_app();
    app.library.inject_albums_for_test(albums(5));
    app.modal = Modal::Playlists;
    app.playlists = vec!["a".into(), "b".into(), "c".into()];
    app.on_mouse(wheel(false));
    assert_eq!(app.playlist_cursor.selected(3), Some(1));
    assert_eq!(app.library.selected_index(), Some(0));
}

#[test]
fn wheel_is_ignored_while_help_is_open() {
    let mut app = new_app();
    app.library.inject_albums_for_test(albums(5));
    app.modal = Modal::Help;
    app.on_mouse(wheel(false));
    assert_eq!(app.library.selected_index(), Some(0));
}

#[test]
fn wheel_is_ignored_while_searching() {
    let mut app = new_app();
    app.library.inject_albums_for_test(albums(5));
    app.library.begin_search();
    app.on_mouse(wheel(false));
    assert_eq!(app.library.selected_index(), Some(0));
}

#[test]
fn a_click_on_the_transport_does_nothing() {
    let mut app = new_app();
    app.library.inject_albums_for_test(albums(3));
    app.hits = library_hits(3);
    app.on_mouse(left_click(10, 22));
    assert_eq!(app.library.selected_index(), Some(0));
    assert!(!app.queue_open);
    assert_eq!(app.modal, Modal::None);
}

#[test]
fn a_footer_help_hit_opens_help() {
    let mut app = new_app();
    app.hits.footer_hints = vec![FooterHit {
        rect: Rect::new(70, 23, 7, 1),
        key: KeyEvent::new(KeyCode::Char('?'), KeyModifiers::NONE),
    }];
    app.on_mouse(left_click(72, 23));
    assert_eq!(app.modal, Modal::Help);
}

#[test]
fn a_footer_esc_hit_closes_devices() {
    let mut app = new_app();
    app.modal = Modal::Devices;
    app.hits.overlay = Some(Rect::new(10, 4, 40, 12));
    app.hits.footer_hints = vec![FooterHit {
        rect: Rect::new(10, 23, 9, 1),
        key: KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
    }];
    app.on_mouse(left_click(12, 23));
    assert_eq!(app.modal, Modal::None);
}

#[test]
fn a_footer_hit_runs_while_an_overlay_is_open() {
    let mut app = new_app();
    app.modal = Modal::Inspector;
    app.hits.overlay = Some(Rect::new(20, 4, 40, 12));
    app.hits.footer_hints = vec![FooterHit {
        rect: Rect::new(0, 23, 7, 1),
        key: KeyEvent::new(KeyCode::Char('?'), KeyModifiers::NONE),
    }];
    app.on_mouse(left_click(2, 23));
    assert_eq!(app.modal, Modal::Help);
}
