//! Spotify mode: `O` switches the keys, the footer and the status poll to
//! Spotify. A second `impl App` beside `spotify_ui.rs`.

use std::time::{Duration, Instant};

use znicz_core::{SpotifyKind, SpotifyLogin, SpotifyOp, SpotifyRepeat, ZniczError};

use crossterm::event::{KeyCode, KeyEvent};

use super::{App, Modal};
use crate::cursor::Cursor;
use crate::spotify_browse::Row;
use crate::spotify_mode::{self, MuteAction, SpotifyKey, now_unix_ms};
use crate::spotify_worker::SpotifyAsk;

/// How often the queue is read again while the drawer is open.
const SPOTIFY_QUEUE_EVERY: Duration = Duration::from_secs(3);

impl App {
    /// A transport key of Spotify mode. Playback ops go to the transport lane, so
    /// a slow browse call never holds them up.
    pub(crate) fn spotify_transport_key(&mut self, key: SpotifyKey) {
        match key {
            SpotifyKey::PlayPause => {
                let op = if self.spotify_state.is_playing() {
                    SpotifyOp::Pause
                } else {
                    // Nothing playing, or paused: resume the device's own track.
                    SpotifyOp::Play {
                        context_uri: None,
                        uris: Vec::new(),
                        offset: None,
                    }
                };
                self.spotify_send_play(op);
            }
            SpotifyKey::Next => {
                self.spotify_send_play(SpotifyOp::Next);
            }
            SpotifyKey::Previous => {
                self.spotify_send_play(SpotifyOp::Previous);
            }
            SpotifyKey::Seek(secs) => {
                let Some(playback) = self.spotify_state.playback.as_ref() else {
                    self.toasts.info(spotify_mode::NOTHING_PLAYING);
                    return;
                };
                let duration = playback.item.as_ref().map_or(0, |t| t.duration_ms);
                let position_ms =
                    spotify_mode::seek_target(playback.progress_at(now_unix_ms()), secs, duration);
                self.spotify_send_play(SpotifyOp::Seek { position_ms });
            }
            SpotifyKey::Volume(delta) => match self.spotify_volume_percent() {
                Some(percent) => {
                    self.spotify_send_volume_percent(spotify_mode::percent_after(percent, delta));
                }
                None => self.toasts.info(spotify_mode::NO_VOLUME),
            },
            SpotifyKey::Mute => {
                let volume = self.spotify_volume_percent();
                match spotify_mode::mute_action(volume, self.spotify_mute_restore) {
                    MuteAction::Mute { remember } => {
                        if self.spotify_send_volume_percent(0) {
                            self.spotify_mute_restore = Some(remember);
                        }
                    }
                    MuteAction::Unmute(percent) => {
                        if self.spotify_send_volume_percent(percent) {
                            self.spotify_mute_restore = None;
                        }
                    }
                    MuteAction::Nothing => self.toasts.info(spotify_mode::NO_VOLUME),
                }
            }
            SpotifyKey::Repeat => {
                let current = self
                    .spotify_state
                    .playback
                    .as_ref()
                    .map_or(SpotifyRepeat::Off, |p| p.repeat);
                let mode = current.next();
                if self.spotify_send_play(SpotifyOp::SetRepeat { mode }) {
                    // The next press builds on this one, not on the last poll.
                    if let Some(playback) = self.spotify_state.playback.as_mut() {
                        playback.repeat = mode;
                    }
                }
            }
            SpotifyKey::Shuffle => {
                let on = self
                    .spotify_state
                    .playback
                    .as_ref()
                    .is_none_or(|p| !p.shuffle);
                self.spotify_send_play(SpotifyOp::SetShuffle { on });
            }
            SpotifyKey::Stop => self.toasts.info(spotify_mode::NO_STOP),
            SpotifyKey::LibraryOnly => self.toasts.info(spotify_mode::NOT_IN_MODE),
            // `r` is the browser's; in the Stage it does nothing.
            SpotifyKey::Reload => {}
            SpotifyKey::QueueAdd => self.spotify_queue_add(),
            SpotifyKey::QueueDrawer => self.toggle_spotify_queue(),
            SpotifyKey::Devices => self.toggle_spotify_devices(),
            // `/` is the browser's: the search prompt opens from the browser's keys.
            SpotifyKey::Search => {}
        }
    }

    /// `a`: queue the highlighted track. Albums, playlists and artists are refused.
    pub(crate) fn spotify_queue_add(&mut self) {
        let level = self.spotify_browser.level();
        let highlighted = level.rows().get(level.cursor).cloned();
        let Some(Row::Item(item)) = highlighted else {
            return;
        };
        if item.kind != SpotifyKind::Track {
            self.toasts.info(spotify_mode::ONE_TRACK_AT_A_TIME);
            return;
        }
        let op = SpotifyOp::QueueAdd {
            uri: item.uri.clone(),
        };
        if !self.ask_spotify(SpotifyAsk::QueueAdd, op) {
            self.toasts.error(spotify_mode::NO_LANE);
        }
    }

    /// `,` in the mode: open or close the Spotify Connect device list. The list
    /// asks the player for its devices, so the rows are current when it opens.
    pub(crate) fn toggle_spotify_devices(&mut self) {
        if self.modal == Modal::SpotifyDevices {
            self.modal = Modal::None;
            return;
        }
        if self.modal != Modal::None {
            return;
        }
        let devices = &self.spotify_state.devices;
        let active = devices.iter().position(|d| d.is_active);
        self.spotify_devices_cursor = Cursor::new();
        if let Some(index) = active {
            self.spotify_devices_cursor.set(index, devices.len());
        }
        self.modal = Modal::SpotifyDevices;
        self.request_devices();
    }

