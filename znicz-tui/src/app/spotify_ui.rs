//! The Spotify rows of the Settings modal: the status poll, the worker's
//! replies, the account and device actions, and the login panel. A second
//! `impl App` beside `settings_ui.rs`.

use std::time::{Duration, Instant};

use znicz_core::{
    SpotifyErrorKind, SpotifyLogin, SpotifyOp, SpotifyReply, SpotifyState, ZniczError,
};

use super::{App, Modal};
use crate::line_edit::LineEdit;
use crate::settings::Tab;
use crate::spotify_rows::{self, LoginOutcome, SpotifyView};
use crate::spotify_worker::{Done, SpotifyAsk};

/// How often the status is read while the Spotify tab is open.
const SPOTIFY_POLL_EVERY: Duration = Duration::from_millis(500);

/// The paste prompt inside the login panel.
// Read by the paste keys (Task 9) and the panel drawing (Task 10).
#[allow(dead_code)]
pub(crate) struct SpotifyPaste {
    pub text: LineEdit,
    /// Spotify refused the address: the reason, shown under the prompt.
    pub error: Option<String>,
}

/// The login panel: the URL to open and, when the user pastes, the prompt.
// Read by the panel keys and drawing (Tasks 9 and 10).
#[allow(dead_code)]
pub(crate) struct LoginPanel {
    pub url: String,
    /// The callback listener is up, so the browser redirect can finish the login.
    pub listening: bool,
    pub paste: Option<SpotifyPaste>,
    /// One line about the browser, when it could not be opened.
    pub notice: Option<String>,
}

/// A change that waits for Enter in the strip.
// Constructed by the logout and client ID rows (Task 8).
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SpotifyConfirm {
    Logout,
    /// The new client ID, which logs the user out.
    ClientId(String),
}

impl App {
    /// The login as the Settings rows show it. Without a client ID there is no
    /// login. The state's default is `NotConfigured`; with a client ID set that
    /// reads as not logged in until the first poll says otherwise. What an
    /// earlier poll learned stays when the tab is shown again.
    pub(crate) fn spotify_login_shown(&self) -> SpotifyLogin {
        if !self.spotify.is_configured() {
            return SpotifyLogin::NotConfigured;
        }
        match &self.spotify_state.login {
            SpotifyLogin::NotConfigured => SpotifyLogin::NotLoggedIn,
            login => login.clone(),
        }
    }

    /// Everything the Spotify rows show, copied for one frame.
    pub(crate) fn spotify_view(&self) -> SpotifyView {
        SpotifyView {
            configured: self.spotify.is_configured(),
            client_id_head: self.spotify.client_id.chars().take(8).collect(),
            device: self.spotify.device.clone(),
            redirect_port: self.spotify.redirect_port,
            open_browser: self.open_browser,
            login: self.spotify_login_shown(),
            account: self.spotify_state.account.clone(),
            devices: self.spotify_devices.clone(),
            unavailable: self.spotify_unavailable.clone(),
        }
    }

    /// Send a request to the worker. False when there is none.
    pub(crate) fn ask_spotify(&self, tag: SpotifyAsk, op: SpotifyOp) -> bool {
        self.spotify_worker
            .as_ref()
            .is_some_and(|worker| worker.send(tag, op))
    }

    /// The status is worth reading: Settings shows the Spotify tab, or a login
    /// panel waits for the login to finish (the user may have changed tab).
    pub(crate) fn spotify_tab_open(&self) -> bool {
        self.modal == Modal::Settings
            && (self.settings_tab == Tab::Spotify || self.spotify_login.is_some())
    }

    /// The Spotify tab was shown: try the player again and refresh the device list.
    pub(crate) fn on_spotify_tab_shown(&mut self) {
        self.spotify_unavailable = None;
        self.spotify_polled_at = None;
        if self.spotify.is_configured() {
            self.ask_spotify(SpotifyAsk::Devices, SpotifyOp::Devices);
        }
    }

