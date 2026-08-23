//! The application: what is on screen, and what the keys do.

use std::path::PathBuf;
use std::time::Duration;

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use znicz_core::{
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
}

pub struct App {
    pub player: PlayerHandle,
    pub focus: Focus,
    pub queue_open: bool,
    pub modal: Modal,
    pub list_width: u16,
    pub queue_cursor: Cursor,
    pub library: LibraryPane,
    pub devices: Vec<AudioDeviceInfo>,
    pub device_cursor: Cursor,
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
            list_width: 80,
            queue_cursor: Cursor::new(),
            library: LibraryPane::new(library),
            devices,
            device_cursor: Cursor::new(),
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

        if self.library.is_typing() {
            self.on_search_key(key);
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

        match self.focus {
            Focus::Queue => self.on_queue_key(key),
            Focus::Library => self.on_library_key(key),
        }
    }

    fn on_esc(&mut self) {
        if self.modal == Modal::Devices {
            self.modal = Modal::None;
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
        match key.code {
            KeyCode::Char('q') => self.should_quit = true,
            KeyCode::Char('?') => self.modal = Modal::Help,
            KeyCode::Char(',') => self.toggle_devices_modal(),

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

            KeyCode::Char('<') => self.library.pan(-1, 0),
            KeyCode::Char('>') => self.library.pan(1, 0),

            KeyCode::Char(' ') => self.toggle_pause(),
            KeyCode::Char('s') => self.apply(Command::Stop, None),
            KeyCode::Char('n') => self.apply(Command::NextTrack, None),
            KeyCode::Char('N') | KeyCode::Char('p') => self.apply(Command::PreviousTrack, None),

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
                self.toasts.info("library reloaded");
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

        let was_empty = self.player.state().queue.is_empty();
        self.apply(
            Command::QueueAdd(paths.clone()),
            Some(match count {
                1 => "added 1 track".to_string(),
                n => format!("added {n} tracks"),
            }),
        );

        // An idle player with a fresh queue should just start.
        if was_empty && self.player.state().status == PlaybackStatus::Stopped {
            if let Some(first) = paths.first() {
                self.apply(Command::Play(first.clone()), None);
            }
        }
    }

    // --- list movement, applied to whichever list has focus ---

    fn list_len(&self) -> usize {
        if self.modal == Modal::Devices {
            self.devices.len()
        } else if self.focus == Focus::Queue && self.queue_open {
            self.player.state().queue.len()
        } else {
            self.library.len()
        }
    }

    fn step(&mut self, delta: isize) {
        let len = self.list_len();
        if self.modal == Modal::Devices {
            self.device_cursor.step(delta, len);
        } else if self.focus == Focus::Queue && self.queue_open {
            self.queue_cursor.step(delta, len);
        } else {
            self.library.step(delta);
        }
    }

    fn page(&mut self, delta: isize) {
        let len = self.list_len();
        if self.modal == Modal::Devices {
            self.device_cursor.page(delta, len);
        } else if self.focus == Focus::Queue && self.queue_open {
            self.queue_cursor.page(delta, len);
        } else {
            self.library.page(delta);
        }
    }

    fn go_first(&mut self) {
        if self.modal == Modal::Devices {
            self.device_cursor.first();
        } else if self.focus == Focus::Queue && self.queue_open {
            self.queue_cursor.first();
        } else {
            self.library.first();
        }
    }

    fn go_last(&mut self) {
        let len = self.list_len();
        if self.modal == Modal::Devices {
            self.device_cursor.last(len);
        } else if self.focus == Focus::Queue && self.queue_open {
            self.queue_cursor.last(len);
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
                    self.toasts.info(message);
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
}
