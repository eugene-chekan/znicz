//! The application: what is on screen, and what the keys do.

use std::path::{Path, PathBuf};
use std::time::Duration;

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use znicz_core::{
    apply_to_player, list_saved, load_path, sanitize_stem, saved_path, skipped_notice, write_path,
    AudioDeviceInfo, AudioOutput, Command, PlaybackStatus, PlayerEvent, PlayerHandle, PlayerState,
    RepeatMode,
};
use znicz_library::{Library, Track};

use crate::cursor::Cursor;
use crate::layout;
use crate::library_pane::{Item, LibraryPane};
use crate::meta::{Entry, MetaCache};
use crate::toast::Toasts;
use crate::views;

/// Longest wait between redraws while nothing happens. Fast enough for a
/// smooth seek bar, slow enough to stay near zero CPU.
const TICK_RATE: Duration = Duration::from_millis(200);

/// Seek step for the plain and shifted keys.
const SEEK_SMALL: i64 = 5;
const SEEK_LARGE: i64 = 30;
const VOLUME_STEP: f32 = 0.05;

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
}

impl Modal {
    pub(crate) fn blocks_list_focus(self) -> bool {
        matches!(self, Self::Devices | Self::Inspector | Self::Playlists)
    }
}

pub struct App {
    pub player: PlayerHandle,
    pub focus: Focus,
    pub queue_open: bool,
    pub modal: Modal,
    pub list_width: u16,
    pub title_slot: usize,
    pub queue_h_offset: usize,
    pub queue_title_slot: usize,
    pub queue_cursor: Cursor,
    pub library: LibraryPane,
    pub devices: Vec<AudioDeviceInfo>,
    pub device_cursor: Cursor,
    pub playlists_dir: PathBuf,
    pub playlists: Vec<String>,
    pub playlist_cursor: Cursor,
    pub playlist_input: Option<String>,
    pub meta: MetaCache,
    pub toasts: Toasts,
    pub should_quit: bool,
}

impl App {
    pub fn new(player: PlayerHandle) -> Self {
        Self::with_library(player, None)
    }

    pub fn with_library(player: PlayerHandle, library: Option<Library>) -> Self {
        let devices = AudioOutput::list_devices().unwrap_or_default();
        Self {
            player,
            focus: Focus::Library,
            queue_open: false,
            modal: Modal::None,
            list_width: 100,
            title_slot: 0,
            queue_h_offset: 0,
            queue_title_slot: 0,
            queue_cursor: Cursor::new(),
            library: LibraryPane::new(library),
            devices,
            device_cursor: Cursor::new(),
            playlists_dir: znicz_library::default_playlists_dir()
                .unwrap_or_else(|| std::env::temp_dir().join("znicz-playlists")),
            playlists: Vec::new(),
            playlist_cursor: Cursor::new(),
            playlist_input: None,
            meta: MetaCache::new(),
            toasts: Toasts::new(),
            should_quit: false,
        }
    }