    /// Each tick: take the worker's replies, then read the status when it is due.
    pub(crate) fn poll_spotify(&mut self) {
        while let Some(done) = self.spotify_worker.as_ref().and_then(|w| w.try_recv()) {
            self.on_spotify_done(done);
        }
        if !self.spotify_tab_open() || self.spotify_unavailable.is_some() {
            return;
        }
        let due = self
            .spotify_polled_at
            .is_none_or(|at| at.elapsed() >= SPOTIFY_POLL_EVERY);
        if !due {
            return;
        }
        self.spotify_polled_at = Some(Instant::now());
        let Some(host) = self.spotify_main.clone() else {
            return;
        };
        // Both calls are on the main connection and answer at once. The status
        // goes first: an older player that does not know `Touch` then costs one
        // reconnect, and the poll stops.
        match host.try_state() {
            Ok(state) => {
                self.on_spotify_state(state);
                // Touch keeps the player's fast poll going.
                let _ = host.apply(SpotifyOp::Touch);
            }
            Err(ZniczError::Spotify {
                kind: SpotifyErrorKind::NoService | SpotifyErrorKind::OutdatedPlayer,
                message,
            }) => self.spotify_unavailable = Some(message),
            // The next poll shows the truth; do not log every 500 ms at warn.
            Err(e) => tracing::debug!("spotify status: {e}"),
        }
    }

    /// A status read. A login that left LoggingIn closes the panel and toasts.
    pub(crate) fn on_spotify_state(&mut self, state: SpotifyState) {
        self.spotify_state = state;
        let login = self.spotify_state.login.clone();
        if self.spotify_login.is_some() && login != SpotifyLogin::LoggingIn {
            self.spotify_login = None;
            match spotify_rows::login_outcome(&login, self.spotify_state.account.as_ref()) {
                Some(LoginOutcome::Success(text)) => self.toasts.success(text),
                Some(LoginOutcome::Failure(text)) => self.toasts.error(text),
                None => {}
            }
        }
    }

    /// A worker reply, by what asked for it.
    pub(crate) fn on_spotify_done(&mut self, done: Done<SpotifyAsk>) {
        match (done.tag, done.result) {
            (SpotifyAsk::BeginLogin, Ok(SpotifyReply::AuthorizeUrl { url, listening, .. })) => {
                self.open_login_panel(url, listening);
            }
            (
                SpotifyAsk::CompleteLogin,
                Err(ZniczError::Spotify {
                    kind: SpotifyErrorKind::WrongAddress,
                    ..
                }),
            ) => {
                if let Some(paste) = self.spotify_login.as_mut().and_then(|p| p.paste.as_mut()) {
                    paste.error = Some(spotify_rows::WRONG_ADDRESS.into());
                }
            }
            // The panel closes when the status shows the login finished.
            (SpotifyAsk::CompleteLogin, Ok(_)) | (SpotifyAsk::CancelLogin, _) => {}
            (SpotifyAsk::Logout, Ok(_)) => self.toasts.success("Logged out of Spotify"),
            (SpotifyAsk::Devices, Ok(SpotifyReply::Devices { devices })) => {
                self.spotify_devices = devices;
            }
            // Devices is read when the tab opens; without a login it fails, and
            // the account row already says so.
            (SpotifyAsk::Devices, Err(e)) => tracing::warn!("spotify devices: {e}"),
            (_, Err(e)) => self.toasts.error(e.to_string()),
            (_, Ok(_)) => {}
        }
    }

