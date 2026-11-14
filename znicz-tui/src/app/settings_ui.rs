//! The Settings modal: its keys, applying and saving each row, and the default
//! output device and folder actions it starts. Lives beside `app.rs` as a second
//! `impl App`.

use std::path::PathBuf;

use crossterm::event::{KeyCode, KeyEvent};
use ratatui_image::picker::ProtocolType;
use znicz_core::{Command, LibraryOp};

use super::{App, DevicesMode, Modal};
use crate::settings::{self, Kind, Row};
use crate::tui_config::{CoverProtocol, TuiConfig};

/// How a key changes a Settings row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Change {
    /// Enter or Space: flip a switch, or step a choice forward and wrap.
    Activate,
    /// `l` or `→`: switch on, or step a choice forward and stop at the end.
    Forward,
    /// `h` or `←`: switch off, or step a choice back and stop at the start.
    Back,
}

/// A value as it is written to the config file.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Stored {
    Bool(bool),
    Int(i64),
    Str(&'static str),
    Text(String),
    /// Drop the key: the System default device.
    Remove,
}

/// What a Settings change can touch, kept so a failed save can undo it.
struct SettingsSnapshot {
    tui: TuiConfig,
    bit_perfect: bool,
    update_check: bool,
    default_device: Option<String>,
    protocol: Option<ProtocolType>,
}

/// The picker's protocol for a configured cover protocol, given the one the
/// start-up terminal query picked (used for `auto`).
fn picker_protocol(wanted: CoverProtocol, queried: ProtocolType) -> ProtocolType {
    match wanted {
        CoverProtocol::Auto => queried,
        CoverProtocol::Kitty => ProtocolType::Kitty,
        CoverProtocol::Sixel => ProtocolType::Sixel,
        CoverProtocol::Halfblocks | CoverProtocol::Off => ProtocolType::Halfblocks,
    }
}

impl App {
    /// `S`: open Settings with the cursor on the first row, or close it.
    pub(super) fn toggle_settings_modal(&mut self) {
        if self.modal == Modal::Settings {
            self.close_modal();
            return;
        }
        self.leave_messages();
        self.settings_rows = settings::rows(&self.library_status.folders);
        let first = settings::first_selectable(&self.settings_rows);
        self.set_settings_cursor(first);
        self.settings_scroll = 0;
        self.modal = Modal::Settings;
    }

    /// Move the Settings cursor `delta` selectable rows, stopping at either end.
    pub(super) fn settings_move(&mut self, delta: isize) {
        for _ in 0..delta.unsigned_abs() {
            let next =
                settings::next_selectable(&self.settings_rows, self.settings_cursor, delta > 0);
            if next == self.settings_cursor {
                break;
            }
            self.set_settings_cursor(next);
        }
    }

    /// Put the Settings cursor on `index`. A pending removal question was asked
    /// about the row the cursor was on, so it is dropped here.
    pub(super) fn set_settings_cursor(&mut self, index: usize) {
        self.settings_confirm = None;
        self.settings_cursor = index;
    }

    /// Keys that change the highlighted row. Returns false for the rest, which
    /// move the cursor, close the modal, or are ignored.
    pub(super) fn on_settings_key(&mut self, key: KeyEvent) -> bool {
        let Some(row) = self.settings_rows.get(self.settings_cursor).cloned() else {
            return false;
        };
        if self.settings_confirm.is_some() && key.code == KeyCode::Enter {
            self.remove_confirmed_folder();
            return true;
        }
        match key.code {
            KeyCode::Enter | KeyCode::Char(' ') => match row {
                Row::DefaultDevice => self.open_default_device_picker(),
                Row::AddFolder => self.open_folder_prompt_from_settings(),
                Row::RescanAll => self.rescan_folders(),
                _ => self.change_setting(&row, Change::Activate),
            },
            KeyCode::Char('l') | KeyCode::Right => self.change_setting(&row, Change::Forward),
            KeyCode::Char('h') | KeyCode::Left => self.change_setting(&row, Change::Back),
            KeyCode::Char('d') => {
                if let Row::Folder(path) = row {
                    self.ask_remove_folder(path);
                }
            }
            _ => return false,
        }
        true
    }

    /// Enter on Default output device: the Devices list, choosing the default.
    fn open_default_device_picker(&mut self) {
        self.devices_mode = DevicesMode::Default;
        let index = match self.default_device.as_deref() {
            None => 0,
            Some(id) => self
                .devices
                .iter()
                .position(|d| d.id == id)
                .map_or(0, |i| i + 1),
        };
        let rows = self.device_rows();
        self.device_cursor.set(index, rows);
        self.modal = Modal::Devices;
    }

    /// Choose the Settings default output. `None` is System default, which
    /// switches to the device the OS marks as default when one is listed. The
    /// choice is saved, and Settings comes back with the cursor on the row.
    pub(super) fn choose_default_device(&mut self, device: Option<(String, String)>) {
        let target = match &device {
            Some(device) => Some(device.clone()),
            None => self
                .devices
                .iter()
                .find(|d| d.is_default)
                .map(|d| (d.id.clone(), d.name.clone())),
        };
        if let Some((id, name)) = target {
            if let Err(e) = self.player.send_blocking(Command::SetDevice(id)) {
                // Nothing switched and nothing is saved; back to Settings.
                self.toasts.error(e.to_string());
                self.return_to_settings_from_devices();
                return;
            }
            self.toasts.success(format!("output: {name}"));
        }
        // The device has switched already. If the save below fails, the old
        // setting comes back but the device stays switched for this run.
        let before = self.settings_snapshot();
        self.default_device = device.map(|(id, _)| id);
        self.return_to_settings_from_devices();
        self.save_setting(&Row::DefaultDevice, before);
    }

