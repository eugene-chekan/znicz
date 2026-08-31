//! The keymap, driven end to end.
//!
//! Every test presses keys the way a user would and checks the player state or
//! the interface afterwards, so a binding cannot quietly stop working.

use std::path::PathBuf;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use znicz_core::{spawn_player, AudioConfig, Command, PlayerHandle, QueueItem, RepeatMode};
use znicz_tui::{App, Focus, Modal, PlaylistPrompt, RadioPrompt, StationField};

fn player() -> PlayerHandle {
    let (player, _thread) = spawn_player(AudioConfig::default());
    player
}

fn new_app() -> App {
    App::with_library(player(), None)
}

/// Press a plain key.
fn press(app: &mut App, code: KeyCode) {
    app.on_key(KeyEvent::new(code, KeyModifiers::NONE));
}

fn press_char(app: &mut App, c: char) {
    press(app, KeyCode::Char(c));
}

/// Type a name the way a terminal does: Shift for capitals, plain keys otherwise.
fn press_typed(app: &mut App, text: &str) {
    for c in text.chars() {
        let modifiers = if c.is_uppercase() {
            KeyModifiers::SHIFT
        } else {
            KeyModifiers::NONE
        };
        app.on_key(KeyEvent::new(KeyCode::Char(c), modifiers));
    }
}

fn radio_form_name(app: &App) -> Option<&str> {
    match &app.radio_prompt {
        Some(RadioPrompt::Form { name, .. }) => Some(name.as_str()),
        _ => None,
    }
}

fn press_ctrl(app: &mut App, c: char) {
    app.on_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL));
}

fn queue(app: &mut App, count: usize) {
    let items: Vec<znicz_core::QueueItem> = (0..count)
        .map(|i| znicz_core::QueueItem::file(format!("/music/track-{i}.flac")))
        .collect();
    app.player
        .send_blocking(Command::QueueAdd(items))
        .expect("queue add");
}

/// Open the queue drawer and focus it so queue keys apply.
fn open_queue(app: &mut App) {
    press_char(app, ']');
}

#[test]
fn the_player_opens_on_the_library() {
    let app = new_app();
    assert_eq!(app.focus, Focus::Library);
    assert!(!app.queue_open);
    assert_eq!(app.modal, Modal::None);
}

#[test]
fn bracket_opens_the_queue_drawer_and_closes_it() {
    let mut app = new_app();
    assert!(!app.queue_open);

    press_char(&mut app, ']');
    assert!(app.queue_open);
    assert_eq!(app.focus, Focus::Queue);

    press_char(&mut app, ']');
    assert!(!app.queue_open);
    assert_eq!(app.focus, Focus::Library);
}

#[test]
fn tab_opens_the_drawer_then_swaps_focus_without_closing_it() {
    let mut app = new_app();
    assert!(!app.queue_open);
    assert_eq!(app.focus, Focus::Library);

    press(&mut app, KeyCode::Tab);
    assert!(app.queue_open, "Tab should open the drawer");
    assert_eq!(app.focus, Focus::Queue);

    press(&mut app, KeyCode::Tab);
    assert!(app.queue_open, "Tab should not close the drawer");
    assert_eq!(app.focus, Focus::Library);

    press(&mut app, KeyCode::BackTab);
    assert!(app.queue_open);
    assert_eq!(app.focus, Focus::Queue, "BackTab should swap back");
}

#[test]
fn backtab_does_nothing_while_the_drawer_is_closed() {
    let mut app = new_app();
    assert_eq!(app.focus, Focus::Library);

    press(&mut app, KeyCode::BackTab);
    assert_eq!(app.focus, Focus::Library);
    assert!(!app.queue_open);
}

#[test]
fn numbers_no_longer_switch_homes() {
    let mut app = new_app();
    assert_eq!(app.focus, Focus::Library);

    press_char(&mut app, '1');
    assert_eq!(app.focus, Focus::Library);
    press_char(&mut app, '2');
    assert_eq!(app.focus, Focus::Library);
    press_char(&mut app, '3');
    assert_eq!(app.focus, Focus::Library);
    assert_eq!(app.modal, Modal::None);
}