    /// Show the login panel for a URL the player handed back. Task 9 adds the
    /// browser and the panel's keys.
    fn open_login_panel(&mut self, url: String, listening: bool) {
        self.spotify_login = Some(LoginPanel {
            url,
            listening,
            paste: None,
            notice: None,
        });
        self.spotify_state.login = SpotifyLogin::LoggingIn;
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::sync::{Arc, Mutex};

    use znicz_core::{
        SpotifyAccount, SpotifyConfig, SpotifyDevice, SpotifyErrorKind, SpotifyHost, SpotifyLogin,
        SpotifyOp, SpotifyReply, SpotifyState, ZniczError,
    };

    use super::*;
    use crate::app::App;
    use crate::app::settings_ui::tests as settings_tests;
    use crate::spotify_worker::{Done, SpotifyAsk, SpotifyWorker};

    /// A host with a scripted status and a log of the ops it was given.
    #[derive(Default)]
    pub(crate) struct FakeHost {
        pub state: Mutex<Option<SpotifyState>>,
        pub status_error: Mutex<Option<ZniczError>>,
        pub ops: Mutex<Vec<SpotifyOp>>,
        pub reply: Mutex<Option<SpotifyReply>>,
    }

    impl SpotifyHost for FakeHost {
        fn apply(&self, op: SpotifyOp) -> znicz_core::Result<SpotifyReply> {
            self.ops.lock().unwrap().push(op);
            Ok(self
                .reply
                .lock()
                .unwrap()
                .clone()
                .unwrap_or(SpotifyReply::Done))
        }
        fn state(&self) -> SpotifyState {
            self.state.lock().unwrap().clone().unwrap_or_default()
        }
        fn try_state(&self) -> znicz_core::Result<SpotifyState> {
            if let Some(error) = self.status_error.lock().unwrap().take() {
                return Err(error);
            }
            Ok(self.state())
        }
    }

    fn spotify_error(kind: SpotifyErrorKind) -> ZniczError {
        ZniczError::Spotify {
            kind,
            message: "the player has no Spotify service".into(),
        }
    }

    fn logged_in_state() -> SpotifyState {
        SpotifyState {
            login: SpotifyLogin::LoggedIn,
            account: Some(SpotifyAccount {
                user: "Eugene".into(),
                premium: Some(true),
            }),
            ..SpotifyState::default()
        }
    }

    fn app_with(dir: &tempfile::TempDir, host: Arc<FakeHost>) -> App {
        let mut app = settings_tests::settings_app(dir).with_settings_values(
            crate::settings::SettingsValues {
                spotify: SpotifyConfig {
                    client_id: "0123456789abcdef0123456789abcdef".into(),
                    ..SpotifyConfig::default()
                },
                ..crate::settings::SettingsValues::default()
            },
        );
        app.spotify_main = Some(host.clone());
        app.spotify_worker = Some(SpotifyWorker::spawn(host).unwrap());
        app
    }

    fn open_spotify_tab(app: &mut App) {
        app.toggle_settings_modal();
        app.set_settings_tab(crate::settings::Tab::Spotify);
    }

    fn wait_until(app: &mut App, done: impl Fn(&App) -> bool) {
        for _ in 0..500 {
            app.poll_spotify();
            if done(app) {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        panic!("the condition never held");
    }

    #[test]
    fn the_status_poll_runs_only_while_the_spotify_tab_is_open() {
        let dir = tempfile::tempdir().unwrap();
        let host = Arc::new(FakeHost::default());
        *host.state.lock().unwrap() = Some(logged_in_state());
        let mut app = app_with(&dir, host.clone());
        app.poll_spotify();
        assert!(app.spotify_polled_at.is_none());
        open_spotify_tab(&mut app);
        app.poll_spotify();
        assert_eq!(app.spotify_state.login, SpotifyLogin::LoggedIn);
        assert!(host.ops.lock().unwrap().contains(&SpotifyOp::Touch));
    }

    #[test]
    fn the_poll_waits_half_a_second_between_reads() {
        let dir = tempfile::tempdir().unwrap();
        let host = Arc::new(FakeHost::default());
        let mut app = app_with(&dir, host.clone());
        open_spotify_tab(&mut app);
        app.poll_spotify();
        let first = app.spotify_polled_at;
        app.poll_spotify();
        assert_eq!(app.spotify_polled_at, first);
    }

    #[test]
    fn a_player_without_spotify_stops_the_poll_and_says_why() {
        let dir = tempfile::tempdir().unwrap();
        let host = Arc::new(FakeHost::default());
        *host.status_error.lock().unwrap() = Some(spotify_error(SpotifyErrorKind::NoService));
        let mut app = app_with(&dir, host.clone());
        open_spotify_tab(&mut app);
        app.poll_spotify();
        assert_eq!(
            app.spotify_unavailable.as_deref(),
            Some("the player has no Spotify service")
        );
        // Make the next poll overdue, so only the stop can hold it back.
        let overdue = std::time::Instant::now()
            .checked_sub(std::time::Duration::from_secs(5))
            .expect("the clock is past five seconds");
        app.spotify_polled_at = Some(overdue);
        app.poll_spotify();
        assert_eq!(
            app.spotify_polled_at,
            Some(overdue),
            "no reconnect on each frame"
        );
    }

    #[test]
    fn a_network_error_on_the_poll_is_logged_not_shown() {
        let dir = tempfile::tempdir().unwrap();
        let host = Arc::new(FakeHost::default());
        *host.status_error.lock().unwrap() = Some(spotify_error(SpotifyErrorKind::Network));
        let mut app = app_with(&dir, host);
        open_spotify_tab(&mut app);
        app.poll_spotify();
        assert!(app.spotify_unavailable.is_none());
    }

    #[test]
    fn the_login_panel_closes_and_toasts_when_the_login_finishes() {
        let dir = tempfile::tempdir().unwrap();
        let host = Arc::new(FakeHost::default());
        let mut app = app_with(&dir, host.clone());
        open_spotify_tab(&mut app);
        app.spotify_login = Some(LoginPanel {
            url: "https://accounts.spotify.com/authorize".into(),
            listening: true,
            paste: None,
            notice: None,
        });
        app.on_spotify_state(logged_in_state());
        assert!(app.spotify_login.is_none());
        assert!(
            settings_tests::toast_texts(&app)
                .iter()
                .any(|t| t == "Logged in as Eugene")
        );
    }

    #[test]
    fn a_cancelled_login_closes_the_panel_without_a_toast() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app_with(&dir, Arc::new(FakeHost::default()));
        open_spotify_tab(&mut app);
        app.spotify_login = Some(LoginPanel {
            url: "u".into(),
            listening: true,
            paste: None,
            notice: None,
        });
        app.on_spotify_state(SpotifyState::default());
        assert!(app.spotify_login.is_none());
        assert!(settings_tests::toast_texts(&app).is_empty());
    }

    #[test]
    fn a_device_list_arrives_from_the_worker() {
        let dir = tempfile::tempdir().unwrap();
        let host = Arc::new(FakeHost::default());
        *host.reply.lock().unwrap() = Some(SpotifyReply::Devices {
            devices: vec![SpotifyDevice {
                id: "d1".into(),
                name: "Kitchen".into(),
                kind: "Speaker".into(),
                is_active: false,
                volume_percent: None,
            }],
        });
        let mut app = app_with(&dir, host);
        open_spotify_tab(&mut app);
        wait_until(&mut app, |app| !app.spotify_devices.is_empty());
        assert_eq!(app.spotify_devices[0].name, "Kitchen");
    }

    #[test]
    fn a_configure_error_is_a_toast() {
        let dir = tempfile::tempdir().unwrap();
        let host = Arc::new(FakeHost::default());
        let mut app = app_with(&dir, host);
        app.on_spotify_done(Done {
            tag: SpotifyAsk::Configure,
            result: Err(ZniczError::Spotify {
                kind: SpotifyErrorKind::Network,
                message: "timed out".into(),
            }),
        });
        assert!(
            settings_tests::toast_texts(&app)
                .iter()
                .any(|t| t == "timed out")
        );
    }

    #[test]
    fn the_spotify_view_shows_not_logged_in_before_the_first_poll() {
        let dir = tempfile::tempdir().unwrap();
        let app = app_with(&dir, Arc::new(FakeHost::default()));
        assert_eq!(app.spotify_login_shown(), SpotifyLogin::NotLoggedIn);
        let unconfigured = settings_tests::settings_app(&dir);
        assert_eq!(
            unconfigured.spotify_login_shown(),
            SpotifyLogin::NotConfigured
        );
    }

    #[test]
    fn the_shown_login_follows_the_state_once_it_is_known_and_survives_a_tab_switch() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app_with(&dir, Arc::new(FakeHost::default()));
        app.spotify_state.login = SpotifyLogin::LoggedIn;
        assert_eq!(app.spotify_login_shown(), SpotifyLogin::LoggedIn);
        // Showing the tab polls again but keeps what was known.
        app.on_spotify_tab_shown();
        assert_eq!(app.spotify_login_shown(), SpotifyLogin::LoggedIn);
    }

    #[test]
    fn the_poll_also_runs_while_a_login_panel_is_open_on_another_tab() {
        let dir = tempfile::tempdir().unwrap();
        let host = Arc::new(FakeHost::default());
        *host.state.lock().unwrap() = Some(logged_in_state());
        let mut app = app_with(&dir, host);
        open_spotify_tab(&mut app);
        app.spotify_login = Some(LoginPanel {
            url: "u".into(),
            listening: true,
            paste: None,
            notice: None,
        });
        app.set_settings_tab(crate::settings::Tab::Display);
        app.poll_spotify();
        assert!(
            app.spotify_login.is_none(),
            "the finished login closed the panel"
        );
    }
}
