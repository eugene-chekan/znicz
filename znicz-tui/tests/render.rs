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
use znicz_tui::help_state::HelpState;
use znicz_tui::keys::Context;
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
            icy_stream_url: None,
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
    let mut painted = 0u16;
    for y in y0..y0 + 8 {
        for x in 0..cover_w {
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
fn show_cover_false_uses_three_text_rows() {
    let mut app = App::with_library(player(), None);
    app.tui.show_cover = false;
    let screen = draw(&mut app, 90, 24);
    assert!(screen.contains("Nothing playing"), "{screen}");
    assert_eq!(app.list_height, 20);
}

#[test]
fn a_short_window_keeps_the_list_instead_of_the_cover() {
    let mut app = App::with_library(player(), None);
    app.tui.show_cover = true;
    draw(&mut app, 48, 14);
    assert_eq!(app.list_height, 11, "14 rows - 2 transport - 1 footer");
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
            art: None,
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

    let screen = draw(&mut app, 100, 40);
    assert!(screen.contains("Keys · Library"), "{screen}");
    assert!(screen.contains("search the library"), "{screen}");
    assert!(screen.contains("play / pause"), "{screen}");
    assert!(screen.contains("seek 30s"), "{screen}");
}

#[test]
fn the_help_overlay_shows_the_app_version_and_close_hint() {
    let mut app = App::with_library(player(), None);
    app.modal = Modal::Help;
    let screen = draw(&mut app, 120, 40);
    assert!(
        screen.contains(&format!("znicz {}", env!("CARGO_PKG_VERSION"))),
        "version missing from help:\n{screen}"
    );
    assert!(
        screen.contains("/ filter · Esc close"),
        "close hint missing"
    );
    assert!(!screen.contains("any key closes"), "{screen}");
}

#[test]
fn help_opened_from_the_queue_lists_queue_keys_first() {
    let mut app = App::with_library(player(), None);
    app.help = HelpState::open(Context::Queue);
    app.modal = Modal::Help;
    let screen = draw(&mut app, 100, 40);
    assert!(screen.contains("Keys · Queue"), "{screen}");
    let queue_row = screen.find("play the selected track").expect("queue keys");
    let playback_row = screen.find("play / pause").expect("playback keys");
    assert!(queue_row < playback_row, "queue keys come first:\n{screen}");
}

#[test]
fn a_help_filter_shows_only_matching_rows() {
    let mut app = App::with_library(player(), None);
    app.modal = Modal::Help;
    app.help.query = LineEdit::from_text("station");
    let screen = draw(&mut app, 100, 40);
    assert!(screen.contains("new station"), "{screen}");
    assert!(
        screen.contains("/ station"),
        "the kept filter shows:\n{screen}"
    );
    assert!(!screen.contains("play / pause"), "{screen}");
}

#[test]
fn a_help_filter_with_no_match_says_so() {
    let mut app = App::with_library(player(), None);
    app.modal = Modal::Help;
    app.help.query = LineEdit::from_text("zzzz");
    let screen = draw(&mut app, 100, 40);
    assert!(screen.contains("no keys match"), "{screen}");
}

#[test]
fn the_footer_inside_help_offers_filter_and_close_only() {
    let mut app = App::with_library(player(), None);
    app.modal = Modal::Help;
    let screen = draw(&mut app, 100, 24);
    let row = screen.lines().last().expect("footer row");
    assert!(row.contains("/ filter · Esc close"), "{row}");
    assert!(!row.contains("? more"), "{row}");
}

#[test]
fn the_stage_signal_panel_shows_the_device_sample_format() {
    let mut app = App::with_library(player(), None);
    app.view = znicz_tui::View::Stage;

    let playing = draw_with(&mut app, &playing_state(), 90, 24);
    assert!(playing.contains("Signal"), "{playing}");
    assert!(
        playing.contains("f32"),
        "sample format belongs here:\n{playing}"
    );
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
fn a_success_message_has_its_mark() {
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
fn a_message_sits_in_the_transport_not_over_the_list() {
    let mut app = App::with_library(player(), None);
    app.toasts.error("could not open device");
    let screen = draw(&mut app, 90, 24);
    let lines: Vec<&str> = screen.lines().collect();
    let row = lines
        .iter()
        .position(|line| line.contains("x could not open device"))
        .expect("message row");
    assert!(
        row >= usize::from(app.list_height) && row < lines.len() - 1,
        "message must be in the transport, row {row}:\n{screen}"
    );
    assert!(lines[lines.len() - 1].contains("search"), "hints remain");
}

#[test]
fn unread_alerts_show_once_the_message_expires() {
    let mut app = App::with_library(player(), None);
    app.toasts.warn("stream dropped");
    app.toasts.expire_live_for_test();
    let screen = draw(&mut app, 90, 24);
    assert!(screen.contains("▲ 1 unread · M"), "{screen}");
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
        art: None,
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
        art: LineEdit::new(),
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

fn footer(screen: &str) -> &str {
    screen.lines().last().expect("a footer row")
}

#[test]
fn the_footer_keeps_more_and_drops_low_priority_hints_when_narrow() {
    let mut app = App::with_library(player(), None);

    let wide = draw(&mut app, 80, 24);
    let row = footer(&wide);
    assert!(row.starts_with("Space pause · / search"), "{row}");
    assert!(row.contains("R radio"), "{row}");
    assert!(row.trim_end().ends_with("? more"), "{row}");

    let narrow = draw(&mut app, 40, 12);
    let row = footer(&narrow);
    assert!(row.starts_with("Space pause"), "{row}");
    assert!(!row.contains("R radio"), "{row}");
    assert!(row.trim_end().ends_with("? more"), "{row}");
}

#[test]
fn the_tree_footer_offers_expand_instead_of_pause() {
    let mut app = App::with_library(player(), None);
    app.library
        .set_preferred_layout(znicz_tui::LibraryLayout::Tree);
    let screen = draw(&mut app, 100, 24);
    let row = footer(&screen);
    assert!(row.contains("o expand"), "{row}");
    assert!(!row.contains("Space pause"), "{row}");
}

#[test]
fn the_queue_footer_lists_queue_keys() {
    let mut app = App::with_library(player(), None);
    app.queue_open = true;
    app.focus = Focus::Queue;
    let screen = draw(&mut app, 100, 24);
    let row = footer(&screen);
    assert!(row.contains("d remove"), "{row}");
    assert!(row.contains("] close"), "{row}");
}

#[test]
fn a_typing_prompt_footer_says_save_and_cancel() {
    let mut app = App::with_library(player(), None);
    app.modal = Modal::Playlists;
    app.playlist_prompt = Some(PlaylistPrompt::Save(LineEdit::from_text("x")));
    let screen = draw(&mut app, 100, 24);
    let row = footer(&screen);
    assert!(row.contains("Enter save · Esc cancel"), "{row}");
}

#[test]
fn a_prompt_footer_has_no_more_hint() {
    let mut app = App::with_library(player(), None);
    app.modal = Modal::Playlists;
    app.playlist_prompt = Some(PlaylistPrompt::Save(LineEdit::from_text("x")));
    let screen = draw(&mut app, 100, 24);
    let row = footer(&screen);
    assert!(!row.contains("? more"), "{row}");
}

#[test]
fn the_footer_while_typing_a_help_filter_offers_keep_and_clear() {
    let mut app = App::with_library(player(), None);
    app.modal = Modal::Help;
    app.help.editing = true;
    let screen = draw(&mut app, 100, 24);
    let row = footer(&screen);
    assert!(row.contains("Enter keep · Esc clear"), "{row}");
    assert!(!row.contains("Esc close"), "{row}");
}

/// Rows from the top of the transport to the bottom of the screen.
fn below_list(screen: &str, list_height: u16) -> Vec<String> {
    screen
        .lines()
        .skip(usize::from(list_height))
        .map(str::to_string)
        .collect()
}

#[test]
fn no_modal_covers_the_transport_or_footer() {
    let modals = [
        Modal::Help,
        Modal::Devices,
        Modal::Playlists,
        Modal::Radio,
        Modal::Messages,
    ];
    for &(width, height) in &[(80u16, 24u16), (120, 40), (60, 18), (40, 12)] {
        let mut app = App::with_library(player(), None);
        let plain = draw(&mut app, width, height);
        let expected = below_list(&plain, app.list_height);
        for modal in modals {
            app.modal = modal;
            let screen = draw(&mut app, width, height);
            let rows = below_list(&screen, app.list_height);
            // The footer row legitimately changes hints per modal; compare the
            // transport rows only.
            let transport = rows.len() - 1;
            assert_eq!(
                rows[..transport],
                expected[..transport],
                "{modal:?} at {width}x{height} covered the transport:\n{screen}"
            );
        }
    }
}

#[test]
fn the_messages_list_shows_history_newest_first() {
    let mut app = App::with_library(player(), None);
    app.toasts.info("added 4 tracks");
    app.toasts.error("device refused 96 kHz");
    app.modal = Modal::Messages;
    let screen = draw(&mut app, 100, 30);
    assert!(screen.contains("Messages"), "{screen}");
    let error = screen.find("device refused 96 kHz").expect("error row");
    let info = screen.find("added 4 tracks").expect("info row");
    assert!(error < info, "newest first:\n{screen}");
    assert!(screen.contains(" 0s"), "age column:\n{screen}");
}

#[test]
fn an_empty_messages_list_says_so() {
    let mut app = App::with_library(player(), None);
    app.modal = Modal::Messages;
    let screen = draw(&mut app, 100, 30);
    assert!(screen.contains("No messages yet"), "{screen}");
}

#[test]
fn the_footer_counts_unread_alerts() {
    let mut app = App::with_library(player(), None);
    app.toasts.warn("stream dropped");
    app.toasts.error("device refused");
    let screen = draw(&mut app, 120, 24);
    let row = screen.lines().last().unwrap();
    assert!(row.contains("M messages (2)"), "{row}");
}

#[test]
fn the_help_border_says_keep_and_clear_while_filtering() {
    let mut app = App::with_library(player(), None);
    app.modal = Modal::Help;
    app.help.editing = true;
    let screen = draw(&mut app, 100, 40);
    let border = screen
        .lines()
        .find(|line| line.contains("znicz "))
        .expect("help bottom border");
    assert!(border.contains("Enter keep · Esc clear"), "{border}");
}

fn library_app() -> App {
    let mut library = znicz_library::Library::open_in_memory().expect("open library");
    for (path, title, artist, album) in [
        ("/music/a1.flac", "So What", "Miles", "Kind of Blue"),
        (
            "/music/a2.flac",
            "Freddie Freeloader",
            "Miles",
            "Kind of Blue",
        ),
        (
            "/music/sour-times.flac",
            "Sour Times",
            "Portishead",
            "Dummy",
        ),
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
    App::with_library(player(), Some(library))
}

fn buffer(app: &mut App, width: u16, height: u16) -> ratatui::buffer::Buffer {
    let state = app.state();
    let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("terminal");
    terminal
        .draw(|frame| views::render(frame, app, &state))
        .expect("draw");
    terminal.backend().buffer().clone()
}

#[test]
fn the_focused_selection_is_accent_and_others_stay_grey() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use ratatui::style::Color;

    let mut app = library_app();
    // First artist row: inside the Artists column border, row 1.
    let artists = buffer(&mut app, 120, 30);
    assert_eq!(artists[(2, 1)].bg, Color::Cyan, "focused column");

    app.on_key(KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE));
    let albums = buffer(&mut app, 120, 30);
    assert_eq!(
        albums[(2, 1)].bg,
        Color::DarkGray,
        "artist stays visible, grey"
    );
}

#[test]
fn the_library_behind_help_keeps_a_grey_selection() {
    use ratatui::style::Color;

    let mut app = library_app();
    app.modal = Modal::Help;
    // Help is at most 64 wide and centred, so column 2 of the first row is clear of it.
    let screen = buffer(&mut app, 140, 30);
    assert_eq!(screen[(2, 1)].bg, Color::DarkGray, "behind a modal: grey");
}

#[test]
fn column_titles_name_their_parent() {
    let mut app = library_app();
    let screen = draw(&mut app, 120, 30);
    assert!(screen.contains("Albums · Miles"), "{screen}");
    assert!(screen.contains("Tracks · Kind of Blue"), "{screen}");
}

#[test]
fn the_tree_title_follows_the_cursor() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    let mut app = library_app();
    app.tui.library_layout = znicz_tui::LibraryLayout::Tree;
    app.on_key(KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE)); // expand Miles
    app.on_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE)); // album row
    let screen = draw(&mut app, 120, 30);
    assert!(
        screen.contains("Library › Miles › Kind of Blue"),
        "{screen}"
    );
}

#[test]
fn paging_titles_show_the_path() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    let mut app = library_app();
    draw(&mut app, 50, 30); // narrow: single-column paging
    app.on_key(KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE));
    let screen = draw(&mut app, 50, 30);
    assert!(screen.contains("Library › Miles"), "{screen}");
}

#[test]
fn the_library_marks_the_playing_track_album_and_artist() {
    let mut app = library_app();
    let screen = draw_with(&mut app, &playing_state(), 140, 30);
    assert!(screen.contains("Portishead ♪"), "{screen}");
    // Select Portishead so its albums and tracks show.
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    app.on_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE));
    let screen = draw_with(&mut app, &playing_state(), 140, 30);
    assert!(screen.contains("Dummy ♪"), "{screen}");
    assert!(
        screen
            .lines()
            .any(|l| l.contains("Sour Times — Portishead") && l.contains('▶')),
        "{screen}"
    );
    assert!(!screen.contains("Miles ♪"), "{screen}");
}