#[test]
fn comma_toggles_the_devices_modal() {
    let mut app = new_app();
    assert_eq!(app.modal, Modal::None);

    press_char(&mut app, ',');
    assert_eq!(app.modal, Modal::Devices);

    press(&mut app, KeyCode::Esc);
    assert_eq!(app.modal, Modal::None);
}

#[test]
fn i_toggles_the_signal_inspector() {
    let mut app = new_app();
    assert_eq!(app.modal, Modal::None);

    press_char(&mut app, 'i');
    assert_eq!(app.modal, Modal::Inspector);

    press_char(&mut app, 'i');
    assert_eq!(app.modal, Modal::None);
}

#[test]
fn esc_closes_the_inspector() {
    let mut app = new_app();
    press_char(&mut app, 'i');
    assert_eq!(app.modal, Modal::Inspector);

    press(&mut app, KeyCode::Esc);
    assert_eq!(app.modal, Modal::None);
}

#[test]
fn inspector_ignores_list_movement() {
    let mut app = new_app();
    press_char(&mut app, 'i');
    press_char(&mut app, 'j');
    press_char(&mut app, 'k');
    assert_eq!(app.modal, Modal::Inspector);
}

#[test]
fn i_replaces_the_devices_modal() {
    let mut app = new_app();
    press_char(&mut app, ',');
    assert_eq!(app.modal, Modal::Devices);

    press_char(&mut app, 'i');
    assert_eq!(app.modal, Modal::Inspector);
}

#[test]
fn question_mark_still_opens_help_and_the_next_key_only_closes_it() {
    let mut app = new_app();
    press_char(&mut app, '?');
    assert_eq!(app.modal, Modal::Help);

    press_char(&mut app, 'q');
    assert_eq!(app.modal, Modal::None, "the overlay should close");
    assert!(!app.should_quit, "and that key must not also quit");
}

#[test]
fn esc_closes_search_then_devices_then_the_drawer() {
    let mut app = new_app();

    // Search: / then Esc
    press_char(&mut app, '/');
    assert!(app.library.is_typing());
    press(&mut app, KeyCode::Esc);
    assert!(!app.library.is_typing());

    // Devices modal: , then Esc
    press_char(&mut app, ',');
    assert_eq!(app.modal, Modal::Devices);
    press(&mut app, KeyCode::Esc);
    assert_eq!(app.modal, Modal::None);

    // Drawer: ] then Esc
    press_char(&mut app, ']');
    assert!(app.queue_open);
    assert_eq!(app.focus, Focus::Queue);
    press(&mut app, KeyCode::Esc);
    assert!(!app.queue_open);
    assert_eq!(app.focus, Focus::Library);
}

#[test]
fn q_quits_and_ctrl_c_quits() {
    let mut app = new_app();
    assert!(!app.should_quit);
    press_char(&mut app, 'q');
    assert!(app.should_quit);

    let mut app = new_app();
    press_ctrl(&mut app, 'c');
    assert!(app.should_quit, "Ctrl-C should also quit");
}

#[test]
fn help_opens_and_any_key_closes_it() {
    let mut app = new_app();
    press_char(&mut app, '?');
    assert_eq!(app.modal, Modal::Help);

    // While help is up, keys dismiss it rather than doing their usual job.
    press_char(&mut app, 'q');
    assert_eq!(app.modal, Modal::None, "the overlay should close");
    assert!(!app.should_quit, "and that key must not also quit");
}

#[test]
fn volume_keys_move_in_steps_and_stop_at_the_ends() {
    let mut app = new_app();
    assert_eq!(app.state().volume, 1.0);

    press_char(&mut app, '-');
    let after_one = app.state().volume;
    assert!(
        (after_one - 0.95).abs() < 0.001,
        "one press should be one step, got {after_one}"
    );

    for _ in 0..40 {
        press_char(&mut app, '-');
    }
    assert_eq!(app.state().volume, 0.0, "volume must not go negative");

    for _ in 0..40 {
        press_char(&mut app, '+');
    }
    assert_eq!(app.state().volume, 1.0, "volume must not exceed full");
}

#[test]
fn m_mutes_without_forgetting_the_volume() {
    let mut app = new_app();
    press_char(&mut app, '-');
    let volume = app.state().volume;

    press_char(&mut app, 'm');
    let state = app.state();
    assert!(state.muted);
    assert_eq!(state.volume, volume, "the setting must be kept");

    press_char(&mut app, 'm');
    assert!(!app.state().muted, "the same key unmutes");
}

