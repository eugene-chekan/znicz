//! The application: what is on screen, and what the keys do.

use std::path::PathBuf;
use std::time::Duration;

use crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyEventKind,
    KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::layout::Rect;
use znicz_core::{
    apply_to_player, list_saved, load_path, sanitize_stem, saved_path, skipped_notice, write_path,
    AudioDeviceInfo, AudioOutput, Command, IpcClient, PlaybackStatus, PlayerEvent, PlayerHandle,
    PlayerOps, PlayerState, RepeatMode,
};
use znicz_library::{Library, Track};

use crate::cover::CoverCache;
use crate::cursor::Cursor;
use crate::hit::HitMap;
use crate::layout;
use crate::library_pane::{Item, LibraryPane};
use crate::line_edit::LineEdit;
use crate::meta::{Entry, MetaCache};
use crate::toast::Toasts;
use crate::tui_config::{CoverProtocol, TuiConfig};
use crate::views;

/// GitHub Windows runners expose WASAPI, but enumerating devices from tests
/// crashes the process. Production (no `CI`) still lists devices at start.
fn load_output_devices() -> Vec<AudioDeviceInfo> {
    if std::env::var_os("CI").is_some() {
        return Vec::new();
    }
    AudioOutput::list_devices().unwrap_or_default()
}

fn make_picker(protocol: CoverProtocol) -> ratatui_image::picker::Picker {
    use ratatui_image::picker::{Picker, ProtocolType};

    let mut picker = match protocol {
        CoverProtocol::Halfblocks | CoverProtocol::Off => Picker::halfblocks(),
        CoverProtocol::Auto | CoverProtocol::Kitty | CoverProtocol::Sixel => {
            Picker::from_query_stdio().unwrap_or_else(|_| Picker::halfblocks())
        }
    };
    match protocol {
        CoverProtocol::Kitty => picker.set_protocol_type(ProtocolType::Kitty),
        CoverProtocol::Sixel => picker.set_protocol_type(ProtocolType::Sixel),
        CoverProtocol::Halfblocks | CoverProtocol::Off => {
            picker.set_protocol_type(ProtocolType::Halfblocks)
        }
        CoverProtocol::Auto => {}
    }
    picker
}

/// Longest wait between redraws while nothing happens. Fast enough for a
/// smooth seek bar, slow enough to stay near zero CPU.
const TICK_RATE: Duration = Duration::from_millis(200);

/// Seek step for the plain and shifted keys.
const SEEK_SMALL: i64 = 5;
const SEEK_LARGE: i64 = 30;
const VOLUME_STEP: f32 = 0.05;

/// Local tests and preview keep an in-process engine. Production talks to
/// `znicz player` over IPC.
pub enum Engine {
    Local(PlayerHandle),
    Remote(IpcClient),
}

impl Engine {
    pub fn send_blocking(&self, command: Command) -> znicz_core::Result<()> {
        PlayerOps::send_blocking(self, command)
    }

    pub fn state(&self) -> PlayerState {
        PlayerOps::state(self)
    }

    pub fn drain_events(&self) -> Vec<PlayerEvent> {
        PlayerOps::drain_events(self)
    }
}

impl PlayerOps for Engine {
    fn send_blocking(&self, command: Command) -> znicz_core::Result<()> {
        match self {
            Self::Local(player) => player.send_blocking(command),
            Self::Remote(player) => player.send_blocking(command),
        }
    }

    fn state(&self) -> PlayerState {
        match self {
            Self::Local(player) => player.state(),
            Self::Remote(player) => player.state().unwrap_or_else(|e| {
                tracing::warn!("ipc state: {e}");
                PlayerState::default()
            }),
        }
    }