    pub fn run(&mut self) -> color_eyre::Result<()> {
        let mut terminal = ratatui::init();
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
                    self.meta.insert(
                        track.path.clone(),
                        Entry {
                            title: track.title.clone(),
                            artist: track.artist().map(str::to_string),
                            album: track.album().map(str::to_string),
                            duration: track.duration,
                        },
                    );
                }
                _ => {}
            }
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

        // Search and the playlist save prompt are plain text. They must run
        // before global keys, or `s` (stop) turns "To Listen" into "To Liten".
        if self.library.is_typing() {
            self.on_search_key(key);
            return;
        }

        if self.playlist_input.is_some() {
            self.on_playlist_input_key(key);
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

        if self.on_global_key(key) {
            return;
        }

        if self.modal == Modal::Devices {
            self.on_devices_key(key);
            return;
        }

        if self.modal == Modal::Playlists {
            self.on_playlists_key(key);
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
            Modal::Devices | Modal::Inspector | Modal::Playlists
        ) {
            self.modal = Modal::None;
            self.playlist_input = None;
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
            KeyCode::Backspace => self.library.pop_char(),
            KeyCode::Char(c) => self.library.push_char(c),
            _ => {}
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
        if self.library.is_typing() || self.playlist_input.is_some() {
            return false;
        }
        match key.code {
            KeyCode::Char('q') => self.should_quit = true,
            KeyCode::Char('?') => self.modal = Modal::Help,
            KeyCode::Char(',') => self.toggle_devices_modal(),
            KeyCode::Char('i') => self.toggle_inspector_modal(),
            KeyCode::Char('P') => self.toggle_playlists_modal(),

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
            KeyCode::Char('n') => self.apply(Command::NextTrack, None),
            KeyCode::Char('N') | KeyCode::Char('p') => self.apply(Command::PreviousTrack, None),

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
            KeyCode::Char('r') => self.cycle_repeat(),
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
            self.playlist_input = None;
        } else {
            self.reload_playlists();
            self.modal = Modal::Playlists;
        }
    }

    fn reload_playlists(&mut self) {
        self.playlists = list_saved(&self.playlists_dir);
        self.playlist_cursor.clamp(self.playlists.len());
    }

    fn on_playlists_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Enter => self.play_selected_playlist(false),
            KeyCode::Char('a') => self.play_selected_playlist(true),
            KeyCode::Char('w') => self.begin_playlist_save(),
            _ => {}
        }
    }

    fn on_playlist_input_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => self.playlist_input = None,
            KeyCode::Enter => self.confirm_playlist_save(),
            KeyCode::Backspace => {
                if let Some(input) = &mut self.playlist_input {
                    input.pop();
                }
            }
            KeyCode::Char(c) => {
                // Every character, including global letters like s / n / i.
                if let Some(input) = &mut self.playlist_input {
                    input.push(c);
                }
            }
            _ => {}
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
                            match result.paths.len() {
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
        if self.player.state().queue.is_empty() {
            self.toasts.warn("queue is empty");
            return;
        }
        self.playlist_input = Some(String::new());
    }

    fn confirm_playlist_save(&mut self) {
        let Some(raw) = self.playlist_input.take() else {
            return;
        };
        let name = match sanitize_stem(&raw) {
            Ok(name) => name,
            Err(e) => {
                self.toasts.error(e.to_string());
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
                    self.queue_cursor.clamp(state.queue.len().saturating_sub(1));
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
            KeyCode::Char('R') => {
                self.library.reload_albums();
                self.toasts.success("library reloaded");
            }
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
        match key.code {
            KeyCode::Enter => {
                let selected = self
                    .device_cursor
                    .selected(self.devices.len())
                    .and_then(|i| self.devices.get(i))
                    .map(|d| (d.id.clone(), d.name.clone()));

                if let Some((id, name)) = selected {
                    self.apply(Command::SetDevice(id), Some(format!("output: {name}")));
                }
            }
            KeyCode::Char('R') => {
                self.devices = AudioOutput::list_devices().unwrap_or_default();
                self.device_cursor.clamp(self.devices.len());
                self.toasts
                    .info(format!("{} devices found", self.devices.len()));
            }
            _ => {}
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
                self.apply(Command::Play(path), None);
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
        let paths: Vec<PathBuf> = tracks.iter().map(|t| t.path.clone()).collect();
        // Tags come from the database, so the queue reads properly at once.
        for track in &tracks {
            self.meta
                .insert(track.path.clone(), entry_from_track(track));
        }

        self.apply(
            Command::QueueAdd(paths),
            Some(match count {
                1 => "added 1 track".to_string(),
                n => format!("added {n} tracks"),
            }),
        );
    }

    // --- list movement, applied to whichever list has focus ---

    fn list_len(&self) -> usize {
        if self.modal == Modal::Playlists {
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
        if self.modal == Modal::Playlists {
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
        if self.modal == Modal::Playlists {
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
        if self.modal == Modal::Playlists {
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
        if self.modal == Modal::Playlists {
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

    fn queue_label_len(&self, path: &Path) -> usize {
        match self.meta.get(path) {
            Some(entry) => entry.label().chars().count(),
            None => path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("?")
                .chars()
                .count(),
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
            .map(|path| self.queue_label_len(path))
            .unwrap_or(0)
    }

    pub(crate) fn clamp_queue_pan(&mut self) {
        let max = self
            .selected_queue_middle()
            .saturating_sub(self.queue_title_slot);
        self.queue_h_offset = self.queue_h_offset.min(max);
    }

    fn seek_relative(&mut self, seconds: i64) {
        let state = self.player.state();
        if state.current_track.is_none() {
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