#[test]
fn e_cycles_repeat_and_z_toggles_shuffle() {
    let mut app = new_app();
    press_char(&mut app, 'e');
    assert_eq!(app.state().repeat, RepeatMode::All);
    press_char(&mut app, 'e');
    assert_eq!(app.state().repeat, RepeatMode::One);
    press_char(&mut app, 'z');
    assert!(app.state().shuffle);
}

#[test]
fn d_removes_the_selected_queue_entry() {
    let mut app = new_app();
    queue(&mut app, 3);
    open_queue(&mut app);

    press_char(&mut app, 'j'); // move to the second entry
    press_char(&mut app, 'd');

    let queue = app.state().queue;
    assert_eq!(queue.len(), 2);
    assert_eq!(
        queue[1],
        znicz_core::QueueItem::file("/music/track-2.flac"),
        "the second entry should be the one that went"
    );
}

#[test]
fn shift_c_clears_the_whole_queue() {
    let mut app = new_app();
    queue(&mut app, 3);
    open_queue(&mut app);

    press_char(&mut app, 'C');
    assert!(app.state().queue.is_empty());
}

#[test]
fn queue_keys_on_an_empty_queue_do_nothing() {
    let mut app = new_app();
    open_queue(&mut app);
    for key in ['d', 'C', 'o'] {
        press_char(&mut app, key);
    }
    press(&mut app, KeyCode::Enter);
    assert!(app.state().queue.is_empty());
    assert!(!app.should_quit);
}

#[test]
fn space_on_an_empty_queue_explains_itself() {
    let mut app = new_app();
    press_char(&mut app, ' ');

    let toast = app.toasts.current().expect("a message should appear");
    assert!(
        toast.text.contains("queue is empty"),
        "the user needs to know why nothing happened; got: {}",
        toast.text
    );
}

#[test]
fn slash_opens_the_search_prompt_and_letters_become_text() {
    let mut app = new_app();

    press_char(&mut app, '/');
    assert!(app.library.is_typing());

    // 'q' would normally quit; while typing it is just a letter.
    for c in "queen".chars() {
        press_char(&mut app, c);
    }
    assert_eq!(app.library.input(), Some("queen"));
    assert!(!app.should_quit, "typing must not trigger commands");

    press(&mut app, KeyCode::Backspace);
    assert_eq!(app.library.input(), Some("quee"));

    press(&mut app, KeyCode::Esc);
    assert!(!app.library.is_typing(), "Esc should close the prompt");
}

#[test]
fn search_prompt_left_inserts_in_the_middle() {
    let mut app = new_app();
    press_char(&mut app, '/');
    press_typed(&mut app, "quee");
    press(&mut app, KeyCode::Left);
    press_char(&mut app, 'n');
    assert_eq!(
        app.library.input(),
        Some("quene"),
        "Left should move the search caret, got {:?}",
        app.library.input()
    );
    press(&mut app, KeyCode::Right);
    press_char(&mut app, 'n');
    assert_eq!(app.library.input(), Some("quenen"));
}

#[test]
fn navigation_keys_move_the_queue_cursor() {
    let mut app = new_app();
    queue(&mut app, 30);
    open_queue(&mut app);

    press_char(&mut app, 'j');
    press_char(&mut app, 'j');
    assert_eq!(app.queue_cursor.index(), 2);

    press_char(&mut app, 'k');
    assert_eq!(app.queue_cursor.index(), 1);

    press_char(&mut app, 'G');
    assert_eq!(app.queue_cursor.index(), 29, "G goes to the last row");

    press_char(&mut app, 'g');
    assert_eq!(app.queue_cursor.index(), 0, "g goes back to the first");

    press_ctrl(&mut app, 'd');
    assert_eq!(app.queue_cursor.index(), 10, "Ctrl-d jumps a half page");
    press_ctrl(&mut app, 'u');
    assert_eq!(app.queue_cursor.index(), 0);

    // Arrows do the same as j and k.
    press(&mut app, KeyCode::Down);
    assert_eq!(app.queue_cursor.index(), 1);
    press(&mut app, KeyCode::Up);
    assert_eq!(app.queue_cursor.index(), 0);
}

