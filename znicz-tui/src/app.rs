//! The application: what is on screen, and what the keys do.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyEventKind,
    KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::crossterm::execute;
use ratatui::crossterm::terminal::{BeginSynchronizedUpdate, EndSynchronizedUpdate};
use ratatui::layout::Rect;
use ratatui_image::picker::ProtocolType;
use znicz_core::{
    AudioDeviceInfo, AudioOutput, Command, IpcClient, LibraryHost, LibraryOp, LibraryStatus,
    PlaybackStatus, PlayerEvent, PlayerHandle, PlayerOps, PlayerState, QueueItem, RepeatMode,
    apply_to_player, list_saved, load_path, sanitize_stem, saved_path, skipped_notice, write_path,
};
use znicz_library::{Library, Track};

use crate::cover::CoverCache;
use crate::cursor::Cursor;
use crate::help_state::{HELP_PAGE, HelpState};
use crate::hit::HitMap;
use crate::keys::Context;
use crate::layout;
use crate::library_pane::{ColumnFocus, EffectiveLayout, Item, LibraryPane, Mode};
use crate::line_edit::LineEdit;
use crate::meta::{Entry, MetaCache};
use crate::toast::Toasts;
use crate::tui_config::{CoverProtocol, TuiConfig};
use crate::update::{Release, Updates};
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
    use ratatui_image::picker::cap_parser::QueryStdioOptions;
    use ratatui_image::picker::{Picker, ProtocolType};

    let mut picker = match protocol {
        CoverProtocol::Halfblocks | CoverProtocol::Off => Picker::halfblocks(),
        CoverProtocol::Auto | CoverProtocol::Kitty | CoverProtocol::Sixel => {
            // Also ask for the background colour so the cover slot can match it.
            Picker::from_query_stdio_with_options(QueryStdioOptions {
                terminal_background_color_osc: true,
                ..QueryStdioOptions::default()
            })
            .unwrap_or_else(|_| Picker::halfblocks())
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

/// Seek step for arrows, and for Shift + arrows.
const SEEK_SMALL: i64 = 5;
const SEEK_LARGE: i64 = 30;
const VOLUME_STEP: f32 = 0.01;

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

    /// The player's version, when it advertises one (remote players only).
    pub fn player_version(&self) -> Option<String> {
        match self {
            Self::Local(_) => None,
            Self::Remote(player) => player.player_version(),
        }
    }

    /// Ask a remote player process to exit. The local engine has no process.
    pub fn shutdown_player(&self) -> znicz_core::Result<()> {
        match self {
            Self::Local(_) => Ok(()),
            Self::Remote(player) => player.shutdown(),
        }
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

/// What fills the screen above the footer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum View {
    /// Library, queue drawer, transport.
    #[default]
    Browse,
    /// Full-screen now playing.
    Stage,
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
    Playlists,
    Radio,
    Messages,
    Update,
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
            Self::Help
                | Self::Devices
                | Self::Playlists
                | Self::Radio
                | Self::Messages
                | Self::Update
        )
    }
}

/// How the TUI ended: quit, or start the (updated) binary again.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunOutcome {
    Quit,
    /// Start the (updated) binary again; `resume` replays where the player was.
    Restart {
        resume: Option<Resume>,
    },
}

/// Where to pick playback up after a restart.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Resume {
    pub index: usize,
    pub seconds: u64,
}

/// Where the Update modal is in offering, installing, and restarting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum UpdateStep {
    Offer,
    NotInstallable,
    Installing,
    Failed(String),
    Done,
    PlayerPlaying,
}

/// Whether restarting should ask about the player: it has a queue and is not stopped.
pub(crate) fn restart_asks_about_player(status: PlaybackStatus, queue_len: usize) -> bool {
    queue_len > 0 && status != PlaybackStatus::Stopped
}

pub struct App {
    pub player: Engine,
    pub focus: Focus,
    pub queue_open: bool,
    pub view: View,
    pub(crate) transport_marquee: crate::marquee::Marquee,
    pub(crate) stage_marquee: crate::marquee::Marquee,
    pub modal: Modal,
    pub help: HelpState,
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
    pub message_cursor: Cursor,
    pub message_h_offset: usize,
    pub radio_prompt: Option<RadioPrompt>,
    pub meta: MetaCache,
    pub toasts: Toasts,
    pub tui: TuiConfig,
    pub covers: CoverCache,
    pub picker: Option<ratatui_image::picker::Picker>,
    pub(crate) cover_image: Option<ratatui_image::protocol::StatefulProtocol>,
    pub(crate) cover_draw_key: Option<(String, u16, u16)>,
    /// The last stream cover shown and its stream URL, drawn while the next
    /// title's cover is still loading.
    pub(crate) last_stream_cover: Option<(String, crate::cover::CoverReady)>,
    pub(crate) playing: Option<crate::playing::PlayingRef>,
    pub(crate) library_list_state: ratatui::widgets::ListState,
    pub(crate) queue_list_state: ratatui::widgets::ListState,
    pub(crate) device_list_state: ratatui::widgets::ListState,
    pub(crate) playlist_list_state: ratatui::widgets::ListState,
    pub(crate) station_list_state: ratatui::widgets::ListState,
    pub(crate) messages_list_state: ratatui::widgets::ListState,
    /// Where library writes go: the player over IPC, or an in-process service in tests.
    pub library_host: Option<Arc<dyn LibraryHost>>,
    /// The writer's status as of the last frame.
    pub library_status: LibraryStatus,
    /// The folder prompt, while it is open.
    pub folder_prompt: Option<LineEdit>,
    /// The first-run prompt was dismissed; do not offer it again this session.
    pub(crate) first_run_dismissed: bool,
    /// `scans_finished` at the last poll, to notice finished jobs.
    scans_seen: Option<u64>,
    /// The least time between two status reads.
    pub library_poll_every: Duration,
    last_library_poll: Option<Instant>,
    pub should_quit: bool,
    /// Where the newer release comes from; `None` disables the check.
    pub updates: Option<Arc<dyn Updates>>,
    /// The newer release, once the check has found one.
    pub update: Option<Release>,
    /// The background check's answer, until it arrives.
    update_check: Option<std::sync::mpsc::Receiver<Option<Release>>>,
    /// The answer to `u` in Help when no update is known, until it arrives.
    manual_check: Option<std::sync::mpsc::Receiver<Result<Option<Release>, String>>>,
    /// The Update modal's current step.
    pub(crate) update_step: UpdateStep,
    /// How far the release notes are scrolled.
    pub(crate) update_scroll: usize,
    /// The install's result, until the install thread reports it.
    install_rx: Option<std::sync::mpsc::Receiver<Result<(), String>>>,
    /// How to update by hand when this copy cannot install; read once per opening.
    pub(crate) update_hint: String,
    /// How the TUI ends; `Restart` once an update asks for it.
    pub outcome: RunOutcome,
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
    /// Read the library writer's status: reload and report finished jobs, and
    /// offer the first-run prompt. Called once per frame.
    pub fn poll_library(&mut self) {
        let Some(host) = self.library_host.clone() else {
            return;
        };
        if self
            .last_library_poll
            .is_some_and(|at| at.elapsed() < self.library_poll_every)
        {
            return;
        }
        self.last_library_poll = Some(Instant::now());
        // An unreachable host is not a status: keep what we knew.
        let Ok(status) = host.try_status() else {
            return;
        };
        if self
            .scans_seen
            .is_some_and(|seen| seen != status.scans_finished)
        {
            if should_reload(&status, self.library.mode()) {
                self.library.reload_browse();
                self.playing = None;
            }
            if let Some(last) = &status.last {
                let home = dirs::home_dir();
                let (level, text) = crate::scan_status::finish_message(last, home.as_deref());
                match level {
                    crate::toast::Level::Error => self.toasts.error(text),
                    crate::toast::Level::Warn => self.toasts.warn(text),
                    crate::toast::Level::Success => self.toasts.success(text),
                    crate::toast::Level::Info => self.toasts.info(text),
                }
            }
        }
        self.scans_seen = Some(status.scans_finished);
        self.library_status = status;
        if self.wants_first_run_prompt() {
            self.open_folder_prompt();
        }
    }

    fn wants_first_run_prompt(&self) -> bool {
        self.view == View::Browse
            && !self.first_run_dismissed
            && self.folder_prompt.is_none()
            && self.modal == Modal::None
            && self.playlist_prompt.is_none()
            && self.radio_prompt.is_none()
            && !self.library.is_typing()
            && !self.library_status.busy()
            && self.library.has_no_music()
    }

    fn open_folder_prompt(&mut self) {
        if self.library_host.is_none() {
            self.toasts.error("no music library configured");
            return;
        }
        self.folder_prompt = Some(LineEdit::from_text(default_folder_text()));
    }