    /// Leave the default-device list for Settings, on the device row.
    fn return_to_settings_from_devices(&mut self) {
        self.devices_mode = DevicesMode::Switch;
        self.modal = Modal::Settings;
        self.select_setting(&Row::DefaultDevice);
    }

    /// Put the Settings cursor on `row`, when the list has it.
    fn select_setting(&mut self, row: &Row) {
        if let Some(index) = self.settings_rows.iter().position(|r| r == row) {
            self.set_settings_cursor(index);
        }
    }

    /// `+ Add a folder…` in Settings: the folder prompt; closing it returns here.
    fn open_folder_prompt_from_settings(&mut self) {
        if self.library_host.is_none() {
            self.toasts.error("no music library configured");
            return;
        }
        self.modal = Modal::None;
        self.folder_prompt_from_settings = true;
        self.open_folder_prompt();
    }

    /// The folder prompt closed: show Settings again if it was opened from there.
    pub(super) fn folder_prompt_closed(&mut self) {
        if std::mem::take(&mut self.folder_prompt_from_settings) {
            self.modal = Modal::Settings;
            self.refresh_settings_rows();
        }
    }

    /// `d` on a folder: ask in the strip, with the number of tracks it removes.
    fn ask_remove_folder(&mut self, path: PathBuf) {
        // Without a count (the library cannot be read) the question names no number.
        let tracks = self.library.count_under(&path);
        self.settings_confirm = Some((path, tracks));
    }

    /// Enter on the removal question. The library job forgets the folder and its
    /// tracks and reports the result when it finishes.
    fn remove_confirmed_folder(&mut self) {
        let Some((path, _)) = self.settings_confirm.take() else {
            return;
        };
        let Some(host) = self.library_host.clone() else {
            self.toasts.error("no music library configured");
            return;
        };
        if let Err(e) = host.apply(LibraryOp::RemoveFolder { path }) {
            self.toasts.error(e.to_string());
        }
    }

    /// Rebuild the folder rows from the library status. The cursor stays on the
    /// same row when it still exists.
    pub(super) fn refresh_settings_rows(&mut self) {
        let rows = settings::rows(&self.library_status.folders);
        if rows == self.settings_rows {
            return;
        }
        let current = self.settings_rows.get(self.settings_cursor).cloned();
        self.settings_rows = rows;
        let index = match current.and_then(|row| self.settings_rows.iter().position(|r| *r == row))
        {
            Some(index) => index,
            None => settings::nearest_selectable(&self.settings_rows, self.settings_cursor),
        };
        self.set_settings_cursor(index);
    }

    /// Change `row` the way `change` asks, then save it. A save that fails puts
    /// the previous values back.
    fn change_setting(&mut self, row: &Row, change: Change) {
        let before = self.settings_snapshot();
        if !self.apply_setting(row, change) {
            return;
        }
        self.after_setting_changed(row);
        self.save_setting(row, before);
    }

    /// Change the value in memory. Returns whether it changed.
    fn apply_setting(&mut self, row: &Row, change: Change) -> bool {
        match row.kind() {
            Kind::Switch => {
                let Some(slot) = self.switch_slot(row) else {
                    return false;
                };
                let value = match change {
                    Change::Activate => !*slot,
                    Change::Forward => true,
                    Change::Back => false,
                };
                let changed = *slot != value;
                *slot = value;
                changed
            }
            Kind::Choice => {
                let (forward, wrap) = match change {
                    Change::Activate => (true, true),
                    Change::Forward => (true, false),
                    Change::Back => (false, false),
                };
                if *row == Row::CoverProtocol {
                    let choices = settings::protocol_choices(self.protocol_support);
                    settings::step_protocol(&mut self.tui, &choices, forward, wrap)
                } else {
                    settings::step_choice(row, &mut self.tui, forward, wrap)
                }
            }
            // Actions and folder rows are run by `on_settings_key`, not changed here.
            Kind::Action | Kind::Folder | Kind::Heading => false,
        }
    }

    /// The stored flag behind a switch row, or `None` for other rows.
    fn switch_slot(&mut self, row: &Row) -> Option<&mut bool> {
        match row {
            Row::ShowCover => Some(&mut self.tui.show_cover),
            Row::CoverSearch => Some(&mut self.tui.cover_search),
            Row::Marquee => Some(&mut self.tui.marquee),
            Row::BitPerfect => Some(&mut self.bit_perfect),
            Row::UpdateCheck => Some(&mut self.check_for_updates),
            _ => None,
        }
    }

    /// Bit-perfect differs from the value the player started with.
    pub(crate) fn bit_perfect_changed(&self) -> bool {
        self.bit_perfect != self.bit_perfect_at_start
    }

    /// The cover protocol differs from the start and needs the start-up terminal
    /// query, which did not run: it takes effect at the next start.
    pub(crate) fn protocol_needs_restart(&self) -> bool {
        self.queried_protocol.is_none()
            && self.tui.cover_protocol != self.cover_protocol_at_start
            && matches!(
                self.tui.cover_protocol,
                CoverProtocol::Auto | CoverProtocol::Kitty | CoverProtocol::Sixel
            )
    }