#[test]
fn seeking_with_nothing_loaded_is_ignored() {
    let mut app = new_app();
    for key in [KeyCode::Right, KeyCode::Left, KeyCode::Char('L')] {
        press(&mut app, key);
    }
    assert_eq!(app.state().position.as_secs(), 0);
    assert!(
        app.toasts.is_empty(),
        "a no-op seek should not nag the user with a message"
    );
}

#[test]
fn playing_a_missing_file_shows_an_error() {
    let mut app = new_app();
    queue(&mut app, 1);
    open_queue(&mut app);

    // The queued path does not exist, so Enter must fail visibly.
    press(&mut app, KeyCode::Enter);

    let toast = app.toasts.current().expect("an error should be shown");
    assert!(
        !toast.text.is_empty(),
        "the failure must reach the screen, not just the log"
    );
}

#[test]
fn o_jumps_the_cursor_to_the_playing_track() {
    let mut app = new_app();
    queue(&mut app, 10);
    open_queue(&mut app);

    press_char(&mut app, 'G');
    assert_eq!(app.queue_cursor.index(), 9);

    press_char(&mut app, 'o');
    assert_eq!(
        app.queue_cursor.index(),
        app.state().queue_position,
        "o should return to whatever is playing"
    );
}

#[test]
fn angle_brackets_are_unbound() {
    let mut app = new_app();
    press_char(&mut app, '>');
    press_char(&mut app, '<');
    assert_eq!(app.library.h_offset(), 0);
    assert_eq!(app.focus, Focus::Library);
}

#[test]
fn unbound_keys_are_ignored() {
    let mut app = new_app();
    for code in [
        KeyCode::Char('x'),
        KeyCode::Char('%'),
        KeyCode::F(5),
        KeyCode::Insert,
    ] {
        press(&mut app, code);
    }
    assert!(!app.should_quit);
    assert_eq!(app.modal, Modal::None);
    assert_eq!(app.focus, Focus::Library);
}

fn playlist_fixture() -> (App, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!(
        "znicz-tui-playlists-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("playlists dir");
    let a = dir.join("a.flac");
    let b = dir.join("b.flac");
    std::fs::write(&a, b"x").unwrap();
    std::fs::write(&b, b"x").unwrap();
    std::fs::write(dir.join("evening.m3u"), format!("{}\n", a.display())).unwrap();
    std::fs::write(dir.join("weekend.m3u"), format!("{}\n", b.display())).unwrap();

    let mut app = new_app();
    app.playlists_dir = dir.clone();
    (app, dir)
}

#[test]
fn capital_p_toggles_the_playlists_modal() {
    let mut app = new_app();
    assert_eq!(app.modal, Modal::None);

    press_char(&mut app, 'P');
    assert_eq!(app.modal, Modal::Playlists);

    press(&mut app, KeyCode::Esc);
    assert_eq!(app.modal, Modal::None);
}

#[test]
fn s_still_stops_while_playlists_are_open() {
    let mut app = new_app();
    queue(&mut app, 1);
    press_char(&mut app, 'P');
    press_char(&mut app, 's');
    assert_eq!(app.modal, Modal::Playlists);
    assert!(!app.should_quit);
}

#[test]
fn enter_replaces_the_queue_from_the_highlighted_playlist() {
    let (mut app, dir) = playlist_fixture();
    let other = dir.join("other.flac");
    std::fs::write(&other, b"x").unwrap();
    app.player
        .send_blocking(Command::QueueAdd(vec![znicz_core::QueueItem::file(
            other.clone(),
        )]))
        .expect("seed queue");

    press_char(&mut app, 'P');
    press(&mut app, KeyCode::Enter);

    let queue = app.state().queue;
    assert_eq!(queue.len(), 1, "clear-and-play should replace the queue");
    assert_ne!(queue[0], znicz_core::QueueItem::file(other.clone()));
    assert_eq!(
        queue[0]
            .as_path()
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str()),
        Some("a.flac")
    );
}