    /// Keys of the Spotify device list: `j`/`k` and the arrows move, `Enter`
    /// moves playback to the chosen device, `,` closes. The list swallows every
    /// other key except the global ones (`q`, `S`, `?`, ...), so no Znicz key
    /// reaches the engine while it is open. `Esc` goes through its own path.
    pub(crate) fn on_spotify_devices_key(&mut self, key: KeyEvent) -> bool {
        let len = self.spotify_state.devices.len();
        match key.code {
            KeyCode::Char('j') | KeyCode::Down => self.spotify_devices_cursor.step(1, len),
            KeyCode::Char('k') | KeyCode::Up => self.spotify_devices_cursor.step(-1, len),
            KeyCode::Enter => self.transfer_to_selected_device(),
            KeyCode::Char(',') => self.toggle_spotify_devices(),
            _ if spotify_mode::keeps_global(key) => return false,
            _ => {}
        }
        true
    }

    /// Enter in the device list: transfer playback to the device under the cursor
    /// on the transport lane, and close the list. An empty list does nothing.
    fn transfer_to_selected_device(&mut self) {
        let len = self.spotify_state.devices.len();
        let Some(device) = self
            .spotify_devices_cursor
            .selected(len)
            .and_then(|index| self.spotify_state.devices.get(index))
            .cloned()
        else {
            return;
        };
        self.modal = Modal::None;
        let op = SpotifyOp::Transfer {
            device_id: device.id,
            play: true,
        };
        if !self.ask_spotify(SpotifyAsk::Transfer { name: device.name }, op) {
            self.toasts.error(spotify_mode::NO_LANE);
        }
    }

    /// `]` in the mode: open or close the drawer. Opening asks Spotify for its queue.
    pub(crate) fn toggle_spotify_queue(&mut self) {
        if self.queue_open {
            self.close_queue();
        } else {
            self.open_queue();
            self.request_spotify_queue();
        }
    }

    /// Ask for Spotify's queue on the browse lane, unless a request is already with the worker.
    pub(crate) fn request_spotify_queue(&mut self) {
        if self.spotify_queue_requested {
            return;
        }
        // Stamped before the ask, so a missing lane is retried at the 3 s pace too.
        self.spotify_queue_asked_at = Some(Instant::now());
        if self.ask_spotify(SpotifyAsk::Queue, SpotifyOp::Queue) {
            self.spotify_queue_requested = true;
        } else {
            self.toasts.error(spotify_mode::NO_LANE);
        }
    }

    /// While the drawer is open in the mode, the queue is read again every 3 s.
    /// Not while Spotify is unavailable: the stop says why, and asking again
    /// would only repeat it.
    pub(crate) fn refresh_spotify_queue(&mut self) {
        if !self.in_spotify_mode() || !self.queue_open || self.spotify_unavailable.is_some() {
            return;
        }
        let due = self
            .spotify_queue_asked_at
            .is_none_or(|at| at.elapsed() >= SPOTIFY_QUEUE_EVERY);
        if due {
            self.request_spotify_queue();
        }
    }

    /// The device's volume, when Spotify reports one.
    fn spotify_volume_percent(&self) -> Option<u8> {
        self.spotify_state
            .playback
            .as_ref()
            .and_then(|p| p.device.as_ref())
            .and_then(|d| d.volume_percent)
    }

    /// Set the device to `percent`. False (with the lane's toast) when the request was not sent.
    /// Once the transport lane takes it, the device's volume is `percent` in the
    /// state too, so the next press builds on it; the next status read overwrites it.
    fn spotify_send_volume_percent(&mut self, percent: u8) -> bool {
        let sent = self.spotify_send_play(SpotifyOp::SetVolume {
            volume: f32::from(percent) / 100.0,
        });
        if sent
            && let Some(device) = self
                .spotify_state
                .playback
                .as_mut()
                .and_then(|p| p.device.as_mut())
        {
            device.volume_percent = Some(percent);
        }
        sent
    }

    /// Whether the keys, the footer and the status poll are Spotify's.
    pub(crate) fn in_spotify_mode(&self) -> bool {
        self.spotify_mode
    }

    /// A change (a transport op or a transfer) that Spotify refused or did not answer:
    /// its message is toasted, and what the error means for the status reads is applied.
    pub(crate) fn spotify_change_failed(&mut self, error: ZniczError) {
        let plan = spotify_mode::error_plan_for(&error);
        self.toasts.error(plan.text.clone());
        // A change that may have happened is checked by the next read at once.
        if plan.reread {
            self.spotify_polled_at = None;
        }
        if plan.stops_polling {
            self.spotify_unavailable = Some(plan.text);
        }
    }

    /// A queue read that failed: the drawer shows the reason in place of the
    /// rows until a read succeeds, and a stop from the player stops the reads.
    pub(crate) fn queue_read_failed(&mut self, error: ZniczError) {
        tracing::debug!("spotify queue: {error}");
        // A read that was sent in the mode and answered after leaving it is stale.
        if !self.in_spotify_mode() {
            return;
        }
        let plan = spotify_mode::error_plan_for(&error);
        if plan.stops_polling {
            self.spotify_unavailable = Some(plan.text.clone());
        }
        self.spotify_queue_error = Some(plan.text);
    }

    /// `O`: enter or leave Spotify mode. Entering needs a login; without one the
    /// login line is toasted and nothing changes.
    pub(crate) fn toggle_spotify_mode(&mut self) {
        if self.in_spotify_mode() {
            self.leave_spotify_mode();
            return;
        }
        if self.spotify_state.login != SpotifyLogin::LoggedIn {
            self.toasts.info(spotify_mode::NOT_LOGGED_IN);
            return;
        }
        self.spotify_mode = true;
        // A stop from an old player is forgotten, and the next tick reads the status at once.
        self.spotify_unavailable = None;
        self.spotify_polled_at = None;
        self.spotify_queue_error = None;
    }

