//! Spotify mode: `O` switches the keys, the footer and the status poll to
//! Spotify. A second `impl App` beside `spotify_ui.rs`.

use std::time::{SystemTime, UNIX_EPOCH};

use znicz_core::{SpotifyLogin, SpotifyOp, SpotifyRepeat};

use super::{App, Modal};
use crate::spotify_mode::{self, MuteAction, SpotifyKey};

/// Unix time in milliseconds: the clock the seek reads, to place the playhead.
fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

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
                    let volume = spotify_mode::volume_after(percent, delta);
                    self.spotify_send_play(SpotifyOp::SetVolume { volume });
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
                self.spotify_send_play(SpotifyOp::SetRepeat {
                    mode: current.next(),
                });
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
            // The devices modal, the queue and the search are wired in later steps:
            // the queue add and drawer in Task 14, the devices modal in Task 15.
            SpotifyKey::Devices
            | SpotifyKey::QueueAdd
            | SpotifyKey::QueueDrawer
            | SpotifyKey::Search => {}
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
    fn spotify_send_volume_percent(&mut self, percent: u8) -> bool {
        self.spotify_send_play(SpotifyOp::SetVolume {
            volume: f32::from(percent) / 100.0,
        })
    }

    /// Whether the keys, the footer and the status poll are Spotify's.
    pub(crate) fn in_spotify_mode(&self) -> bool {
        self.spotify_mode
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
    }

    fn leave_spotify_mode(&mut self) {
        self.spotify_mode = false;
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

    use crossterm::event::KeyCode;
    use znicz_core::{SpotifyLogin, SpotifyState};

    use std::sync::Arc;
    use std::time::{SystemTime, UNIX_EPOCH};

    use znicz_core::{SpotifyDevice, SpotifyOp, SpotifyPlayback, SpotifyRepeat, SpotifyTrack};

    use crate::app::settings_ui::tests as settings_tests;
    use crate::app::settings_ui::tests::press;
    use crate::app::spotify_ui::tests::{FakeHost, sent_ops, wait_for_op};
    use crate::app::{App, Modal, View};
    use crate::keys::{Context, context_hints};
    use crate::spotify_mode;

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
        assert!(sent_ops(&host).is_empty());
    }

    #[test]
    fn plus_sends_one_percent_more_and_minus_one_less() {
        let dir = tempfile::tempdir().unwrap();
        let host = Arc::new(FakeHost::default());
        let mut app = mode_app(&dir, &host);
        app.spotify_state.playback = Some(playing(Some(40), true));
        press(&mut app, KeyCode::Char('+'));
        assert!(wait_for_op(&host, |op| *op == SpotifyOp::SetVolume { volume: 0.41 }));
        press(&mut app, KeyCode::Char('-'));
        assert!(wait_for_op(&host, |op| *op == SpotifyOp::SetVolume { volume: 0.39 }));
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
        assert!(sent_ops(&host).is_empty());
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
        assert!(sent_ops(&host).is_empty());
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
        assert!(sent_ops(&host).is_empty());
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
}
