//! The Settings modal: its keys, applying and saving each row, and the default
//! output device and folder actions it starts. Lives beside `app.rs` as a second
//! `impl App`.

use std::path::PathBuf;
use std::time::Duration;

use crossterm::event::{KeyCode, KeyEvent};
use ratatui_image::picker::ProtocolType;
use znicz_core::{Command, FadeTimes, LibraryOp, SpotifyConfig, SpotifyOp};
use znicz_spotify::browser::OpenBrowser;

use super::spotify_ui::SpotifyConfirm;
use super::{App, DevicesMode, Modal};
use crate::line_edit::LineEdit;
use crate::settings::{self, Kind, Row, Tab};
use crate::spotify_rows;
use crate::spotify_worker::SpotifyAsk;
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
    fades: FadeTimes,
    spotify: SpotifyConfig,
    open_browser: OpenBrowser,
}

/// A slider row's typed value, being entered in the Settings strip.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SliderPrompt {
    pub row: Row,
    pub text: LineEdit,
    /// The last Enter was refused: the row's range, which the strip shows.
    pub error: Option<String>,
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
        self.settings_on_tab_bar = false;
        self.settings_rows = settings::rows(self.settings_tab, &self.library_status.folders);
        self.set_settings_cursor(0);
        self.settings_scroll = 0;
        self.modal = Modal::Settings;
        if self.settings_tab == Tab::Spotify {
            self.on_spotify_tab_shown();
        }
    }

    /// Show `tab` with the cursor on its first row. Focus stays where it was.
    /// Go to `tab` from a key or a click. The tab already shown keeps its
    /// cursor and any pending removal question.
    pub(super) fn switch_settings_tab(&mut self, tab: Tab) {
        if tab != self.settings_tab {
            self.set_settings_tab(tab);
        }
    }

    pub(super) fn set_settings_tab(&mut self, tab: Tab) {
        self.settings_tab = tab;
        self.settings_rows = settings::rows(tab, &self.library_status.folders);
        self.settings_scroll = 0;
        self.set_settings_cursor(0);
        if tab == Tab::Spotify {
            self.on_spotify_tab_shown();
        }
    }

    /// Move the Settings cursor `delta` rows, stopping at either end. The tab
    /// bar has focus, so there is no row to move to.
    pub(super) fn settings_move(&mut self, delta: isize) {
        // On the tab bar there is no row to move to; while a value is typed
        // the row stays put.
        if self.settings_on_tab_bar || self.settings_prompt.is_some() {
            return;
        }
        let last = self.settings_rows.len().saturating_sub(1);
        let target = self.settings_cursor.saturating_add_signed(delta).min(last);
        if target != self.settings_cursor {
            self.set_settings_cursor(target);
        }
    }

    /// Put the Settings cursor on `index`. A pending removal question or typed
    /// slider value was about the row the cursor was on, so both are dropped here.
    pub(super) fn set_settings_cursor(&mut self, index: usize) {
        self.settings_confirm = None;
        self.spotify_confirm = None;
        self.settings_prompt = None;
        self.settings_cursor = index;
    }

    /// Keys for the Settings modal. Returns false for the rest, which move the
    /// cursor, close the modal, or are ignored.
    pub(super) fn on_settings_key(&mut self, key: KeyEvent) -> bool {
        // An open slider prompt takes every key: tabs and rows stay put.
        if self.settings_prompt.is_some() {
            self.on_slider_prompt_key(key);
            return true;
        }
        // The login panel owns c, Enter and Esc; other keys fall through.
        if self.spotify_login.is_some() {
            return self.on_spotify_panel_key(key);
        }
        // Tab keys work from either focus, and focus stays where it is.
        let tab = match key.code {
            KeyCode::Tab => Some(self.settings_tab.next()),
            KeyCode::BackTab => Some(self.settings_tab.previous()),
            KeyCode::Char(c) => Tab::from_key(c),
            _ => None,
        };
        if let Some(tab) = tab {
            self.switch_settings_tab(tab);
            return true;
        }
        if self.settings_on_tab_bar {
            return self.on_tab_bar_key(key.code);
        }
        let Some(row) = self.settings_rows.get(self.settings_cursor).cloned() else {
            return false;
        };
        if self.settings_confirm.is_some() && key.code == KeyCode::Enter {
            self.remove_confirmed_folder();
            return true;
        }
        if self.spotify_confirm.is_some() && key.code == KeyCode::Enter {
            self.confirm_spotify();
            return true;
        }
        match key.code {
            KeyCode::Up | KeyCode::Char('k') if self.settings_cursor == 0 => {
                self.settings_on_tab_bar = true;
                self.settings_confirm = None;
                self.spotify_confirm = None;
            }
            KeyCode::Enter | KeyCode::Char(' ') => match row {
                Row::DefaultDevice => self.open_default_device_picker(),
                Row::AddFolder => self.open_folder_prompt_from_settings(),
                Row::RescanAll => self.rescan_folders(),
                Row::ClientId | Row::RedirectPort => self.open_text_prompt(&row),
                Row::Account => self.spotify_account_enter(),
                _ if settings::slider(&row).is_some() => self.open_slider_prompt(&row),
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

    /// Keys while the tab bar has focus. Returns false for the keys that belong
    /// to the rest of the modal or the app (Esc, `?`, global keys).
    fn on_tab_bar_key(&mut self, code: KeyCode) -> bool {
        match code {
            KeyCode::Left | KeyCode::Char('h') if self.settings_tab != Tab::Display => {
                self.set_settings_tab(self.settings_tab.previous());
            }
            KeyCode::Right | KeyCode::Char('l') if self.settings_tab != Tab::Spotify => {
                self.set_settings_tab(self.settings_tab.next());
            }
            KeyCode::Down | KeyCode::Char('j') | KeyCode::Enter => {
                self.settings_on_tab_bar = false;
                self.set_settings_cursor(0);
            }
            // The ends of the bar, and row keys, which have no row here.
            KeyCode::Left
            | KeyCode::Right
            | KeyCode::Char('h')
            | KeyCode::Char('l')
            | KeyCode::Up
            | KeyCode::Char('k')
            | KeyCode::Char(' ')
            | KeyCode::Char('d') => {}
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

    /// Show the tab `row` lives on, with the cursor on `row`, when the list has it.
    fn select_setting(&mut self, row: &Row) {
        self.set_settings_tab(row.tab());
        self.settings_on_tab_bar = false;
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
        let rows = settings::rows(self.settings_tab, &self.library_status.folders);
        if rows == self.settings_rows {
            return;
        }
        let current = self.settings_rows.get(self.settings_cursor).cloned();
        self.settings_rows = rows;
        let index = match current.and_then(|row| self.settings_rows.iter().position(|r| *r == row))
        {
            Some(index) => index,
            None => self
                .settings_cursor
                .min(self.settings_rows.len().saturating_sub(1)),
        };
        self.set_settings_cursor(index);
    }

    /// Change `row` the way `change` asks, then save it. A save that fails puts
    /// the previous values back.
    fn change_setting(&mut self, row: &Row, change: Change) {
        self.change_with(row, |app| app.apply_setting(row, change));
    }

    /// Run `apply` on the in-memory value of `row`. When it reports a change, do
    /// what the row needs and save it, so a step and a typed value share one path.
    /// A save that fails puts the previous values back.
    pub(super) fn change_with(&mut self, row: &Row, apply: impl FnOnce(&mut Self) -> bool) {
        let before = self.settings_snapshot();
        if !apply(self) {
            return;
        }
        self.after_setting_changed(row);
        self.save_setting(row, before);
    }

    /// Enter or Space on a slider row: a prompt holding the current value, in
    /// the row's unit, with the caret at the end.
    fn open_slider_prompt(&mut self, row: &Row) {
        let (Some(slider), Some(time)) = (
            settings::slider(row),
            settings::slider_time(row, &self.fades),
        ) else {
            return;
        };
        self.settings_prompt = Some(SliderPrompt {
            row: row.clone(),
            text: LineEdit::from_text(settings::unit_number(time, slider.unit)),
            error: None,
        });
    }

    /// Keys while a slider prompt is open. Enter saves the typed value, Esc
    /// cancels it, and every other key edits the text.
    fn on_slider_prompt_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => self.settings_prompt = None,
            KeyCode::Enter => self.submit_slider_prompt(),
            _ => {
                if let Some(prompt) = self.settings_prompt.as_mut() {
                    prompt.text.on_key(key);
                    prompt.error = None;
                }
            }
        }
    }

    /// Enter in the slider prompt. A value in range closes the prompt and is
    /// saved like a step; anything else keeps the prompt open with the range.
    fn submit_slider_prompt(&mut self) {
        match self.settings_prompt.as_ref().map(|p| p.row.clone()) {
            Some(Row::ClientId) => return self.submit_client_id_prompt(),
            Some(Row::RedirectPort) => return self.submit_port_prompt(),
            _ => {}
        }
        let Some(prompt) = self.settings_prompt.as_mut() else {
            return;
        };
        let Some(slider) = settings::slider(&prompt.row) else {
            self.settings_prompt = None;
            return;
        };
        match settings::parse_slider_value(slider, prompt.text.as_str()) {
            Err(range) => prompt.error = Some(range),
            Ok(time) => {
                let row = prompt.row.clone();
                self.settings_prompt = None;
                self.change_with(&row, |app| app.set_slider_time(&row, time));
            }
        }
    }

    /// Set a slider row's time to `time`. Returns whether it changed.
    fn set_slider_time(&mut self, row: &Row, time: Duration) -> bool {
        let Some(slot) = settings::fade_slot(row, &mut self.fades) else {
            return false;
        };
        let changed = *slot != time;
        *slot = time;
        changed
    }

    /// Step the Playback device choice through the active device, the listed
    /// devices and the configured one. Without a client ID the row does nothing.
    fn step_playback_device(&mut self, forward: bool, wrap: bool) -> bool {
        if !self.spotify.is_configured() {
            return false;
        }
        let view = self.spotify_view();
        let choices = spotify_rows::device_choices(&view);
        let new = settings::step(&choices, &self.spotify.device, forward, wrap);
        let changed = new != self.spotify.device;
        self.spotify.device = new;
        changed
    }

    /// Set the redirect port. Returns whether it changed.
    fn set_redirect_port(&mut self, port: u16) -> bool {
        let changed = self.spotify.redirect_port != port;
        self.spotify.redirect_port = port;
        changed
    }

    /// Enter on the client ID or redirect port: a prompt with the current value.
    fn open_text_prompt(&mut self, row: &Row) {
        let text = match row {
            Row::ClientId => self.spotify.client_id.clone(),
            Row::RedirectPort => self.spotify.redirect_port.to_string(),
            _ => return,
        };
        self.settings_prompt = Some(SliderPrompt {
            row: row.clone(),
            text: LineEdit::from_text(text),
            error: None,
        });
    }

    /// Enter in the port prompt: a number in range saves; anything else keeps the prompt open.
    fn submit_port_prompt(&mut self) {
        let Some(prompt) = self.settings_prompt.as_mut() else {
            return;
        };
        match spotify_rows::parse_port(prompt.text.as_str()) {
            Err(range) => prompt.error = Some(range),
            Ok(port) => {
                self.settings_prompt = None;
                self.change_with(&Row::RedirectPort, |app| app.set_redirect_port(port));
            }
        }
    }

    /// Enter in the client ID prompt. The value is trimmed and checked; a
    /// refused value keeps the prompt open with the reason. Changing or clearing
    /// a set ID waits for Enter in the strip, because the player logs out.
    fn submit_client_id_prompt(&mut self) {
        let Some(prompt) = self.settings_prompt.as_mut() else {
            return;
        };
        let text = prompt.text.as_str().trim().to_string();
        if let Some(problem) = spotify_rows::client_id_problem(&text) {
            prompt.error = Some(problem.into());
            return;
        }
        self.settings_prompt = None;
        if text == self.spotify.client_id {
            return;
        }
        // A set ID asks first, whatever the login state: the player logs out
        // and deletes the token. Only the first ID saves directly.
        if !self.spotify.client_id.is_empty() {
            self.spotify_confirm = Some(SpotifyConfirm::ClientId(text));
            return;
        }
        self.change_with(&Row::ClientId, |app| app.set_client_id(text));
    }

    /// Set the client ID. Returns whether it changed.
    pub(super) fn set_client_id(&mut self, id: String) -> bool {
        let changed = self.spotify.client_id != id;
        self.spotify.client_id = id;
        changed
    }

    /// Send the player its copy of `[spotify]` after a player-side row saved.
    /// Called only from `save_setting`, after the file was written. An empty
    /// client ID in that copy logs the user out and deletes the token, so the
    /// copy is never sent while the config has a problem, even if a caller
    /// forgets the order.
    fn send_spotify_config(&self, row: &Row) {
        if self.config_problem.is_some() || !spotify_rows::sent_to_player(row) {
            return;
        }
        self.ask_spotify(
            SpotifyAsk::Configure,
            SpotifyOp::Configure {
                config: self.spotify.clone(),
            },
        );
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
                if *row == Row::OpenBrowser {
                    let new = settings::step(
                        &spotify_rows::OPEN_BROWSERS,
                        &self.open_browser,
                        forward,
                        wrap,
                    );
                    let changed = new != self.open_browser;
                    self.open_browser = new;
                    return changed;
                }
                if *row == Row::PlaybackDevice {
                    return self.step_playback_device(forward, wrap);
                }
                if *row == Row::CoverProtocol {
                    let choices = settings::protocol_choices(self.protocol_support);
                    settings::step_protocol(&mut self.tui, &choices, forward, wrap)
                } else {
                    settings::step_choice(row, &mut self.tui, forward, wrap)
                }
            }
            // Enter and Space open the typed-value prompt (`open_slider_prompt`);
            // here only the arrows step, one grid value at a time, and stop at the ends.
            Kind::Slider(slider) => {
                let forward = match change {
                    Change::Forward => true,
                    Change::Back => false,
                    Change::Activate => return false,
                };
                let Some(slot) = settings::fade_slot(row, &mut self.fades) else {
                    return false;
                };
                let new = settings::step_slider(slider, *slot, forward);
                let changed = *slot != new;
                *slot = new;
                changed
            }
            // Actions and folder rows are run by `on_settings_key`, not changed here.
            Kind::Action | Kind::Folder => false,
            // Typed rows are set by their prompt, not by the arrows.
            Kind::Text | Kind::Number => false,
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
            Row::FadePlay
            | Row::FadePause
            | Row::FadeStop
            | Row::FadeTrack
            | Row::FadeMute
            | Row::Crossfade => self.apply_fades(),
            _ => {}
        }
    }

    /// Send all five fade times to the player, the way volume and mute go
    /// (`App::apply`); the player applies them at once.
    fn apply_fades(&mut self) {
        self.apply(Command::SetFades(self.fades), None);
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
        if let Some(slot) = settings::fade_slot(row, &mut self.fades) {
            return Some(Stored::Int(slot.as_millis() as i64));
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
            Row::ClientId if self.spotify.client_id.is_empty() => Stored::Remove,
            Row::ClientId => Stored::Text(self.spotify.client_id.clone()),
            Row::RedirectPort => Stored::Int(i64::from(self.spotify.redirect_port)),
            Row::OpenBrowser => Stored::Str(spotify_rows::open_browser_name(self.open_browser)),
            Row::PlaybackDevice if self.spotify.device.is_empty() => Stored::Remove,
            Row::PlaybackDevice => Stored::Text(self.spotify.device.clone()),
            _ => return None,
        })
    }

    /// Write a changed row to the config file. Without a usable file the change
    /// stays for this run and the user is told why.
    fn save_setting(&mut self, row: &Row, before: SettingsSnapshot) {
        let (Some((section, key)), Some(value)) = (row.config_key(), self.stored_value(row)) else {
            return;
        };
        // A Spotify row that is not written goes back to its old value: the
        // screen must not show a client ID or device the player does not have.
        let spotify = row.tab() == Tab::Spotify;
        let Some(file) = self.config_file.clone() else {
            if spotify {
                self.restore_settings(before);
                self.toasts.warn("Not saved: no config file path");
                return;
            }
            if !self.run_only_warned {
                self.run_only_warned = true;
                self.toasts
                    .info("Settings apply to this run only (no config file path)");
            }
            return;
        };
        let problem_text = self
            .config_problem
            .as_ref()
            .map(|problem| match problem.location() {
                Some(place) => format!("Not saved: config.toml has an error ({place})"),
                None => "Not saved: config.toml could not be read".to_string(),
            });
        if let Some(text) = problem_text {
            if spotify {
                self.restore_settings(before);
            }
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
        match written {
            Err(e) => {
                self.restore_settings(before);
                self.toasts.error(format!("Could not save settings: {e}"));
            }
            // The only place Configure is sent: the file holds the change now.
            Ok(()) => self.send_spotify_config(row),
        }
    }

    fn settings_snapshot(&self) -> SettingsSnapshot {
        SettingsSnapshot {
            tui: self.tui,
            bit_perfect: self.bit_perfect,
            update_check: self.check_for_updates,
            default_device: self.default_device.clone(),
            protocol: self.picker.as_ref().map(|p| p.protocol_type()),
            fades: self.fades,
            spotify: self.spotify.clone(),
            open_browser: self.open_browser,
        }
    }

    fn restore_settings(&mut self, before: SettingsSnapshot) {
        self.tui = before.tui;
        self.bit_perfect = before.bit_perfect;
        self.check_for_updates = before.update_check;
        self.default_device = before.default_device;
        self.spotify = before.spotify;
        self.open_browser = before.open_browser;
        if let (Some(picker), Some(protocol)) = (self.picker.as_mut(), before.protocol) {
            picker.set_protocol_type(protocol);
        }
        if self.fades != before.fades {
            // The player already has the new times; give it the old ones back.
            self.fades = before.fades;
            self.apply_fades();
        }
        self.cover_draw_key = None;
        self.library.set_preferred_layout(self.tui.library_layout);
    }

    /// A removal question, a slider prompt and a Spotify question belong to the
    /// Settings modal: once that modal is no longer on screen (closed, or
    /// replaced by another modal), they go. The login panel goes too. The login
    /// itself keeps running in the player.
    pub(super) fn drop_removal_outside_settings(&mut self) {
        if self.modal != Modal::Settings {
            self.settings_confirm = None;
            self.settings_prompt = None;
            self.spotify_confirm = None;
            self.spotify_login = None;
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::path::Path;
    use std::sync::Arc;
    use std::time::Duration;

    use crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
    use znicz_core::{AudioDeviceInfo, LibraryHost, LibraryOp, LibraryStatus, PlayerHandle};
    use znicz_library::Library;

    use super::*;
    use crate::app::SpotifyConfirm;
    use crate::config_file::{ConfigFile, ConfigProblem};
    use crate::library_pane::{EffectiveLayout, LibraryPane};
    use crate::spotify_worker::SpotifyWorker;
    use znicz_core::SpotifyLogin;
    use znicz_core::{SpotifyConfig, SpotifyDevice, SpotifyOp};
    use znicz_spotify::browser::OpenBrowser;

    fn test_player() -> PlayerHandle {
        let (player, _thread) = znicz_core::spawn_player(znicz_core::AudioConfig::default());
        player
    }

    pub(crate) fn toast_texts(app: &App) -> Vec<String> {
        app.toasts.recent().iter().map(|t| t.text.clone()).collect()
    }

    pub(crate) fn press(app: &mut App, code: KeyCode) {
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

    use crate::settings::{Row, SettingsValues, Tab};
    use crate::theme;
    use crate::tui_config::{CoverSize, LibraryLayout};
    use znicz_core::FadeTimes;

    pub(crate) fn settings_app(dir: &tempfile::TempDir) -> App {
        App::with_library(test_player(), None)
            .with_config_file(ConfigFile::new(dir.path().join("config.toml")))
    }

    /// Put the Settings cursor on `row`, switching to its tab first, as pressing
    /// keys to reach it would.
    fn select(app: &mut App, row: Row) {
        app.settings_tab = row.tab();
        app.settings_rows = settings::rows(app.settings_tab, &app.library_status.folders);
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
    fn cursor_moves_within_the_tab_and_stops_at_the_ends() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = settings_app(&dir);
        press(&mut app, KeyCode::Char('S'));
        assert_eq!(app.settings_rows[app.settings_cursor], Row::ShowCover);
        press(&mut app, KeyCode::Up);
        assert_eq!(
            app.settings_rows[app.settings_cursor],
            Row::ShowCover,
            "the cursor stays on the first row"
        );
        assert!(
            app.settings_on_tab_bar,
            "k on the first row focuses the tab bar"
        );
        press(&mut app, KeyCode::Down);
        assert!(!app.settings_on_tab_bar);
        for _ in 0..6 {
            press(&mut app, KeyCode::Char('j'));
        }
        assert_eq!(app.settings_rows[app.settings_cursor], Row::Icons);
        press(&mut app, KeyCode::Char('j'));
        assert_eq!(
            app.settings_rows[app.settings_cursor],
            Row::Icons,
            "j at the bottom stays put"
        );
        press(&mut app, KeyCode::Char('k'));
        assert_eq!(app.settings_rows[app.settings_cursor], Row::Marquee);
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

        // Row 0 is Show cover; row 1 is Cover size.
        click(&mut app, hit.inner.x + 1, hit.inner.y + 1);
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
        assert_eq!(
            app.settings_tab,
            Tab::Audio,
            "back on the tab that opened it"
        );
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
                if index != avoid {
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
        select(&mut app, Row::Folder(PathBuf::from("/music/a")));
        let folder = app
            .settings_rows
            .iter()
            .position(|r| matches!(r, Row::Folder(_)))
            .unwrap();
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
        // The Library tab's last row is Rescan all folders; Enter there rescans
        // and must not remove the folder.
        assert_eq!(app.settings_rows[app.settings_cursor], Row::RescanAll);
        press(&mut app, KeyCode::Enter);

        assert!(
            !sent(&host)
                .iter()
                .any(|op| matches!(op, LibraryOp::RemoveFolder { .. })),
            "Enter must not remove the folder"
        );
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

    // --- settings: tabs and focus ---

    #[test]
    fn tab_and_shift_tab_switch_tabs_and_wrap() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = settings_app(&dir);
        press(&mut app, KeyCode::Char('S'));
        assert_eq!(app.settings_tab, Tab::Display);
        press(&mut app, KeyCode::Tab);
        assert_eq!(app.settings_tab, Tab::Audio);
        assert_eq!(app.settings_rows[app.settings_cursor], Row::DefaultDevice);
        press(&mut app, KeyCode::BackTab);
        press(&mut app, KeyCode::BackTab);
        assert_eq!(app.settings_tab, Tab::Spotify);
    }

    #[test]
    fn the_right_arrow_stops_at_the_last_tab_spotify() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = settings_app(&dir);
        press(&mut app, KeyCode::Char('S'));
        press(&mut app, KeyCode::Char('6'));
        assert_eq!(app.settings_tab, Tab::Spotify);
        // The tab has no rows yet, so there is no row to press Up on.
        app.settings_on_tab_bar = true;
        press(&mut app, KeyCode::Right);
        assert_eq!(app.settings_tab, Tab::Spotify);
    }

    #[test]
    fn switching_tabs_drops_a_pending_folder_removal() {
        let dir = tempfile::tempdir().unwrap();
        let (mut app, host) = host_app(&dir, &["/music/jazz"]);
        press(&mut app, KeyCode::Char('S'));
        press(&mut app, KeyCode::Char('4'));
        press(&mut app, KeyCode::Char('d'));
        assert!(app.settings_confirm.is_some());
        press(&mut app, KeyCode::Tab);
        press(&mut app, KeyCode::BackTab);
        assert!(app.settings_confirm.is_none());
        press(&mut app, KeyCode::Enter);
        assert!(
            !sent(&host)
                .iter()
                .any(|op| matches!(op, LibraryOp::RemoveFolder { .. })),
            "nothing was removed"
        );
    }

    #[test]
    fn the_shown_tabs_number_keeps_the_cursor() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = settings_app(&dir);
        press(&mut app, KeyCode::Char('S'));
        press(&mut app, KeyCode::Down);
        press(&mut app, KeyCode::Down);
        press(&mut app, KeyCode::Char('1'));
        assert_eq!(app.settings_cursor, 2);
    }

    #[test]
    fn number_keys_jump_to_a_tab() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = settings_app(&dir);
        press(&mut app, KeyCode::Char('S'));
        press(&mut app, KeyCode::Char('3'));
        assert_eq!(app.settings_tab, Tab::Playback);
        assert_eq!(app.settings_rows[0], Row::FadePlay);
    }

    #[test]
    fn up_on_the_first_row_focuses_the_tab_bar_where_arrows_switch_tabs() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = settings_app(&dir);
        press(&mut app, KeyCode::Char('S'));
        press(&mut app, KeyCode::Up);
        assert!(app.settings_on_tab_bar);
        press(&mut app, KeyCode::Right);
        assert_eq!(app.settings_tab, Tab::Audio);
        press(&mut app, KeyCode::Char('l'));
        assert_eq!(app.settings_tab, Tab::Playback);
        press(&mut app, KeyCode::Down);
        assert!(!app.settings_on_tab_bar);
        assert_eq!(app.settings_cursor, 0);
    }

    #[test]
    fn arrows_on_the_tab_bar_stop_at_the_ends() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = settings_app(&dir);
        press(&mut app, KeyCode::Char('S'));
        press(&mut app, KeyCode::Up);
        press(&mut app, KeyCode::Left);
        assert_eq!(app.settings_tab, Tab::Display);
        press(&mut app, KeyCode::Char('6'));
        press(&mut app, KeyCode::Up);
        press(&mut app, KeyCode::Right);
        assert_eq!(app.settings_tab, Tab::Spotify);
    }

    #[test]
    fn row_keys_do_nothing_on_the_tab_bar() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = settings_app(&dir);
        press(&mut app, KeyCode::Char('S'));
        let before = app.tui.show_cover;
        press(&mut app, KeyCode::Up);
        press(&mut app, KeyCode::Char(' '));
        assert_eq!(app.tui.show_cover, before);
        assert_eq!(app.modal, Modal::Settings);
    }

    #[test]
    fn esc_and_help_work_from_the_tab_bar() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = settings_app(&dir);
        press(&mut app, KeyCode::Char('S'));
        press(&mut app, KeyCode::Up);
        press(&mut app, KeyCode::Char('?'));
        assert_eq!(app.modal, Modal::Help);

        let mut app = settings_app(&dir);
        press(&mut app, KeyCode::Char('S'));
        press(&mut app, KeyCode::Up);
        press(&mut app, KeyCode::Esc);
        assert_eq!(app.modal, Modal::None);
    }

    #[test]
    fn a_footer_click_on_the_tab_hint_switches_tabs() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = settings_app(&dir);
        press(&mut app, KeyCode::Char('S'));
        app.hits.footer_hints = vec![crate::hit::FooterHit {
            rect: ratatui::layout::Rect::new(0, 0, 10, 1),
            key: KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE),
        }];
        click(&mut app, 0, 0);
        assert_eq!(app.settings_tab, Tab::Audio);
    }

    #[test]
    fn a_fade_change_saves_and_reaches_the_player() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = settings_app(&dir);
        press(&mut app, KeyCode::Char('S'));
        press(&mut app, KeyCode::Char('3'));
        press(&mut app, KeyCode::Char('l')); // play 150 -> 200
        assert_eq!(app.fades.play, Duration::from_millis(200));
        let saved = saved(&dir);
        assert_eq!(saved["audio"]["fade"]["play_ms"].as_integer(), Some(200));
    }

    #[test]
    fn fade_rows_stop_at_the_ends() {
        let dir = tempfile::tempdir().unwrap();
        let fades = FadeTimes {
            play: Duration::from_millis(2000),
            ..FadeTimes::default()
        };
        let mut app = settings_app(&dir).with_settings_values(SettingsValues {
            fades,
            ..SettingsValues::default()
        });
        press(&mut app, KeyCode::Char('S'));
        press(&mut app, KeyCode::Char('3'));
        press(&mut app, KeyCode::Char('l'));
        assert_eq!(app.fades.play, Duration::from_millis(2000), "stops at 2 s");
        press(&mut app, KeyCode::Char('h'));
        assert_eq!(app.fades.play, Duration::from_millis(1950), "one step back");
        assert_eq!(
            saved(&dir)["audio"]["fade"]["play_ms"].as_integer(),
            Some(1950)
        );
    }

    #[test]
    fn a_failed_fade_save_restores_the_time() {
        let dir = tempfile::tempdir().unwrap();
        let blocker = dir.path().join("not-a-folder");
        std::fs::write(&blocker, "a file").unwrap();
        let mut app = App::with_library(test_player(), None)
            .with_config_file(ConfigFile::new(blocker.join("config.toml")));
        press(&mut app, KeyCode::Char('S'));
        press(&mut app, KeyCode::Char('3'));
        press(&mut app, KeyCode::Char('l'));
        assert_eq!(
            app.fades.play,
            FadeTimes::default().play,
            "the old time is back"
        );
    }

    #[test]
    fn the_last_tab_is_kept_for_the_next_opening() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = settings_app(&dir);
        press(&mut app, KeyCode::Char('S'));
        press(&mut app, KeyCode::Char('4'));
        press(&mut app, KeyCode::Esc);
        press(&mut app, KeyCode::Char('S'));
        assert_eq!(app.settings_tab, Tab::Library);
        assert!(!app.settings_on_tab_bar);
    }

    #[test]
    fn the_tab_bar_shows_every_tab() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = settings_app(&dir);
        press(&mut app, KeyCode::Char('S'));
        let screen = screen_rows(&mut app).join("\n");
        for label in [
            "Display", "Audio", "Playback", "Library", "General", "Spotify",
        ] {
            assert!(screen.contains(label), "{label} missing:\n{screen}");
        }
    }

    #[test]
    fn the_modal_keeps_its_height_across_tabs() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = settings_app(&dir);
        press(&mut app, KeyCode::Char('S'));
        let _ = screen_rows(&mut app);
        let display = app.hits.overlay;
        press(&mut app, KeyCode::Char('5'));
        let _ = screen_rows(&mut app);
        assert_eq!(app.hits.overlay, display);
    }

    /// The frame's cells, for checking how a frame is styled.
    fn screen_buffer(app: &mut App) -> ratatui::buffer::Buffer {
        let state = app.player.state();
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 30)).unwrap();
        terminal
            .draw(|frame| crate::views::render(frame, app, &state))
            .unwrap();
        terminal.backend().buffer().clone()
    }

    #[test]
    fn the_cursor_shows_on_the_rows_or_on_the_tab_bar_never_both() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = settings_app(&dir);
        press(&mut app, KeyCode::Char('S'));
        let cursor_bg = theme::selected().bg.expect("the cursor has a background");
        let cursor = screen_buffer(&mut app);
        let popup = app.hits.overlay.expect("the modal is drawn");
        let tab = (popup.x + 2, popup.y + 1);
        let cursor_row = |app: &App| {
            let hit = app.hits.overlay_list.expect("the list is drawn");
            (hit.inner.x + 1, hit.inner.y)
        };
        let (x, y) = cursor_row(&app);
        assert_eq!(cursor[(x, y)].bg, cursor_bg, "rows have the cursor");
        assert_ne!(cursor[tab].bg, cursor_bg, "the tab bar has none");

        press(&mut app, KeyCode::Up);
        let on_bar = screen_buffer(&mut app);
        let (x, y) = cursor_row(&app);
        assert_ne!(on_bar[(x, y)].bg, cursor_bg, "rows lost the cursor");
        assert_eq!(on_bar[tab].bg, cursor_bg, "the tab bar has it");
    }

    #[test]
    fn the_tab_bar_draws_without_panicking_in_tiny_terminals() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = settings_app(&dir);
        press(&mut app, KeyCode::Char('S'));
        let state = app.player.state();
        for (width, height) in [(1, 1), (3, 2), (10, 3), (20, 5), (40, 10), (60, 4)] {
            let mut terminal =
                ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height)).unwrap();
            terminal
                .draw(|frame| crate::views::render(frame, &mut app, &state))
                .unwrap();
        }
    }

    #[test]
    fn a_click_on_a_tab_label_shows_that_tab() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = settings_app(&dir);
        press(&mut app, KeyCode::Char('S'));
        let buffer = screen_buffer(&mut app);
        for (rect, tab) in app.hits.settings_tabs.clone() {
            let first = tab.label().chars().next().unwrap().to_string();
            assert_eq!(buffer[(rect.x, rect.y)].symbol(), first, "{tab:?} is drawn");
        }

        let (rect, _) = app
            .hits
            .settings_tabs
            .iter()
            .copied()
            .find(|(_, tab)| *tab == Tab::Audio)
            .expect("Audio has a label");
        click(&mut app, rect.x, rect.y);
        assert_eq!(app.settings_tab, Tab::Audio);
        assert!(!app.settings_on_tab_bar, "focus stays on the rows");
    }

    /// Settings on the Playback tab with the Crossfade prompt open.
    fn open_crossfade_prompt(app: &mut App) {
        press(app, KeyCode::Char('S'));
        press(app, KeyCode::Char('3'));
        press(app, KeyCode::Char('G'));
        press(app, KeyCode::Enter);
        assert!(app.settings_prompt.is_some());
    }

    // --- settings: slider prompt ---

    /// Type each character of `text` as a key press.
    fn type_text(app: &mut App, text: &str) {
        for c in text.chars() {
            press(app, KeyCode::Char(c));
        }
    }

    #[test]
    fn enter_on_a_slider_opens_a_prompt_and_saves_a_typed_value() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = settings_app(&dir);
        press(&mut app, KeyCode::Char('S'));
        press(&mut app, KeyCode::Char('3'));
        press(&mut app, KeyCode::Char('G')); // Crossfade, last row
        press(&mut app, KeyCode::Enter);
        assert!(app.settings_prompt.is_some());
        for _ in 0..4 {
            press(&mut app, KeyCode::Backspace);
        }
        type_text(&mut app, "3.5");
        press(&mut app, KeyCode::Enter);
        assert!(app.settings_prompt.is_none());
        assert_eq!(app.fades.crossfade, Duration::from_millis(3500));
        assert_eq!(
            saved(&dir)["audio"]["fade"]["crossfade_ms"].as_integer(),
            Some(3500)
        );
    }

    #[test]
    fn a_bad_value_keeps_the_prompt_open_with_the_range() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = settings_app(&dir);
        press(&mut app, KeyCode::Char('S'));
        press(&mut app, KeyCode::Char('3'));
        press(&mut app, KeyCode::Char('G'));
        press(&mut app, KeyCode::Enter);
        for _ in 0..4 {
            press(&mut app, KeyCode::Backspace);
        }
        type_text(&mut app, "20");
        press(&mut app, KeyCode::Enter);

        assert!(app.settings_prompt.is_some(), "the prompt stays open");
        assert_eq!(app.fades.crossfade, Duration::ZERO, "nothing changed");
        let screen = screen_rows(&mut app).join("\n");
        assert!(screen.contains("0–12 s"), "{screen}");
    }

    #[test]
    fn paging_keys_leave_the_row_alone_while_a_value_is_typed() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = settings_app(&dir);
        press(&mut app, KeyCode::Char('S'));
        press(&mut app, KeyCode::Char('3'));
        press(&mut app, KeyCode::Enter);
        assert!(app.settings_prompt.is_some());
        app.on_key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL));
        assert_eq!(app.settings_cursor, 0);
        assert!(app.settings_prompt.is_some());
    }

    #[test]
    fn esc_closes_the_prompt_without_saving_and_keeps_settings_open() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = settings_app(&dir);
        press(&mut app, KeyCode::Char('S'));
        press(&mut app, KeyCode::Char('3'));
        press(&mut app, KeyCode::Enter); // Fade in, the first row
        assert!(app.settings_prompt.is_some());
        for _ in 0..3 {
            press(&mut app, KeyCode::Backspace);
        }
        type_text(&mut app, "999");
        press(&mut app, KeyCode::Esc);

        assert!(app.settings_prompt.is_none());
        assert_eq!(app.fades.play, FadeTimes::default().play, "value unchanged");
        assert_eq!(app.modal, Modal::Settings, "Esc closed only the prompt");
        press(&mut app, KeyCode::Esc);
        assert_eq!(app.modal, Modal::None, "a second Esc closes Settings");
    }

    #[test]
    fn keys_type_into_the_prompt_instead_of_switching_tabs() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = settings_app(&dir);
        press(&mut app, KeyCode::Char('S'));
        press(&mut app, KeyCode::Char('3'));
        press(&mut app, KeyCode::Enter);
        press(&mut app, KeyCode::Char('2'));
        press(&mut app, KeyCode::Tab);
        assert_eq!(
            app.settings_tab,
            Tab::Playback,
            "the digit and Tab did not switch"
        );
        let text = app
            .settings_prompt
            .as_ref()
            .unwrap()
            .text
            .as_str()
            .to_string();
        assert_eq!(text, "1502");

        press(&mut app, KeyCode::Char('h'));
        press(&mut app, KeyCode::Char('l'));
        assert_eq!(app.settings_tab, Tab::Playback);
        let text = app
            .settings_prompt
            .as_ref()
            .unwrap()
            .text
            .as_str()
            .to_string();
        assert_eq!(text, "1502hl");
    }

    #[test]
    fn a_slider_row_draws_a_bar_and_its_value() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = settings_app(&dir);
        press(&mut app, KeyCode::Char('S'));
        press(&mut app, KeyCode::Char('3'));
        let screen = screen_rows(&mut app);
        let row = screen
            .iter()
            .find(|line| line.contains("Fade on stop"))
            .expect("row drawn");
        assert!(row.contains("━"), "{row}");
        assert!(row.contains("300 ms"), "{row}");
    }

    #[test]
    fn the_prompt_strip_shows_the_label_range_and_caret() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = settings_app(&dir);
        open_crossfade_prompt(&mut app);
        let screen = screen_rows(&mut app).join("\n");
        assert!(screen.contains("Crossfade (0–12 s): 0█"), "{screen}");
        assert!(!screen.contains("Overlap the end of a track"), "{screen}");
    }

    #[test]
    fn a_prompt_does_not_outlive_its_settings_modal() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = settings_app(&dir);
        open_crossfade_prompt(&mut app);
        app.modal = Modal::None;
        press(&mut app, KeyCode::F(12));
        assert!(app.settings_prompt.is_none());
    }

    #[test]
    fn the_wheel_does_not_move_the_cursor_while_a_prompt_is_open() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = settings_app(&dir);
        open_crossfade_prompt(&mut app);
        let before = app.settings_cursor;
        app.on_mouse(MouseEvent {
            kind: MouseEventKind::ScrollUp,
            column: 0,
            row: 0,
            modifiers: KeyModifiers::NONE,
        });
        assert_eq!(app.settings_cursor, before);
        assert!(app.settings_prompt.is_some());
    }

    // --- Spotify rows: saves, Configure ---

    fn spotify_app(
        dir: &tempfile::TempDir,
        host: Arc<crate::app::spotify_ui::tests::FakeHost>,
    ) -> App {
        let mut app = settings_app(dir).with_settings_values(SettingsValues {
            spotify: SpotifyConfig {
                client_id: "0123456789abcdef0123456789abcdef".into(),
                ..SpotifyConfig::default()
            },
            ..SettingsValues::default()
        });
        app.spotify_worker = Some(SpotifyWorker::spawn(host.clone()).unwrap());
        app.spotify_main = Some(host);
        app
    }

    fn sent_ops(host: &crate::app::spotify_ui::tests::FakeHost) -> Vec<SpotifyOp> {
        // The worker is asynchronous: wait until it has been quiet for one
        // sample (at least 50 ms, at most 2 s), then read what it was given.
        let mut last = usize::MAX;
        for _ in 0..80 {
            std::thread::sleep(Duration::from_millis(25));
            let now = host.ops.lock().unwrap().len();
            if now == last {
                break;
            }
            last = now;
        }
        host.ops.lock().unwrap().clone()
    }

    fn one_device(name: &str) -> Vec<SpotifyDevice> {
        vec![SpotifyDevice {
            id: "d1".into(),
            name: name.into(),
            kind: "Speaker".into(),
            is_active: false,
            volume_percent: None,
        }]
    }

    #[test]
    fn enter_steps_the_open_browser_setting_and_saves_it_without_sending_it() {
        let dir = tempfile::tempdir().unwrap();
        let host = Arc::new(crate::app::spotify_ui::tests::FakeHost::default());
        let mut app = spotify_app(&dir, host.clone());
        press(&mut app, KeyCode::Char('S'));
        press(&mut app, KeyCode::Char('6'));
        select(&mut app, Row::OpenBrowser);
        press(&mut app, KeyCode::Enter);
        assert_eq!(
            saved(&dir)["spotify"]["open_browser"].as_str(),
            Some("always")
        );
        assert!(
            sent_ops(&host)
                .iter()
                .all(|op| !matches!(op, SpotifyOp::Configure { .. }))
        );
    }

    #[test]
    fn stepping_the_device_saves_it_and_sends_configure() {
        let dir = tempfile::tempdir().unwrap();
        let host = Arc::new(crate::app::spotify_ui::tests::FakeHost::default());
        let mut app = spotify_app(&dir, host.clone());
        app.spotify_devices = one_device("Kitchen");
        press(&mut app, KeyCode::Char('S'));
        press(&mut app, KeyCode::Char('6'));
        select(&mut app, Row::PlaybackDevice);
        press(&mut app, KeyCode::Enter);
        assert_eq!(saved(&dir)["spotify"]["device"].as_str(), Some("Kitchen"));
        let ops = sent_ops(&host);
        assert!(ops.iter().any(|op| matches!(
            op,
            SpotifyOp::Configure { config } if config.device == "Kitchen"
        )));
    }

    #[test]
    fn choosing_the_active_device_removes_the_key() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("config.toml"),
            "[spotify]\ndevice = \"Kitchen\"\n",
        )
        .unwrap();
        let host = Arc::new(crate::app::spotify_ui::tests::FakeHost::default());
        let mut app = settings_app(&dir).with_settings_values(SettingsValues {
            spotify: SpotifyConfig {
                client_id: "0123456789abcdef0123456789abcdef".into(),
                device: "Kitchen".into(),
                ..SpotifyConfig::default()
            },
            ..SettingsValues::default()
        });
        app.spotify_worker = Some(SpotifyWorker::spawn(host.clone()).unwrap());
        app.spotify_devices = one_device("Kitchen");
        press(&mut app, KeyCode::Char('S'));
        press(&mut app, KeyCode::Char('6'));
        select(&mut app, Row::PlaybackDevice);
        press(&mut app, KeyCode::Left);
        assert!(saved(&dir)["spotify"].get("device").is_none());
        assert!(app.spotify.device.is_empty());
    }

    #[test]
    fn a_port_typed_in_the_prompt_is_saved_and_sent() {
        let dir = tempfile::tempdir().unwrap();
        let host = Arc::new(crate::app::spotify_ui::tests::FakeHost::default());
        let mut app = spotify_app(&dir, host.clone());
        press(&mut app, KeyCode::Char('S'));
        press(&mut app, KeyCode::Char('6'));
        select(&mut app, Row::RedirectPort);
        press(&mut app, KeyCode::Enter);
        // Replace the default 8898 with 9001.
        for _ in 0..4 {
            press(&mut app, KeyCode::Backspace);
        }
        type_text(&mut app, "9001");
        press(&mut app, KeyCode::Enter);
        assert_eq!(
            saved(&dir)["spotify"]["redirect_port"].as_integer(),
            Some(9001)
        );
        assert!(sent_ops(&host).iter().any(|op| matches!(
            op,
            SpotifyOp::Configure { config } if config.redirect_port == 9001
        )));
    }

    #[test]
    fn a_port_out_of_range_keeps_the_prompt_open() {
        let dir = tempfile::tempdir().unwrap();
        let host = Arc::new(crate::app::spotify_ui::tests::FakeHost::default());
        let mut app = spotify_app(&dir, host);
        press(&mut app, KeyCode::Char('S'));
        press(&mut app, KeyCode::Char('6'));
        select(&mut app, Row::RedirectPort);
        press(&mut app, KeyCode::Enter);
        for _ in 0..4 {
            press(&mut app, KeyCode::Backspace);
        }
        type_text(&mut app, "80");
        press(&mut app, KeyCode::Enter);
        let prompt = app.settings_prompt.as_ref().expect("still open");
        assert_eq!(prompt.error.as_deref(), Some("1024–65535"));
        assert!(!dir.path().join("config.toml").exists());
    }

    #[test]
    fn a_broken_config_is_not_written_and_not_sent_and_the_row_goes_back() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "[spotify\nclient_id = 1\n").unwrap();
        let host = Arc::new(crate::app::spotify_ui::tests::FakeHost::default());
        let mut app = settings_app(&dir)
            .with_config_file(ConfigFile::new(&path))
            .with_config_problem(ConfigProblem::Parse {
                message: "bad".into(),
                line: 1,
                column: 1,
            })
            .with_settings_values(SettingsValues {
                spotify: SpotifyConfig {
                    client_id: "0123456789abcdef0123456789abcdef".into(),
                    ..SpotifyConfig::default()
                },
                ..SettingsValues::default()
            });
        app.spotify_worker = Some(SpotifyWorker::spawn(host.clone()).unwrap());
        press(&mut app, KeyCode::Char('S'));
        press(&mut app, KeyCode::Char('6'));
        select(&mut app, Row::OpenBrowser);
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.open_browser, OpenBrowser::Auto);
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "[spotify\nclient_id = 1\n"
        );
        assert!(
            !sent_ops(&host)
                .iter()
                .any(|op| matches!(op, SpotifyOp::Configure { .. }))
        );
        assert!(toast_texts(&app).iter().any(|t| t.starts_with("Not saved")));
    }

    #[test]
    fn a_broken_config_restores_a_port_and_sends_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "[spotify\nclient_id = 1\n").unwrap();
        let host = Arc::new(crate::app::spotify_ui::tests::FakeHost::default());
        let mut app = settings_app(&dir)
            .with_config_file(ConfigFile::new(&path))
            .with_config_problem(ConfigProblem::Parse {
                message: "bad".into(),
                line: 1,
                column: 1,
            });
        app.spotify_worker = Some(SpotifyWorker::spawn(host.clone()).unwrap());
        press(&mut app, KeyCode::Char('S'));
        press(&mut app, KeyCode::Char('6'));
        select(&mut app, Row::RedirectPort);
        press(&mut app, KeyCode::Enter);
        for _ in 0..4 {
            press(&mut app, KeyCode::Backspace);
        }
        type_text(&mut app, "9001");
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.spotify.redirect_port, 8898);
        assert!(
            !sent_ops(&host)
                .iter()
                .any(|op| matches!(op, SpotifyOp::Configure { .. }))
        );
    }

    #[test]
    fn the_first_save_creates_the_file_and_sends_what_the_player_already_has_plus_the_change() {
        // No config file yet: the app and the player both run on the defaults.
        let dir = tempfile::tempdir().unwrap();
        let host = Arc::new(crate::app::spotify_ui::tests::FakeHost::default());
        let mut app = settings_app(&dir);
        app.spotify_worker = Some(SpotifyWorker::spawn(host.clone()).unwrap());
        assert!(!dir.path().join("config.toml").exists());
        press(&mut app, KeyCode::Char('S'));
        press(&mut app, KeyCode::Char('6'));
        select(&mut app, Row::RedirectPort);
        press(&mut app, KeyCode::Enter);
        for _ in 0..4 {
            press(&mut app, KeyCode::Backspace);
        }
        type_text(&mut app, "9001");
        press(&mut app, KeyCode::Enter);
        assert_eq!(
            saved(&dir)["spotify"]["redirect_port"].as_integer(),
            Some(9001)
        );
        let sent = sent_ops(&host);
        assert!(sent.iter().any(|op| matches!(
            op,
            SpotifyOp::Configure { config }
                if config.redirect_port == 9001 && config.client_id.is_empty() && config.device.is_empty()
        )));
    }

    #[test]
    fn without_a_config_file_path_a_spotify_row_goes_back_and_nothing_is_sent() {
        let dir = tempfile::tempdir().unwrap();
        let host = Arc::new(crate::app::spotify_ui::tests::FakeHost::default());
        let mut app = spotify_app(&dir, host.clone());
        app.config_file = None;
        app.spotify_devices = one_device("Kitchen");
        press(&mut app, KeyCode::Char('S'));
        press(&mut app, KeyCode::Char('6'));
        select(&mut app, Row::PlaybackDevice);
        press(&mut app, KeyCode::Enter);
        assert!(app.spotify.device.is_empty());
        assert!(
            !sent_ops(&host)
                .iter()
                .any(|op| matches!(op, SpotifyOp::Configure { .. }))
        );
        assert!(toast_texts(&app).iter().any(|t| t.starts_with("Not saved")));
    }

    /// Every Spotify change a user can make, one after the other.
    fn change_every_spotify_row(app: &mut App) {
        press(app, KeyCode::Char('S'));
        press(app, KeyCode::Char('6'));
        select(app, Row::OpenBrowser);
        press(app, KeyCode::Enter);
        select(app, Row::PlaybackDevice);
        press(app, KeyCode::Enter);
        select(app, Row::RedirectPort);
        press(app, KeyCode::Enter);
        for _ in 0..4 {
            press(app, KeyCode::Backspace);
        }
        type_text(app, "9001");
        press(app, KeyCode::Enter);
        select(app, Row::ClientId);
        press(app, KeyCode::Enter);
        for _ in 0..ID.len() {
            press(app, KeyCode::Backspace);
        }
        type_text(app, NEW_ID);
        press(app, KeyCode::Enter);
        // A set client ID always asks first, so the change waits for Enter.
        press(app, KeyCode::Enter);
    }

    fn kitchen() -> Vec<SpotifyDevice> {
        vec![SpotifyDevice {
            id: "d1".into(),
            name: "Kitchen".into(),
            kind: "Speaker".into(),
            is_active: false,
            volume_percent: None,
        }]
    }

    #[test]
    fn a_broken_config_never_changes_the_player_through_any_spotify_row() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "[spotify\n").unwrap();
        let host = Arc::new(crate::app::spotify_ui::tests::FakeHost::default());
        let mut app = spotify_app(&dir, host.clone())
            .with_config_file(ConfigFile::new(&path))
            .with_config_problem(ConfigProblem::Parse {
                message: "bad".into(),
                line: 1,
                column: 1,
            });
        app.spotify_devices = kitchen();
        change_every_spotify_row(&mut app);
        assert!(
            !sent_ops(&host)
                .iter()
                .any(|op| matches!(op, SpotifyOp::Configure { .. })),
            "nothing reaches the player"
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "[spotify\n");
        assert_eq!(app.spotify.redirect_port, SpotifyConfig::DEFAULT_PORT);
        assert_eq!(
            app.spotify.client_id, ID,
            "the row shows what the player has"
        );
        assert!(app.spotify.device.is_empty());
        assert_eq!(app.open_browser, OpenBrowser::Auto);
    }

    #[test]
    fn a_clean_config_sends_one_configure_per_player_side_change_and_never_an_empty_id_unasked() {
        let dir = tempfile::tempdir().unwrap();
        let host = Arc::new(crate::app::spotify_ui::tests::FakeHost::default());
        let mut app = spotify_app(&dir, host.clone());
        app.spotify_devices = kitchen();
        change_every_spotify_row(&mut app);
        let configures: Vec<_> = sent_ops(&host)
            .into_iter()
            .filter_map(|op| match op {
                SpotifyOp::Configure { config } => Some(config),
                _ => None,
            })
            .collect();
        // Device, port, client ID: three saves. The browser setting is not sent.
        assert_eq!(configures.len(), 3, "{configures:?}");
        assert!(
            configures.iter().all(|c| !c.client_id.is_empty()),
            "an ID was never blanked"
        );
        assert_eq!(configures.last().unwrap().client_id, NEW_ID);
        assert_eq!(configures.last().unwrap().redirect_port, 9001);
        assert_eq!(configures.last().unwrap().device, "Kitchen");
    }

    // --- Spotify rows: client ID, logout question, Account ---

    const ID: &str = "0123456789abcdef0123456789abcdef";
    const NEW_ID: &str = "fedcba9876543210fedcba9876543210";

    #[test]
    fn a_client_id_that_is_not_32_hex_keeps_the_prompt_open_with_why() {
        let dir = tempfile::tempdir().unwrap();
        let host = Arc::new(crate::app::spotify_ui::tests::FakeHost::default());
        let mut app = spotify_app(&dir, host);
        press(&mut app, KeyCode::Char('S'));
        press(&mut app, KeyCode::Char('6'));
        select(&mut app, Row::ClientId);
        press(&mut app, KeyCode::Enter);
        for _ in 0..ID.len() {
            press(&mut app, KeyCode::Backspace);
        }
        type_text(&mut app, "xyz");
        press(&mut app, KeyCode::Enter);
        let prompt = app.settings_prompt.as_ref().expect("still open");
        assert_eq!(prompt.error.as_deref(), Some("32 hex characters"));
        assert_eq!(app.spotify.client_id, ID);
        assert!(!dir.path().join("config.toml").exists());
    }

    #[test]
    fn clearing_the_client_id_while_logged_in_asks_first_too() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("config.toml"),
            format!("[spotify]\nclient_id = \"{ID}\"\n"),
        )
        .unwrap();
        let host = Arc::new(crate::app::spotify_ui::tests::FakeHost::default());
        let mut app = spotify_app(&dir, host.clone());
        app.spotify_state.login = SpotifyLogin::LoggedIn;
        press(&mut app, KeyCode::Char('S'));
        press(&mut app, KeyCode::Char('6'));
        select(&mut app, Row::ClientId);
        press(&mut app, KeyCode::Enter);
        for _ in 0..ID.len() {
            press(&mut app, KeyCode::Backspace);
        }
        press(&mut app, KeyCode::Enter);
        assert_eq!(
            app.spotify_confirm,
            Some(SpotifyConfirm::ClientId(String::new()))
        );
        assert_eq!(app.spotify.client_id, ID);
        assert!(
            saved(&dir)["spotify"].get("client_id").is_some(),
            "the file still has the ID"
        );
        assert!(
            !sent_ops(&host)
                .iter()
                .any(|op| matches!(op, SpotifyOp::Configure { .. }))
        );
    }

    #[test]
    fn an_empty_client_id_clears_the_key() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("config.toml"),
            format!("[spotify]\nclient_id = \"{ID}\"\n"),
        )
        .unwrap();
        let host = Arc::new(crate::app::spotify_ui::tests::FakeHost::default());
        let mut app = spotify_app(&dir, host.clone());
        press(&mut app, KeyCode::Char('S'));
        press(&mut app, KeyCode::Char('6'));
        select(&mut app, Row::ClientId);
        press(&mut app, KeyCode::Enter);
        for _ in 0..ID.len() {
            press(&mut app, KeyCode::Backspace);
        }
        press(&mut app, KeyCode::Enter);
        // Clearing a set ID asks first, whatever the login state is.
        assert_eq!(
            app.spotify_confirm,
            Some(SpotifyConfirm::ClientId(String::new()))
        );
        assert_eq!(app.spotify.client_id, ID, "nothing changes before Enter");
        assert!(
            saved(&dir)["spotify"].get("client_id").is_some(),
            "the file still has the ID"
        );
        assert!(
            !sent_ops(&host)
                .iter()
                .any(|op| matches!(op, SpotifyOp::Configure { .. }))
        );
        press(&mut app, KeyCode::Enter);
        assert!(app.settings_prompt.is_none());
        assert!(app.spotify.client_id.is_empty());
        assert!(saved(&dir)["spotify"].get("client_id").is_none());
        assert!(sent_ops(&host).iter().any(|op| matches!(
            op,
            SpotifyOp::Configure { config } if config.client_id.is_empty()
        )));
    }

    #[test]
    fn changing_a_set_client_id_asks_first_even_before_the_login_is_known() {
        let dir = tempfile::tempdir().unwrap();
        let host = Arc::new(crate::app::spotify_ui::tests::FakeHost::default());
        let mut app = spotify_app(&dir, host.clone());
        // No poll has run: the login state is still the default.
        assert_eq!(app.spotify_state.login, SpotifyLogin::NotConfigured);
        press(&mut app, KeyCode::Char('S'));
        press(&mut app, KeyCode::Char('6'));
        select(&mut app, Row::ClientId);
        press(&mut app, KeyCode::Enter);
        for _ in 0..ID.len() {
            press(&mut app, KeyCode::Backspace);
        }
        type_text(&mut app, NEW_ID);
        press(&mut app, KeyCode::Enter);
        assert_eq!(
            app.spotify_confirm,
            Some(SpotifyConfirm::ClientId(NEW_ID.into()))
        );
        assert_eq!(app.spotify.client_id, ID);
        assert!(!dir.path().join("config.toml").exists());
        assert!(
            !sent_ops(&host)
                .iter()
                .any(|op| matches!(op, SpotifyOp::Configure { .. }))
        );
    }

    #[test]
    fn setting_the_first_client_id_saves_without_a_question() {
        let dir = tempfile::tempdir().unwrap();
        let host = Arc::new(crate::app::spotify_ui::tests::FakeHost::default());
        let mut app = settings_app(&dir).with_settings_values(SettingsValues::default());
        app.spotify_worker = Some(SpotifyWorker::spawn(host.clone()).unwrap());
        press(&mut app, KeyCode::Char('S'));
        press(&mut app, KeyCode::Char('6'));
        select(&mut app, Row::ClientId);
        press(&mut app, KeyCode::Enter);
        type_text(&mut app, ID);
        press(&mut app, KeyCode::Enter);
        assert!(app.spotify_confirm.is_none());
        assert_eq!(app.spotify.client_id, ID);
        assert_eq!(saved(&dir)["spotify"]["client_id"].as_str(), Some(ID));
        assert!(sent_ops(&host).iter().any(|op| matches!(
            op,
            SpotifyOp::Configure { config } if config.client_id == ID
        )));
    }

    #[test]
    fn a_new_client_id_while_logged_in_asks_first_and_says_it_logs_out() {
        let dir = tempfile::tempdir().unwrap();
        let host = Arc::new(crate::app::spotify_ui::tests::FakeHost::default());
        let mut app = spotify_app(&dir, host.clone());
        app.spotify_state.login = SpotifyLogin::LoggedIn;
        press(&mut app, KeyCode::Char('S'));
        press(&mut app, KeyCode::Char('6'));
        select(&mut app, Row::ClientId);
        press(&mut app, KeyCode::Enter);
        for _ in 0..ID.len() {
            press(&mut app, KeyCode::Backspace);
        }
        type_text(&mut app, NEW_ID);
        press(&mut app, KeyCode::Enter);
        assert_eq!(
            app.spotify_confirm,
            Some(SpotifyConfirm::ClientId(NEW_ID.into()))
        );
        assert_eq!(app.spotify.client_id, ID, "nothing changes before Enter");
        assert!(
            !dir.path().join("config.toml").exists(),
            "nothing is written before Enter"
        );
        assert!(
            !sent_ops(&host)
                .iter()
                .any(|op| matches!(op, SpotifyOp::Configure { .. }))
        );
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.spotify.client_id, NEW_ID);
        assert_eq!(saved(&dir)["spotify"]["client_id"].as_str(), Some(NEW_ID));
        assert!(sent_ops(&host).iter().any(|op| matches!(
            op,
            SpotifyOp::Configure { config } if config.client_id == NEW_ID
        )));
    }

    #[test]
    fn esc_cancels_a_pending_client_id_change() {
        let dir = tempfile::tempdir().unwrap();
        let host = Arc::new(crate::app::spotify_ui::tests::FakeHost::default());
        let mut app = spotify_app(&dir, host);
        app.spotify_state.login = SpotifyLogin::LoggedIn;
        press(&mut app, KeyCode::Char('S'));
        press(&mut app, KeyCode::Char('6'));
        app.spotify_confirm = Some(SpotifyConfirm::ClientId(NEW_ID.into()));
        press(&mut app, KeyCode::Esc);
        assert!(app.spotify_confirm.is_none());
        assert_eq!(app.spotify.client_id, ID);
        assert!(
            app.modal == Modal::Settings,
            "Esc only cancels the question"
        );
    }

    #[test]
    fn enter_on_account_without_a_client_id_does_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let host = Arc::new(crate::app::spotify_ui::tests::FakeHost::default());
        let mut app = settings_app(&dir);
        app.spotify_worker = Some(SpotifyWorker::spawn(host.clone()).unwrap());
        press(&mut app, KeyCode::Char('S'));
        press(&mut app, KeyCode::Char('6'));
        select(&mut app, Row::Account);
        press(&mut app, KeyCode::Enter);
        assert!(app.spotify_login.is_none());
        assert!(app.spotify_confirm.is_none());
        assert!(
            sent_ops(&host)
                .iter()
                .all(|op| !matches!(op, SpotifyOp::BeginLogin))
        );
    }

    #[test]
    fn enter_on_account_when_logged_out_begins_a_login() {
        let dir = tempfile::tempdir().unwrap();
        let host = Arc::new(crate::app::spotify_ui::tests::FakeHost::default());
        let mut app = spotify_app(&dir, host.clone());
        app.spotify_state.login = SpotifyLogin::NotLoggedIn;
        press(&mut app, KeyCode::Char('S'));
        press(&mut app, KeyCode::Char('6'));
        select(&mut app, Row::Account);
        press(&mut app, KeyCode::Enter);
        assert!(sent_ops(&host).contains(&SpotifyOp::BeginLogin));
    }

    #[test]
    fn enter_on_account_when_logged_in_asks_then_logs_out() {
        let dir = tempfile::tempdir().unwrap();
        let host = Arc::new(crate::app::spotify_ui::tests::FakeHost::default());
        let mut app = spotify_app(&dir, host.clone());
        app.spotify_state.login = SpotifyLogin::LoggedIn;
        press(&mut app, KeyCode::Char('S'));
        press(&mut app, KeyCode::Char('6'));
        select(&mut app, Row::Account);
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.spotify_confirm, Some(SpotifyConfirm::Logout));
        assert!(!sent_ops(&host).contains(&SpotifyOp::Logout));
        press(&mut app, KeyCode::Enter);
        assert!(sent_ops(&host).contains(&SpotifyOp::Logout));
    }

    // --- Spotify tab: drawing ---

    #[test]
    fn the_spotify_tab_shows_its_five_rows_and_the_tab_label() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = settings_app(&dir);
        press(&mut app, KeyCode::Char('S'));
        press(&mut app, KeyCode::Char('6'));
        let screen = screen_rows(&mut app).join("\n");
        for text in [
            "Client ID",
            "Redirect port",
            "Open browser",
            "Account",
            "Playback device",
            "Spotify",
        ] {
            assert!(screen.contains(text), "{text} missing:\n{screen}");
        }
    }

    #[test]
    fn without_a_client_id_the_strip_points_at_it() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = settings_app(&dir);
        press(&mut app, KeyCode::Char('S'));
        press(&mut app, KeyCode::Char('6'));
        select(&mut app, Row::Account);
        let screen = screen_rows(&mut app).join("\n");
        assert!(screen.contains("Set a client ID first"), "{screen}");
    }

    #[test]
    fn the_login_panel_shows_the_url_and_the_keys_in_the_list_area() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = settings_app(&dir);
        press(&mut app, KeyCode::Char('S'));
        press(&mut app, KeyCode::Char('6'));
        app.spotify_login = Some(crate::app::LoginPanel {
            url: "https://accounts.spotify.com/authorize?client_id=0123".into(),
            listening: true,
            paste: None,
            notice: None,
        });
        let screen = screen_rows(&mut app).join("\n");
        assert!(
            screen.contains("Open this URL in any browser and approve:"),
            "{screen}"
        );
        assert!(
            screen.contains("accounts.spotify.com/authorize"),
            "{screen}"
        );
        assert!(screen.contains("Waiting for the browser"), "{screen}");
    }

    #[test]
    fn the_panel_draws_in_a_small_terminal_without_panicking() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = settings_app(&dir);
        press(&mut app, KeyCode::Char('S'));
        press(&mut app, KeyCode::Char('6'));
        app.spotify_login = Some(crate::app::LoginPanel {
            url: "https://accounts.spotify.com/authorize?".repeat(8),
            listening: false,
            paste: None,
            notice: Some("Could not open a browser: no opener.".into()),
        });
        let _ = screen_buffer(&mut app);
    }

    #[test]
    fn a_long_url_wraps_in_the_panel_and_none_of_it_is_lost() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = settings_app(&dir);
        press(&mut app, KeyCode::Char('S'));
        press(&mut app, KeyCode::Char('6'));
        let url = format!(
            "https://accounts.spotify.com/authorize?{}",
            "a1b2c3d4".repeat(12)
        );
        app.spotify_login = Some(crate::app::LoginPanel {
            url,
            listening: true,
            paste: None,
            notice: None,
        });
        let screen = screen_rows(&mut app);
        // The URL is one word longer than the popup is wide: it must run over
        // several rows (broken at the width), not be cut at the first.
        let rows_with_url = screen
            .iter()
            .filter(|row| row.contains("a1b2c3d4a1b2c3d4"))
            .count();
        assert!(rows_with_url >= 2, "{}", screen.join("\n"));
    }

    #[test]
    fn the_paste_prompt_shows_in_the_strip_with_the_reason_when_refused() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = settings_app(&dir);
        press(&mut app, KeyCode::Char('S'));
        press(&mut app, KeyCode::Char('6'));
        app.spotify_login = Some(crate::app::LoginPanel {
            url: "u".into(),
            listening: true,
            paste: Some(crate::app::SpotifyPaste {
                text: crate::line_edit::LineEdit::from_text("code=x"),
                error: Some("that address is not from this login".into()),
            }),
            notice: None,
        });
        let screen = screen_rows(&mut app).join("\n");
        assert!(screen.contains("Address: code=x"), "{screen}");
        assert!(
            screen.contains("that address is not from this login"),
            "{screen}"
        );
    }

    #[test]
    fn the_logout_question_and_the_client_id_question_show_in_the_strip() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = settings_app(&dir);
        press(&mut app, KeyCode::Char('S'));
        press(&mut app, KeyCode::Char('6'));
        app.spotify_confirm = Some(crate::app::SpotifyConfirm::Logout);
        assert!(
            screen_rows(&mut app)
                .join("\n")
                .contains("Log out of Spotify?")
        );
        app.spotify_confirm = Some(crate::app::SpotifyConfirm::ClientId("x".into()));
        assert!(
            screen_rows(&mut app)
                .join("\n")
                .contains("Changing the client ID logs you out")
        );
    }
}
