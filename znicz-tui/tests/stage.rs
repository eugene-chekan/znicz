//! The Stage view: toggling, keys, and drawing.

use std::path::PathBuf;
use std::time::Duration;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::backend::TestBackend;
use ratatui::Terminal;
use znicz_core::{
    spawn_player, AudioConfig, OutputInfo, PlaybackStatus, PlayerState, TrackInfo, TrackTags,
};
use znicz_tui::{views, App, View};

fn app() -> App {
    let (player, _thread) = spawn_player(AudioConfig::default());
    App::with_library(player, None)
}

fn press(app: &mut App, c: char) {
    app.on_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
}

fn draw_with(app: &mut App, state: &PlayerState, w: u16, h: u16) -> String {
    let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
    terminal.draw(|f| views::render(f, app, state)).unwrap();
    let buf = terminal.backend().buffer();
    let mut out = String::new();
    for y in 0..buf.area().height {
        for x in 0..buf.area().width {
            out.push_str(buf[(x, y)].symbol());
        }
        out.push('\n');
    }
    out
}

fn playing() -> PlayerState {
    PlayerState {
        status: PlaybackStatus::Playing,
        current_track: Some(TrackInfo {
            path: Some(PathBuf::from("/music/sour-times.flac")),
            url: None,
            icy_stream_url: None,
            title: "Sour Times".into(),
            codec: "FLAC".into(),
            sample_rate: 96_000,
            channels: 2,
            bits_per_sample: Some(24),
            bitrate_kbps: Some(2882),
            duration: Some(Duration::from_secs(251)),
            tags: TrackTags {
                artist: Some("Portishead".into()),
                album: Some("Dummy".into()),
                ..TrackTags::default()
            },
        }),
        device_name: Some("Topping E30 II".into()),
        output: Some(OutputInfo {
            sample_rate: 96_000,
            channels: 2,
            sample_format: "f32".into(),
            bit_perfect: true,
        }),
        ..PlayerState::default()
    }
}

#[test]
fn v_toggles_stage() {
    let mut app = app();
    press(&mut app, 'v');
    assert_eq!(app.view, View::Stage);
    press(&mut app, 'v');
    assert_eq!(app.view, View::Browse);
}

#[test]
fn esc_closes_the_drawer_first_then_leaves_stage() {
    let mut app = app();
    press(&mut app, 'v');
    press(&mut app, ']');
    assert!(app.queue_open);
    app.on_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert!(!app.queue_open);
    assert_eq!(app.view, View::Stage);
    app.on_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert_eq!(app.view, View::Browse);
}

#[test]
fn library_keys_do_nothing_on_stage() {
    let mut app = app();
    press(&mut app, 'v');
    press(&mut app, '/');
    assert!(!app.library.is_typing());
    press(&mut app, 'F');
    assert!(app.folder_prompt.is_none());
}

#[test]
fn stage_draws_details_and_the_signal_panel() {
    let mut app = app();
    app.view = View::Stage;
    let screen = draw_with(&mut app, &playing(), 120, 40);
    for want in [
        "Sour Times",
        "Portishead",
        "Signal",
        "bit perfect",
        "Topping E30 II",
    ] {
        assert!(screen.contains(want), "missing {want:?}:\n{screen}");
    }
    assert!(
        !screen.contains("Library"),
        "no library pane on stage:\n{screen}"
    );
}

#[test]
fn stage_draws_at_every_size() {
    let mut app = app();
    app.view = View::Stage;
    for &(w, h) in &[
        (80u16, 24u16),
        (120, 40),
        (60, 18),
        (40, 12),
        (30, 8),
        (20, 5),
        (10, 3),
    ] {
        let screen = draw_with(&mut app, &playing(), w, h);
        assert_eq!(screen.lines().count(), usize::from(h));
    }
}

fn app_with_library_rows() -> App {
    let mut library = znicz_library::Library::open_in_memory().expect("open library");
    for (path, title, artist, album) in [
        ("/music/a1.flac", "So What", "Miles", "Kind of Blue"),
        ("/music/b1.flac", "Roads", "Portishead", "Dummy"),
    ] {
        library
            .upsert_track_for_test(
                std::path::Path::new(path),
                title,
                Some(artist.into()),
                Some(album.into()),
                None,
            )
            .expect("insert track");
    }
    let (player, _thread) = spawn_player(AudioConfig::default());
    App::with_library(player, Some(library))
}

#[test]
fn tab_on_stage_does_not_move_the_hidden_library_columns() {
    let mut app = app_with_library_rows();
    let before = app.library.column_focus();
    press(&mut app, 'v');
    app.on_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    assert_eq!(app.library.column_focus(), before);
    assert!(app.queue_open);
}

#[test]
fn alt_arrow_on_stage_does_not_pan_the_hidden_library() {
    let mut app = app_with_library_rows();
    press(&mut app, 'v');
    app.on_key(KeyEvent::new(KeyCode::Right, KeyModifiers::ALT));
    assert_eq!(app.library.h_offset(), 0);
}

#[test]
fn i_toggles_stage_too() {
    let mut app = app();
    press(&mut app, 'i');
    assert_eq!(app.view, View::Stage);
    press(&mut app, 'i');
    assert_eq!(app.view, View::Browse);
}

#[test]
fn the_stage_footer_offers_browse() {
    let mut app = app();
    app.view = View::Stage;
    let screen = draw_with(&mut app, &playing(), 100, 30);
    let footer = screen.lines().last().unwrap();
    assert!(footer.contains("v browse"), "{footer}");
    assert!(footer.contains("Space pause"), "{footer}");
}

#[test]
fn the_queue_sheet_covers_the_whole_stage_area() {
    let mut app = app();
    app.view = View::Stage;
    app.queue_open = true;
    let screen = draw_with(&mut app, &playing(), 60, 24);
    // Stage spans rows 0..22 (20 list + 3 transport - the message row).
    let last_stage_row = screen.lines().nth(21).unwrap();
    assert!(last_stage_row.contains('└'), "{screen}");
}

#[test]
fn tab_keeps_focus_on_the_queue_while_on_stage() {
    let mut app = app();
    app.view = View::Stage;
    draw_with(&mut app, &playing(), 120, 40);
    app.on_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    assert!(app.queue_open, "Tab opens a closed drawer");
    let focus = app.focus;
    draw_with(&mut app, &playing(), 120, 40);
    app.on_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    assert!(app.queue_open);
    assert_eq!(app.focus, focus, "Tab: no swap to the hidden library");
    app.on_key(KeyEvent::new(KeyCode::BackTab, KeyModifiers::NONE));
    assert_eq!(app.focus, focus, "BackTab: no swap to the hidden library");
}

#[test]
fn r_does_not_reload_the_hidden_library_on_stage() {
    let mut app = app();
    app.view = View::Stage;
    press(&mut app, 'r');
    assert!(app.toasts.current().is_none());
}

#[test]
fn clicking_the_right_edge_on_stage_does_not_open_the_queue() {
    use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
    let mut app = app();
    app.view = View::Stage;
    draw_with(&mut app, &playing(), 120, 40);
    app.on_mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: 119,
        row: 5,
        modifiers: KeyModifiers::NONE,
    });
    assert!(!app.queue_open);
}