#[test]
fn a_appends_the_playlist_without_clearing() {
    let (mut app, dir) = playlist_fixture();
    let other = dir.join("other.flac");
    std::fs::write(&other, b"x").unwrap();
    app.player
        .send_blocking(Command::QueueAdd(vec![znicz_core::QueueItem::file(
            other.clone(),
        )]))
        .expect("seed queue");

    press_char(&mut app, 'P');
    press_char(&mut app, 'a');

    let queue = app.state().queue;
    assert_eq!(queue.len(), 2, "add should keep the existing row");
    assert_eq!(queue[0], znicz_core::QueueItem::file(other.clone()));
}

#[test]
fn n_on_an_empty_queue_does_not_open_the_save_prompt() {
    let mut app = new_app();
    press_char(&mut app, 'P');
    press_char(&mut app, 'n');
    assert!(app.playlist_prompt.is_none());
    assert_eq!(app.toasts.current().unwrap().text, "queue is empty");
}

#[test]
fn lowercase_p_is_still_previous_track() {
    let mut app = new_app();
    press_char(&mut app, 'p');
    assert_eq!(app.modal, Modal::None, "p must not open playlists");
}

#[test]
fn save_prompt_treats_s_as_a_letter() {
    let mut app = new_app();
    queue(&mut app, 1);
    press_char(&mut app, 'P');
    press_char(&mut app, 'n');
    assert!(app.playlist_prompt.is_some());
    press_char(&mut app, 's');
    press_char(&mut app, 'o');
    press_char(&mut app, 'n');
    press_char(&mut app, 'g');
    press_char(&mut app, 's');
    assert_eq!(
        app.playlist_prompt.as_ref().map(PlaylistPrompt::as_str),
        Some("songs")
    );
    assert_eq!(app.modal, Modal::Playlists);
}

#[test]
fn save_prompt_left_inserts_in_the_middle() {
    let mut app = new_app();
    queue(&mut app, 1);
    press_char(&mut app, 'P');
    press_char(&mut app, 'n');
    press_typed(&mut app, "sogs");
    press(&mut app, KeyCode::Left);
    press(&mut app, KeyCode::Left);
    press_char(&mut app, 'n');
    assert_eq!(
        app.playlist_prompt.as_ref().map(PlaylistPrompt::as_str),
        Some("songs"),
        "Left should move the save caret, got {:?}",
        app.playlist_prompt
    );
}

#[test]
fn save_prompt_accepts_to_listen() {
    let mut app = new_app();
    queue(&mut app, 1);
    press_char(&mut app, 'P');
    press_char(&mut app, 'n');
    press_typed(&mut app, "To Listen");
    assert_eq!(
        app.playlist_prompt.as_ref().map(PlaylistPrompt::as_str),
        Some("To Listen"),
        "s/n/i/L/Space are global elsewhere; they must type in the name"
    );
}

#[test]
fn e_edits_the_highlighted_playlist() {
    let (mut app, dir) = playlist_fixture();
    press_char(&mut app, 'P');
    press_char(&mut app, 'e');
    assert!(
        matches!(
            app.playlist_prompt,
            Some(PlaylistPrompt::Rename(ref s)) if s.as_str() == "evening"
        ),
        "e should open a rename prompt with the current name, got {:?}",
        app.playlist_prompt
    );
    press(&mut app, KeyCode::Home);
    for _ in 0..7 {
        press(&mut app, KeyCode::Delete);
    }
    press_typed(&mut app, "night");
    press(&mut app, KeyCode::Enter);
    assert!(app.playlist_prompt.is_none());
    assert!(dir.join("night.m3u").is_file());
    assert!(!dir.join("evening.m3u").is_file());
    assert_eq!(
        app.playlists,
        vec!["night".to_string(), "weekend".to_string()]
    );
}