    fn drain_events(&self) -> Vec<PlayerEvent> {
        match self {
            Self::Local(player) => player.drain_events(),
            Self::Remote(_) => Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Library,
    Queue,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Modal {
    None,
    Help,
    Devices,
    Inspector,
    Playlists,
    Radio,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StationField {
    Name,
    Url,
    Art,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RadioPrompt {
    /// New station (`original` is None) or edit of that name.
    Form {
        name: LineEdit,
        url: LineEdit,
        art: LineEdit,
        field: StationField,
        original: Option<String>,
    },
    Copy(LineEdit),
}

impl RadioPrompt {
    fn new_station() -> Self {
        Self::Form {
            name: LineEdit::new(),
            url: LineEdit::new(),
            art: LineEdit::new(),
            field: StationField::Name,
            original: None,
        }
    }

    fn edit_station(station: &znicz_core::Station) -> Self {
        Self::Form {
            name: LineEdit::from_text(station.name.clone()),
            url: LineEdit::from_text(station.url.clone()),
            art: LineEdit::from_text(
                station
                    .art
                    .as_ref()
                    .map(|p| p.display().to_string())
                    .unwrap_or_default(),
            ),
            field: StationField::Name,
            original: Some(station.name.clone()),
        }
    }

    fn buffer_mut(&mut self) -> &mut LineEdit {
        match self {
            Self::Form {
                name,
                url,
                art,
                field,
                ..
            } => match field {
                StationField::Name => name,
                StationField::Url => url,
                StationField::Art => art,
            },
            Self::Copy(s) => s,
        }
    }

    fn cycle_field(&mut self, forward: bool) {
        if let Self::Form { field, .. } = self {
            *field = match (*field, forward) {
                (StationField::Name, true) => StationField::Url,
                (StationField::Url, true) => StationField::Art,
                (StationField::Art, true) => StationField::Name,
                (StationField::Name, false) => StationField::Art,
                (StationField::Url, false) => StationField::Name,
                (StationField::Art, false) => StationField::Url,
            };
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlaylistPrompt {
    Save(LineEdit),
    Rename(LineEdit),
    Copy(LineEdit),
}

impl PlaylistPrompt {
    pub fn as_str(&self) -> &str {
        match self {
            Self::Save(s) | Self::Rename(s) | Self::Copy(s) => s.as_str(),
        }
    }

    fn buffer_mut(&mut self) -> &mut LineEdit {
        match self {
            Self::Save(s) | Self::Rename(s) | Self::Copy(s) => s,
        }
    }
}

impl Modal {
    pub(crate) fn blocks_list_focus(self) -> bool {
        matches!(
            self,
            Self::Devices | Self::Inspector | Self::Playlists | Self::Radio
        )
    }
}

pub struct App {
    pub player: Engine,
    pub focus: Focus,
    pub queue_open: bool,
    pub modal: Modal,
    pub list_width: u16,
    pub list_height: u16,
    pub title_slot: usize,
    pub queue_h_offset: usize,
    pub queue_title_slot: usize,
    pub queue_cursor: Cursor,
    pub library: LibraryPane,
    pub hits: HitMap,
    pub devices: Vec<AudioDeviceInfo>,
    pub device_cursor: Cursor,
    pub playlists_dir: PathBuf,
    pub playlists: Vec<String>,
    pub playlist_cursor: Cursor,
    pub playlist_prompt: Option<PlaylistPrompt>,
    pub stations_path: PathBuf,
    pub stations: Vec<znicz_core::Station>,
    pub station_cursor: Cursor,
    pub radio_prompt: Option<RadioPrompt>,
    pub meta: MetaCache,
    pub toasts: Toasts,
    pub tui: TuiConfig,
    pub covers: CoverCache,
    pub picker: Option<ratatui_image::picker::Picker>,
    pub(crate) cover_image: Option<ratatui_image::protocol::StatefulProtocol>,
    pub(crate) cover_draw_key: Option<(String, u16, u16)>,
    pub(crate) library_list_state: ratatui::widgets::ListState,
    pub(crate) queue_list_state: ratatui::widgets::ListState,
    pub(crate) device_list_state: ratatui::widgets::ListState,
    pub(crate) playlist_list_state: ratatui::widgets::ListState,
    pub(crate) station_list_state: ratatui::widgets::ListState,
    pub should_quit: bool,
}

fn point_in(rect: Rect, column: u16, row: u16) -> bool {
    rect.contains(ratatui::layout::Position { x: column, y: row })
}

struct MouseCapture;

impl MouseCapture {
    fn enable() -> Self {
        let _ = crossterm::execute!(std::io::stdout(), EnableMouseCapture);
        Self
    }
}

impl Drop for MouseCapture {
    fn drop(&mut self) {
        let _ = crossterm::execute!(std::io::stdout(), DisableMouseCapture);
    }
}

impl App {
    pub fn new(player: PlayerHandle) -> Self {
        Self::with_library(player, None)
    }

    pub fn with_library(player: PlayerHandle, library: Option<Library>) -> Self {
        Self::with_engine(Engine::Local(player), library)
    }

    pub fn with_remote(player: IpcClient, library: Option<Library>) -> Self {
        Self::with_engine(Engine::Remote(player), library)
    }

    fn with_engine(player: Engine, library: Option<Library>) -> Self {
        Self {
            player,
            focus: Focus::Library,
            queue_open: false,
            modal: Modal::None,
            list_width: 100,
            list_height: 0,
            title_slot: 0,
            queue_h_offset: 0,
            queue_title_slot: 0,
            queue_cursor: Cursor::new(),
            library: LibraryPane::new(library),
            hits: HitMap::default(),
            devices: load_output_devices(),
            device_cursor: Cursor::new(),
            playlists_dir: znicz_library::default_playlists_dir()
                .unwrap_or_else(|| std::env::temp_dir().join("znicz-playlists")),
            playlists: Vec::new(),
            playlist_cursor: Cursor::new(),
            playlist_prompt: None,
            stations_path: znicz_library::default_stations_path()
                .unwrap_or_else(|| std::env::temp_dir().join("znicz-stations.toml")),
            stations: Vec::new(),
            station_cursor: Cursor::new(),
            radio_prompt: None,
            meta: MetaCache::new(),
            toasts: Toasts::new(),
            tui: TuiConfig::default(),
            covers: CoverCache::new(),
            picker: Some(ratatui_image::picker::Picker::halfblocks()),
            cover_image: None,
            cover_draw_key: None,
            library_list_state: ratatui::widgets::ListState::default(),
            queue_list_state: ratatui::widgets::ListState::default(),
            device_list_state: ratatui::widgets::ListState::default(),
            playlist_list_state: ratatui::widgets::ListState::default(),
            station_list_state: ratatui::widgets::ListState::default(),
            should_quit: false,
        }
    }

    pub fn run(&mut self) -> color_eyre::Result<()> {
        let mut terminal = ratatui::init();
        let _mouse = MouseCapture::enable();
        self.picker = Some(make_picker(self.tui.cover_protocol));
        tracing::info!(
            protocol = ?self.picker.as_ref().map(|p| p.protocol_type()),
            "cover renderer"
        );
        let result = self.run_loop(&mut terminal);
        ratatui::restore();
        result
    }

    fn run_loop(&mut self, terminal: &mut ratatui::DefaultTerminal) -> color_eyre::Result<()> {
        loop {
            self.poll_player_events();
            self.toasts.tick();

            let state = self.player.state();
            terminal.draw(|frame| views::render(frame, self, &state))?;

            // Wake up for input, or on the tick to move the seek bar along.
            if event::poll(TICK_RATE)? {
                // Handle everything already queued before drawing again,
                // so held keys do not lag a frame behind.
                loop {
                    match event::read()? {
                        Event::Key(key) if key.kind == KeyEventKind::Press => self.on_key(key),
                        Event::Mouse(mouse) => self.on_mouse(mouse),
                        _ => {}
                    }
                    if !event::poll(Duration::ZERO)? {
                        break;
                    }
                }
            }

            if self.should_quit {
                break;
            }
        }
        Ok(())
    }

    /// Turn player events into something the user can actually see.
    fn poll_player_events(&mut self) {
        for event in self.player.drain_events() {
            match event {
                PlayerEvent::Error(message) => {
                    tracing::error!("player error: {message}");
                    self.toasts.error(message);
                }
                PlayerEvent::TrackStarted(track) => {
                    // Seed the cache so the queue row is right immediately.
                    if let Some(path) = track.path.clone() {
                        self.meta.insert(
                            path,
                            Entry {
                                title: track.title.clone(),
                                artist: track.artist().map(str::to_string),
                                album: track.album().map(str::to_string),
                                duration: track.duration,
                            },
                        );
                    }
                }
                _ => {}
            }
        }
    }

    // --- mouse handling ---

    pub fn on_mouse(&mut self, mouse: MouseEvent) {
        match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => self.on_left_click(mouse.column, mouse.row),
            MouseEventKind::ScrollUp => self.on_wheel(-1),
            MouseEventKind::ScrollDown => self.on_wheel(1),
            _ => {}
        }
    }

    fn on_wheel(&mut self, delta: isize) {
        if self.library.is_typing() || self.playlist_prompt.is_some() || self.radio_prompt.is_some()
        {
            return;
        }
        match self.modal {
            Modal::Devices => self.device_cursor.step(delta, self.devices.len()),
            Modal::Playlists => self.playlist_cursor.step(delta, self.playlists.len()),
            Modal::Radio => self.station_cursor.step(delta, self.stations.len()),
            Modal::Help | Modal::Inspector => {}
            Modal::None => match self.focus {
                Focus::Library => self.library.step(delta),
                Focus::Queue if self.queue_open => {
                    let len = self.player.state().queue.len();
                    self.queue_cursor.step(delta, len);
                }
                Focus::Queue => {}
            },
        }
    }

    fn on_left_click(&mut self, column: u16, row: u16) {
        if self.library.is_typing() {
            let on_prompt = self
                .hits
                .search_prompt
                .is_some_and(|r| point_in(r, column, row));
            if !on_prompt {
                self.library.cancel_search();
                self.toasts.info("search cancelled");
            }
            return;
        }
        if self.playlist_prompt.is_some() || self.radio_prompt.is_some() {
            let inside = self.hits.overlay.is_some_and(|r| point_in(r, column, row));
            if !inside {
                self.playlist_prompt = None;
                self.radio_prompt = None;
            }
            return;
        }
        if let Some(hit) = self
            .hits
            .footer_hints
            .iter()
            .find(|h| point_in(h.rect, column, row))
        {
            let key = hit.key;
            self.on_key(key);
            return;
        }
        if self.hits.close.is_some_and(|r| point_in(r, column, row)) {
            if self.modal != Modal::None {
                self.modal = Modal::None;
                self.playlist_prompt = None;
                self.radio_prompt = None;
            } else if self.queue_open {
                self.close_queue();
            }
            return;
        }
        if self.modal != Modal::None {
            self.on_overlay_click(column, row);
            return;
        }

        if self.queue_open {
            if let Some(hit) = self.hits.queue {
                if let Some(index) = hit.row_at(column, row) {
                    let len = self.player.state().queue.len();
                    self.queue_cursor.set(index, len);
                    self.focus = Focus::Queue;
                    return;
                }
            }
            return;
        }

        if self
            .hits
            .queue_toggle
            .is_some_and(|r| point_in(r, column, row))
        {
            self.open_queue();
            return;
        }

        if let Some(hit) = self.hits.library {
            if let Some(index) = hit.row_at(column, row) {
                self.library.set_index(index);
                self.focus = Focus::Library;
            }
        }
    }

    fn on_overlay_click(&mut self, column: u16, row: u16) {
        if let Some(hit) = self.hits.overlay_list {
            if let Some(index) = hit.row_at(column, row) {
                match self.modal {
                    Modal::Devices => self.device_cursor.set(index, self.devices.len()),
                    Modal::Playlists => self.playlist_cursor.set(index, self.playlists.len()),
                    Modal::Radio => self.station_cursor.set(index, self.stations.len()),
                    _ => {}
                }
                return;
            }
        }
        if self.hits.overlay.is_some_and(|r| point_in(r, column, row)) {
            return;
        }
    }

    // --- key handling ---

    /// Handle one keypress.
    ///
    /// Public so the bindings can be driven from tests without a real terminal.
    pub fn on_key(&mut self, key: KeyEvent) {
        if self.modal == Modal::Help {
            self.modal = Modal::None;
            return;
        }

        // Search, playlist save, and radio prompts are plain text. They must run
        // before global keys, or `s` (stop) turns "To Listen" into "To Liten".
        if self.library.is_typing() {
            self.on_search_key(key);
            return;
        }

        if self.playlist_prompt.is_some() {
            self.on_playlist_input_key(key);
            return;
        }

        if self.radio_prompt.is_some() {
            self.on_radio_input_key(key);
            return;
        }

        if key.modifiers.contains(KeyModifiers::CONTROL) {
            self.on_control_key(key.code);
            return;
        }

        if key.code == KeyCode::Esc {
            self.on_esc();
            return;
        }

        // Named-list overlays steal a/n/e/c/d (and Enter) from the global map
        // so new/edit are not next/repeat while P or R is open.
        if self.modal == Modal::Playlists && self.on_playlists_key(key) {
            return;
        }
        if self.modal == Modal::Radio && self.on_radio_key(key) {
            return;
        }

        if self.on_global_key(key) {
            return;
        }

        if self.modal == Modal::Devices {
            self.on_devices_key(key);
            return;
        }

        if self.modal == Modal::Inspector {
            return;
        }

        match self.focus {
            Focus::Queue => self.on_queue_key(key),
            Focus::Library => self.on_library_key(key),
        }
    }

    fn on_esc(&mut self) {
        if matches!(
            self.modal,
            Modal::Devices | Modal::Inspector | Modal::Playlists | Modal::Radio
        ) {
            self.modal = Modal::None;
            self.playlist_prompt = None;
            self.radio_prompt = None;
        } else if self.focus == Focus::Queue && self.queue_open {
            self.close_queue();
        } else {
            self.library.back();
        }
    }

    fn on_search_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => {
                self.library.cancel_search();
                self.toasts.info("search cancelled");
            }
            KeyCode::Enter => {
                let message = self.library.submit_search();
                self.toasts.info(message);
            }
            _ => {
                if let Some(input) = self.library.prompt_mut() {
                    input.on_key(key);
                }
            }
        }
    }

    fn on_control_key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Char('c') => self.should_quit = true,
            KeyCode::Char('d') => self.page(HALF_PAGE),
            KeyCode::Char('u') => self.page(-HALF_PAGE),
            _ => {}
        }
    }

    /// Returns true when the key was a global one and needs no focus handling.
    fn on_global_key(&mut self, key: KeyEvent) -> bool {
        if self.library.is_typing() || self.playlist_prompt.is_some() || self.radio_prompt.is_some()
        {
            return false;
        }
        match key.code {
            KeyCode::Char('q') => self.should_quit = true,
            KeyCode::Char('?') => self.modal = Modal::Help,
            KeyCode::Char(',') => self.toggle_devices_modal(),
            KeyCode::Char('i') => self.toggle_inspector_modal(),
            KeyCode::Char('P') => self.toggle_playlists_modal(),
            KeyCode::Char('R') => self.toggle_radio_modal(),

            KeyCode::Char(']') => {
                if self.queue_open {
                    self.close_queue();
                } else {
                    self.open_queue();
                }
            }

            KeyCode::Tab => {
                if !self.queue_open {
                    self.open_queue();
                    self.focus = Focus::Queue;
                } else if layout::is_sheet(self.list_width, true) {
                    // no-op in sheet mode
                } else {
                    self.swap_focus();
                }
            }
            KeyCode::BackTab => {
                if !self.queue_open {
                    // no-op while drawer is closed
                } else if layout::is_sheet(self.list_width, true) {
                    // no-op in sheet mode
                } else {
                    self.swap_focus();
                }
            }

            KeyCode::Char(' ') => self.toggle_pause(),
            KeyCode::Char('s') => self.apply(Command::Stop, None),
            KeyCode::Char('n') => self.skip_track(false),
            KeyCode::Char('p') => self.skip_track(true),

            KeyCode::Right if key.modifiers.contains(KeyModifiers::ALT) => {
                self.pan_titles(1);
            }
            KeyCode::Left if key.modifiers.contains(KeyModifiers::ALT) => {
                self.pan_titles(-1);
            }
            KeyCode::Right | KeyCode::Char('l') => self.seek_relative(SEEK_SMALL),
            KeyCode::Left | KeyCode::Char('h') => self.seek_relative(-SEEK_SMALL),
            KeyCode::Char('L') => self.seek_relative(SEEK_LARGE),
            KeyCode::Char('H') => self.seek_relative(-SEEK_LARGE),

            KeyCode::Char('+') | KeyCode::Char('=') => self.adjust_volume(VOLUME_STEP),
            KeyCode::Char('-') | KeyCode::Char('_') => self.adjust_volume(-VOLUME_STEP),
            KeyCode::Char('m') => self.toggle_mute(),
            KeyCode::Char('e') => self.cycle_repeat(),
            KeyCode::Char('r') => self.reload_front(),
            KeyCode::Char('z') => self.toggle_shuffle(),

            KeyCode::Down | KeyCode::Char('j') => self.step(1),
            KeyCode::Up | KeyCode::Char('k') => self.step(-1),
            KeyCode::PageDown => self.page(HALF_PAGE * 2),
            KeyCode::PageUp => self.page(-HALF_PAGE * 2),
            KeyCode::Char('g') | KeyCode::Home => self.go_first(),
            KeyCode::Char('G') | KeyCode::End => self.go_last(),

            _ => return false,
        }
        true
    }

    fn open_queue(&mut self) {
        self.queue_open = true;
        self.focus = Focus::Queue;
    }

    fn close_queue(&mut self) {
        self.queue_open = false;
        self.focus = Focus::Library;
        self.library.clamp_pan(self.title_slot());
    }

    fn title_slot(&self) -> usize {
        self.title_slot
    }

    fn swap_focus(&mut self) {
        self.focus = match self.focus {
            Focus::Library => Focus::Queue,
            Focus::Queue => Focus::Library,
        };
    }

    fn toggle_devices_modal(&mut self) {
        self.modal = if self.modal == Modal::Devices {
            Modal::None
        } else {
            Modal::Devices
        };
    }

    fn toggle_inspector_modal(&mut self) {
        self.modal = if self.modal == Modal::Inspector {
            Modal::None
        } else {
            Modal::Inspector
        };
    }

    fn toggle_playlists_modal(&mut self) {
        if self.modal == Modal::Playlists {
            self.modal = Modal::None;
            self.playlist_prompt = None;
        } else {
            self.reload_playlists();
            self.modal = Modal::Playlists;
        }
    }

    fn toggle_radio_modal(&mut self) {
        if self.modal == Modal::Radio {
            self.modal = Modal::None;
            self.radio_prompt = None;
        } else {
            self.playlist_prompt = None;
            self.reload_stations();
            self.modal = Modal::Radio;
        }
    }

    fn reload_playlists(&mut self) {
        self.playlists = list_saved(&self.playlists_dir);
        self.playlist_cursor.clamp(self.playlists.len());
    }

    fn reload_stations(&mut self) {
        match znicz_core::load_stations(&self.stations_path) {
            Ok(stations) => {
                self.stations = stations;
                self.station_cursor.clamp(self.stations.len());
            }
            Err(e) => self.toasts.error(e.to_string()),
        }
    }

    fn reload_front(&mut self) {
        match self.modal {
            Modal::Radio => self.reload_stations(),
            Modal::Playlists => self.reload_playlists(),
            Modal::Devices => {
                self.devices = load_output_devices();
                self.device_cursor.clamp(self.devices.len());
                self.toasts
                    .info(format!("{} devices found", self.devices.len()));
            }
            _ => {
                self.library.reload_albums();
                self.toasts.success("library reloaded");
            }
        }
    }

    fn on_playlists_key(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Enter => self.play_selected_playlist(false),
            KeyCode::Char('a') => self.play_selected_playlist(true),
            KeyCode::Char('n') => self.begin_playlist_save(),
            KeyCode::Char('e') => self.begin_playlist_rename(),
            KeyCode::Char('c') => self.begin_playlist_copy(),
            KeyCode::Char('d') => self.delete_selected_playlist(),
            _ => return false,
        }
        true
    }

    fn on_radio_key(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Enter => self.play_selected_station(),
            KeyCode::Char('a') => self.queue_selected_station(),
            KeyCode::Char('n') => {
                self.radio_prompt = Some(RadioPrompt::new_station());
            }
            KeyCode::Char('e') => self.begin_station_edit(),
            KeyCode::Char('c') => self.begin_station_copy(),
            KeyCode::Char('d') => self.delete_selected_station(),
            _ => return false,
        }
        true
    }

    fn on_radio_input_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => self.radio_prompt = None,
            KeyCode::Enter => self.confirm_radio_prompt(),
            KeyCode::Tab | KeyCode::Down => {
                if let Some(prompt) = self.radio_prompt.as_mut() {
                    prompt.cycle_field(true);
                }
            }
            KeyCode::BackTab | KeyCode::Up => {
                if let Some(prompt) = self.radio_prompt.as_mut() {
                    prompt.cycle_field(false);
                }
            }
            _ => {
                if let Some(prompt) = self.radio_prompt.as_mut() {
                    prompt.buffer_mut().on_key(key);
                }
            }
        }
    }

    fn selected_station(&self) -> Option<&znicz_core::Station> {
        self.station_cursor
            .selected(self.stations.len())
            .map(|i| &self.stations[i])
    }

    fn selected_station_name(&self) -> Option<String> {
        self.selected_station().map(|s| s.name.clone())
    }

    fn play_selected_station(&mut self) {
        let Some(station) = self.selected_station().cloned() else {
            self.toasts.info("no stations");
            return;
        };
        match znicz_core::play_station(&self.player, &station, false) {
            Ok(()) => self.toasts.success(format!("playing {}", station.name)),
            Err(e) => self.toasts.error(e.to_string()),
        }
    }

    fn queue_selected_station(&mut self) {
        let Some(station) = self.selected_station().cloned() else {
            self.toasts.info("no stations");
            return;
        };
        match znicz_core::play_station(&self.player, &station, true) {
            Ok(()) => self.toasts.success(format!("added {}", station.name)),
            Err(e) => self.toasts.error(e.to_string()),
        }
    }

    fn begin_station_edit(&mut self) {
        let Some(station) = self.selected_station().cloned() else {
            self.toasts.info("no stations");
            return;
        };
        self.radio_prompt = Some(RadioPrompt::edit_station(&station));
    }

    fn begin_station_copy(&mut self) {
        let Some(name) = self.selected_station_name() else {
            self.toasts.info("no stations");
            return;
        };
        self.radio_prompt = Some(RadioPrompt::Copy(LineEdit::from_text(name)));
    }

    fn delete_selected_station(&mut self) {
        let Some(name) = self.selected_station_name() else {
            self.toasts.info("no stations");
            return;
        };
        if let Err(e) = znicz_core::remove_station(&mut self.stations, &name) {
            self.toasts.error(e.to_string());
            return;
        }
        self.persist_stations();
    }

    fn persist_stations(&mut self) {
        match znicz_core::save_stations(&self.stations_path, &self.stations) {
            Ok(()) => self.station_cursor.clamp(self.stations.len()),
            Err(e) => self.toasts.error(e.to_string()),
        }
    }

    fn confirm_radio_prompt(&mut self) {
        match self.radio_prompt.take() {
            Some(RadioPrompt::Form {
                name,
                url,
                art,
                field,
                original,
            }) => {
                let result = match original.as_deref() {
                    None => znicz_core::add_station(&mut self.stations, &name, &url),
                    Some(old) => znicz_core::update_station(&mut self.stations, old, &name, &url),
                };
                if let Err(e) = result {
                    self.toasts.error(e.to_string());
                    self.radio_prompt = Some(RadioPrompt::Form {
                        name,
                        url,
                        art,
                        field,
                        original,
                    });
                    return;
                }
                let station_name = match znicz_core::validate_name(&name) {
                    Ok(n) => n,
                    Err(e) => {
                        self.toasts.error(e.to_string());
                        self.radio_prompt = Some(RadioPrompt::Form {
                            name,
                            url,
                            art,
                            field,
                            original,
                        });
                        return;
                    }
                };
                let art_arg = {
                    let trimmed = art.as_str().trim();
                    if trimmed.is_empty() {
                        None
                    } else {
                        Some(trimmed)
                    }
                };
                if let Err(e) =
                    znicz_core::set_station_art(&mut self.stations, &station_name, art_arg)
                {
                    self.toasts.error(e.to_string());
                    self.persist_stations();
                    self.radio_prompt = Some(RadioPrompt::Form {
                        name,
                        url,
                        art,
                        field,
                        original: Some(station_name),
                    });
                    return;
                }
                self.persist_stations();
            }
            Some(RadioPrompt::Copy(new_name)) => {
                let Some(old_name) = self.selected_station_name() else {
                    self.toasts.info("no stations");
                    return;
                };
                if let Err(e) = znicz_core::copy_station(&mut self.stations, &old_name, &new_name) {
                    self.toasts.error(e.to_string());
                    self.radio_prompt = Some(RadioPrompt::Copy(new_name));
                    return;
                }
                self.persist_stations();
            }
            None => {}
        }
    }

    fn on_playlist_input_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => self.playlist_prompt = None,
            KeyCode::Enter => self.confirm_playlist_prompt(),
            _ => {
                if let Some(input) = self.playlist_prompt.as_mut() {
                    input.buffer_mut().on_key(key);
                }
            }
        }
    }

    fn selected_playlist_name(&self) -> Option<String> {
        self.playlist_cursor
            .selected(self.playlists.len())
            .map(|i| self.playlists[i].clone())
    }

    fn play_selected_playlist(&mut self, append: bool) {
        let Some(name) = self.selected_playlist_name() else {
            return;
        };
        let Some(path) = saved_path(&self.playlists_dir, &name) else {
            self.toasts.error(format!("no playlist named {name}"));
            return;
        };
        match load_path(&path) {
            Ok(result) => match apply_to_player(&self.player, &result, append) {
                Ok(()) => {
                    if let Some(message) = skipped_notice(&result) {
                        self.toasts.warn(message);
                    } else if append {
                        self.toasts.success(format!(
                            "added {} from {name}",
                            match result.items.len() {
                                1 => "1 track".to_string(),
                                n => format!("{n} tracks"),
                            }
                        ));
                    } else {
                        self.toasts.success(format!("playing {name}"));
                    }
                }
                Err(e) => self.toasts.error(e.to_string()),
            },
            Err(e) => self.toasts.error(e.to_string()),
        }
    }

    fn begin_playlist_save(&mut self) {
        let queue = self.player.state().queue;
        if queue.is_empty() {
            self.toasts.warn("queue is empty");
            return;
        }
        self.playlist_prompt = Some(PlaylistPrompt::Save(LineEdit::new()));
    }

    fn begin_playlist_rename(&mut self) {
        let Some(name) = self.selected_playlist_name() else {
            self.toasts.info("no playlists");
            return;
        };
        self.playlist_prompt = Some(PlaylistPrompt::Rename(LineEdit::from_text(name)));
    }

    fn begin_playlist_copy(&mut self) {
        let Some(name) = self.selected_playlist_name() else {
            self.toasts.info("no playlists");
            return;
        };
        self.playlist_prompt = Some(PlaylistPrompt::Copy(LineEdit::from_text(name)));
    }

    fn delete_selected_playlist(&mut self) {
        let Some(name) = self.selected_playlist_name() else {
            self.toasts.info("no playlists");
            return;
        };
        if let Err(e) = znicz_core::remove_saved(&self.playlists_dir, &name) {
            self.toasts.error(e.to_string());
            return;
        }
        self.toasts.success(format!("deleted {name}"));
        self.reload_playlists();
    }

    fn confirm_playlist_prompt(&mut self) {
        match self.playlist_prompt.take() {
            Some(PlaylistPrompt::Save(raw)) => self.confirm_playlist_save(raw),
            Some(PlaylistPrompt::Rename(raw)) => self.confirm_playlist_rename(raw),
            Some(PlaylistPrompt::Copy(raw)) => self.confirm_playlist_copy(raw),
            None => {}
        }
    }

    fn confirm_playlist_save(&mut self, raw: LineEdit) {
        let name = match sanitize_stem(&raw) {
            Ok(name) => name,
            Err(e) => {
                self.toasts.error(e.to_string());
                self.playlist_prompt = Some(PlaylistPrompt::Save(raw));
                return;
            }
        };
        let path = self.playlists_dir.join(&name);
        let queue = self.player.state().queue;
        match write_path(&path, &queue) {
            Ok(()) => {
                let stem = name
                    .strip_suffix(".m3u8")
                    .or_else(|| name.strip_suffix(".m3u"))
                    .unwrap_or(&name);
                self.toasts.success(format!("saved {stem}"));
                self.reload_playlists();
            }
            Err(e) => self.toasts.error(e.to_string()),
        }
    }

    fn confirm_playlist_rename(&mut self, raw: LineEdit) {
        let Some(old_name) = self.selected_playlist_name() else {
            self.toasts.info("no playlists");
            return;
        };
        match znicz_core::rename_saved(&self.playlists_dir, &old_name, &raw) {
            Ok(new_name) => {
                self.toasts.success(format!("renamed {new_name}"));
                self.reload_playlists();
                if let Some(index) = self.playlists.iter().position(|n| n == &new_name) {
                    self.playlist_cursor.set(index, self.playlists.len());
                }
            }
            Err(e) => {
                self.toasts.error(e.to_string());
                self.playlist_prompt = Some(PlaylistPrompt::Rename(raw));
            }
        }
    }

    fn confirm_playlist_copy(&mut self, raw: LineEdit) {
        let Some(old_name) = self.selected_playlist_name() else {
            self.toasts.info("no playlists");
            return;
        };
        match znicz_core::copy_saved(&self.playlists_dir, &old_name, &raw) {
            Ok(new_name) => {
                self.toasts.success(format!("copied {new_name}"));
                self.reload_playlists();
                if let Some(index) = self.playlists.iter().position(|n| n == &new_name) {
                    self.playlist_cursor.set(index, self.playlists.len());
                }
            }
            Err(e) => {
                self.toasts.error(e.to_string());
                self.playlist_prompt = Some(PlaylistPrompt::Copy(raw));
            }
        }
    }

    fn on_queue_key(&mut self, key: KeyEvent) {
        let state = self.player.state();
        match key.code {
            KeyCode::Enter => {
                if let Some(index) = self.queue_cursor.selected(state.queue.len()) {
                    self.apply(Command::QueuePlayIndex(index), None);
                }
            }
            KeyCode::Char('d') | KeyCode::Delete => {
                if let Some(index) = self.queue_cursor.selected(state.queue.len()) {
                    self.apply(
                        Command::QueueRemove(index),
                        Some("removed from queue".into()),
                    );
                    let len = self.player.state().queue.len();
                    self.queue_cursor.clamp(len);
                }
            }
            KeyCode::Char('C') => {
                self.apply(Command::QueueClear, Some("queue cleared".into()));
                self.queue_cursor.first();
            }
            KeyCode::Char('o') => {
                self.queue_cursor
                    .set(state.queue_position, state.queue.len());
            }
            _ => {}
        }
    }

    fn on_library_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('/') => self.library.begin_search(),
            KeyCode::Enter => self.library_enter(),
            KeyCode::Char('a') => {
                let tracks = self.library.selected_tracks();
                self.enqueue(tracks);
            }
            KeyCode::Char('A') => {
                let tracks = self.library.listed_tracks();
                self.enqueue(tracks);
            }
            _ => {}
        }
    }

    fn on_devices_key(&mut self, key: KeyEvent) {
        if key.code == KeyCode::Enter {
            let selected = self
                .device_cursor
                .selected(self.devices.len())
                .and_then(|i| self.devices.get(i))
                .map(|d| (d.id.clone(), d.name.clone()));

            if let Some((id, name)) = selected {
                self.apply(Command::SetDevice(id), Some(format!("output: {name}")));
            }
        }
    }

    /// Enter in the library: open an album, or play a track right away.
    fn library_enter(&mut self) {
        match self.library.selected() {
            Some(Item::Album(_)) => {
                self.library.enter();
            }
            Some(Item::Track(track)) => {
                let path = track.path.clone();
                let entry = entry_from_track(track);
                self.meta.insert(path.clone(), entry);
                self.apply(Command::Play(znicz_core::QueueItem::file(path)), None);
            }
            None => {}
        }
    }

    fn enqueue(&mut self, tracks: Vec<Track>) {
        if tracks.is_empty() {
            self.toasts.warn("nothing to add");
            return;
        }

        let count = tracks.len();
        let items: Vec<znicz_core::QueueItem> = tracks
            .iter()
            .map(|t| znicz_core::QueueItem::file(t.path.clone()))
            .collect();
        // Tags come from the database, so the queue reads properly at once.
        for track in &tracks {
            self.meta
                .insert(track.path.clone(), entry_from_track(track));
        }

        self.apply(
            Command::QueueAdd(items),
            Some(match count {
                1 => "added 1 track".to_string(),
                n => format!("added {n} tracks"),
            }),
        );
    }

    // --- list movement, applied to whichever list has focus ---

    fn list_len(&self) -> usize {
        if self.modal == Modal::Radio {
            self.stations.len()
        } else if self.modal == Modal::Playlists {
            self.playlists.len()
        } else if self.modal == Modal::Devices {
            self.devices.len()
        } else if self.focus == Focus::Queue && self.queue_open {
            self.player.state().queue.len()
        } else {
            self.library.len()
        }
    }

    fn step(&mut self, delta: isize) {
        let len = self.list_len();
        if self.modal == Modal::Inspector {
            return;
        }
        if self.modal == Modal::Radio {
            self.station_cursor.step(delta, len);
        } else if self.modal == Modal::Playlists {
            self.playlist_cursor.step(delta, len);
        } else if self.modal == Modal::Devices {
            self.device_cursor.step(delta, len);
        } else if self.focus == Focus::Queue && self.queue_open {
            self.queue_cursor.step(delta, len);
            self.queue_h_offset = 0;
        } else {
            self.library.step(delta);
        }
    }

    fn page(&mut self, delta: isize) {
        let len = self.list_len();
        if self.modal == Modal::Inspector {
            return;
        }
        if self.modal == Modal::Radio {
            self.station_cursor.page(delta, len);
        } else if self.modal == Modal::Playlists {
            self.playlist_cursor.page(delta, len);
        } else if self.modal == Modal::Devices {
            self.device_cursor.page(delta, len);
        } else if self.focus == Focus::Queue && self.queue_open {
            self.queue_cursor.page(delta, len);
            self.queue_h_offset = 0;
        } else {
            self.library.page(delta);
        }
    }

    fn go_first(&mut self) {
        if self.modal == Modal::Inspector {
            return;
        }
        if self.modal == Modal::Radio {
            self.station_cursor.first();
        } else if self.modal == Modal::Playlists {
            self.playlist_cursor.first();
        } else if self.modal == Modal::Devices {
            self.device_cursor.first();
        } else if self.focus == Focus::Queue && self.queue_open {
            self.queue_cursor.first();
            self.queue_h_offset = 0;
        } else {
            self.library.first();
        }
    }

    fn go_last(&mut self) {
        let len = self.list_len();
        if self.modal == Modal::Inspector {
            return;
        }
        if self.modal == Modal::Radio {
            self.station_cursor.last(len);
        } else if self.modal == Modal::Playlists {
            self.playlist_cursor.last(len);
        } else if self.modal == Modal::Devices {
            self.device_cursor.last(len);
        } else if self.focus == Focus::Queue && self.queue_open {
            self.queue_cursor.last(len);
            self.queue_h_offset = 0;
        } else {
            self.library.last();
        }
    }

    // --- playback actions ---

    /// Send a command, wait for the engine, and report the outcome on screen.
    ///
    /// Waiting matters here: the very next frame reads the player state, so a
    /// fire-and-forget send would draw the old volume or the old track.
    fn apply(&mut self, command: Command, success: Option<String>) {
        match self.player.send_blocking(command) {
            Ok(()) => {
                if let Some(message) = success {
                    self.toasts.success(message);
                }
            }
            Err(e) => self.toasts.error(e.to_string()),
        }
    }

    fn toggle_pause(&mut self) {
        let state = self.player.state();
        match state.status {
            PlaybackStatus::Playing => self.apply(Command::Pause, None),
            PlaybackStatus::Paused => self.apply(Command::Resume, None),
            // Nothing loaded: start whatever the queue points at.
            PlaybackStatus::Stopped => {
                if state.queue.is_empty() {
                    self.toasts
                        .warn("queue is empty — add tracks from the library");
                } else {
                    self.apply(Command::QueuePlayIndex(state.queue_position), None);
                }
            }
        }
    }

    fn adjust_volume(&mut self, delta: f32) {
        let state = self.player.state();
        let volume = (state.volume + delta).clamp(0.0, 1.0);
        self.apply(Command::SetVolume(volume), None);
    }

    fn toggle_mute(&mut self) {
        let muted = !self.player.state().muted;
        self.apply(
            Command::SetMuted(muted),
            Some(if muted {
                "muted".into()
            } else {
                "unmuted".into()
            }),
        );
    }

    fn cycle_repeat(&mut self) {
        let mode = self.player.state().repeat.next();
        self.apply(
            Command::SetRepeat(mode),
            Some(format!("repeat {}", mode.label())),
        );
    }

    fn toggle_shuffle(&mut self) {
        let shuffle = !self.player.state().shuffle;
        self.apply(
            Command::SetShuffle(shuffle),
            Some(if shuffle {
                "shuffle on".into()
            } else {
                "shuffle off".into()
            }),
        );
    }

    fn pan_titles(&mut self, dir: isize) {
        if self.focus == Focus::Queue && self.queue_open {
            self.pan_queue(dir);
        } else {
            self.library.pan(dir, self.title_slot);
        }
    }

    fn pan_queue(&mut self, dir: isize) {
        let max = self
            .selected_queue_middle()
            .saturating_sub(self.queue_title_slot) as isize;
        let next = self.queue_h_offset as isize + dir;
        self.queue_h_offset = next.clamp(0, max.max(0)) as usize;
    }

    pub fn queue_offset_for(&self, index: usize, len: usize) -> usize {
        if self.queue_cursor.selected(len) == Some(index) {
            self.queue_h_offset
        } else {
            0
        }
    }

    fn queue_label_len(&self, item: &znicz_core::QueueItem) -> usize {
        match item {
            znicz_core::QueueItem::Stream { name, .. } => name.chars().count(),
            znicz_core::QueueItem::File { path } => match self.meta.get(path) {
                Some(entry) => entry.label().chars().count(),
                None => path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("?")
                    .chars()
                    .count(),
            },
        }
    }

    fn selected_queue_middle(&self) -> usize {
        let state = self.player.state();
        let Some(index) = self.queue_cursor.selected(state.queue.len()) else {
            return 0;
        };
        state
            .queue
            .get(index)
            .map(|item| self.queue_label_len(item))
            .unwrap_or(0)
    }

    pub(crate) fn clamp_queue_pan(&mut self) {
        let max = self
            .selected_queue_middle()
            .saturating_sub(self.queue_title_slot);
        self.queue_h_offset = self.queue_h_offset.min(max);
    }

    fn skip_track(&mut self, previous: bool) {
        let queue = self.player.state().queue;
        if queue.len() == 1 && queue[0].is_stream() {
            self.toasts.info(if previous {
                "radio has no previous track"
            } else {
                "radio has no next track"
            });
            return;
        }
        self.apply(
            if previous {
                Command::PreviousTrack
            } else {
                Command::NextTrack
            },
            None,
        );
    }

    fn queue_row_is_stream(&self) -> bool {
        let state = self.player.state();
        state
            .queue
            .get(state.queue_position)
            .is_some_and(|item| item.is_stream())
    }

    fn seek_relative(&mut self, seconds: i64) {
        let state = self.player.state();
        if state.current_track.is_none() && !self.queue_row_is_stream() {
            return;
        }
        let target = state.position.as_secs() as i64 + seconds;
        self.apply(
            Command::Seek(Duration::from_secs(target.max(0) as u64)),
            None,
        );
    }

    pub fn state(&self) -> PlayerState {
        self.player.state()
    }
}