    /// What a changed row needs beyond its new value.
    fn after_setting_changed(&mut self, row: &Row) {
        match row {
            Row::ShowCover | Row::CoverSize => self.cover_draw_key = None,
            Row::CoverProtocol => self.apply_cover_protocol(),
            Row::LibraryLayout => self.library.set_preferred_layout(self.tui.library_layout),
            _ => {}
        }
    }

    /// Point the picker at the new cover protocol. Without a start-up query the
    /// terminal's answer is unknown, so auto, kitty and sixel wait for a restart.
    fn apply_cover_protocol(&mut self) {
        self.cover_draw_key = None;
        let wanted = self.tui.cover_protocol;
        let protocol = match self.queried_protocol {
            Some(queried) => picker_protocol(wanted, queried),
            None => match wanted {
                CoverProtocol::Halfblocks | CoverProtocol::Off => ProtocolType::Halfblocks,
                _ => return,
            },
        };
        if let Some(picker) = self.picker.as_mut() {
            picker.set_protocol_type(protocol);
        }
    }

    /// The value a Settings row saves, as the config file takes it.
    fn stored_value(&mut self, row: &Row) -> Option<Stored> {
        if let Some(flag) = self.switch_slot(row) {
            return Some(Stored::Bool(*flag));
        }
        Some(match row {
            Row::CoverSize => Stored::Int(i64::from(self.tui.cover_size.percent())),
            Row::CoverProtocol => Stored::Str(settings::protocol_name(self.tui.cover_protocol)),
            Row::LibraryLayout => Stored::Str(settings::layout_name(self.tui.library_layout)),
            Row::Icons => Stored::Str(settings::icons_name(self.tui.icons)),
            Row::DefaultDevice => match &self.default_device {
                Some(id) => Stored::Text(id.clone()),
                None => Stored::Remove,
            },
            _ => return None,
        })
    }

    /// Write a changed row to the config file. Without a usable file the change
    /// stays for this run and the user is told why.
    fn save_setting(&mut self, row: &Row, before: SettingsSnapshot) {
        let (Some((section, key)), Some(value)) = (row.config_key(), self.stored_value(row)) else {
            return;
        };
        let Some(file) = self.config_file.clone() else {
            if !self.run_only_warned {
                self.run_only_warned = true;
                self.toasts
                    .info("Settings apply to this run only (no config file path)");
            }
            return;
        };
        if let Some(problem) = &self.config_problem {
            let text = match problem.location() {
                Some(place) => format!("Not saved: config.toml has an error ({place})"),
                None => "Not saved: config.toml could not be read".to_string(),
            };
            self.toasts.warn(text);
            return;
        }
        let written = match value {
            Stored::Bool(v) => file.set_bool(section, key, v),
            Stored::Int(v) => file.set_int(section, key, v),
            Stored::Str(v) => file.set_str(section, key, v),
            Stored::Text(v) => file.set_str(section, key, &v),
            Stored::Remove => file.remove(section, key),
        };
        if let Err(e) = written {
            self.restore_settings(before);
            self.toasts.error(format!("Could not save settings: {e}"));
        }
    }

    fn settings_snapshot(&self) -> SettingsSnapshot {
        SettingsSnapshot {
            tui: self.tui,
            bit_perfect: self.bit_perfect,
            update_check: self.check_for_updates,
            default_device: self.default_device.clone(),
            protocol: self.picker.as_ref().map(|p| p.protocol_type()),
        }
    }

    fn restore_settings(&mut self, before: SettingsSnapshot) {
        self.tui = before.tui;
        self.bit_perfect = before.bit_perfect;
        self.check_for_updates = before.update_check;
        self.default_device = before.default_device;
        if let (Some(picker), Some(protocol)) = (self.picker.as_mut(), before.protocol) {
            picker.set_protocol_type(protocol);
        }
        self.cover_draw_key = None;
        self.library.set_preferred_layout(self.tui.library_layout);
    }

    /// A removal question belongs to the Settings modal: once that modal is no
    /// longer on screen (closed, or replaced by another modal or a prompt), it goes.
    pub(super) fn drop_removal_outside_settings(&mut self) {
        if self.modal != Modal::Settings {
            self.settings_confirm = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::sync::Arc;
    use std::time::Duration;

    use crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
    use znicz_core::{AudioDeviceInfo, LibraryHost, LibraryOp, LibraryStatus, PlayerHandle};
    use znicz_library::Library;

    use super::*;
    use crate::config_file::{ConfigFile, ConfigProblem};
    use crate::library_pane::{EffectiveLayout, LibraryPane};

    fn test_player() -> PlayerHandle {
        let (player, _thread) = znicz_core::spawn_player(znicz_core::AudioConfig::default());
        player
    }

    fn toast_texts(app: &App) -> Vec<String> {
        app.toasts.recent().iter().map(|t| t.text.clone()).collect()
    }

    fn press(app: &mut App, code: KeyCode) {
        app.on_key(KeyEvent::new(code, KeyModifiers::NONE));
    }

    /// The screen as one string per row, for checking what a frame shows.
    fn screen_rows(app: &mut App) -> Vec<String> {
        let state = app.player.state();
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 30)).unwrap();
        terminal
            .draw(|frame| crate::views::render(frame, app, &state))
            .unwrap();
        let buffer = terminal.backend().buffer();
        let width = buffer.area.width as usize;
        buffer
            .content()
            .chunks(width)
            .map(|row| row.iter().map(|cell| cell.symbol()).collect())
            .collect()
    }