    /// Leave Spotify mode. Nothing is sent: Spotify keeps playing where it is.
    pub(crate) fn leave_spotify_mode(&mut self) {
        self.spotify_mode = false;
        self.spotify_queue_error = None;
        self.spotify_browser.reset();
        self.spotify_search = None;
        if self.modal == Modal::SpotifyDevices {
            self.modal = Modal::None;
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use znicz_core::{Command, QueueItem, SpotifyLogin, SpotifyState};

    use std::sync::Arc;
    use std::time::{SystemTime, UNIX_EPOCH};

    use std::time::Duration;

    use znicz_core::{
        SpotifyDevice, SpotifyItem, SpotifyKind, SpotifyOp, SpotifyPage, SpotifyPlayback,
        SpotifyRepeat, SpotifyReply, SpotifyTrack,
    };

    use crate::app::settings_ui::tests as settings_tests;
    use crate::app::settings_ui::tests::press;
    use crate::app::spotify_ui::tests::{
        FakeHost, settle, wait_for_count, wait_for_op, wait_until,
    };
    use crate::app::{App, Modal, View};
    use crate::keys::{Context, context_hints};
    use crate::spotify_mode;
    use crate::spotify_worker::{Done, SpotifyAsk};
    use znicz_core::{SpotifyErrorKind, ZniczError};

    fn now_ms() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_millis() as u64)
    }

    /// Spotify mode on, logged in, with both lanes on one fake host.
    fn mode_app(dir: &tempfile::TempDir, host: &Arc<FakeHost>) -> App {
        let mut app = settings_tests::settings_app(dir)
            .with_spotify_worker(host.clone())
            .unwrap()
            .with_spotify_transport(host.clone())
            .unwrap();
        app.spotify_main = Some(host.clone());
        app.spotify_state = SpotifyState {
            login: SpotifyLogin::LoggedIn,
            ..SpotifyState::default()
        };
        press(&mut app, KeyCode::Char('O'));
        assert!(app.in_spotify_mode());
        app
    }

    /// A track of 3 minutes at 4 s, read now, on a device at `volume`.
    fn playing(volume: Option<u8>, is_playing: bool) -> SpotifyPlayback {
        SpotifyPlayback {
            item: Some(SpotifyTrack {
                uri: "spotify:track:1".into(),
                title: "So What".into(),
                duration_ms: 180_000,
                ..SpotifyTrack::default()
            }),
            unsupported: false,
            is_playing,
            progress_ms: 4_000,
            read_at_ms: now_ms(),
            repeat: SpotifyRepeat::Off,
            shuffle: false,
            device: Some(SpotifyDevice {
                id: "desk".into(),
                name: "Desk".into(),
                kind: "Computer".into(),
                is_active: true,
                volume_percent: volume,
            }),
            context_uri: None,
        }
    }

    fn resume() -> SpotifyOp {
        SpotifyOp::Play {
            context_uri: None,
            uris: Vec::new(),
            offset: None,
        }
    }

    #[test]
    fn space_pauses_a_playing_track_and_resumes_a_paused_one() {
        let dir = tempfile::tempdir().unwrap();
        let host = Arc::new(FakeHost::default());
        let mut app = mode_app(&dir, &host);
        app.spotify_state.playback = Some(playing(Some(40), true));
        press(&mut app, KeyCode::Char(' '));
        assert!(wait_for_op(&host, |op| *op == SpotifyOp::Pause));
        app.spotify_state.playback = Some(playing(Some(40), false));
        press(&mut app, KeyCode::Char(' '));
        assert!(wait_for_op(&host, |op| *op == resume()));
    }

    #[test]
    fn the_transport_keys_work_in_the_stage_too() {
        let dir = tempfile::tempdir().unwrap();
        let host = Arc::new(FakeHost::default());
        let mut app = mode_app(&dir, &host);
        press(&mut app, KeyCode::Char('v'));
        assert_eq!(app.view, View::Stage);
        app.spotify_state.playback = Some(playing(Some(40), true));
        press(&mut app, KeyCode::Char(' '));
        assert!(wait_for_op(&host, |op| *op == SpotifyOp::Pause));
    }

    #[test]
    fn the_seek_arrows_send_seek_from_the_clock_position() {
        let dir = tempfile::tempdir().unwrap();
        let host = Arc::new(FakeHost::default());
        let mut app = mode_app(&dir, &host);
        app.spotify_state.playback = Some(playing(Some(40), true));
        press(&mut app, KeyCode::Right);
        assert!(wait_for_op(&host, |op| matches!(
            op,
            SpotifyOp::Seek { position_ms } if (9_000..=15_000).contains(position_ms)
        )));
    }

    #[test]
    fn seeking_back_past_zero_clamps_to_zero() {
        let dir = tempfile::tempdir().unwrap();
        let host = Arc::new(FakeHost::default());
        let mut app = mode_app(&dir, &host);
        app.spotify_state.playback = Some(playing(Some(40), true));
        press(&mut app, KeyCode::Left);
        assert!(wait_for_op(&host, |op| *op == SpotifyOp::Seek { position_ms: 0 }));
    }

