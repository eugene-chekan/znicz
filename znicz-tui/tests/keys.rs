//! The keymap, driven end to end.
//!
//! Every test presses keys the way a user would and checks the player state or
//! the interface afterwards, so a binding cannot quietly stop working.

use std::path::PathBuf;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use znicz_core::{AudioConfig, Command, PlayerHandle, RepeatMode, spawn_player};
use znicz_tui::App;
use znicz_tui::app::Pane;

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

fn press_ctrl(app: &mut App, c: char) {
    app.on_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL));
}

fn queue(app: &mut App, count: usize) {
    let paths: Vec<PathBuf> = (0..count)
        .map(|i| PathBuf::from(format!("/music/track-{i}.flac")))
        .collect();
    app.player
        .send_blocking(Command::QueueAdd(paths))
        .expect("queue add");
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
    assert!(app.show_help);

    // While help is up, keys dismiss it rather than doing their usual job.
    press_char(&mut app, 'q');
    assert!(!app.show_help, "the overlay should close");
    assert!(!app.should_quit, "and that key must not also quit");
}

#[test]
fn numbers_and_tab_both_switch_panes() {
    let mut app = new_app();
    assert_eq!(app.pane, Pane::Queue);

    press_char(&mut app, '2');
    assert_eq!(app.pane, Pane::Library);
    press_char(&mut app, '3');
    assert_eq!(app.pane, Pane::Devices);
    press_char(&mut app, '1');
    assert_eq!(app.pane, Pane::Queue);

    press(&mut app, KeyCode::Tab);
    assert_eq!(app.pane, Pane::Library);
    press(&mut app, KeyCode::BackTab);
    assert_eq!(app.pane, Pane::Queue, "Shift-Tab should go back");
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
fn r_cycles_repeat_and_z_toggles_shuffle() {
    let mut app = new_app();
    assert_eq!(app.state().repeat, RepeatMode::Off);

    press_char(&mut app, 'r');
    assert_eq!(app.state().repeat, RepeatMode::All);
    press_char(&mut app, 'r');
    assert_eq!(app.state().repeat, RepeatMode::One);
    press_char(&mut app, 'r');
    assert_eq!(
        app.state().repeat,
        RepeatMode::Off,
        "three presses come back"
    );

    assert!(!app.state().shuffle);
    press_char(&mut app, 'z');
    assert!(app.state().shuffle);
    press_char(&mut app, 'z');
    assert!(!app.state().shuffle);
}

#[test]
fn d_removes_the_selected_queue_entry() {
    let mut app = new_app();
    queue(&mut app, 3);

    press_char(&mut app, 'j'); // move to the second entry
    press_char(&mut app, 'd');

    let queue = app.state().queue;
    assert_eq!(queue.len(), 2);
    assert_eq!(
        queue[1],
        PathBuf::from("/music/track-2.flac"),
        "the second entry should be the one that went"
    );
}

#[test]
fn shift_c_clears_the_whole_queue() {
    let mut app = new_app();
    queue(&mut app, 3);

    press_char(&mut app, 'C');
    assert!(app.state().queue.is_empty());
}

#[test]
fn queue_keys_on_an_empty_queue_do_nothing() {
    let mut app = new_app();
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
    app.pane = Pane::Library;

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
fn navigation_keys_move_the_queue_cursor() {
    let mut app = new_app();
    queue(&mut app, 30);

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
    assert!(!app.show_help);
    assert_eq!(app.pane, Pane::Queue);
}