    fn on_folder_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => {
                self.folder_prompt = None;
                self.first_run_dismissed = true;
            }
            KeyCode::Enter => self.submit_folder(),
            KeyCode::Tab => self.complete_folder(),
            _ => {
                if let Some(edit) = self.folder_prompt.as_mut() {
                    edit.on_key(key);
                }
            }
        }
    }

    fn submit_folder(&mut self) {
        let Some(typed) = self
            .folder_prompt
            .as_ref()
            .map(|e| e.as_str().trim().to_string())
        else {
            return;
        };
        let home = dirs::home_dir().unwrap_or_default();
        let path = crate::folder_complete::expand(&typed, &home);
        if !path.is_dir() {
            self.toasts.error(format!("not a folder: {typed}"));
            return;
        }
        let Some(host) = self.library_host.clone() else {
            self.toasts.error("no music library configured");
            return;
        };
        match host.apply(LibraryOp::AddFolder { path }) {
            Ok(()) => {
                self.folder_prompt = None;
                self.first_run_dismissed = true;
                self.toasts.info(format!("scanning {typed}"));
            }
            Err(e) => self.toasts.error(e.to_string()),
        }
    }

    fn complete_folder(&mut self) {
        let Some(edit) = self.folder_prompt.as_mut() else {
            return;
        };
        let home = dirs::home_dir().unwrap_or_default();
        let done = crate::folder_complete::complete(edit.as_str(), &home);
        *edit = LineEdit::from_text(done.text);
        if done.matches.len() > 1 {
            let mut shown: Vec<&str> = done.matches.iter().take(5).map(String::as_str).collect();
            if done.matches.len() > 5 {
                shown.push("…");
            }
            self.toasts.info(shown.join(" · "));
        }
    }

    fn rescan_folders(&mut self) {
        let Some(host) = self.library_host.clone() else {
            self.toasts.error("no music library configured");
            return;
        };
        let count = self.library_status.folders.len();
        if count == 0 {
            self.toasts
                .info("no music folders yet — press F to add one");
            return;
        }
        match host.apply(LibraryOp::RescanAll) {
            Ok(()) => self.toasts.info(match count {
                1 => "rescanning 1 folder".to_string(),
                n => format!("rescanning {n} folders"),
            }),
            Err(e) => self.toasts.error(e.to_string()),
        }
    }

    /// Re-resolve what is playing when the current file changes.
    pub fn refresh_playing(&mut self, state: &PlayerState) {
        let info = state.current_track.as_ref();
        let path = info.and_then(|t| t.path.as_ref());
        if self.playing.as_ref().map(|p| &p.path) == path {
            return;
        }
        self.playing = info.and_then(|info| {
            let path = info.path.as_ref()?;
            self.library
                .locate(path)
                .map(|track| crate::playing::PlayingRef::from_track(&track))
                .or_else(|| crate::playing::PlayingRef::from_info(info))
        });
    }

    pub fn new(player: PlayerHandle) -> Self {
        Self::with_library(player, None)
    }

    pub fn with_library(player: PlayerHandle, library: Option<Library>) -> Self {
        Self::with_engine(Engine::Local(player), library)
    }

    pub fn with_remote(player: IpcClient, library: Option<Library>) -> Self {
        let host: Arc<dyn LibraryHost> = Arc::new(player.clone());
        let mut app = Self::with_engine(Engine::Remote(player), library);
        app.library_host = Some(host);
        app
    }

    /// Check for a newer release when the TUI runs, and let the user update.
    pub fn with_updates(mut self, updates: Arc<dyn Updates>) -> Self {
        self.updates = Some(updates);
        self
    }

    fn with_engine(player: Engine, library: Option<Library>) -> Self {
        Self {
            player,
            focus: Focus::Library,
            queue_open: false,
            view: View::Browse,
            transport_marquee: Default::default(),
            stage_marquee: Default::default(),
            modal: Modal::None,
            help: HelpState::default(),
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
                .unwrap_or_else(|| temp_fallback("znicz-playlists")),
            playlists: Vec::new(),
            playlist_cursor: Cursor::new(),
            playlist_prompt: None,
            stations_path: znicz_library::default_stations_path()
                .unwrap_or_else(|| temp_fallback("znicz-stations.toml")),
            stations: Vec::new(),
            station_cursor: Cursor::new(),
            message_cursor: Cursor::new(),
            message_h_offset: 0,
            radio_prompt: None,
            meta: MetaCache::new(),
            toasts: Toasts::new(),
            tui: TuiConfig::default(),
            covers: CoverCache::new(),
            picker: Some(ratatui_image::picker::Picker::halfblocks()),
            cover_image: None,
            cover_draw_key: None,
            last_stream_cover: None,
            playing: None,
            library_list_state: ratatui::widgets::ListState::default(),
            queue_list_state: ratatui::widgets::ListState::default(),
            device_list_state: ratatui::widgets::ListState::default(),
            playlist_list_state: ratatui::widgets::ListState::default(),
            station_list_state: ratatui::widgets::ListState::default(),
            messages_list_state: ratatui::widgets::ListState::default(),
            library_host: None,
            library_status: LibraryStatus::default(),
            folder_prompt: None,
            first_run_dismissed: false,
            scans_seen: None,
            library_poll_every: Duration::from_millis(500),
            last_library_poll: None,
            should_quit: false,
            updates: None,
            update: None,
            update_check: None,
            manual_check: None,
            update_step: UpdateStep::Offer,
            update_scroll: 0,
            install_rx: None,
            update_hint: String::new(),
            outcome: RunOutcome::Quit,
        }
    }

    pub fn run(&mut self) -> color_eyre::Result<RunOutcome> {
        let mut terminal = ratatui::init();
        let _mouse = MouseCapture::enable();
        self.picker = Some(make_picker(self.tui.cover_protocol));
        tracing::info!(
            protocol = ?self.picker.as_ref().map(|p| p.protocol_type()),
            "cover renderer"
        );
        self.spawn_update_check();
        let result = self.run_loop(&mut terminal);
        ratatui::restore();
        result?;
        Ok(self.outcome.clone())
    }

    /// Ask for the newer release on a background thread; `poll_updates` reads it.
    fn spawn_update_check(&mut self) {
        let Some(updates) = self.updates.clone() else {
            return;
        };
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(updates.latest());
        });
        self.update_check = Some(rx);
    }

    /// Take the background check's answer when it has arrived.
    pub(crate) fn poll_updates(&mut self) {
        let Some(rx) = &self.update_check else {
            return;
        };
        match rx.try_recv() {
            Ok(Some(release)) => {
                self.update_check = None;
                self.on_update_found(release);
            }
            Ok(None) | Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                self.update_check = None;
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
        }
    }

    /// Remember a newer release, and announce it once per version.
    pub(crate) fn on_update_found(&mut self, release: Release) {
        let version = release.version.clone();
        self.update = Some(release);
        let Some(updates) = self.updates.clone() else {
            return;
        };
        if updates.notified().as_deref() != Some(version.as_str()) {
            self.toasts
                .info(format!("Znicz {version} is available · ? then u to update"));
            updates.set_notified(&version);
        }
    }

    /// Run the update check now, on this thread (tests).
    #[cfg(test)]
    pub(crate) fn check_updates_now(&mut self) {
        let Some(updates) = self.updates.clone() else {
            return;
        };
        if let Some(release) = updates.latest() {
            self.on_update_found(release);
        }
    }

    /// `u` in Help: open the known update, or else check for one right away.
    pub(crate) fn update_or_check(&mut self) {
        if self.update.is_some() {
            self.open_update();
            return;
        }
        let Some(updates) = self.updates.clone() else {
            self.toasts.warn("Updates are not available in this build");
            return;
        };
        if self.manual_check.is_some() {
            return;
        }
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(updates.check_now());
        });
        self.manual_check = Some(rx);
        self.toasts.info("Checking for updates…");
    }

    /// Take the answer to `u` in Help when it has arrived.
    pub(crate) fn poll_manual_check(&mut self) {
        let Some(rx) = &self.manual_check else {
            return;
        };
        let result = match rx.try_recv() {
            Ok(result) => result,
            Err(std::sync::mpsc::TryRecvError::Empty) => return,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                self.manual_check = None;
                self.toasts.error("Update check failed");
                return;
            }
        };
        self.manual_check = None;
        match result {
            Ok(Some(release)) => {
                let version = release.version.clone();
                if let Some(updates) = &self.updates {
                    updates.set_notified(&version);
                }
                self.update = Some(release);
                if matches!(self.modal, Modal::None | Modal::Help) {
                    self.open_update();
                } else {
                    self.toasts
                        .info(format!("Znicz {version} is available · ? then u to update"));
                }
            }
            Ok(None) => {
                self.toasts
                    .success(format!("Znicz {} is up to date", env!("CARGO_PKG_VERSION")));
            }
            Err(error) => {
                self.toasts.error(format!("Update check failed: {error}"));
            }
        }
    }

    /// Opened from Help with `u`. A running install, or a finished one, keeps its
    /// step: the modal may have been closed while it worked.
    pub(crate) fn open_update(&mut self) {
        let keeps_step = self.install_rx.is_some() || self.update_step == UpdateStep::Done;
        if !keeps_step {
            let installable = self.updates.as_ref().is_some_and(|u| u.can_install());
            self.update_step = if installable {
                UpdateStep::Offer
            } else {
                UpdateStep::NotInstallable
            };
        }
        self.update_hint = self.updates.as_ref().map(|u| u.hint()).unwrap_or_default();
        self.update_scroll = 0;
        self.modal = Modal::Update;
    }

    /// Take the install's result when the install thread has reported it.
    pub(crate) fn poll_install(&mut self) {
        let Some(rx) = &self.install_rx else {
            return;
        };
        let result = match rx.try_recv() {
            Ok(result) => result,
            Err(std::sync::mpsc::TryRecvError::Empty) => return,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                Err("the install stopped unexpectedly".to_string())
            }
        };
        self.install_rx = None;
        // With the modal closed nothing on screen would show the result, so say it in a toast.
        let modal_open = self.modal == Modal::Update;
        self.update_step = match result {
            Ok(()) => {
                if !modal_open {
                    let version = self.update.as_ref().map_or("", |r| r.version.as_str());
                    self.toasts
                        .info(format!("Znicz {version} installed · ? then u to restart"));
                }
                UpdateStep::Done
            }
            Err(error) => {
                if !modal_open {
                    self.toasts.error(format!("Update failed: {error}"));
                }
                UpdateStep::Failed(error)
            }
        };
    }

    /// Install the offered release on a background thread.
    fn start_install(&mut self) {
        let (Some(updates), Some(release)) = (self.updates.clone(), self.update.clone()) else {
            self.close_modal();
            return;
        };
        let (tx, rx) = std::sync::mpsc::channel();
        let version = release.version;
        std::thread::spawn(move || {
            let _ = tx.send(updates.install(&version));
        });
        self.install_rx = Some(rx);
        self.update_step = UpdateStep::Installing;
    }

    /// Leave the TUI so the caller restarts the binary.
    fn leave_for_restart(&mut self, resume: Option<Resume>, stop_player: bool) {
        if stop_player {
            if let Err(e) = self.player.shutdown_player() {
                tracing::warn!("player shutdown before restart: {e}");
            }
        }
        self.outcome = RunOutcome::Restart { resume };
        self.should_quit = true;
    }

    fn overlays(&self) -> Overlays {
        Overlays {
            modal: self.modal != Modal::None,
            drawer: self.queue_open,
        }
    }

    fn run_loop(&mut self, terminal: &mut ratatui::DefaultTerminal) -> color_eyre::Result<()> {
        let mut repaint = false;
        loop {
            self.poll_player_events();
            self.poll_updates();
            self.poll_manual_check();
            self.poll_install();
            self.toasts.tick();

            let state = self.player.state();
            if repaint {
                repaint = false;
                // Clear and redraw as one update, so the terminal never shows
                // the blank screen in between (ignored where unsupported).
                execute!(terminal.backend_mut(), BeginSynchronizedUpdate)?;
                terminal.clear()?;
                let drawn = terminal
                    .draw(|frame| views::render(frame, self, &state))
                    .map(|_| ());
                execute!(terminal.backend_mut(), EndSynchronizedUpdate)?;
                drawn?;
            } else {
                terminal.draw(|frame| views::render(frame, self, &state))?;
            }

            // Wake up for input, or on the tick to move the seek bar along.
            if event::poll(TICK_RATE)? {
                let overlays_before = self.overlays();
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
                let overlays_after = self.overlays();
                if needs_full_redraw(self.view, overlays_before, overlays_after) {
                    if cover_needs_retransmit(self.picker.as_ref().map(|p| p.protocol_type())) {
                        self.cover_draw_key = None;
                    }
                    repaint = true;
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
        self.library.set_preferred_layout(self.tui.library_layout);
        match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => self.on_left_click(mouse.column, mouse.row),
            MouseEventKind::ScrollUp => self.on_wheel(-1),
            MouseEventKind::ScrollDown => self.on_wheel(1),
            _ => {}
        }
    }

    fn on_wheel(&mut self, delta: isize) {
        if self.library.is_typing()
            || self.folder_prompt.is_some()
            || self.playlist_prompt.is_some()
            || self.radio_prompt.is_some()
        {
            return;
        }
        match self.modal {
            Modal::Devices => self.device_cursor.step(delta, self.devices.len()),
            Modal::Playlists => self.playlist_cursor.step(delta, self.playlists.len()),
            Modal::Radio => self.station_cursor.step(delta, self.stations.len()),
            Modal::Messages => {
                self.message_cursor.step(delta, self.toasts.history_len());
                self.message_h_offset = 0;
            }
            Modal::Help => self.help.scroll_by(delta),
            Modal::Update => {
                self.update_scroll = self.update_scroll.saturating_add_signed(delta);
            }
            Modal::None => match self.focus {
                Focus::Library if self.view == View::Browse => {
                    self.library.step(delta, self.library_strip())
                }
                Focus::Library => {}
                Focus::Queue if self.queue_open => {
                    let len = self.player.state().queue.len();
                    self.queue_cursor.step(delta, len);
                }
                Focus::Queue => {}
            },
        }
    }

    fn on_left_click(&mut self, column: u16, row: u16) {
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
        if self.folder_prompt.is_some() {
            return;
        }
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
        if self.hits.close.is_some_and(|r| point_in(r, column, row)) {
            // An install cannot be abandoned half-way; its close waits for the result.
            if self.modal == Modal::Update && self.update_step == UpdateStep::Installing {
                return;
            }
            if self.modal != Modal::None {
                self.close_modal();
            } else if self.queue_open {
                self.close_queue();
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
            let list = Rect::new(0, 0, self.list_width, self.list_height);
            if matches!(layout::drawer(list, true), layout::Drawer::Overlay(_)) {
                self.library_click_at(column, row);
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

        self.library_click_at(column, row);
    }

    fn library_click_at(&mut self, column: u16, row: u16) -> bool {
        let strip = self.library_strip();
        for (col, hit) in [
            (ColumnFocus::Artists, self.hits.library_artists),
            (ColumnFocus::Albums, self.hits.library_albums),
            (ColumnFocus::Tracks, self.hits.library_tracks),
        ] {
            if let Some(hit) = hit {
                if let Some(index) = hit.row_at(column, row) {
                    self.library.set_column_index(col, index);
                    self.focus = Focus::Library;
                    return true;
                }
            }
        }
        if let Some(hit) = self.hits.library {
            if let Some(index) = hit.row_at(column, row) {
                self.library.set_index(index, strip);
                self.focus = Focus::Library;
                return true;
            }
        }
        false
    }

    fn on_overlay_click(&mut self, column: u16, row: u16) {
        if let Some(hit) = self.hits.overlay_list {
            if let Some(index) = hit.row_at(column, row) {
                match self.modal {
                    Modal::Devices => self.device_cursor.set(index, self.devices.len()),
                    Modal::Playlists => self.playlist_cursor.set(index, self.playlists.len()),
                    Modal::Radio => self.station_cursor.set(index, self.stations.len()),
                    Modal::Messages => {
                        self.message_cursor.set(index, self.toasts.history_len());
                        self.message_h_offset = 0;
                    }
                    _ => {}
                }
            }
        }
    }

    // --- key handling ---

    /// Handle one keypress.
    ///
    /// Public so the bindings can be driven from tests without a real terminal.
    pub fn on_key(&mut self, key: KeyEvent) {
        self.library.set_preferred_layout(self.tui.library_layout);
        if self.modal == Modal::Help {
            self.on_help_key(key);
            return;
        }
        if self.modal == Modal::Update {
            self.on_update_key(key);
            return;
        }

        // Search, playlist save, and radio prompts are plain text. They must run
        // before global keys, or `s` (stop) turns "To Listen" into "To Liten".
        if self.library.is_typing() {
            self.on_search_key(key);
            return;
        }

        if self.folder_prompt.is_some() {
            self.on_folder_key(key);
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

        if self.modal == Modal::Messages && self.on_messages_key(key) {
            return;
        }

        if self.on_global_key(key) {
            return;
        }

        if self.modal == Modal::Devices {
            self.on_devices_key(key);
            return;
        }

        if self.modal == Modal::Messages {
            return;
        }

        if self.view == View::Stage && self.focus == Focus::Library {
            return;
        }

        match self.focus {
            Focus::Queue => self.on_queue_key(key),
            Focus::Library => self.on_library_key(key),
        }
    }

    fn toggle_stage(&mut self) {
        self.view = match self.view {
            View::Browse => View::Stage,
            View::Stage => View::Browse,
        };
        self.stage_marquee = Default::default();
        self.transport_marquee = Default::default();
    }

    fn on_esc(&mut self) {
        if matches!(
            self.modal,
            Modal::Devices | Modal::Playlists | Modal::Radio | Modal::Messages | Modal::Update
        ) {
            self.close_modal();
        } else if self.queue_open && (self.focus == Focus::Queue || self.view == View::Stage) {
            self.close_queue();
        } else if self.view == View::Stage {
            self.view = View::Browse;
        } else {
            self.library.back();
        }
    }

    /// Open help on whatever the user is looking at now.
    fn open_help(&mut self) {
        let return_to = self.modal;
        self.help = HelpState::open(self.context());
        self.help.return_to = return_to;
        self.modal = Modal::Help;
    }

    /// Keys while help is open: scroll, filter, close. Everything else is inert.
    fn on_help_key(&mut self, key: KeyEvent) {
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            self.should_quit = true;
            return;
        }
        if self.help.editing {
            match key.code {
                KeyCode::Esc => {
                    self.help.query = LineEdit::new();
                    self.help.editing = false;
                    self.help.scroll = 0;
                }
                KeyCode::Enter => self.help.editing = false,
                _ => {
                    if self.help.query.on_key(key) {
                        self.help.scroll = 0;
                    }
                }
            }
            return;
        }

        if key.modifiers.contains(KeyModifiers::CONTROL) {
            match key.code {
                KeyCode::Char('c') => self.should_quit = true,
                KeyCode::Char('d') => self.help.scroll_by(HELP_PAGE),
                KeyCode::Char('u') => self.help.scroll_by(-HELP_PAGE),
                _ => {}
            }
            return;
        }

        match key.code {
            KeyCode::Esc | KeyCode::Char('?') | KeyCode::Char('q') => self.close_modal(),
            KeyCode::Char('/') => self.help.editing = true,
            KeyCode::Char('j') | KeyCode::Down => self.help.scroll_by(1),
            KeyCode::Char('k') | KeyCode::Up => self.help.scroll_by(-1),
            KeyCode::PageDown => self.help.scroll_by(HELP_PAGE),
            KeyCode::PageUp => self.help.scroll_by(-HELP_PAGE),
            KeyCode::Char('g') | KeyCode::Home => self.help.scroll = 0,
            // Clamped to the last page when help is drawn.
            KeyCode::Char('G') | KeyCode::End => self.help.scroll = usize::MAX,
            KeyCode::Char('u') => self.update_or_check(),
            _ => {}
        }
    }

    /// Keys while the Update modal is open.
    fn on_update_key(&mut self, key: KeyEvent) {
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            // Quitting mid-install could leave a half-written copy; the install ends on its own.
            if self.update_step != UpdateStep::Installing {
                self.should_quit = true;
            }
            return;
        }
        match self.update_step {
            UpdateStep::Installing => {}
            UpdateStep::Done => match key.code {
                KeyCode::Enter => {
                    let state = self.player.state();
                    if restart_asks_about_player(state.status, state.queue.len()) {
                        self.update_step = UpdateStep::PlayerPlaying;
                    } else {
                        self.leave_for_restart(None, true);
                    }
                }
                KeyCode::Esc => {
                    self.update = None;
                    self.close_modal();
                }
                _ => {}
            },
            UpdateStep::PlayerPlaying => match key.code {
                KeyCode::Enter => {
                    let state = self.player.state();
                    let seconds = match state.queue.get(state.queue_position) {
                        Some(QueueItem::File { .. }) => state.position.as_secs(),
                        _ => 0,
                    };
                    let resume = Resume {
                        index: state.queue_position,
                        seconds,
                    };
                    self.leave_for_restart(Some(resume), true);
                }
                KeyCode::Esc => self.leave_for_restart(None, false),
                _ => {}
            },
            UpdateStep::Offer | UpdateStep::NotInstallable | UpdateStep::Failed(_) => {
                let scroll = match key.code {
                    KeyCode::Char('j') | KeyCode::Down => Some(1),
                    KeyCode::Char('k') | KeyCode::Up => Some(-1),
                    KeyCode::PageDown => Some(HELP_PAGE),
                    KeyCode::PageUp => Some(-HELP_PAGE),
                    _ => None,
                };
                if let Some(delta) = scroll {
                    self.update_scroll = self.update_scroll.saturating_add_signed(delta);
                    return;
                }
                match key.code {
                    KeyCode::Enter if self.update_step == UpdateStep::Offer => self.start_install(),
                    KeyCode::Enter | KeyCode::Esc | KeyCode::Char('q') => self.close_modal(),
                    _ => {}
                }
            }
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
        if self.library.is_typing()
            || self.folder_prompt.is_some()
            || self.playlist_prompt.is_some()
            || self.radio_prompt.is_some()
        {
            return false;
        }
        match key.code {
            KeyCode::Char('q') => self.should_quit = true,
            KeyCode::Char('?') => self.open_help(),
            KeyCode::Char(',') => self.toggle_devices_modal(),
            KeyCode::Char('i') => self.toggle_stage(),
            KeyCode::Char('v') => self.toggle_stage(),
            KeyCode::Char('P') => self.toggle_playlists_modal(),
            KeyCode::Char('R') => self.toggle_radio_modal(),
            KeyCode::Char('M') => self.toggle_messages_modal(),

            KeyCode::Char(']') if self.modal.blocks_list_focus() => {}
            KeyCode::Char(']') => {
                if self.queue_open {
                    self.close_queue();
                } else {
                    self.open_queue();
                }
            }

            KeyCode::Tab if self.modal.blocks_list_focus() => {}
            KeyCode::Tab => {
                let strip = self.library_strip();
                if self.view == View::Browse
                    && self.focus == Focus::Library
                    && matches!(self.library.mode(), Mode::Browse)
                    && self.library.effective_layout(strip) == EffectiveLayout::Columns
                    && !self.library.artists().is_empty()
                    && self.modal == Modal::None
                    && !self.library.is_typing()
                {
                    self.library.cycle_column(true);
                } else if !self.queue_open {
                    // `]` opens the drawer; Tab only moves focus
                } else if layout::is_sheet(self.list_width, true) || self.view == View::Stage {
                    // no-op in sheet mode, and on Stage (the library is hidden)
                } else {
                    self.swap_focus();
                }
            }
            KeyCode::BackTab if self.modal.blocks_list_focus() => {}
            KeyCode::BackTab => {
                let strip = self.library_strip();
                if self.view == View::Browse
                    && self.focus == Focus::Library
                    && matches!(self.library.mode(), Mode::Browse)
                    && self.library.effective_layout(strip) == EffectiveLayout::Columns
                    && !self.library.artists().is_empty()
                    && self.modal == Modal::None
                    && !self.library.is_typing()
                {
                    self.library.cycle_column(false);
                } else if !self.queue_open {
                    // no-op while drawer is closed
                } else if layout::is_sheet(self.list_width, true) || self.view == View::Stage {
                    // no-op in sheet mode, and on Stage (the library is hidden)
                } else {
                    self.swap_focus();
                }
            }

            KeyCode::Char(' ') => {
                if !self.library_toggle_tree_expand() {
                    self.toggle_pause();
                }
            }
            KeyCode::Char('s') => self.apply(Command::Stop, None),
            KeyCode::Char('n') => self.skip_track(false),
            KeyCode::Char('p') => self.skip_track(true),

            // Alt-↑/↓ belong to the queue drawer only; elsewhere they fall
            // through and move the cursor like ↑/↓.
            KeyCode::Up
                if key.modifiers.contains(KeyModifiers::ALT) && self.queue_reorder_active() =>
            {
                self.move_queue_row(-1)
            }
            KeyCode::Down
                if key.modifiers.contains(KeyModifiers::ALT) && self.queue_reorder_active() =>
            {
                self.move_queue_row(1)
            }
            KeyCode::Right if key.modifiers.contains(KeyModifiers::ALT) => {
                self.pan_titles(1);
            }
            KeyCode::Left if key.modifiers.contains(KeyModifiers::ALT) => {
                self.pan_titles(-1);
            }
            KeyCode::Right | KeyCode::Left => {
                if let Some(seconds) = seek_seconds(key) {
                    self.seek_relative(seconds);
                }
            }
            KeyCode::Char('l') => self.library_horizontal(true),
            KeyCode::Char('h') => self.library_horizontal(false),

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
        self.library.clamp_pan(
            self.title_slot(),
            self.library_strip(),
            self.playing.as_ref(),
        );
    }

    /// What the footer and help describe right now.
    pub fn context(&self) -> Context {
        if self.library.is_typing() {
            return Context::Search;
        }
        if self.folder_prompt.is_some() {
            return Context::FolderPrompt;
        }
        match &self.radio_prompt {
            Some(RadioPrompt::Form { .. }) => return Context::StationPrompt,
            Some(RadioPrompt::Copy(_)) => return Context::Prompt,
            None => {}
        }
        if self.playlist_prompt.is_some() {
            return Context::Prompt;
        }
        match self.modal {
            Modal::Help if self.help.editing => Context::HelpFilter,
            Modal::Help => Context::Help,
            Modal::Devices => Context::Devices,
            Modal::Playlists => Context::Playlists,
            Modal::Radio => Context::Radio,
            Modal::Messages => Context::Messages,
            Modal::Update => Context::Update,
            Modal::None
                if self.view == View::Stage && !(self.queue_open && self.focus == Focus::Queue) =>
            {
                Context::Stage
            }
            Modal::None => match self.focus {
                Focus::Queue => Context::Queue,
                Focus::Library
                    if self.library.effective_layout(self.library_strip())
                        == EffectiveLayout::Tree =>
                {
                    Context::LibraryTree
                }
                Focus::Library => Context::Library,
            },
        }
    }

    fn library_strip(&self) -> usize {
        let list = Rect::new(0, 0, self.list_width.max(2), self.list_height.max(3));
        layout::strip_inner(list, self.queue_open)
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

    /// Another modal is about to replace Messages: tidy what it leaves behind.
    fn leave_messages(&mut self) {
        if self.modal == Modal::Messages {
            self.toasts.clear_fresh();
        }
    }

    fn toggle_devices_modal(&mut self) {
        self.leave_messages();
        self.modal = if self.modal == Modal::Devices {
            Modal::None
        } else {
            Modal::Devices
        };
    }

    fn toggle_playlists_modal(&mut self) {
        if self.modal == Modal::Playlists {
            self.modal = Modal::None;
            self.playlist_prompt = None;
        } else {
            self.leave_messages();
            self.reload_playlists();
            self.modal = Modal::Playlists;
        }
    }

    fn toggle_radio_modal(&mut self) {
        if self.modal == Modal::Radio {
            self.modal = Modal::None;
            self.radio_prompt = None;
        } else {
            self.leave_messages();
            self.playlist_prompt = None;
            self.reload_stations();
            self.modal = Modal::Radio;
        }
    }

    fn toggle_messages_modal(&mut self) {
        if self.modal == Modal::Messages {
            self.close_modal();
        } else {
            self.toasts.mark_read();
            self.message_cursor.first();
            self.message_h_offset = 0;
            self.modal = Modal::Messages;
        }
    }

    /// Close whatever modal is open, tidying what it leaves behind.
    fn close_modal(&mut self) {
        if self.modal == Modal::Help {
            self.modal = self.help.return_to;
            return;
        }
        self.leave_messages();
        self.modal = Modal::None;
        self.playlist_prompt = None;
        self.radio_prompt = None;
    }

    /// Keys the Messages list owns. Returns false for anything else.
    fn on_messages_key(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Char('C') => {
                // No message here: it would land in the history just cleared.
                self.toasts.clear_history();
                self.message_cursor.first();
                self.message_h_offset = 0;
                true
            }
            _ => false,
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
            Modal::Messages | Modal::Update => {}
            Modal::Playlists => self.reload_playlists(),
            Modal::Devices => {
                self.devices = load_output_devices();
                self.device_cursor.clamp(self.devices.len());
                self.toasts
                    .info(format!("{} devices found", self.devices.len()));
            }
            _ if self.view == View::Stage => {}
            _ => {
                self.library.reload_albums();
                // Re-resolve from the reloaded library on the next frame.
                self.playing = None;
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
        let strip = self.library_strip();
        match key.code {
            KeyCode::Char('/') => self.library.begin_search(),
            KeyCode::Enter => self.library_enter(),
            KeyCode::Char('a') => {
                let tracks = self.library.selected_tracks(strip);
                self.enqueue(tracks);
            }
            KeyCode::Char('A') => {
                let tracks = self.library.listed_tracks(strip);
                self.enqueue(tracks);
            }
            KeyCode::Char('J') => self.jump_to_playing(),
            KeyCode::Char('F') => self.open_folder_prompt(),
            KeyCode::Char('U') => self.rescan_folders(),
            KeyCode::Char('o') => {
                let _ = self.library_toggle_tree_expand();
            }
            _ => {}
        }
    }

    /// `h` / `l`: move left or right in the library. Ignored anywhere else.
    fn library_horizontal(&mut self, forward: bool) {
        if self.view != View::Browse || self.focus != Focus::Library || self.modal != Modal::None {
            return;
        }
        let strip = self.library_strip();
        self.library.move_horizontal(forward, strip);
    }

    /// `J`: select the playing track in the library.
    fn jump_to_playing(&mut self) {
        if self.modal != Modal::None {
            return;
        }
        let path = self.player.state().current_track.and_then(|t| t.path);
        self.jump_to_path(path.as_deref());
    }

    /// Select `path` in the library, or say why not. `None`: nothing (or a stream) plays.
    pub fn jump_to_path(&mut self, path: Option<&Path>) {
        let Some(path) = path else {
            self.toasts.info("nothing playing in the library");
            return;
        };
        let strip = self.library_strip();
        let found = match self.library.locate(path) {
            Some(track) => self.library.reveal(&track, strip),
            None => false,
        };
        if !found {
            self.toasts.info("playing track is not in the library");
        }
    }

    /// Space (global) and `o` (library): expand/collapse tree parents.
    fn library_toggle_tree_expand(&mut self) -> bool {
        let strip = self.library_strip();
        if self.view != View::Browse
            || self.focus != Focus::Library
            || !matches!(self.library.mode(), Mode::Browse)
            || self.modal != Modal::None
        {
            return false;
        }
        if self.library.effective_layout(strip) != EffectiveLayout::Tree {
            return false;
        }
        match self.library.selected(strip) {
            Some(Item::Artist(_) | Item::Album(_)) => {
                self.library.toggle_expand_at_cursor();
                true
            }
            _ => false,
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

    /// Enter in the library: land from search, drill browse, or play a track.
    fn library_enter(&mut self) {
        let strip = self.library_strip();
        enum Action {
            FocusArtist(String),
            FocusAlbum {
                album: String,
                album_artist: Option<String>,
            },
            BrowseEnter,
            Play(Track),
        }
        let action = match self.library.selected(strip) {
            Some(Item::Artist(artist)) => {
                if matches!(self.library.mode(), Mode::Search(_)) {
                    Action::FocusArtist(artist.name.clone())
                } else {
                    Action::BrowseEnter
                }
            }
            Some(Item::Album(album)) => {
                if matches!(self.library.mode(), Mode::Search(_)) {
                    Action::FocusAlbum {
                        album: album.album.clone(),
                        album_artist: album.album_artist.clone(),
                    }
                } else {
                    Action::BrowseEnter
                }
            }
            Some(Item::Track(track)) => Action::Play(track.clone()),
            None => return,
        };
        match action {
            Action::FocusArtist(name) => {
                self.library.focus_artist(&name);
            }
            Action::FocusAlbum {
                album,
                album_artist,
            } => {
                self.library.focus_album(&album, album_artist.as_deref());
            }
            Action::BrowseEnter => {
                let _ = self.library.enter(strip);
            }
            Action::Play(track) => {
                let path = track.path.clone();
                let entry = entry_from_track(&track);
                self.meta.insert(path.clone(), entry);
                self.apply(Command::Play(znicz_core::QueueItem::file(path)), None);
            }
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
        if self.modal == Modal::Messages {
            self.toasts.history_len()
        } else if self.modal == Modal::Radio {
            self.stations.len()
        } else if self.modal == Modal::Playlists {
            self.playlists.len()
        } else if self.modal == Modal::Devices {
            self.devices.len()
        } else if self.focus == Focus::Queue && self.queue_open {
            self.player.state().queue.len()
        } else {
            self.library.len(self.library_strip())
        }
    }

    fn step(&mut self, delta: isize) {
        let len = self.list_len();
        if self.modal == Modal::Messages {
            self.message_cursor.step(delta, len);
            self.message_h_offset = 0;
        } else if self.modal == Modal::Radio {
            self.station_cursor.step(delta, len);
        } else if self.modal == Modal::Playlists {
            self.playlist_cursor.step(delta, len);
        } else if self.modal == Modal::Devices {
            self.device_cursor.step(delta, len);
        } else if self.focus == Focus::Queue && self.queue_open {
            self.queue_cursor.step(delta, len);
            self.queue_h_offset = 0;
        } else if self.view == View::Browse {
            self.library.step(delta, self.library_strip());
        }
    }

    fn page(&mut self, delta: isize) {
        let len = self.list_len();
        if self.modal == Modal::Messages {
            self.message_cursor.page(delta, len);
            self.message_h_offset = 0;
        } else if self.modal == Modal::Radio {
            self.station_cursor.page(delta, len);
        } else if self.modal == Modal::Playlists {
            self.playlist_cursor.page(delta, len);
        } else if self.modal == Modal::Devices {
            self.device_cursor.page(delta, len);
        } else if self.focus == Focus::Queue && self.queue_open {
            self.queue_cursor.page(delta, len);
            self.queue_h_offset = 0;
        } else if self.view == View::Browse {
            self.library.page(delta, self.library_strip());
        }
    }

    fn go_first(&mut self) {
        if self.modal == Modal::Messages {
            self.message_cursor.first();
            self.message_h_offset = 0;
        } else if self.modal == Modal::Radio {
            self.station_cursor.first();
        } else if self.modal == Modal::Playlists {
            self.playlist_cursor.first();
        } else if self.modal == Modal::Devices {
            self.device_cursor.first();
        } else if self.focus == Focus::Queue && self.queue_open {
            self.queue_cursor.first();
            self.queue_h_offset = 0;
        } else if self.view == View::Browse {
            self.library.first(self.library_strip());
        }
    }

    fn go_last(&mut self) {
        let len = self.list_len();
        if self.modal == Modal::Messages {
            self.message_cursor.last(len);
            self.message_h_offset = 0;
        } else if self.modal == Modal::Radio {
            self.station_cursor.last(len);
        } else if self.modal == Modal::Playlists {
            self.playlist_cursor.last(len);
        } else if self.modal == Modal::Devices {
            self.device_cursor.last(len);
        } else if self.focus == Focus::Queue && self.queue_open {
            self.queue_cursor.last(len);
            self.queue_h_offset = 0;
        } else if self.view == View::Browse {
            self.library.last(self.library_strip());
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
        let volume = (((state.volume + delta) * 100.0).round() / 100.0).clamp(0.0, 1.0);
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

    /// `Alt-↑` / `Alt-↓`: move the highlighted queue row; the cursor follows it.
    fn queue_reorder_active(&self) -> bool {
        self.modal == Modal::None && self.queue_open && self.focus == Focus::Queue
    }

    fn move_queue_row(&mut self, delta: isize) {
        if !self.queue_reorder_active() {
            return;
        }
        let len = self.player.state().queue.len();
        let Some(from) = self.queue_cursor.selected(len) else {
            return;
        };
        let to = from as isize + delta;
        if to < 0 || to as usize >= len {
            return;
        }
        let to = to as usize;
        match self.player.send_blocking(Command::QueueMove { from, to }) {
            Ok(()) => {
                self.queue_cursor.set(to, len);
                self.queue_h_offset = 0;
            }
            Err(e) => self.toasts.error(e.to_string()),
        }
    }

    fn pan_titles(&mut self, dir: isize) {
        if self.modal == Modal::Messages {
            let next = self.message_h_offset as isize + dir;
            // Clamped to the text when drawn.
            self.message_h_offset = next.max(0) as usize;
            return;
        }
        if self.focus == Focus::Queue && self.queue_open {
            self.pan_queue(dir);
        } else if self.view == View::Browse {
            self.library.pan(
                dir,
                self.title_slot(),
                self.library_strip(),
                self.playing.as_ref(),
            );
        }
    }

    fn pan_queue(&mut self, dir: isize) {
        let state = self.player.state();
        let max = self
            .selected_queue_middle(&state)
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

    fn selected_queue_middle(&self, state: &PlayerState) -> usize {
        let Some(index) = self.queue_cursor.selected(state.queue.len()) else {
            return 0;
        };
        state
            .queue
            .get(index)
            .map(|item| self.queue_label_len(item))
            .unwrap_or(0)
    }

    pub(crate) fn clamp_queue_pan(&mut self, state: &PlayerState) {
        let max = self
            .selected_queue_middle(state)
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

/// Seconds an arrow key seeks: 5 plain, 30 with Shift. `None` for other keys.
fn seek_seconds(key: KeyEvent) -> Option<i64> {
    let step = if key.modifiers.contains(KeyModifiers::SHIFT) {
        SEEK_LARGE
    } else {
        SEEK_SMALL
    };
    match key.code {
        KeyCode::Right => Some(step),
        KeyCode::Left => Some(-step),
        _ => None,
    }
}

fn entry_from_track(track: &Track) -> Entry {
    Entry {
        title: track.title.clone(),
        artist: track.artist.clone(),
        album: track.album.clone(),
        duration: track.duration_secs.map(Duration::from_secs_f64),
    }
}

/// Which overlays sit over the main view this frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Overlays {
    pub(crate) modal: bool,
    pub(crate) drawer: bool,
}

/// Kitty and Sixel images bypass ratatui's cell diff, so on Stage any overlay
/// that goes away leaves its cells painted over the cover until a full repaint.
pub(crate) fn needs_full_redraw(view: View, before: Overlays, after: Overlays) -> bool {
    let modal_closed = before.modal && !after.modal;
    let drawer_closed = before.drawer && !after.drawer;
    view == View::Stage && (modal_closed || drawer_closed)
}

/// After a full clear, Sixel and iTerm2 covers are re-sent from the encoded
/// data ratatui-image keeps in the image cell, so they need no new encode.
/// Kitty transmits an image once and only places it afterwards: encode it
/// again so it is transmitted again.
pub(crate) fn cover_needs_retransmit(protocol: Option<ProtocolType>) -> bool {
    matches!(protocol, Some(ProtocolType::Kitty))
}

/// Label for the repeat indicator in the status bar.
pub fn repeat_label(mode: RepeatMode) -> &'static str {
    match mode {
        RepeatMode::Off => "repeat off",
        RepeatMode::All => "repeat all",
        RepeatMode::One => "repeat one",
    }
}

/// A finished job reloads the browse lists only once the queue is drained and
/// the user is not looking at search results.
fn should_reload(status: &LibraryStatus, mode: &Mode) -> bool {
    !status.busy() && !matches!(mode, Mode::Search(_))
}

/// The folder prompt's starting text: the platform music folder, else home.
fn default_folder_text() -> String {
    let home = dirs::home_dir();
    match (dirs::audio_dir(), home.as_deref()) {
        (Some(music), Some(home)) if music.is_dir() => crate::folder_complete::tilde(&music, home),
        (Some(music), None) if music.is_dir() => music.display().to_string(),
        _ => "~/".to_string(),
    }
}

/// Where to keep data when the user data directory cannot be found: the
/// temp dir, which the system may empty, so say so in the log.
fn temp_fallback(name: &str) -> PathBuf {
    let path = std::env::temp_dir().join(name);
    tracing::warn!(path = %path.display(), "no user data directory; using a temporary path");
    path
}

#[cfg(test)]
mod tests {
    use znicz_core::LibraryStatus;

    #[test]
    fn reload_waits_for_the_last_job_and_leaves_search_alone() {
        let idle = LibraryStatus::default();
        let queued = LibraryStatus {
            queued: 1,
            ..LibraryStatus::default()
        };
        assert!(super::should_reload(&idle, &super::Mode::Browse));
        assert!(!super::should_reload(&queued, &super::Mode::Browse));
        assert!(!super::should_reload(
            &idle,
            &super::Mode::Search("x".into())
        ));
    }

    use super::*;

    #[test]
    fn closing_a_modal_or_the_drawer_over_the_stage_forces_a_repaint() {
        let none = Overlays {
            modal: false,
            drawer: false,
        };
        let modal = Overlays {
            modal: true,
            drawer: false,
        };
        let drawer = Overlays {
            modal: false,
            drawer: true,
        };
        let both = Overlays {
            modal: true,
            drawer: true,
        };
        assert!(needs_full_redraw(View::Stage, modal, none));
        assert!(needs_full_redraw(View::Stage, drawer, none));
        assert!(
            needs_full_redraw(View::Stage, both, drawer),
            "modal closes, drawer stays"
        );
        assert!(
            needs_full_redraw(View::Stage, both, modal),
            "drawer closes, modal stays"
        );
        assert!(!needs_full_redraw(View::Browse, both, none));
        assert!(!needs_full_redraw(View::Stage, none, both));
        assert!(!needs_full_redraw(View::Stage, both, both));
        assert!(!needs_full_redraw(View::Stage, drawer, both));
    }

    #[test]
    fn only_kitty_covers_are_encoded_again_after_a_repaint() {
        assert!(cover_needs_retransmit(Some(ProtocolType::Kitty)));
        assert!(!cover_needs_retransmit(Some(ProtocolType::Sixel)));
        assert!(!cover_needs_retransmit(Some(ProtocolType::Iterm2)));
        assert!(!cover_needs_retransmit(Some(ProtocolType::Halfblocks)));
        assert!(!cover_needs_retransmit(None));
    }

    #[test]
    fn toggling_the_view_restarts_both_marquees() {
        let (player, _thread) = znicz_core::spawn_player(znicz_core::AudioConfig::default());
        let mut app = App::with_library(player, None);
        let t0 = std::time::Instant::now();
        let text = "a long title that scrolls";
        app.stage_marquee.offset(text, 5, t0);
        app.transport_marquee.offset(text, 5, t0);
        let later = t0 + std::time::Duration::from_millis(4_000);
        assert!(app.stage_marquee.offset(text, 5, later) > 0);
        app.toggle_stage();
        app.toggle_stage();
        assert_eq!(app.stage_marquee.offset(text, 5, later), 0);
        assert_eq!(app.transport_marquee.offset(text, 5, later), 0);
    }

    #[test]
    fn arrows_seek_five_seconds_and_shift_arrows_thirty() {
        let plain = |code| KeyEvent::new(code, KeyModifiers::NONE);
        let shifted = |code| KeyEvent::new(code, KeyModifiers::SHIFT);
        assert_eq!(seek_seconds(plain(KeyCode::Right)), Some(5));
        assert_eq!(seek_seconds(plain(KeyCode::Left)), Some(-5));
        assert_eq!(seek_seconds(shifted(KeyCode::Right)), Some(30));
        assert_eq!(seek_seconds(shifted(KeyCode::Left)), Some(-30));
        assert_eq!(seek_seconds(plain(KeyCode::Char('l'))), None);
    }

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

    #[test]
    fn reloading_the_library_forgets_the_resolved_playing_row() {
        let mut app = App::with_library(test_player(), None);
        app.playing = Some(crate::playing::PlayingRef {
            path: std::path::PathBuf::from("/music/a.flac"),
            artist: Some("A".into()),
            album: None,
        });
        app.on_key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE));
        assert_eq!(app.playing, None, "the next frame must re-resolve");
    }

    use crate::update::{Release, Updates};
    use std::sync::Mutex;

    /// A fake updater: a fixed release, a recorded install, a notified marker,
    /// and the answer `check_now` gives (counting its calls).
    pub(crate) struct FakeUpdates {
        pub(crate) release: Option<Release>,
        pub(crate) installable: bool,
        pub(crate) install_result: Mutex<Result<(), String>>,
        pub(crate) installs: Mutex<Vec<String>>,
        pub(crate) notified: Mutex<Option<String>>,
        pub(crate) check_result: Mutex<Result<Option<Release>, String>>,
        pub(crate) checks: Mutex<usize>,
    }

    fn fake_release(version: &str) -> Release {
        Release {
            version: version.into(),
            notes: "Notes".into(),
            url: "https://example.test/release".into(),
        }
    }

    impl FakeUpdates {
        pub(crate) fn offering(version: &str) -> Self {
            Self {
                release: Some(fake_release(version)),
                ..Self::checking(Ok(None))
            }
        }

        /// No release known from the daily check; `check_now` answers `result`.
        pub(crate) fn checking(result: Result<Option<Release>, String>) -> Self {
            Self {
                release: None,
                installable: true,
                install_result: Mutex::new(Ok(())),
                installs: Mutex::new(Vec::new()),
                notified: Mutex::new(None),
                check_result: Mutex::new(result),
                checks: Mutex::new(0),
            }
        }
    }

    impl Updates for FakeUpdates {
        fn latest(&self) -> Option<Release> {
            self.release.clone()
        }

        fn can_install(&self) -> bool {
            self.installable
        }

        fn install(&self, version: &str) -> Result<(), String> {
            self.installs.lock().unwrap().push(version.to_string());
            self.install_result.lock().unwrap().clone()
        }

        fn hint(&self) -> String {
            "rebuild it".into()
        }

        fn notified(&self) -> Option<String> {
            self.notified.lock().unwrap().clone()
        }

        fn set_notified(&self, version: &str) {
            *self.notified.lock().unwrap() = Some(version.to_string());
        }

        fn check_now(&self) -> Result<Option<Release>, String> {
            *self.checks.lock().unwrap() += 1;
            self.check_result.lock().unwrap().clone()
        }
    }

    fn toast_texts(app: &App) -> Vec<String> {
        app.toasts.recent().iter().map(|t| t.text.clone()).collect()
    }

    #[test]
    fn a_new_release_is_announced_once_and_remembered() {
        let fake = Arc::new(FakeUpdates::offering("9.9.9"));
        let mut app = App::with_library(test_player(), None).with_updates(fake.clone());
        app.check_updates_now();
        assert!(app.update.is_some());
        assert!(
            toast_texts(&app)
                .iter()
                .any(|t| t.contains("9.9.9 is available"))
        );
        assert_eq!(fake.notified(), Some("9.9.9".into()));
    }

    #[test]
    fn a_release_already_announced_stays_quiet() {
        let fake = FakeUpdates::offering("9.9.9");
        *fake.notified.lock().unwrap() = Some("9.9.9".into());
        let fake = Arc::new(fake);
        let mut app = App::with_library(test_player(), None).with_updates(fake.clone());
        app.check_updates_now();
        assert!(app.update.is_some(), "the update is still known");
        assert!(!toast_texts(&app).iter().any(|t| t.contains("is available")));
    }

    #[test]
    fn u_in_help_opens_the_update_modal_only_when_one_is_known() {
        let mut app = App::with_library(test_player(), None)
            .with_updates(Arc::new(FakeUpdates::offering("9.9.9")));
        app.check_updates_now();
        app.on_key(KeyEvent::new(KeyCode::Char('?'), KeyModifiers::NONE));
        assert_eq!(app.modal, Modal::Help);
        app.on_key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::NONE));
        assert_eq!(app.modal, Modal::Update);
        app.on_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(app.modal, Modal::None, "Esc closes the update modal");
    }

    #[test]
    fn u_in_help_does_nothing_without_an_update() {
        let mut app = App::with_library(test_player(), None);
        app.on_key(KeyEvent::new(KeyCode::Char('?'), KeyModifiers::NONE));
        app.on_key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::NONE));
        assert_eq!(app.modal, Modal::Help);
    }

    /// Poll the manual check until it reports, for at most two seconds.
    fn wait_for_manual_check(app: &mut App) {
        let deadline = Instant::now() + Duration::from_secs(2);
        while app.manual_check.is_some() && Instant::now() < deadline {
            app.poll_manual_check();
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn u_in_help_checks_now_when_no_update_is_known() {
        let fake = Arc::new(FakeUpdates::checking(Ok(Some(fake_release("9.9.9")))));
        let mut app = App::with_library(test_player(), None).with_updates(fake.clone());
        press(&mut app, KeyCode::Char('?'));
        press(&mut app, KeyCode::Char('u'));
        assert!(
            toast_texts(&app)
                .iter()
                .any(|t| t == "Checking for updates…")
        );
        wait_for_manual_check(&mut app);
        assert_eq!(app.modal, Modal::Update);
        assert_eq!(
            app.update.as_ref().map(|r| r.version.as_str()),
            Some("9.9.9")
        );
    }

    #[test]
    fn a_manual_check_that_finds_nothing_says_up_to_date() {
        let fake = Arc::new(FakeUpdates::checking(Ok(None)));
        let mut app = App::with_library(test_player(), None).with_updates(fake);
        press(&mut app, KeyCode::Char('?'));
        press(&mut app, KeyCode::Char('u'));
        wait_for_manual_check(&mut app);
        assert!(app.update.is_none());
        assert!(
            toast_texts(&app)
                .iter()
                .any(|t| t.contains("is up to date"))
        );
    }

    #[test]
    fn a_failed_manual_check_shows_the_error() {
        let fake = Arc::new(FakeUpdates::checking(Err("offline".into())));
        let mut app = App::with_library(test_player(), None).with_updates(fake);
        press(&mut app, KeyCode::Char('?'));
        press(&mut app, KeyCode::Char('u'));
        wait_for_manual_check(&mut app);
        assert!(toast_texts(&app).iter().any(|t| t.contains("offline")));
    }

    #[test]
    fn a_second_u_while_checking_starts_no_second_check() {
        let fake = Arc::new(FakeUpdates::checking(Ok(None)));
        let mut app = App::with_library(test_player(), None).with_updates(fake.clone());
        press(&mut app, KeyCode::Char('?'));
        press(&mut app, KeyCode::Char('u'));
        press(&mut app, KeyCode::Char('u'));
        wait_for_manual_check(&mut app);
        assert_eq!(*fake.checks.lock().unwrap(), 1);
    }

    #[test]
    fn u_with_a_known_update_opens_it_without_checking() {
        let fake = Arc::new(FakeUpdates::offering("9.9.9"));
        let mut app = App::with_library(test_player(), None).with_updates(fake.clone());
        app.check_updates_now();
        press(&mut app, KeyCode::Char('?'));
        press(&mut app, KeyCode::Char('u'));
        assert_eq!(app.modal, Modal::Update);
        assert_eq!(*fake.checks.lock().unwrap(), 0);
    }

    fn app_offering(fake: Arc<FakeUpdates>) -> App {
        let mut app = App::with_library(test_player(), None).with_updates(fake);
        app.check_updates_now();
        app
    }

    fn press(app: &mut App, code: KeyCode) {
        app.on_key(KeyEvent::new(code, KeyModifiers::NONE));
    }

    /// Poll the install until it leaves `Installing`, for at most two seconds.
    fn wait_for_install(app: &mut App) {
        let deadline = Instant::now() + Duration::from_secs(2);
        while app.update_step == UpdateStep::Installing && Instant::now() < deadline {
            app.poll_install();
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn rendered(app: &mut App) -> String {
        let state = app.player.state();
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 30)).unwrap();
        terminal
            .draw(|frame| views::render(frame, app, &state))
            .unwrap();
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect()
    }

    /// The screen as one string per row, for checking what a frame shows.
    fn screen_rows(app: &mut App) -> Vec<String> {
        let state = app.player.state();
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 30)).unwrap();
        terminal
            .draw(|frame| views::render(frame, app, &state))
            .unwrap();
        let buffer = terminal.backend().buffer();
        let width = buffer.area.width as usize;
        buffer
            .content()
            .chunks(width)
            .map(|row| row.iter().map(|cell| cell.symbol()).collect())
            .collect()
    }

    #[test]
    fn a_long_queue_draws_the_rows_around_the_cursor() {
        let mut app = App::with_library(test_player(), None);
        let items: Vec<QueueItem> = (0..200)
            .map(|i| QueueItem::File {
                path: PathBuf::from(format!("/music/track-{i:03}.flac")),
            })
            .collect();
        app.player
            .send_blocking(Command::QueueAdd(items))
            .expect("queue add");
        press(&mut app, KeyCode::Char(']'));

        app.queue_cursor.set(150, 200);
        let rows = screen_rows(&mut app);
        assert!(
            rows.iter()
                .any(|r| r.contains("track-150") && r.contains("151")),
            "the selected track shows its row number 151"
        );
        assert!(
            !rows.iter().any(|r| r.contains("track-000")),
            "the first row has scrolled out of view"
        );

        app.queue_cursor.set(0, 200);
        let rows = screen_rows(&mut app);
        assert!(
            rows.iter()
                .any(|r| r.contains("track-000") && r.contains("  1 ")),
            "back at the top, row 1 is drawn first"
        );
        assert!(!rows.iter().any(|r| r.contains("track-150")));
    }

    #[test]
    #[ignore]
    fn perf_queue_render_2000() {
        let mut app = App::with_library(test_player(), None);
        let items: Vec<QueueItem> = (0..2000)
            .map(|i| QueueItem::File {
                path: PathBuf::from(format!("/music/{i}.flac")),
            })
            .collect();
        app.player
            .send_blocking(Command::QueueAdd(items))
            .expect("queue add");
        press(&mut app, KeyCode::Char(']'));
        assert!(app.queue_open, "] should open the queue drawer");

        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(80, 40)).unwrap();
        let state = app.player.state();
        assert_eq!(state.queue.len(), 2000);

        const FRAMES: u32 = 100;
        let start = Instant::now();
        for _ in 0..FRAMES {
            terminal
                .draw(|frame| views::render(frame, &mut app, &state))
                .unwrap();
        }
        let mean_us = start.elapsed().as_secs_f64() * 1e6 / f64::from(FRAMES);
        println!("PERF perf_queue_render_2000: {mean_us:.1} µs");
    }

    #[test]
    fn enter_installs_the_offered_version_and_then_asks_to_restart() {
        let fake = Arc::new(FakeUpdates::offering("9.9.9"));
        let mut app = app_offering(fake.clone());
        app.open_update();
        assert_eq!(app.update_step, UpdateStep::Offer);
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.update_step, UpdateStep::Installing);
        wait_for_install(&mut app);
        assert_eq!(app.update_step, UpdateStep::Done);
        assert_eq!(fake.installs.lock().unwrap().as_slice(), ["9.9.9"]);
    }

    #[test]
    fn a_failed_install_shows_the_error() {
        let fake = FakeUpdates::offering("9.9.9");
        *fake.install_result.lock().unwrap() = Err("boom".into());
        let mut app = app_offering(Arc::new(fake));
        app.open_update();
        press(&mut app, KeyCode::Enter);
        wait_for_install(&mut app);
        assert_eq!(app.update_step, UpdateStep::Failed("boom".into()));
        assert!(rendered(&mut app).contains("boom"));
    }

    #[test]
    fn a_copy_that_cannot_install_closes_on_enter_without_installing() {
        let mut fake = FakeUpdates::offering("9.9.9");
        fake.installable = false;
        let fake = Arc::new(fake);
        let mut app = app_offering(fake.clone());
        app.open_update();
        assert_eq!(app.update_step, UpdateStep::NotInstallable);
        let text = rendered(&mut app);
        assert!(text.contains("rebuild it"));
        assert!(text.contains("not installed by the Znicz installer"));
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.modal, Modal::None);
        assert!(fake.installs.lock().unwrap().is_empty());
    }

    #[test]
    fn enter_after_done_restarts_when_the_player_is_idle() {
        let mut app = app_offering(Arc::new(FakeUpdates::offering("9.9.9")));
        app.open_update();
        app.update_step = UpdateStep::Done;
        press(&mut app, KeyCode::Enter);
        assert!(app.should_quit);
        assert_eq!(app.outcome, RunOutcome::Restart { resume: None });
    }

    #[test]
    fn esc_after_done_closes_the_modal_and_forgets_the_update() {
        let mut app = app_offering(Arc::new(FakeUpdates::offering("9.9.9")));
        app.open_update();
        app.update_step = UpdateStep::Done;
        press(&mut app, KeyCode::Esc);
        assert_eq!(app.modal, Modal::None);
        assert!(app.update.is_none());
        assert!(!app.should_quit);
    }

    #[test]
    fn restart_asks_about_the_player_only_when_it_has_a_queue_and_is_not_stopped() {
        use PlaybackStatus::*;
        assert!(!restart_asks_about_player(Stopped, 0));
        assert!(!restart_asks_about_player(Stopped, 3));
        assert!(!restart_asks_about_player(Playing, 0));
        assert!(restart_asks_about_player(Playing, 3));
        assert!(restart_asks_about_player(Paused, 3));
    }

    #[test]
    fn enter_while_the_player_plays_restarts_it_with_resume() {
        let mut app = app_offering(Arc::new(FakeUpdates::offering("9.9.9")));
        app.open_update();
        app.update_step = UpdateStep::PlayerPlaying;
        let index = app.player.state().queue_position;
        press(&mut app, KeyCode::Enter);
        assert!(app.should_quit);
        assert!(matches!(
            app.outcome,
            RunOutcome::Restart { resume: Some(Resume { index: i, .. }) } if i == index
        ));
    }

    #[test]
    fn esc_while_the_player_plays_restarts_without_touching_it() {
        let mut app = app_offering(Arc::new(FakeUpdates::offering("9.9.9")));
        app.open_update();
        app.update_step = UpdateStep::PlayerPlaying;
        press(&mut app, KeyCode::Esc);
        assert!(app.should_quit);
        assert_eq!(app.outcome, RunOutcome::Restart { resume: None });
    }

    #[test]
    fn the_update_title_names_the_version() {
        let mut app = app_offering(Arc::new(FakeUpdates::offering("9.9.9")));
        app.open_update();
        assert!(rendered(&mut app).contains("Update to 9.9.9"));
    }

    fn click(app: &mut App, column: u16, row: u16) {
        app.on_mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column,
            row,
            modifiers: KeyModifiers::NONE,
        });
    }

    #[test]
    fn the_close_button_keeps_an_install_running_modal_open() {
        let mut app = app_offering(Arc::new(FakeUpdates::offering("9.9.9")));
        app.open_update();
        app.hits.close = Some(Rect::new(10, 10, 3, 1));
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.update_step, UpdateStep::Installing);
        click(&mut app, 11, 10);
        assert_eq!(app.modal, Modal::Update);
        assert_eq!(app.update_step, UpdateStep::Installing);
    }

    #[test]
    fn the_close_button_still_closes_the_update_modal_when_not_installing() {
        let mut app = app_offering(Arc::new(FakeUpdates::offering("9.9.9")));
        app.open_update();
        app.hits.close = Some(Rect::new(10, 10, 3, 1));
        click(&mut app, 11, 10);
        assert_eq!(app.modal, Modal::None);
    }

    #[test]
    fn reopening_the_update_modal_keeps_a_running_install_and_a_done_one() {
        let mut app = app_offering(Arc::new(FakeUpdates::offering("9.9.9")));
        app.open_update();
        press(&mut app, KeyCode::Enter);
        app.modal = Modal::None;
        app.open_update();
        assert_eq!(app.update_step, UpdateStep::Installing);

        app.update_step = UpdateStep::Done;
        app.modal = Modal::None;
        app.open_update();
        assert_eq!(app.update_step, UpdateStep::Done);
    }

    #[test]
    fn an_install_that_ends_with_the_modal_closed_is_announced_in_a_toast() {
        let mut app = app_offering(Arc::new(FakeUpdates::offering("9.9.9")));
        app.open_update();
        press(&mut app, KeyCode::Enter);
        app.modal = Modal::None;
        wait_for_install(&mut app);
        assert!(
            toast_texts(&app)
                .iter()
                .any(|t| t == "Znicz 9.9.9 installed · ? then u to restart"),
            "{:?}",
            toast_texts(&app)
        );
    }

    #[test]
    fn a_failed_install_with_the_modal_closed_shows_an_error_toast() {
        let fake = FakeUpdates::offering("9.9.9");
        *fake.install_result.lock().unwrap() = Err("boom".into());
        let mut app = app_offering(Arc::new(fake));
        app.open_update();
        press(&mut app, KeyCode::Enter);
        app.modal = Modal::None;
        wait_for_install(&mut app);
        assert!(
            toast_texts(&app).iter().any(|t| t == "Update failed: boom"),
            "{:?}",
            toast_texts(&app)
        );
    }

    #[test]
    fn ctrl_c_does_not_quit_while_an_install_runs() {
        let mut app = app_offering(Arc::new(FakeUpdates::offering("9.9.9")));
        app.open_update();
        press(&mut app, KeyCode::Enter);
        app.on_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));
        assert!(!app.should_quit);
        assert_eq!(app.update_step, UpdateStep::Installing);
    }

    #[test]
    fn ctrl_c_still_quits_from_the_update_modal_when_idle() {
        let mut app = app_offering(Arc::new(FakeUpdates::offering("9.9.9")));
        app.open_update();
        app.on_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));
        assert!(app.should_quit);
    }

    #[test]
    fn the_hint_is_read_once_when_the_modal_opens() {
        let mut fake = FakeUpdates::offering("9.9.9");
        fake.installable = false;
        let mut app = app_offering(Arc::new(fake));
        assert!(app.update_hint.is_empty());
        app.open_update();
        assert_eq!(app.update_step, UpdateStep::NotInstallable);
        assert_eq!(app.update_hint, "rebuild it");
    }

    #[test]
    fn the_wheel_scrolls_the_update_notes() {
        let mut app = app_offering(Arc::new(FakeUpdates::offering("9.9.9")));
        app.open_update();
        app.on_mouse(MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: 0,
            row: 0,
            modifiers: KeyModifiers::NONE,
        });
        assert_eq!(app.update_scroll, 1);
    }
}
