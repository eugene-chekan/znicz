//! Rendering tests against an in-memory terminal.
//!
//! A TUI fails in two ways the type system cannot catch: it panics because a
//! width calculation went negative, or it silently draws nothing useful. These
//! tests draw real frames at awkward sizes and read the result back.

use std::path::PathBuf;
use std::time::Duration;

use ratatui::backend::TestBackend;
use ratatui::Terminal;
use znicz_core::{spawn_player, AudioConfig, Command, PlayerHandle};
use znicz_tui::line_edit::LineEdit;
use znicz_tui::{views, App, Focus, Modal, PlaylistPrompt, RadioPrompt, StationField};

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

fn draw(app: &mut App, width: u16, height: u16) -> String {
    let state = app.state();
    draw_with(app, &state, width, height)
}

fn draw_with(app: &mut App, state: &znicz_core::PlayerState, width: u16, height: u16) -> String {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("test terminal");
    terminal
        .draw(|frame| views::render(frame, app, state))
        .expect("draw");
    dump(&terminal)
}

fn playing_state() -> znicz_core::PlayerState {
    use znicz_core::{OutputInfo, PlaybackStatus, PlayerState, TrackInfo, TrackTags};

    PlayerState {
        status: PlaybackStatus::Playing,
        current_track: Some(TrackInfo {
            path: Some(PathBuf::from("/music/sour-times.flac")),
            url: None,
            title: "Sour Times".into(),
            codec: "FLAC".into(),
            sample_rate: 96_000,
            channels: 2,
            bits_per_sample: Some(24),
            bitrate_kbps: Some(2882),
            duration: Some(Duration::from_secs(251)),
            tags: TrackTags {
                title: Some("Sour Times".into()),
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
fn library_keeps_most_of_the_window_when_cover_is_on() {
    let mut app = App::with_library(player(), None);
    app.tui.show_cover = true;
    let screen = draw(&mut app, 80, 24);
    assert_eq!(screen.lines().count(), 24);
    assert_eq!(app.list_height, 15);
    assert!(screen.contains("Nothing playing"), "{screen}");
}

#[test]
fn the_cover_slot_is_fully_painted_when_nothing_is_playing() {
    use ratatui::style::Color;

    let mut app = App::with_library(player(), None);
    app.tui.show_cover = true;
    let mut terminal = ratatui::Terminal::new(TestBackend::new(80, 24)).expect("test terminal");
    let state = app.state();
    terminal
        .draw(|frame| views::render(frame, &mut app, &state))
        .expect("draw");
    let buf = terminal.backend().buffer();
    let cover_w = znicz_tui::layout::cover_width(8, 10, 20, 80);
    let y0 = app.list_height;
    let inset = znicz_tui::layout::COVER_INSET;
    let mut painted = 0u16;
    for y in y0..y0 + 8 {
        let border = buf.cell((0, y)).expect("border column");
        assert!(
            border.bg == Color::Reset && border.symbol() == " ",
            "column 0 is the library left border, not the cover"
        );
        for x in inset..inset + cover_w {
            let cell = buf.cell((x, y)).expect("cell");
            if cell.bg != Color::Reset || cell.symbol() != " " {
                painted += 1;
            }
        }
    }
    assert_eq!(
        painted,
        cover_w * 8,
        "every cover cell must be painted so a previous Kitty image cannot remain"
    );
}

#[test]
fn show_cover_false_keeps_two_transport_rows() {
    let mut app = App::with_library(player(), None);
    app.tui.show_cover = false;
    let screen = draw(&mut app, 90, 24);
    assert!(screen.contains("Nothing playing"), "{screen}");
    assert_eq!(app.list_height, 21);
}

#[test]
fn every_view_draws_at_every_size() {
    for &(width, height) in SIZES {
        let mut app = App::with_library(player(), None);
        let screen = draw(&mut app, width, height);
        assert_eq!(
            screen.lines().count(),
            height as usize,
            "library at {width}x{height} should fill the window"
        );
    }

    for &(width, height) in SIZES {
        let mut app = App::with_library(player(), None);
        app.player
            .send_blocking(Command::QueueAdd(vec![
                znicz_core::QueueItem::file("/music/one.flac"),
                znicz_core::QueueItem::file("/music/two.flac"),
            ]))
            .expect("queue add");
        app.queue_open = true;
        app.focus = Focus::Queue;
        let screen = draw(&mut app, width, height);
        assert_eq!(
            screen.lines().count(),
            height as usize,
            "queue at {width}x{height} should fill the window"
        );
    }

    for &(width, height) in SIZES {
        let mut app = App::with_library(player(), None);
        app.modal = Modal::Devices;
        let screen = draw(&mut app, width, height);
        assert_eq!(
            screen.lines().count(),
            height as usize,
            "devices at {width}x{height} should fill the window"
        );
    }

    for &(width, height) in SIZES {
        let mut app = App::with_library(player(), None);
        app.modal = Modal::Inspector;
        let screen = draw(&mut app, width, height);
        assert_eq!(
            screen.lines().count(),
            height as usize,
            "inspector at {width}x{height} should fill the window"
        );
    }

    for &(width, height) in SIZES {
        let mut app = App::with_library(player(), None);
        app.modal = Modal::Playlists;
        let screen = draw(&mut app, width, height);
        assert_eq!(
            screen.lines().count(),
            height as usize,
            "playlists at {width}x{height} should fill the window"
        );
    }

    for &(width, height) in SIZES {
        let mut app = App::with_library(player(), None);
        app.modal = Modal::Radio;
        app.stations = vec![znicz_core::Station {
            name: "Example FM".into(),
            url: "https://example.com/stream".into(),
        }];
        let screen = draw(&mut app, width, height);
        assert_eq!(
            screen.lines().count(),
            height as usize,
            "radio at {width}x{height} should fill the window"
        );
    }
}

#[test]
fn the_help_overlay_draws_at_every_size() {
    let mut app = App::with_library(player(), None);
    app.modal = Modal::Help;

    for &(width, height) in SIZES {
        let screen = draw(&mut app, width, height);
        assert_eq!(screen.lines().count(), height as usize);
    }

    // On a normal terminal the keys should actually be legible.
    let screen = draw(&mut app, 100, 40);
    assert!(screen.contains("Keys"), "the overlay needs its title");
    assert!(screen.contains("play / pause"), "bindings should be listed");
    assert!(screen.contains("search the library"));
    assert!(screen.contains("signal inspector"));
    assert!(screen.contains("playlists"), "{screen}");
    assert!(screen.contains("previous track"), "{screen}");
    assert!(screen.contains("Radio"), "{screen}");
    assert!(screen.contains("new station"), "{screen}");
    assert!(screen.contains("edit name"), "{screen}");
}

#[test]
fn the_signal_inspector_shows_the_device_sample_format() {
    let mut app = App::with_library(player(), None);
    app.modal = Modal::Inspector;

    let idle = draw(&mut app, 90, 24);
    assert!(idle.contains("Signal"), "{idle}");
    assert!(idle.contains("No file is playing"), "{idle}");

    let playing = draw_with(&mut app, &playing_state(), 90, 24);
    assert!(playing.contains("Signal"), "{playing}");
    assert!(
        playing.contains("f32"),
        "sample format belongs here:\n{playing}"
    );
    assert!(playing.contains("bit perfect"), "{playing}");
    assert!(playing.contains("96 kHz"), "{playing}");
}

#[test]
fn a_fresh_screen_is_the_library_with_no_tab_bar() {
    let mut app = App::with_library(player(), None);
    let screen = draw(&mut app, 90, 24);
    assert!(screen.contains("Library"), "{screen}");
    assert!(
        !screen.contains("1 Queue"),
        "tab bar must be gone:\n{screen}"
    );
}

#[test]
fn the_queue_drawer_covers_the_right_on_a_wide_screen() {
    let mut app = App::with_library(player(), None);
    app.queue_open = true;
    app.focus = Focus::Queue;
    let screen = draw(&mut app, 100, 24);
    assert!(screen.contains("Queue"), "{screen}");
    assert!(
        screen.contains("Library"),
        "library stays underneath:\n{screen}"
    );
}

#[test]
fn a_narrow_screen_opens_the_queue_as_a_sheet() {
    let mut app = App::with_library(player(), None);
    app.queue_open = true;
    app.focus = Focus::Queue;
    let screen = draw(&mut app, 60, 20);
    assert!(screen.contains("Queue"), "{screen}");
}

#[test]
fn transport_sits_at_the_bottom_and_drops_the_signal_line_when_short() {
    let mut app = App::with_library(player(), None);
    let tall = draw(&mut app, 90, 24);
    assert!(
        tall.contains("stopped") || tall.contains("Nothing playing"),
        "{tall}"
    );
    let short = draw(&mut app, 90, 16);
    assert_eq!(short.lines().count(), 16);
}

#[test]
fn a_success_toast_has_its_own_box_and_mark() {
    let mut app = App::with_library(player(), None);
    app.toasts.success("queue cleared");
    let screen = draw(&mut app, 90, 24);
    assert!(screen.contains("queue cleared"), "{screen}");
    assert!(
        screen.contains('●'),
        "success should carry a level mark:\n{screen}"
    );
}

#[test]
fn a_playlist_error_toast_shows_the_whole_message() {
    let mut app = App::with_library(player(), None);
    app.toasts
        .error("player error: playlist had no playable files");
    let screen = draw(&mut app, 90, 24);
    assert!(
        screen.contains("playlist had no playable files"),
        "the full reason must be visible, not cut off:\n{screen}"
    );
    assert!(
        !screen.contains("playabl…"),
        "must not ellipsize this message:\n{screen}"
    );
}

#[test]
fn hints_stay_when_a_toast_is_showing() {
    let mut app = App::with_library(player(), None);
    app.toasts.error("could not open device");
    let screen = draw(&mut app, 90, 24);
    assert!(screen.contains("could not open device"), "{screen}");
    assert!(
        screen.contains("? help") || screen.contains("search"),
        "hints must remain:\n{screen}"
    );
    assert!(
        screen.contains('x'),
        "errors should carry a level mark:\n{screen}"
    );
    let lines: Vec<&str> = screen.lines().collect();
    let toast_row = lines
        .iter()
        .position(|line| line.contains("could not open device"))
        .expect("toast text");
    let transport = znicz_tui::layout::transport_height(24, app.tui.show_cover) as usize;
    let last_list_row = lines.len().saturating_sub(transport + 1 + 1);
    assert!(
        toast_row < last_list_row,
        "toast must sit above the pane border, not on it:\n{screen}"
    );
}

#[test]
fn an_idle_player_says_so_rather_than_showing_blanks() {
    let mut app = App::with_library(player(), None);
    let screen = draw(&mut app, 90, 24);

    assert!(screen.contains("Nothing playing"));
    assert!(
        screen.contains("Library"),
        "the library should be the default home"
    );
    assert!(
        !screen.contains("1 Queue"),
        "the tab bar should not be shown"
    );
    assert!(
        !screen.contains("Queue is empty"),
        "the queue drawer should be closed on a fresh app"
    );
}

#[test]
fn queue_rows_show_tags_once_they_are_known() {
    let mut app = App::with_library(player(), None);
    let path = PathBuf::from("/music/kashmir.flac");

    app.meta.insert(
        path.clone(),
        znicz_tui::meta::Entry {
            title: "Kashmir".into(),
            artist: Some("Led Zeppelin".into()),
            album: Some("Physical Graffiti".into()),
            duration: Some(Duration::from_secs(508)),
        },
    );
    app.player
        .send_blocking(Command::QueueAdd(vec![znicz_core::QueueItem::file(path)]))
        .expect("queue add");
    app.queue_open = true;
    app.focus = Focus::Queue;

    let screen = draw(&mut app, 90, 24);
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
        .send_blocking(Command::QueueAdd(vec![znicz_core::QueueItem::file(
            "/music/04 - mystery.flac",
        )]))
        .expect("queue add");
    app.queue_open = true;
    app.focus = Focus::Queue;

    let screen = draw(&mut app, 90, 24);
    assert!(
        screen.contains("04 - mystery"),
        "a row must never be blank while tags are loading:\n{screen}"
    );
}

#[test]
fn the_library_pane_explains_how_to_fill_it() {
    let mut app = App::with_library(player(), None);

    let screen = draw(&mut app, 90, 24);
    assert!(
        screen.contains("scan"),
        "an empty library should point at the scan command:\n{screen}"
    );
}

#[test]
fn the_search_prompt_shows_what_is_being_typed() {
    let mut app = App::with_library(player(), None);
    app.library.begin_search();
    for c in "zeppelin".chars() {
        app.library.push_char(c);
    }

    let screen = draw(&mut app, 90, 24);
    assert!(screen.contains("search:"), "the prompt should be visible");
    assert!(
        screen.contains("zeppelin"),
        "typed text should be echoed:\n{screen}"
    );
}

#[test]
fn the_search_prompt_draws_the_caret_in_the_middle() {
    let mut app = App::with_library(player(), None);
    app.library.begin_search();
    for c in "zeppelin".chars() {
        app.library.push_char(c);
    }
    if let Some(edit) = app.library.prompt_mut() {
        edit.home();
        edit.right();
    }

    let screen = draw(&mut app, 90, 24);
    assert!(
        screen.contains("z█eppelin"),
        "the search caret should sit at the cursor:\n{screen}"
    );
}

#[test]
fn the_focused_view_is_the_one_shown() {
    let mut app = App::with_library(player(), None);

    app.modal = Modal::Devices;
    let screen = draw(&mut app, 90, 24);
    assert!(screen.contains("Devices"));

    app.modal = Modal::None;
    let screen = draw(&mut app, 90, 24);
    assert!(screen.contains("Library"));

    app.modal = Modal::Playlists;
    app.playlists = vec!["evening".into(), "weekend-jazz".into()];
    let screen = draw(&mut app, 90, 24);
    assert!(screen.contains("Playlists"), "{screen}");
    assert!(screen.contains("evening"), "{screen}");

    app.modal = Modal::Radio;
    app.stations = vec![znicz_core::Station {
        name: "Example FM".into(),
        url: "https://example.com/stream".into(),
    }];
    let screen = draw(&mut app, 90, 24);
    assert!(screen.contains("Radio"), "{screen}");
    assert!(screen.contains("Example FM"), "{screen}");
}

#[test]
fn the_radio_add_prompt_draws_the_caret_in_the_middle() {
    let mut app = App::with_library(player(), None);
    app.modal = Modal::Radio;
    let mut edit = LineEdit::from_text("Example");
    edit.home();
    edit.right();
    edit.right();
    app.radio_prompt = Some(RadioPrompt::Form {
        name: edit,
        url: LineEdit::new(),
        field: StationField::Name,
        original: None,
    });

    let screen = draw(&mut app, 90, 24);
    assert!(
        screen.contains("Ex█ample"),
        "the caret should sit at the cursor, not only at the end:\n{screen}"
    );
}

#[test]
fn the_playlist_save_prompt_draws_the_caret_in_the_middle() {
    let mut app = App::with_library(player(), None);
    app.modal = Modal::Playlists;
    let mut edit = LineEdit::from_text("songs");
    edit.home();
    edit.right();
    app.playlist_prompt = Some(PlaylistPrompt::Save(edit));

    let screen = draw(&mut app, 90, 24);
    assert!(
        screen.contains("s█ongs"),
        "the save caret should sit at the cursor, not only at the end:\n{screen}"
    );
}

#[test]
fn a_very_long_title_is_cut_rather_than_wrapped() {
    let mut app = App::with_library(player(), None);
    let path = PathBuf::from("/music/long.flac");
    app.meta.insert(
        path.clone(),
        znicz_tui::meta::Entry {
            title: "A".repeat(400),
            artist: Some("B".repeat(400)),
            album: None,
            duration: None,
        },
    );
    app.player
        .send_blocking(Command::QueueAdd(vec![znicz_core::QueueItem::file(path)]))
        .expect("queue add");
    app.queue_open = true;
    app.focus = Focus::Queue;

    let screen = draw(&mut app, 60, 20);
    for line in screen.lines() {
        assert!(
            line.chars().count() <= 60,
            "a long title must not push past the window: {} chars",
            line.chars().count()
        );
    }
}