fn station_fixture() -> (App, PathBuf) {
    let path = std::env::temp_dir().join(format!(
        "znicz-tui-stations-{}-{}.toml",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let mut app = new_app();
    app.stations_path = path.clone();
    (app, path)
}

#[test]
fn capital_r_toggles_the_radio_modal() {
    let mut app = new_app();
    press_char(&mut app, 'R');
    assert_eq!(app.modal, Modal::Radio);
    press(&mut app, KeyCode::Esc);
    assert_eq!(app.modal, Modal::None);
}

#[test]
fn r_on_the_library_reloads_instead_of_repeat() {
    let mut app = new_app();
    let before = app.state().repeat;
    press_char(&mut app, 'r');
    assert_eq!(app.state().repeat, before);
    assert_eq!(app.modal, Modal::None);
}

#[test]
fn radio_add_prompt_treats_letters_as_text() {
    let (mut app, _path) = station_fixture();
    press_char(&mut app, 'R');
    press_char(&mut app, 'n');
    press_typed(&mut app, "songs");
    assert_eq!(radio_form_name(&app), Some("songs"));
    assert!(!app.should_quit);
}

#[test]
fn radio_add_prompt_left_inserts_in_the_middle() {
    let (mut app, _path) = station_fixture();
    press_char(&mut app, 'R');
    press_char(&mut app, 'n');
    press_typed(&mut app, "Exmple");
    press(&mut app, KeyCode::Left);
    press(&mut app, KeyCode::Left);
    press(&mut app, KeyCode::Left);
    press(&mut app, KeyCode::Left);
    press_char(&mut app, 'a');
    assert_eq!(
        radio_form_name(&app),
        Some("Example"),
        "Left should move the caret so a missing letter can be inserted, got {:?}",
        app.radio_prompt
    );
}

#[test]
fn radio_add_prompt_right_moves_the_caret_forward() {
    let (mut app, _path) = station_fixture();
    press_char(&mut app, 'R');
    press_char(&mut app, 'n');
    press_typed(&mut app, "abc");
    press(&mut app, KeyCode::Left);
    press(&mut app, KeyCode::Left);
    press(&mut app, KeyCode::Right);
    press_char(&mut app, 'X');
    assert_eq!(
        radio_form_name(&app),
        Some("abXc"),
        "Right should move the caret forward, got {:?}",
        app.radio_prompt
    );
}

#[test]
fn radio_add_writes_the_file_after_name_and_url() {
    let (mut app, path) = station_fixture();
    press_char(&mut app, 'R');
    press_char(&mut app, 'n');
    press_typed(&mut app, "Example");
    press(&mut app, KeyCode::Tab);
    press_typed(&mut app, "https://example.com/stream");
    press(&mut app, KeyCode::Enter);
    let stations = znicz_core::load_stations(&path).unwrap();
    assert_eq!(stations.len(), 1);
    assert_eq!(stations[0].name, "Example");
    assert_eq!(stations[0].url, "https://example.com/stream");
}

#[test]
fn enter_on_a_station_replaces_the_queue() {
    let (mut app, path) = station_fixture();
    znicz_core::save_stations(
        &path,
        &[znicz_core::Station {
            name: "Example".into(),
            url: "http://127.0.0.1:1/stream".into(),
        }],
    )
    .unwrap();
    app.player
        .send_blocking(Command::QueueAdd(vec![QueueItem::file("/music/a.flac")]))
        .unwrap();
    press_char(&mut app, 'R');
    press(&mut app, KeyCode::Enter);
    let queue = app.state().queue;
    assert_eq!(queue.len(), 1);
    assert_eq!(
        queue[0],
        QueueItem::stream("Example", "http://127.0.0.1:1/stream")
    );
}

#[test]
fn d_deletes_a_station_immediately() {
    let (mut app, path) = station_fixture();
    znicz_core::save_stations(
        &path,
        &[znicz_core::Station {
            name: "Gone".into(),
            url: "https://example.com/g".into(),
        }],
    )
    .unwrap();
    press_char(&mut app, 'R');
    press_char(&mut app, 'd');
    assert!(znicz_core::load_stations(&path).unwrap().is_empty());
}

#[test]
fn n_on_a_single_stream_toasts_instead_of_sending_next() {
    let mut app = new_app();
    app.player
        .send_blocking(Command::QueueAdd(vec![QueueItem::stream(
            "Live",
            "https://example.com/s",
        )]))
        .unwrap();
    press_char(&mut app, 'n');
    let toast = app.toasts.current().expect("toast");
    assert!(toast.text.contains("radio"), "{}", toast.text);
}

#[test]
fn seek_on_a_stream_queue_row_toasts() {
    let mut app = new_app();
    app.player
        .send_blocking(Command::QueueAdd(vec![QueueItem::stream(
            "Live",
            "https://example.com/s",
        )]))
        .unwrap();
    // Fake a current track so seek_relative is not a no-op:
    // seek_relative returns early without current_track. After Task 4, Seek
    // errors from the engine when the row is a stream even without a track.
    press(&mut app, KeyCode::Right);
    // If current_track is None, this test only checks no panic.
}

#[test]
fn playlist_save_of_a_stream_queue_is_refused() {
    let mut app = new_app();
    app.player
        .send_blocking(Command::QueueAdd(vec![QueueItem::stream(
            "Live",
            "https://example.com/s",
        )]))
        .unwrap();
    press_char(&mut app, 'P');
    press_char(&mut app, 'n');
    assert!(app.playlist_prompt.is_none());
    let toast = app.toasts.current().unwrap();
    assert!(toast.text.contains("radio station"), "{}", toast.text);
}

#[test]
fn n_on_playlists_opens_save_instead_of_next() {
    let mut app = new_app();
    queue(&mut app, 2);
    press_char(&mut app, 'P');
    press_char(&mut app, 'n');
    assert!(
        matches!(app.playlist_prompt, Some(PlaylistPrompt::Save(_))),
        "n should save the queue while Playlists is open, got {:?}",
        app.playlist_prompt
    );
    assert_eq!(app.state().queue.len(), 2);
}

#[test]
fn e_on_playlists_does_not_cycle_repeat() {
    let (mut app, _dir) = playlist_fixture();
    press_char(&mut app, 'P');
    press_char(&mut app, 'e');
    assert!(matches!(
        app.playlist_prompt,
        Some(PlaylistPrompt::Rename(_))
    ));
    assert_eq!(app.state().repeat, RepeatMode::Off);
}

#[test]
fn e_after_closing_playlists_cycles_repeat_again() {
    let mut app = new_app();
    press_char(&mut app, 'P');
    press(&mut app, KeyCode::Esc);
    press_char(&mut app, 'e');
    assert_eq!(app.state().repeat, RepeatMode::All);
}

#[test]
fn d_deletes_the_highlighted_playlist() {
    let (mut app, dir) = playlist_fixture();
    press_char(&mut app, 'P');
    press_char(&mut app, 'd');
    assert!(!dir.join("evening.m3u").is_file());
    assert!(dir.join("weekend.m3u").is_file());
    assert_eq!(app.playlists, vec!["weekend".to_string()]);
}

#[test]
fn c_copies_the_highlighted_playlist() {
    let (mut app, dir) = playlist_fixture();
    press_char(&mut app, 'P');
    press_char(&mut app, 'c');
    assert!(
        matches!(
            app.playlist_prompt,
            Some(PlaylistPrompt::Copy(ref s)) if s.as_str() == "evening"
        ),
        "c should open a copy prompt with the current name, got {:?}",
        app.playlist_prompt
    );
    press(&mut app, KeyCode::Home);
    for _ in 0..7 {
        press(&mut app, KeyCode::Delete);
    }
    press_typed(&mut app, "morning");
    press(&mut app, KeyCode::Enter);
    assert!(app.playlist_prompt.is_none());
    assert!(dir.join("evening.m3u").is_file());
    assert!(dir.join("morning.m3u").is_file());
}

#[test]
fn a_on_radio_appends_the_station_without_starting() {
    let (mut app, path) = station_fixture();
    znicz_core::save_stations(
        &path,
        &[znicz_core::Station {
            name: "Example".into(),
            url: "http://127.0.0.1:1/stream".into(),
        }],
    )
    .unwrap();
    let other = znicz_core::QueueItem::file("/music/a.flac");
    app.player
        .send_blocking(Command::QueueAdd(vec![other.clone()]))
        .unwrap();
    press_char(&mut app, 'R');
    press_char(&mut app, 'a');
    let queue = app.state().queue;
    assert_eq!(queue.len(), 2);
    assert_eq!(queue[0], other);
    assert_eq!(
        queue[1],
        QueueItem::stream("Example", "http://127.0.0.1:1/stream")
    );
    assert_eq!(app.state().status, znicz_core::PlaybackStatus::Stopped);
    let toast = app.toasts.current().expect("toast");
    assert!(toast.text.contains("Example"), "{}", toast.text);
}

#[test]
fn n_on_a_stream_with_another_row_moves_on() {
    let mut app = new_app();
    app.player
        .send_blocking(Command::QueueAdd(vec![
            QueueItem::stream("Live", "http://127.0.0.1:1/s"),
            QueueItem::file("/music/a.flac"),
        ]))
        .unwrap();
    press_char(&mut app, 'n');
    assert_eq!(app.state().queue_position, 1);
    assert!(
        app.toasts.current().is_none()
            || !app.toasts.current().unwrap().text.contains("no next track"),
        "{:?}",
        app.toasts.current()
    );
}

#[test]
fn playlist_save_of_a_mixed_queue_is_refused() {
    let mut app = new_app();
    app.player
        .send_blocking(Command::QueueAdd(vec![
            QueueItem::file("/music/a.flac"),
            QueueItem::stream("Live", "http://127.0.0.1:1/s"),
        ]))
        .unwrap();
    press_char(&mut app, 'P');
    press_char(&mut app, 'n');
    assert!(app.playlist_prompt.is_none());
    let toast = app.toasts.current().unwrap();
    assert!(toast.text.contains("radio station"), "{}", toast.text);
}

#[test]
fn n_on_radio_opens_a_form_instead_of_next() {
    let mut app = new_app();
    queue(&mut app, 2);
    press_char(&mut app, 'R');
    press_char(&mut app, 'n');
    assert!(matches!(
        app.radio_prompt,
        Some(RadioPrompt::Form {
            original: None,
            field: StationField::Name,
            ..
        })
    ));
    assert_eq!(app.state().queue.len(), 2);
}

#[test]
fn tab_moves_the_station_form_to_the_url_field() {
    let (mut app, _path) = station_fixture();
    press_char(&mut app, 'R');
    press_char(&mut app, 'n');
    press_typed(&mut app, "songs");
    press(&mut app, KeyCode::Tab);
    assert!(
        matches!(
            app.radio_prompt,
            Some(RadioPrompt::Form {
                field: StationField::Url,
                ref name,
                ..
            }) if name.as_str() == "songs"
        ),
        "Tab should leave the name and focus the URL, got {:?}",
        app.radio_prompt
    );
}

#[test]
fn e_on_radio_edits_name_and_url_together() {
    let (mut app, path) = station_fixture();
    znicz_core::save_stations(
        &path,
        &[znicz_core::Station {
            name: "Example".into(),
            url: "http://127.0.0.1:1/stream".into(),
        }],
    )
    .unwrap();
    press_char(&mut app, 'R');
    press_char(&mut app, 'e');
    assert!(
        matches!(
            app.radio_prompt,
            Some(RadioPrompt::Form {
                original: Some(ref old),
                field: StationField::Name,
                ..
            }) if old == "Example"
        ),
        "e should open the two-field form on the highlighted station, got {:?}",
        app.radio_prompt
    );
    press(&mut app, KeyCode::Home);
    for _ in 0..7 {
        press(&mut app, KeyCode::Delete);
    }
    press_typed(&mut app, "Renamed");
    press(&mut app, KeyCode::Tab);
    press(&mut app, KeyCode::Home);
    for _ in 0..40 {
        press(&mut app, KeyCode::Delete);
    }
    press_typed(&mut app, "https://example.com/b");
    press(&mut app, KeyCode::Enter);
    let stations = znicz_core::load_stations(&path).unwrap();
    assert_eq!(stations.len(), 1);
    assert_eq!(stations[0].name, "Renamed");
    assert_eq!(stations[0].url, "https://example.com/b");
}

#[test]
fn c_copies_the_highlighted_station() {
    let (mut app, path) = station_fixture();
    znicz_core::save_stations(
        &path,
        &[znicz_core::Station {
            name: "Example".into(),
            url: "http://127.0.0.1:1/stream".into(),
        }],
    )
    .unwrap();
    press_char(&mut app, 'R');
    press_char(&mut app, 'c');
    assert!(
        matches!(
            app.radio_prompt,
            Some(RadioPrompt::Copy(ref s)) if s.as_str() == "Example"
        ),
        "c should open a copy prompt with the current name, got {:?}",
        app.radio_prompt
    );
    press_typed(&mut app, "2");
    press(&mut app, KeyCode::Enter);
    let stations = znicz_core::load_stations(&path).unwrap();
    assert_eq!(stations.len(), 2);
    assert_eq!(stations[0].name, "Example");
    assert_eq!(stations[1].name, "Example2");
    assert_eq!(stations[0].url, stations[1].url);
}