    fn click(app: &mut App, column: u16, row: u16) {
        app.on_mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column,
            row,
            modifiers: KeyModifiers::NONE,
        });
    }

    use crate::settings::{Row, SettingsValues};
    use crate::tui_config::{CoverSize, LibraryLayout};

    fn settings_app(dir: &tempfile::TempDir) -> App {
        App::with_library(test_player(), None)
            .with_config_file(ConfigFile::new(dir.path().join("config.toml")))
    }

    /// Put the Settings cursor on `row`, as pressing keys to reach it would.
    fn select(app: &mut App, row: Row) {
        app.settings_cursor = app
            .settings_rows
            .iter()
            .position(|r| *r == row)
            .expect("the row is listed");
    }

    /// The config file as the test sees it on disk.
    fn saved(dir: &tempfile::TempDir) -> toml::Table {
        let text = std::fs::read_to_string(dir.path().join("config.toml")).unwrap();
        toml::from_str(&text).unwrap()
    }

    #[test]
    fn stage_keeps_its_cover_settings_while_settings_is_open() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = settings_app(&dir);
        app.view = crate::app::View::Stage;
        press(&mut app, KeyCode::Char('S'));
        screen_rows(&mut app);
        select(&mut app, Row::ShowCover);
        press(&mut app, KeyCode::Enter);
        let rows = screen_rows(&mut app);
        // Saved and shown in the row, but the Stage still draws the cover.
        assert!(!app.tui.show_cover);
        assert!(app.shown_tui().show_cover);
        let row = rows
            .iter()
            .find(|line| line.contains("Show cover"))
            .expect("row drawn");
        assert!(row.contains("on close"), "{row}");
        press(&mut app, KeyCode::Esc);
        screen_rows(&mut app);
        assert!(app.stage_frozen.is_none());
        assert!(!app.shown_tui().show_cover);
    }

    #[test]
    fn protocols_the_terminal_did_not_report_are_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = settings_app(&dir);
        app.protocol_support = Some(crate::settings::ProtocolSupport {
            kitty: false,
            sixel: true,
        });
        press(&mut app, KeyCode::Char('S'));
        select(&mut app, Row::CoverProtocol);
        press(&mut app, KeyCode::Char('l'));
        assert_eq!(app.tui.cover_protocol, crate::CoverProtocol::Sixel);
        press(&mut app, KeyCode::Char('h'));
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.tui.cover_protocol, crate::CoverProtocol::Sixel);
    }

    #[test]
    fn an_unsupported_protocol_from_the_file_says_so() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = settings_app(&dir);
        app.tui.cover_protocol = crate::CoverProtocol::Kitty;
        app.protocol_support = Some(crate::settings::ProtocolSupport::default());
        press(&mut app, KeyCode::Char('S'));
        let rows = screen_rows(&mut app);
        let row = rows
            .iter()
            .find(|line| line.contains("Cover protocol"))
            .expect("protocol row drawn");
        assert!(row.contains("not supported"), "{row}");
    }

    #[test]
    fn s_opens_settings_and_esc_closes() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = settings_app(&dir);
        press(&mut app, KeyCode::Char('S'));
        assert_eq!(app.modal, Modal::Settings);
        press(&mut app, KeyCode::Esc);
        assert_eq!(app.modal, Modal::None);
        press(&mut app, KeyCode::Char('S'));
        press(&mut app, KeyCode::Char('S'));
        assert_eq!(app.modal, Modal::None, "S again closes it");
    }

    #[test]
    fn cursor_skips_headings() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = settings_app(&dir);
        press(&mut app, KeyCode::Char('S'));
        assert_eq!(app.settings_rows[app.settings_cursor], Row::ShowCover);
        press(&mut app, KeyCode::Up);
        assert_eq!(
            app.settings_rows[app.settings_cursor],
            Row::ShowCover,
            "k at the top stays put"
        );
        for _ in 0..7 {
            press(&mut app, KeyCode::Char('j'));
        }
        assert_eq!(
            app.settings_rows[app.settings_cursor],
            Row::DefaultDevice,
            "the Audio heading is skipped"
        );
        press(&mut app, KeyCode::Char('k'));
        assert_eq!(app.settings_rows[app.settings_cursor], Row::Icons);
    }

    #[test]
    fn enter_on_show_cover_flips_it_and_saves() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = settings_app(&dir);
        press(&mut app, KeyCode::Char('S'));
        press(&mut app, KeyCode::Enter);
        assert!(!app.tui.show_cover);
        assert_eq!(saved(&dir)["tui"]["show_cover"].as_bool(), Some(false));
    }

    #[test]
    fn l_and_h_step_cover_size_and_stop_at_the_ends() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = settings_app(&dir);
        press(&mut app, KeyCode::Char('S'));
        select(&mut app, Row::CoverSize);

        app.cover_draw_key = Some(("cover".into(), 1, 1));
        for _ in 0..3 {
            press(&mut app, KeyCode::Char('l'));
        }
        assert_eq!(app.tui.cover_size, CoverSize::P25);
        press(&mut app, KeyCode::Char('l'));
        assert_eq!(app.tui.cover_size, CoverSize::P25, "stops at 25%");
        assert_eq!(saved(&dir)["tui"]["cover_size"].as_integer(), Some(25));
        assert_eq!(app.cover_draw_key, None, "the cover is drawn again");

        for _ in 0..3 {
            press(&mut app, KeyCode::Left);
        }
        assert_eq!(app.tui.cover_size, CoverSize::P100);
        press(&mut app, KeyCode::Char('h'));
        assert_eq!(app.tui.cover_size, CoverSize::P100, "stops at 100%");
        assert_eq!(saved(&dir)["tui"]["cover_size"].as_integer(), Some(100));
    }

    #[test]
    fn enter_steps_and_wraps() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = settings_app(&dir);
        press(&mut app, KeyCode::Char('S'));
        select(&mut app, Row::CoverSize);
        for _ in 0..3 {
            press(&mut app, KeyCode::Enter);
        }
        assert_eq!(app.tui.cover_size, CoverSize::P25);
        press(&mut app, KeyCode::Char(' '));
        assert_eq!(app.tui.cover_size, CoverSize::P100, "wraps to 100%");
        assert_eq!(saved(&dir)["tui"]["cover_size"].as_integer(), Some(100));
    }

    #[test]
    fn the_strip_shows_the_highlighted_rows_explanation() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = settings_app(&dir);
        press(&mut app, KeyCode::Char('S'));
        let screen = screen_rows(&mut app).join("\n");
        assert!(screen.contains("Show album art on the Stage."), "{screen}");

        press(&mut app, KeyCode::Down);
        let screen = screen_rows(&mut app).join("\n");
        assert!(
            screen.contains("How big the album art is on the Stage"),
            "{screen}"
        );
        assert!(!screen.contains("Show album art on the Stage."), "{screen}");
    }

    #[test]
    fn a_failed_save_restores_the_value_and_toasts() {
        let dir = tempfile::tempdir().unwrap();
        // A file where the config's folder should be: creating the folder fails.
        let blocker = dir.path().join("not-a-folder");
        std::fs::write(&blocker, "a file").unwrap();
        let mut app = App::with_library(test_player(), None)
            .with_config_file(ConfigFile::new(blocker.join("config.toml")));
        press(&mut app, KeyCode::Char('S'));
        press(&mut app, KeyCode::Enter);

        assert!(app.tui.show_cover, "the old value is back");
        let texts = toast_texts(&app);
        assert!(
            texts
                .last()
                .is_some_and(|t| t.starts_with("Could not save settings: ")),
            "{texts:?}"
        );
    }

    #[test]
    fn without_a_config_file_the_change_applies_and_toasts_once() {
        let mut app = App::with_library(test_player(), None);
        press(&mut app, KeyCode::Char('S'));
        press(&mut app, KeyCode::Enter);
        assert!(!app.tui.show_cover, "the change applies for this run");
        press(&mut app, KeyCode::Enter);
        assert!(app.tui.show_cover);

        let note = "Settings apply to this run only (no config file path)";
        let texts = toast_texts(&app);
        assert_eq!(texts.iter().filter(|t| *t == note).count(), 1, "{texts:?}");
    }

    #[test]
    fn a_broken_config_is_not_written() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let broken = "[tui\nshow_cover = true\n";
        std::fs::write(&path, broken).unwrap();
        let mut app = App::with_library(test_player(), None)
            .with_config_file(ConfigFile::new(path.clone()))
            .with_config_problem(ConfigProblem::Parse {
                line: 1,
                column: 5,
                message: "expected `]`".into(),
            });
        press(&mut app, KeyCode::Char('S'));
        press(&mut app, KeyCode::Enter);

        assert!(!app.tui.show_cover, "the change applies for this run");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), broken);
        let texts = toast_texts(&app);
        assert_eq!(
            texts.last().map(String::as_str),
            Some("Not saved: config.toml has an error (line 1, column 5)"),
            "{texts:?}"
        );
    }

    #[test]
    fn help_lists_s() {
        let sections = crate::keys::help_sections(crate::keys::Context::Library);
        assert!(
            sections
                .iter()
                .flat_map(|s| s.bindings.iter())
                .any(|b| b.keys == "S" && b.action == "settings")
        );
    }

    #[test]
    fn bit_perfect_shows_on_restart_after_a_change() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = settings_app(&dir).with_settings_values(SettingsValues {
            bit_perfect: true,
            ..SettingsValues::default()
        });
        press(&mut app, KeyCode::Char('S'));
        select(&mut app, Row::BitPerfect);
        press(&mut app, KeyCode::Enter);

        assert!(!app.bit_perfect);
        assert!(app.bit_perfect_changed());
        assert_eq!(saved(&dir)["audio"]["bit_perfect"].as_bool(), Some(false));
        let screen = screen_rows(&mut app).join("\n");
        assert!(screen.contains("on restart"), "{screen}");
    }

    #[test]
    fn layout_change_applies_live() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = settings_app(&dir);
        press(&mut app, KeyCode::Char('S'));
        select(&mut app, Row::LibraryLayout);
        press(&mut app, KeyCode::Enter);

        assert_eq!(app.tui.library_layout, LibraryLayout::Tree);
        let strip = app.library_strip();
        assert_eq!(app.library.effective_layout(strip), EffectiveLayout::Tree);
        assert_eq!(saved(&dir)["tui"]["library_layout"].as_str(), Some("tree"));
    }

    #[test]
    fn a_cover_protocol_without_a_terminal_query_waits_for_a_restart() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = settings_app(&dir);
        press(&mut app, KeyCode::Char('S'));
        select(&mut app, Row::CoverProtocol);

        press(&mut app, KeyCode::Enter);
        assert_eq!(app.tui.cover_protocol, CoverProtocol::Kitty);
        assert!(app.protocol_needs_restart());
        let screen = screen_rows(&mut app).join("\n");
        assert!(screen.contains("on restart"), "{screen}");

        press(&mut app, KeyCode::Enter);
        assert!(app.protocol_needs_restart(), "sixel also needs a restart");
        press(&mut app, KeyCode::Enter);
        assert!(!app.protocol_needs_restart(), "halfblocks works now");
        assert_eq!(
            saved(&dir)["tui"]["cover_protocol"].as_str(),
            Some("halfblocks")
        );
    }

    #[test]
    fn a_click_selects_a_row_but_not_a_heading_and_the_wheel_moves() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = settings_app(&dir);
        press(&mut app, KeyCode::Char('S'));
        screen_rows(&mut app);
        let hit = app.hits.overlay_list.expect("the list is drawn");

        // Row 0 is the "Display" heading; row 2 is Cover size.
        click(&mut app, hit.inner.x + 1, hit.inner.y);
        assert_eq!(app.settings_cursor, 1, "a heading is not selected");
        click(&mut app, hit.inner.x + 1, hit.inner.y + 2);
        assert_eq!(app.settings_rows[app.settings_cursor], Row::CoverSize);

        app.on_mouse(MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: 0,
            row: 0,
            modifiers: KeyModifiers::NONE,
        });
        assert_eq!(app.settings_rows[app.settings_cursor], Row::CoverProtocol);
    }

    #[test]
    fn bit_perfect_back_at_its_start_value_drops_the_restart_hint() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = settings_app(&dir).with_settings_values(SettingsValues {
            bit_perfect: true,
            ..SettingsValues::default()
        });
        press(&mut app, KeyCode::Char('S'));
        select(&mut app, Row::BitPerfect);
        press(&mut app, KeyCode::Enter);
        assert!(app.bit_perfect_changed());
        press(&mut app, KeyCode::Enter);
        assert!(
            !app.bit_perfect_changed(),
            "the player still has bit-perfect on"
        );
        let screen = screen_rows(&mut app).join("\n");
        assert!(!screen.contains("on restart"), "{screen}");
    }

    #[test]
    fn a_cover_protocol_back_at_its_start_value_drops_the_restart_hint() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = settings_app(&dir);
        press(&mut app, KeyCode::Char('S'));
        select(&mut app, Row::CoverProtocol);
        // Auto is the start value; five steps (auto, kitty, sixel, halfblocks, off)
        // come back to it.
        press(&mut app, KeyCode::Enter);
        assert!(app.protocol_needs_restart());
        for _ in 0..4 {
            press(&mut app, KeyCode::Enter);
        }
        assert_eq!(app.tui.cover_protocol, CoverProtocol::Auto);
        assert!(!app.protocol_needs_restart(), "auto is what the start used");
    }

    // --- settings: default device and folders ---

    use crate::line_edit::LineEdit;

    /// A library host that records the ops it is sent and reports a settable status.
    #[derive(Default)]
    struct RecordingHost {
        ops: std::sync::Mutex<Vec<LibraryOp>>,
        status: std::sync::Mutex<LibraryStatus>,
        refuse: bool,
    }

    impl LibraryHost for RecordingHost {
        fn apply(&self, op: LibraryOp) -> znicz_core::Result<()> {
            if self.refuse {
                return Err(znicz_core::ZniczError::Library("refused".into()));
            }
            self.ops.lock().unwrap().push(op);
            Ok(())
        }
        fn status(&self) -> LibraryStatus {
            self.status.lock().unwrap().clone()
        }
    }

    fn device(id: &str, name: &str, is_default: bool) -> AudioDeviceInfo {
        AudioDeviceInfo {
            id: id.into(),
            name: name.into(),
            is_default,
        }
    }

    /// Settings over a recording host that lists `folders`.
    fn host_app(dir: &tempfile::TempDir, folders: &[&str]) -> (App, Arc<RecordingHost>) {
        let host = Arc::new(RecordingHost::default());
        let folders: Vec<PathBuf> = folders.iter().map(PathBuf::from).collect();
        host.status.lock().unwrap().folders = folders.clone();
        let mut app = settings_app(dir);
        app.library_status.folders = folders;
        app.library_host = Some(host.clone());
        (app, host)
    }

    fn sent(host: &RecordingHost) -> Vec<LibraryOp> {
        host.ops.lock().unwrap().clone()
    }

    #[test]
    fn choosing_a_default_device_switches_and_saves() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = settings_app(&dir);
        app.devices = vec![
            device("usb", "USB DAC", false),
            device("hdmi", "HDMI", true),
        ];
        press(&mut app, KeyCode::Char('S'));
        select(&mut app, Row::DefaultDevice);
        press(&mut app, KeyCode::Enter);

        assert_eq!(app.modal, Modal::Devices);
        assert_eq!(app.devices_mode, DevicesMode::Default);
        let screen = screen_rows(&mut app).join("\n");
        assert!(screen.contains("Default output device"), "{screen}");
        assert!(screen.contains("System default"), "{screen}");

        press(&mut app, KeyCode::Char('j'));
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.modal, Modal::Settings, "back to Settings");
        assert_eq!(app.default_device.as_deref(), Some("usb"));
        assert_eq!(saved(&dir)["audio"]["device"].as_str(), Some("usb"));
        assert_eq!(app.settings_rows[app.settings_cursor], Row::DefaultDevice);
        let screen = screen_rows(&mut app).join("\n");
        assert!(
            screen.contains("USB DAC"),
            "the row shows the name:\n{screen}"
        );
    }

    #[test]
    fn system_default_removes_the_key() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("config.toml"),
            "[audio]\ndevice = \"usb\"\n",
        )
        .unwrap();
        let mut app = settings_app(&dir).with_settings_values(SettingsValues {
            default_device: Some("usb".into()),
            ..SettingsValues::default()
        });
        app.devices = vec![device("usb", "USB DAC", false)];
        press(&mut app, KeyCode::Char('S'));
        select(&mut app, Row::DefaultDevice);
        press(&mut app, KeyCode::Enter);
        // The picker opens on the current device; System default is row 0.
        press(&mut app, KeyCode::Char('k'));
        press(&mut app, KeyCode::Enter);

        assert_eq!(app.default_device, None);
        assert_eq!(app.modal, Modal::Settings);
        let file = saved(&dir);
        assert!(file["audio"].as_table().unwrap().get("device").is_none());
    }

    #[test]
    fn esc_in_the_default_picker_returns_to_settings() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = settings_app(&dir);
        press(&mut app, KeyCode::Char('S'));
        select(&mut app, Row::DefaultDevice);
        press(&mut app, KeyCode::Enter);
        press(&mut app, KeyCode::Esc);

        assert_eq!(app.modal, Modal::Settings);
        assert_eq!(app.devices_mode, DevicesMode::Switch);
        assert_eq!(app.settings_rows[app.settings_cursor], Row::DefaultDevice);
        assert_eq!(app.default_device, None, "nothing was chosen");
    }

    #[test]
    fn add_folder_opens_the_prompt_and_returns_to_settings() {
        let dir = tempfile::tempdir().unwrap();
        let music = tempfile::tempdir().unwrap();
        let (mut app, host) = host_app(&dir, &[]);
        press(&mut app, KeyCode::Char('S'));
        select(&mut app, Row::AddFolder);

        press(&mut app, KeyCode::Enter);
        assert!(app.folder_prompt.is_some());
        assert_eq!(app.modal, Modal::None);
        press(&mut app, KeyCode::Esc);
        assert!(app.folder_prompt.is_none());
        assert_eq!(app.modal, Modal::Settings, "Esc returns to Settings");

        press(&mut app, KeyCode::Enter);
        app.folder_prompt = Some(LineEdit::from_text(music.path().display().to_string()));
        press(&mut app, KeyCode::Enter);
        assert!(app.folder_prompt.is_none());
        assert_eq!(app.modal, Modal::Settings, "a submitted folder returns too");
        assert!(
            sent(&host)
                .iter()
                .any(|op| matches!(op, LibraryOp::AddFolder { .. }))
        );
    }

    #[test]
    fn rescan_sends_rescan_all() {
        let dir = tempfile::tempdir().unwrap();
        let (mut app, host) = host_app(&dir, &["/music/a"]);
        press(&mut app, KeyCode::Char('S'));
        select(&mut app, Row::RescanAll);
        press(&mut app, KeyCode::Enter);

        assert_eq!(sent(&host).len(), 1);
        assert!(matches!(sent(&host)[0], LibraryOp::RescanAll));
        assert_eq!(app.modal, Modal::Settings);
    }

    #[test]
    fn d_asks_then_enter_sends_remove_folder() {
        let dir = tempfile::tempdir().unwrap();
        let mut library = Library::open_in_memory().unwrap();
        library
            .upsert_track_for_test(Path::new("/music/a/one.flac"), "One", None, None, None)
            .unwrap();
        let (mut app, host) = host_app(&dir, &["/music/a"]);
        app.library = LibraryPane::new(Some(library));
        press(&mut app, KeyCode::Char('S'));
        select(&mut app, Row::Folder(PathBuf::from("/music/a")));
        press(&mut app, KeyCode::Char('d'));

        assert_eq!(
            app.settings_confirm,
            Some((PathBuf::from("/music/a"), Some(1)))
        );
        let screen = screen_rows(&mut app).join("\n");
        // The strip wraps this sentence over two lines, so check its pieces.
        assert!(screen.contains("Enter remove · Esc keep"), "{screen}");
        assert!(
            screen.contains("Remove /music/a and its 1 track"),
            "{screen}"
        );

        press(&mut app, KeyCode::Enter);
        assert!(app.settings_confirm.is_none());
        assert!(matches!(
            sent(&host).as_slice(),
            [LibraryOp::RemoveFolder { path }] if path == Path::new("/music/a")
        ));
        assert_eq!(app.modal, Modal::Settings);
    }

    #[test]
    fn esc_cancels_the_removal() {
        let dir = tempfile::tempdir().unwrap();
        let (mut app, host) = host_app(&dir, &["/music/a"]);
        press(&mut app, KeyCode::Char('S'));
        select(&mut app, Row::Folder(PathBuf::from("/music/a")));
        press(&mut app, KeyCode::Char('d'));
        press(&mut app, KeyCode::Esc);

        assert!(app.settings_confirm.is_none());
        assert_eq!(app.modal, Modal::Settings, "Esc cancels, it does not close");
        assert!(sent(&host).is_empty());
    }

    #[test]
    fn a_refused_removal_is_a_toast() {
        let dir = tempfile::tempdir().unwrap();
        let (mut app, host) = host_app(&dir, &["/music/a"]);
        let refusing = Arc::new(RecordingHost {
            refuse: true,
            ..RecordingHost::default()
        });
        app.library_host = Some(refusing);
        press(&mut app, KeyCode::Char('S'));
        select(&mut app, Row::Folder(PathBuf::from("/music/a")));
        press(&mut app, KeyCode::Char('d'));
        press(&mut app, KeyCode::Enter);

        assert!(sent(&host).is_empty());
        assert!(toast_texts(&app).last().is_some_and(|t| t == "refused"));
    }

    #[test]
    fn folder_rows_follow_the_library_status() {
        let dir = tempfile::tempdir().unwrap();
        let (mut app, host) = host_app(&dir, &["/music/a"]);
        app.library_poll_every = Duration::ZERO;
        press(&mut app, KeyCode::Char('S'));
        select(&mut app, Row::Folder(PathBuf::from("/music/a")));

        // The removal job finishes: the writer no longer lists the folder.
        host.status.lock().unwrap().folders.clear();
        screen_rows(&mut app);

        assert!(
            !app.settings_rows
                .iter()
                .any(|row| matches!(row, Row::Folder(_)))
        );
        assert_eq!(
            app.settings_rows[app.settings_cursor],
            Row::AddFolder,
            "the cursor lands on the next row"
        );
    }

    /// Click a visible Settings row other than `avoid`, as a user would.
    fn click_another_setting_row(app: &mut App, avoid: usize) {
        screen_rows(app);
        let hit = app.hits.overlay_list.expect("the list is drawn");
        let column = hit.inner.x + 1;
        for row in hit.inner.y..hit.inner.y + hit.inner.height {
            if let Some(index) = hit.row_at(column, row) {
                if index != avoid && app.settings_rows[index].selectable() {
                    click(app, column, row);
                    return;
                }
            }
        }
        panic!("no other row is on screen");
    }

    #[test]
    fn a_removal_question_does_not_survive_closing_and_reopening_settings() {
        let dir = tempfile::tempdir().unwrap();
        let (mut app, host) = host_app(&dir, &["/music/a"]);
        press(&mut app, KeyCode::Char('S'));
        select(&mut app, Row::Folder(PathBuf::from("/music/a")));
        press(&mut app, KeyCode::Char('d'));
        press(&mut app, KeyCode::Char('S'));
        press(&mut app, KeyCode::Char('S'));
        assert!(app.settings_confirm.is_none());
        press(&mut app, KeyCode::Enter);

        assert!(sent(&host).is_empty(), "Enter must not remove the folder");
    }

    #[test]
    fn a_removal_question_does_not_survive_a_click_on_another_row() {
        let dir = tempfile::tempdir().unwrap();
        let (mut app, host) = host_app(&dir, &["/music/a"]);
        press(&mut app, KeyCode::Char('S'));
        let folder = app
            .settings_rows
            .iter()
            .position(|r| matches!(r, Row::Folder(_)))
            .unwrap();
        select(&mut app, Row::Folder(PathBuf::from("/music/a")));
        press(&mut app, KeyCode::Char('d'));
        click_another_setting_row(&mut app, folder);
        press(&mut app, KeyCode::Enter);

        assert!(sent(&host).is_empty(), "Enter must not remove the folder");
    }

    #[test]
    fn a_removal_question_does_not_survive_jumping_to_the_last_row() {
        let dir = tempfile::tempdir().unwrap();
        let (mut app, host) = host_app(&dir, &["/music/a"]);
        press(&mut app, KeyCode::Char('S'));
        select(&mut app, Row::Folder(PathBuf::from("/music/a")));
        press(&mut app, KeyCode::Char('d'));
        press(&mut app, KeyCode::Char('G'));
        assert_eq!(app.settings_rows[app.settings_cursor], Row::UpdateCheck);
        press(&mut app, KeyCode::Enter);

        assert!(sent(&host).is_empty(), "Enter must not remove the folder");
    }

    #[test]
    fn a_removal_question_without_a_count_names_no_number() {
        let dir = tempfile::tempdir().unwrap();
        let (mut app, _host) = host_app(&dir, &["/music/a"]);
        press(&mut app, KeyCode::Char('S'));
        select(&mut app, Row::Folder(PathBuf::from("/music/a")));
        press(&mut app, KeyCode::Char('d'));

        assert_eq!(
            app.settings_confirm,
            Some((PathBuf::from("/music/a"), None))
        );
        let screen = screen_rows(&mut app).join("\n");
        assert!(screen.contains("Remove /music/a"), "{screen}");
        assert!(screen.contains("and its tracks from the"), "{screen}");
    }
}