    #[test]
    fn seek_without_playback_toasts_and_sends_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let host = Arc::new(FakeHost::default());
        let mut app = mode_app(&dir, &host);
        press(&mut app, KeyCode::Right);
        assert!(
            settings_tests::toast_texts(&app)
                .iter()
                .any(|t| t == spotify_mode::NOTHING_PLAYING)
        );
        assert!(settle(&app, &[&*host]).is_empty());
    }

    #[test]
    fn plus_sends_one_percent_more_and_minus_one_less() {
        let dir = tempfile::tempdir().unwrap();
        let host = Arc::new(FakeHost::default());
        let mut app = mode_app(&dir, &host);
        app.spotify_state.playback = Some(playing(Some(40), true));
        press(&mut app, KeyCode::Char('+'));
        assert!(wait_for_op(&host, |op| *op == SpotifyOp::SetVolume { volume: 0.41 }));
        // The minus builds on the 41 just sent, not on the 40 of the last read.
        press(&mut app, KeyCode::Char('-'));
        assert!(wait_for_op(&host, |op| *op == SpotifyOp::SetVolume { volume: 0.40 }));
    }

    #[test]
    fn m_mutes_to_zero_and_m_again_restores_the_remembered_percent() {
        let dir = tempfile::tempdir().unwrap();
        let host = Arc::new(FakeHost::default());
        let mut app = mode_app(&dir, &host);
        app.spotify_state.playback = Some(playing(Some(40), true));
        press(&mut app, KeyCode::Char('m'));
        assert!(wait_for_op(&host, |op| *op == SpotifyOp::SetVolume { volume: 0.0 }));
        app.spotify_state.playback = Some(playing(Some(0), true));
        press(&mut app, KeyCode::Char('m'));
        assert!(wait_for_op(&host, |op| *op == SpotifyOp::SetVolume { volume: 0.40 }));
    }

    #[test]
    fn m_on_a_silent_device_with_nothing_remembered_makes_sound() {
        let dir = tempfile::tempdir().unwrap();
        let host = Arc::new(FakeHost::default());
        let mut app = mode_app(&dir, &host);
        app.spotify_state.playback = Some(playing(Some(0), true));
        press(&mut app, KeyCode::Char('m'));
        assert!(wait_for_op(&host, |op| *op == SpotifyOp::SetVolume { volume: 0.5 }));
    }

    #[test]
    fn m_with_unknown_volume_toasts_and_sends_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let host = Arc::new(FakeHost::default());
        let mut app = mode_app(&dir, &host);
        app.spotify_state.playback = Some(playing(None, true));
        press(&mut app, KeyCode::Char('m'));
        assert!(
            settings_tests::toast_texts(&app)
                .iter()
                .any(|t| t == spotify_mode::NO_VOLUME)
        );
        assert!(settle(&app, &[&*host]).is_empty());
    }

    #[test]
    fn e_sends_the_next_repeat_mode_and_z_flips_shuffle() {
        let dir = tempfile::tempdir().unwrap();
        let host = Arc::new(FakeHost::default());
        let mut app = mode_app(&dir, &host);
        app.spotify_state.playback = Some(playing(Some(40), true));
        press(&mut app, KeyCode::Char('e'));
        assert!(wait_for_op(&host, |op| *op
            == SpotifyOp::SetRepeat {
                mode: SpotifyRepeat::Context
            }));
        press(&mut app, KeyCode::Char('z'));
        assert!(wait_for_op(&host, |op| *op == SpotifyOp::SetShuffle { on: true }));
    }

    #[test]
    fn s_is_a_toast_and_sends_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let host = Arc::new(FakeHost::default());
        let mut app = mode_app(&dir, &host);
        press(&mut app, KeyCode::Char('s'));
        assert!(
            settings_tests::toast_texts(&app)
                .iter()
                .any(|t| t == spotify_mode::NO_STOP)
        );
        assert!(settle(&app, &[&*host]).is_empty());
    }

    #[test]
    fn a_library_only_key_toasts_not_in_spotify_mode() {
        let dir = tempfile::tempdir().unwrap();
        let host = Arc::new(FakeHost::default());
        let mut app = mode_app(&dir, &host);
        press(&mut app, KeyCode::Char('A'));
        assert!(
            settings_tests::toast_texts(&app)
                .iter()
                .any(|t| t == spotify_mode::NOT_IN_MODE)
        );
        assert!(settle(&app, &[&*host]).is_empty());
    }

    fn logged_in(dir: &tempfile::TempDir) -> App {
        let mut app = settings_tests::settings_app(dir);
        app.spotify_state = SpotifyState {
            login: SpotifyLogin::LoggedIn,
            ..SpotifyState::default()
        };
        app
    }

    #[test]
    fn o_toggles_spotify_mode_from_browse_and_from_stage() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = logged_in(&dir);
        press(&mut app, KeyCode::Char('O'));
        assert!(app.in_spotify_mode());
        press(&mut app, KeyCode::Char('v'));
        assert_eq!(app.view, View::Stage);
        assert!(app.in_spotify_mode());
        press(&mut app, KeyCode::Char('O'));
        assert!(!app.in_spotify_mode());
    }

    #[test]
    fn o_without_login_toasts_the_login_line_and_stays_out() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = settings_tests::settings_app(&dir);
        press(&mut app, KeyCode::Char('O'));
        assert!(!app.in_spotify_mode());
        assert!(
            settings_tests::toast_texts(&app)
                .iter()
                .any(|t| t == spotify_mode::NOT_LOGGED_IN)
        );
    }

    #[test]
    fn v_and_i_still_switch_views_inside_the_mode() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = logged_in(&dir);
        press(&mut app, KeyCode::Char('O'));
        press(&mut app, KeyCode::Char('v'));
        assert_eq!(app.view, View::Stage);
        press(&mut app, KeyCode::Char('i'));
        assert_eq!(app.view, View::Browse);
        assert!(app.in_spotify_mode());
    }

    #[test]
    fn entering_the_mode_clears_the_unavailable_stop() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = logged_in(&dir);
        app.spotify_unavailable = Some("old".into());
        app.spotify_polled_at = Some(Instant::now());
        press(&mut app, KeyCode::Char('O'));
        assert!(app.spotify_unavailable.is_none());
        assert!(app.spotify_polled_at.is_none());
    }

    #[test]
    fn o_does_nothing_while_settings_is_open() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = logged_in(&dir);
        press(&mut app, KeyCode::Char('S'));
        press(&mut app, KeyCode::Char('O'));
        assert!(!app.in_spotify_mode());
        assert_eq!(app.modal, Modal::Settings);
    }

    #[test]
    fn the_context_is_spotify_browse_or_stage_in_the_mode() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = logged_in(&dir);
        press(&mut app, KeyCode::Char('O'));
        assert_eq!(app.context(), Context::SpotifyBrowse);
        press(&mut app, KeyCode::Char('v'));
        assert_eq!(app.context(), Context::SpotifyStage);
        press(&mut app, KeyCode::Char('O'));
        assert_eq!(app.context(), Context::Stage);
    }

    #[test]
    fn the_footer_ends_in_o_znicz_in_the_mode() {
        for context in [Context::SpotifyStage, Context::SpotifyBrowse] {
            assert_eq!(
                context_hints(context).last().unwrap().label,
                "O znicz",
                "{context:?}"
            );
        }
    }

    fn track_item(uri: &str, title: &str) -> SpotifyItem {
        SpotifyItem {
            position: None,
            kind: SpotifyKind::Track,
            uri: uri.into(),
            name: title.into(),
            subtitle: "Miles Davis".into(),
            image_url: None,
            track: Some(SpotifyTrack {
                uri: uri.into(),
                title: title.into(),
                duration_ms: 1_000,
                ..SpotifyTrack::default()
            }),
        }
    }

    fn album_item() -> SpotifyItem {
        SpotifyItem {
            position: None,
            kind: SpotifyKind::Album,
            uri: "spotify:album:kob".into(),
            name: "Kind of Blue".into(),
            subtitle: "Miles Davis".into(),
            image_url: None,
            track: None,
        }
    }

    fn page_of(items: Vec<SpotifyItem>) -> SpotifyReply {
        let total = items.len() as u32;
        SpotifyReply::Page {
            page: SpotifyPage {
                next_offset: None,
                items,
                offset: 0,
                total,
            },
        }
    }

    fn queue_count(app: &App, host: &FakeHost) -> usize {
        settle(app, &[host])
            .iter()
            .filter(|op| **op == SpotifyOp::Queue)
            .count()
    }

    #[test]
    fn a_sends_queue_add_for_the_highlighted_track() {
        let dir = tempfile::tempdir().unwrap();
        let host = Arc::new(FakeHost::default());
        *host.reply.lock().unwrap() = Some(page_of(vec![track_item(
            "spotify:track:so-what",
            "So What",
        )]));
        let mut app = mode_app(&dir, &host);
        // Search, Playlists, Liked songs: two steps down, then open.
        press(&mut app, KeyCode::Char('j'));
        press(&mut app, KeyCode::Char('j'));
        press(&mut app, KeyCode::Char('l'));
        wait_until(&mut app, |a| a.spotify_browser.level().items.len() == 1);
        press(&mut app, KeyCode::Char('a'));
        assert!(wait_for_op(&host, |op| *op
            == SpotifyOp::QueueAdd {
                uri: "spotify:track:so-what".into()
            }));
    }

    #[test]
    fn a_on_an_album_refuses_with_a_toast_and_sends_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let host = Arc::new(FakeHost::default());
        *host.reply.lock().unwrap() = Some(page_of(vec![album_item()]));
        let mut app = mode_app(&dir, &host);
        // Search, Playlists, Liked songs, Albums: three steps down, then open.
        press(&mut app, KeyCode::Char('j'));
        press(&mut app, KeyCode::Char('j'));
        press(&mut app, KeyCode::Char('j'));
        press(&mut app, KeyCode::Char('l'));
        wait_until(&mut app, |a| a.spotify_browser.level().items.len() == 1);
        press(&mut app, KeyCode::Char('a'));
        assert!(
            settings_tests::toast_texts(&app)
                .iter()
                .any(|t| t == spotify_mode::ONE_TRACK_AT_A_TIME)
        );
        assert!(
            !settle(&app, &[&*host])
                .iter()
                .any(|op| matches!(op, SpotifyOp::QueueAdd { .. }))
        );
    }

    #[test]
    fn bracket_opens_the_spotify_queue_and_asks_for_it() {
        let dir = tempfile::tempdir().unwrap();
        let host = Arc::new(FakeHost::default());
        *host.reply.lock().unwrap() = Some(SpotifyReply::Queue {
            current: Some(SpotifyTrack {
                title: "So What".into(),
                ..SpotifyTrack::default()
            }),
            next: vec![SpotifyTrack {
                title: "Blue in Green".into(),
                ..SpotifyTrack::default()
            }],
        });
        let mut app = mode_app(&dir, &host);
        press(&mut app, KeyCode::Char(']'));
        assert!(app.queue_open, "] should open the drawer in the mode");
        assert!(wait_for_op(&host, |op| *op == SpotifyOp::Queue));
        wait_until(&mut app, |a| a.spotify_queue.is_some());
        let (current, next) = app.spotify_queue.clone().unwrap();
        assert_eq!(current.map(|t| t.title), Some("So What".to_string()));
        assert_eq!(next.len(), 1);
    }

    #[test]
    fn bracket_in_the_mode_shows_the_spotify_queue_context_and_closes_again() {
        let dir = tempfile::tempdir().unwrap();
        let host = Arc::new(FakeHost::default());
        let mut app = mode_app(&dir, &host);
        press(&mut app, KeyCode::Char(']'));
        assert_eq!(app.context(), Context::SpotifyQueue);
        press(&mut app, KeyCode::Char(']'));
        assert!(!app.queue_open);
    }

    /// Two devices, Kitchen then Desk, with `active` (a name) marked as playing.
    fn two_devices(active: &str) -> Vec<SpotifyDevice> {
        [("k1", "Kitchen"), ("d1", "Desk")]
            .into_iter()
            .map(|(id, name)| SpotifyDevice {
                id: id.into(),
                name: name.into(),
                kind: "Speaker".into(),
                is_active: name == active,
                volume_percent: Some(40),
            })
            .collect()
    }

    fn transfer_to(device_id: &str) -> SpotifyOp {
        SpotifyOp::Transfer {
            device_id: device_id.into(),
            play: true,
        }
    }

    #[test]
    fn comma_in_the_mode_opens_the_spotify_list_not_the_output_list() {
        let dir = tempfile::tempdir().unwrap();
        let host = Arc::new(FakeHost::default());
        let mut app = mode_app(&dir, &host);
        press(&mut app, KeyCode::Char(','));
        assert_eq!(app.modal, Modal::SpotifyDevices);
        assert!(wait_for_op(&host, |op| *op == SpotifyOp::Devices));
    }

    #[test]
    fn enter_transfers_playback_to_the_chosen_device() {
        let dir = tempfile::tempdir().unwrap();
        let host = Arc::new(FakeHost::default());
        let mut app = mode_app(&dir, &host);
        app.spotify_state.devices = two_devices("");
        press(&mut app, KeyCode::Char(','));
        press(&mut app, KeyCode::Char('j'));
        press(&mut app, KeyCode::Enter);
        assert!(wait_for_op(&host, |op| *op == transfer_to("d1")));
        assert_eq!(app.modal, Modal::None);
    }

    #[test]
    fn the_cursor_starts_on_the_active_device() {
        let dir = tempfile::tempdir().unwrap();
        let host = Arc::new(FakeHost::default());
        let mut app = mode_app(&dir, &host);
        app.spotify_state.devices = two_devices("Desk");
        press(&mut app, KeyCode::Char(','));
        press(&mut app, KeyCode::Enter);
        assert!(wait_for_op(&host, |op| *op == transfer_to("d1")));
    }

    #[test]
    fn a_transfer_that_does_not_take_over_shows_the_error() {
        let dir = tempfile::tempdir().unwrap();
        let host = Arc::new(FakeHost::default());
        *host.transfer_refusal.lock().unwrap() = Some("device did not take over".into());
        let mut app = mode_app(&dir, &host);
        app.spotify_state.devices = two_devices("");
        press(&mut app, KeyCode::Char(','));
        press(&mut app, KeyCode::Enter);
        wait_until(&mut app, |a| {
            settings_tests::toast_texts(a)
                .iter()
                .any(|t| t == "device did not take over")
        });
    }

    #[test]
    fn a_transfer_that_takes_over_says_moved_to_the_device() {
        let dir = tempfile::tempdir().unwrap();
        let host = Arc::new(FakeHost::default());
        let mut app = mode_app(&dir, &host);
        app.spotify_state.devices = two_devices("");
        press(&mut app, KeyCode::Char(','));
        press(&mut app, KeyCode::Char('j'));
        press(&mut app, KeyCode::Enter);
        wait_until(&mut app, |a| {
            settings_tests::toast_texts(a)
                .iter()
                .any(|t| t == "Moved to Desk")
        });
    }

    #[test]
    fn esc_closes_the_spotify_list_and_sends_no_transfer() {
        let dir = tempfile::tempdir().unwrap();
        let host = Arc::new(FakeHost::default());
        let mut app = mode_app(&dir, &host);
        app.spotify_state.devices = two_devices("");
        press(&mut app, KeyCode::Char(','));
        press(&mut app, KeyCode::Esc);
        assert_eq!(app.modal, Modal::None);
        assert!(
            !settle(&app, &[&*host])
                .iter()
                .any(|op| matches!(op, SpotifyOp::Transfer { .. }))
        );
    }

    #[test]
    fn enter_on_an_empty_device_list_sends_nothing_and_stays_open() {
        let dir = tempfile::tempdir().unwrap();
        let host = Arc::new(FakeHost::default());
        let mut app = mode_app(&dir, &host);
        press(&mut app, KeyCode::Char(','));
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.modal, Modal::SpotifyDevices);
        assert!(
            !settle(&app, &[&*host])
                .iter()
                .any(|op| matches!(op, SpotifyOp::Transfer { .. }))
        );
    }

    #[test]
    fn an_open_drawer_asks_for_the_queue_again_after_3s() {
        let dir = tempfile::tempdir().unwrap();
        let host = Arc::new(FakeHost::default());
        *host.reply.lock().unwrap() = Some(SpotifyReply::Queue {
            current: None,
            next: Vec::new(),
        });
        let mut app = mode_app(&dir, &host);
        press(&mut app, KeyCode::Char(']'));
        wait_until(&mut app, |a| a.spotify_queue.is_some());
        assert_eq!(queue_count(&app, &host), 1);
        app.spotify_queue_asked_at = Some(Instant::now() - Duration::from_secs(4));
        app.poll_spotify();
        assert_eq!(wait_for_count(&host, |op| *op == SpotifyOp::Queue, 2), 2);
        assert_eq!(queue_count(&app, &host), 2);
    }

    #[test]
    fn leaving_the_mode_sends_nothing_to_spotify() {
        let dir = tempfile::tempdir().unwrap();
        let host = Arc::new(FakeHost::default());
        let mut app = mode_app(&dir, &host);
        press(&mut app, KeyCode::Char('O'));
        assert!(!app.in_spotify_mode());
        assert!(settle(&app, &[&*host]).is_empty());
    }

    #[test]
    fn each_library_only_key_toasts_not_in_spotify_mode() {
        let dir = tempfile::tempdir().unwrap();
        let host = Arc::new(FakeHost::default());
        let mut app = mode_app(&dir, &host);
        for key in ['A', 'L', 'F', 'U', 'J'] {
            press(&mut app, KeyCode::Char(key));
            assert!(
                settings_tests::toast_texts(&app)
                    .iter()
                    .any(|t| t == spotify_mode::NOT_IN_MODE),
                "{key} should toast"
            );
        }
        assert!(app.in_spotify_mode());
        assert!(settle(&app, &[&*host]).is_empty());
    }

    /// Two files in the Znicz queue, which no Spotify key may touch.
    fn seed_znicz_queue(app: &App) {
        app.player
            .send_blocking(Command::QueueAdd(vec![
                QueueItem::file("/music/a.flac"),
                QueueItem::file("/music/b.flac"),
            ]))
            .unwrap();
    }

    fn key_with(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, modifiers)
    }

    #[test]
    fn the_stage_queue_drawer_keys_never_reach_the_znicz_queue() {
        let dir = tempfile::tempdir().unwrap();
        let host = Arc::new(FakeHost::default());
        let mut app = mode_app(&dir, &host);
        seed_znicz_queue(&app);
        press(&mut app, KeyCode::Char('v'));
        press(&mut app, KeyCode::Char(']'));
        assert_eq!(app.view, View::Stage);
        assert!(app.queue_open);
        let queue = app.player.state().queue;
        let status = app.player.state().status;
        let toasts = settings_tests::toast_texts(&app);
        for key in [
            key_with(KeyCode::Enter, KeyModifiers::NONE),
            key_with(KeyCode::Char('d'), KeyModifiers::NONE),
            key_with(KeyCode::Char('C'), KeyModifiers::NONE),
            key_with(KeyCode::Down, KeyModifiers::ALT),
            key_with(KeyCode::Up, KeyModifiers::ALT),
        ] {
            app.on_key(key);
        }
        assert_eq!(
            app.player.state().queue,
            queue,
            "the Znicz queue is unchanged"
        );
        assert_eq!(app.player.state().status, status, "nothing started");
        assert_eq!(settings_tests::toast_texts(&app), toasts, "no Znicz toast");
    }

    #[test]
    fn the_spotify_device_list_keeps_space_and_n_away_from_znicz() {
        let dir = tempfile::tempdir().unwrap();
        let host = Arc::new(FakeHost::default());
        let mut app = mode_app(&dir, &host);
        seed_znicz_queue(&app);
        press(&mut app, KeyCode::Char(','));
        assert_eq!(app.modal, Modal::SpotifyDevices);
        let queue = app.player.state().queue;
        let status = app.player.state().status;
        let toasts = settings_tests::toast_texts(&app);
        press(&mut app, KeyCode::Char(' '));
        press(&mut app, KeyCode::Char('n'));
        assert_eq!(app.modal, Modal::SpotifyDevices, "the list stays open");
        assert_eq!(app.player.state().queue, queue);
        assert_eq!(app.player.state().status, status, "nothing started");
        assert_eq!(settings_tests::toast_texts(&app), toasts, "no Znicz toast");
    }

    #[test]
    fn esc_with_the_spotify_drawer_open_closes_the_drawer_before_going_back() {
        let dir = tempfile::tempdir().unwrap();
        let host = Arc::new(FakeHost::default());
        let mut app = mode_app(&dir, &host);
        // Playlists: one level down.
        press(&mut app, KeyCode::Char('j'));
        press(&mut app, KeyCode::Char('l'));
        assert_eq!(app.spotify_browser.depth(), 2);
        press(&mut app, KeyCode::Char(']'));
        assert!(app.queue_open);
        press(&mut app, KeyCode::Esc);
        assert!(!app.queue_open, "the first Esc closes the drawer");
        assert_eq!(app.spotify_browser.depth(), 2, "and keeps the level");
        press(&mut app, KeyCode::Esc);
        assert_eq!(app.spotify_browser.depth(), 1, "the next Esc goes back");
    }

    #[test]
    fn quick_volume_presses_build_on_the_last_sent_value() {
        let dir = tempfile::tempdir().unwrap();
        let host = Arc::new(FakeHost::default());
        let mut app = mode_app(&dir, &host);
        app.spotify_state.playback = Some(playing(Some(40), true));
        for _ in 0..3 {
            press(&mut app, KeyCode::Char('+'));
        }
        let sent = wait_for_count(&host, |op| matches!(op, SpotifyOp::SetVolume { .. }), 3);
        assert_eq!(sent, 3);
        let volumes: Vec<f32> = settle(&app, &[&*host])
            .iter()
            .filter_map(|op| match op {
                SpotifyOp::SetVolume { volume } => Some(*volume),
                _ => None,
            })
            .collect();
        assert_eq!(volumes, vec![0.41, 0.42, 0.43]);
    }

    #[test]
    fn quick_repeat_presses_build_on_the_last_sent_mode() {
        let dir = tempfile::tempdir().unwrap();
        let host = Arc::new(FakeHost::default());
        let mut app = mode_app(&dir, &host);
        app.spotify_state.playback = Some(playing(Some(40), true));
        press(&mut app, KeyCode::Char('e'));
        press(&mut app, KeyCode::Char('e'));
        let sent = wait_for_count(&host, |op| matches!(op, SpotifyOp::SetRepeat { .. }), 2);
        assert_eq!(sent, 2);
        let modes: Vec<SpotifyRepeat> = settle(&app, &[&*host])
            .iter()
            .filter_map(|op| match op {
                SpotifyOp::SetRepeat { mode } => Some(*mode),
                _ => None,
            })
            .collect();
        assert_eq!(modes, vec![SpotifyRepeat::Context, SpotifyRepeat::Track]);
    }

    #[test]
    fn with_no_browse_lane_the_open_drawer_retries_at_most_every_3s() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = logged_in(&dir);
        press(&mut app, KeyCode::Char('O'));
        press(&mut app, KeyCode::Char(']'));
        let no_lane = |app: &App| {
            settings_tests::toast_texts(app)
                .iter()
                .filter(|t| *t == spotify_mode::NO_LANE)
                .count()
        };
        assert_eq!(no_lane(&app), 1);
        app.refresh_spotify_queue();
        app.refresh_spotify_queue();
        assert_eq!(no_lane(&app), 1, "a tick does not ask again at once");
    }

    #[test]
    fn a_queue_add_that_succeeds_makes_the_drawer_read_at_once() {
        let dir = tempfile::tempdir().unwrap();
        let host = Arc::new(FakeHost::default());
        let mut app = mode_app(&dir, &host);
        app.spotify_queue_asked_at = Some(Instant::now());
        app.on_spotify_done(Done {
            tag: SpotifyAsk::QueueAdd,
            result: Ok(SpotifyReply::Done),
        });
        assert!(
            app.spotify_queue_asked_at.is_none(),
            "the next tick reads the queue"
        );
    }

    fn change_fails(app: &mut App, kind: SpotifyErrorKind, message: &str) {
        app.on_spotify_done(Done {
            tag: SpotifyAsk::Transport,
            result: Err(ZniczError::Spotify {
                kind,
                message: message.into(),
            }),
        });
    }

    #[test]
    fn a_network_error_on_a_change_says_it_may_still_happen_and_rereads() {
        let dir = tempfile::tempdir().unwrap();
        let host = Arc::new(FakeHost::default());
        let mut app = mode_app(&dir, &host);
        app.spotify_polled_at = Some(Instant::now());
        change_fails(&mut app, SpotifyErrorKind::Network, "timed out");
        assert!(
            settings_tests::toast_texts(&app)
                .iter()
                .any(|t| t.contains("may still happen"))
        );
        assert!(app.spotify_polled_at.is_none());
    }

    #[test]
    fn no_active_device_says_how_to_pick_one() {
        let dir = tempfile::tempdir().unwrap();
        let host = Arc::new(FakeHost::default());
        let mut app = mode_app(&dir, &host);
        change_fails(
            &mut app,
            SpotifyErrorKind::NoActiveDevice,
            "Spotify has no active device: open Spotify somewhere, or pick one with ,",
        );
        assert!(
            settings_tests::toast_texts(&app)
                .iter()
                .any(|t| t.contains("pick one with ,"))
        );
    }

    #[test]
    fn an_outdated_player_stops_polling_in_the_mode() {
        let dir = tempfile::tempdir().unwrap();
        let host = Arc::new(FakeHost::default());
        let mut app = mode_app(&dir, &host);
        change_fails(&mut app, SpotifyErrorKind::OutdatedPlayer, "update Znicz");
        assert_eq!(app.spotify_unavailable.as_deref(), Some("update Znicz"));
    }

    #[test]
    fn losing_the_login_in_the_mode_leaves_it_and_says_so() {
        let dir = tempfile::tempdir().unwrap();
        let host = Arc::new(FakeHost::default());
        let mut app = mode_app(&dir, &host);
        app.on_spotify_state(SpotifyState {
            login: SpotifyLogin::NotLoggedIn,
            ..SpotifyState::default()
        });
        assert!(!app.in_spotify_mode());
        assert!(
            settings_tests::toast_texts(&app)
                .iter()
                .any(|t| t == spotify_mode::LOGGED_OUT)
        );
        assert!(settle(&app, &[&*host]).is_empty());
    }

    #[test]
    fn a_queue_read_error_is_kept_until_a_read_succeeds() {
        let dir = tempfile::tempdir().unwrap();
        let host = Arc::new(FakeHost::default());
        let mut app = mode_app(&dir, &host);
        app.on_spotify_done(Done {
            tag: SpotifyAsk::Queue,
            result: Err(ZniczError::Spotify {
                kind: SpotifyErrorKind::Network,
                message: "timed out".into(),
            }),
        });
        assert_eq!(
            app.spotify_queue_error.as_deref(),
            Some(spotify_mode::NETWORK_TEXT)
        );
        assert!(
            app.spotify_unavailable.is_none(),
            "a network error keeps polling"
        );
        app.on_spotify_done(Done {
            tag: SpotifyAsk::Queue,
            result: Ok(SpotifyReply::Queue {
                current: None,
                next: Vec::new(),
            }),
        });
        assert!(app.spotify_queue_error.is_none(), "a good read clears it");
    }

    #[test]
    fn entering_or_leaving_the_mode_resets_the_queue_error() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = logged_in(&dir);
        press(&mut app, KeyCode::Char('O'));
        assert!(app.in_spotify_mode());
        app.spotify_queue_error = Some("old read failure".into());
        press(&mut app, KeyCode::Char('O'));
        assert!(!app.in_spotify_mode());
        assert!(app.spotify_queue_error.is_none(), "leaving clears it");
        app.spotify_queue_error = Some("another".into());
        press(&mut app, KeyCode::Char('O'));
        assert!(app.in_spotify_mode());
        assert!(app.spotify_queue_error.is_none(), "entering clears it");
    }

    #[test]
    fn a_queue_reply_or_failure_after_leaving_the_mode_is_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let host = Arc::new(FakeHost::default());
        let mut app = mode_app(&dir, &host);
        press(&mut app, KeyCode::Char('O'));
        assert!(!app.in_spotify_mode());
        app.on_spotify_done(Done {
            tag: SpotifyAsk::Queue,
            result: Err(ZniczError::Spotify {
                kind: SpotifyErrorKind::NoService,
                message: "the player has no Spotify service".into(),
            }),
        });
        assert!(
            app.spotify_queue_error.is_none(),
            "a late failure is dropped"
        );
        assert!(app.spotify_unavailable.is_none(), "a late stop is dropped");
        app.on_spotify_done(Done {
            tag: SpotifyAsk::Queue,
            result: Ok(SpotifyReply::Queue {
                current: None,
                next: Vec::new(),
            }),
        });
        assert!(app.spotify_queue.is_none(), "a late reply is dropped");
    }

    #[test]
    fn a_queue_read_that_finds_no_spotify_stops_the_drawer_asking_again() {
        let dir = tempfile::tempdir().unwrap();
        let host = Arc::new(FakeHost::default());
        let mut app = mode_app(&dir, &host);
        app.queue_open = true;
        app.on_spotify_done(Done {
            tag: SpotifyAsk::Queue,
            result: Err(ZniczError::Spotify {
                kind: SpotifyErrorKind::NoService,
                message: "the player has no Spotify service".into(),
            }),
        });
        assert_eq!(
            app.spotify_unavailable.as_deref(),
            Some("the player has no Spotify service")
        );
        assert_eq!(
            app.spotify_queue_error.as_deref(),
            Some("the player has no Spotify service")
        );
        let asked = queue_count(&app, &host);
        app.spotify_queue_asked_at = Some(Instant::now() - Duration::from_secs(4));
        app.refresh_spotify_queue();
        assert_eq!(
            queue_count(&app, &host),
            asked,
            "no request while unavailable"
        );
    }
}