/// Rows moved by Ctrl-d / Ctrl-u. A fixed step keeps the jump predictable
/// regardless of window size.
const HALF_PAGE: isize = 10;

fn entry_from_track(track: &Track) -> Entry {
    Entry {
        title: track.title.clone(),
        artist: track.artist.clone(),
        album: track.album.clone(),
        duration: track.duration_secs.map(Duration::from_secs_f64),
    }
}

/// Label for the repeat indicator in the status bar.
pub fn repeat_label(mode: RepeatMode) -> &'static str {
    match mode {
        RepeatMode::Off => "repeat off",
        RepeatMode::All => "repeat all",
        RepeatMode::One => "repeat one",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeat_labels_stay_readable() {
        assert_eq!(repeat_label(RepeatMode::Off), "repeat off");
        assert_eq!(repeat_label(RepeatMode::One), "repeat one");
        assert_eq!(repeat_label(RepeatMode::All), "repeat all");
    }

    #[test]
    fn a_library_track_carries_its_tags_into_the_queue() {
        let track = Track {
            id: 1,
            path: PathBuf::from("/music/a.flac"),
            title: "Kashmir".into(),
            artist: Some("Led Zeppelin".into()),
            album: Some("Physical Graffiti".into()),
            album_artist: None,
            genre: None,
            year: None,
            track_number: None,
            disc_number: None,
            codec: None,
            sample_rate: None,
            channels: None,
            bits_per_sample: None,
            duration_secs: Some(508.0),
        };

        let entry = entry_from_track(&track);
        assert_eq!(entry.label(), "Led Zeppelin — Kashmir");
        assert_eq!(entry.duration, Some(Duration::from_secs(508)));
    }

    fn test_player() -> PlayerHandle {
        let (player, _thread) = znicz_core::spawn_player(znicz_core::AudioConfig::default());
        player
    }

    fn long_album() -> znicz_library::AlbumSummary {
        znicz_library::AlbumSummary {
            album: "x".repeat(50),
            album_artist: None,
            year: None,
            track_count: 1,
            total_secs: Some(125.0),
        }
    }

    fn dummy_track(title: &str) -> Track {
        Track {
            id: 1,
            path: PathBuf::from("/music/dummy.flac"),
            title: title.into(),
            artist: Some("Artist".into()),
            album: Some("Album".into()),
            album_artist: None,
            genre: None,
            year: None,
            track_number: Some(1),
            disc_number: None,
            codec: None,
            sample_rate: None,
            channels: None,
            bits_per_sample: None,
            duration_secs: Some(120.0),
        }
    }

    #[test]
    fn radio_form_tab_cycles_name_url_art() {
        let mut prompt = RadioPrompt::new_station();
        assert!(matches!(
            prompt,
            RadioPrompt::Form {
                field: StationField::Name,
                ..
            }
        ));
        prompt.cycle_field(true);
        assert!(matches!(
            prompt,
            RadioPrompt::Form {
                field: StationField::Url,
                ..
            }
        ));
        prompt.cycle_field(true);
        assert!(matches!(
            prompt,
            RadioPrompt::Form {
                field: StationField::Art,
                ..
            }
        ));
        prompt.cycle_field(true);
        assert!(matches!(
            prompt,
            RadioPrompt::Form {
                field: StationField::Name,
                ..
            }
        ));
    }

    #[test]
    fn alt_arrows_pan_and_angle_brackets_do_nothing() {
        let mut app = App::with_library(test_player(), None);
        app.library.inject_albums_for_test(vec![long_album()]);
        app.title_slot = 20;

        app.on_key(KeyEvent::new(KeyCode::Char('>'), KeyModifiers::NONE));
        app.on_key(KeyEvent::new(KeyCode::Char('<'), KeyModifiers::ALT));
        assert_eq!(app.library.h_offset(), 0, "< and > should not pan or bind");

        app.on_key(KeyEvent::new(KeyCode::Right, KeyModifiers::ALT));
        assert_eq!(app.library.h_offset(), 1, "Alt+→ should pan titles");

        app.on_key(KeyEvent::new(KeyCode::Left, KeyModifiers::ALT));
        assert_eq!(app.library.h_offset(), 0, "Alt+← should pan back");
    }

    #[test]
    fn adding_to_an_empty_queue_does_not_start_playback() {
        let mut app = App::with_library(test_player(), None);
        app.library
            .inject_tracks_for_test(vec![dummy_track("Quiet Add")]);
        app.on_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE));

        let state = app.player.state();
        assert_eq!(state.queue.len(), 1, "the track should be queued");
        assert_eq!(
            state.status,
            PlaybackStatus::Stopped,
            "adding must not start playback"
        );
    }
}
