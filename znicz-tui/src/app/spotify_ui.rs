//! The Spotify rows of the Settings modal: the status poll, the worker's
//! replies, the account and device actions, and the login panel. A second
//! `impl App` beside `settings_ui.rs`.

use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent};
use znicz_core::{
    SpotifyErrorKind, SpotifyLogin, SpotifyOp, SpotifyReply, SpotifyState, ZniczError,
};

use super::{App, Modal};
use crate::line_edit::LineEdit;
use crate::settings::{Row, Tab};
use crate::spotify_rows::{self, LoginOutcome, SpotifyView};
use crate::spotify_worker::{Done, SpotifyAsk};

/// How often the status is read while the Spotify tab is open.
const SPOTIFY_POLL_EVERY: Duration = Duration::from_millis(500);

/// The paste prompt inside the login panel.
pub(crate) struct SpotifyPaste {
    pub text: LineEdit,
    /// Spotify refused the address: the reason, shown under the prompt.
    pub error: Option<String>,
}

/// The login panel: the URL to open and, when the user pastes, the prompt.
pub(crate) struct LoginPanel {
    pub url: String,
    /// The callback listener is up, so the browser redirect can finish the login.
    pub listening: bool,
    pub paste: Option<SpotifyPaste>,
    /// One line about the browser, when it could not be opened.
    pub notice: Option<String>,
}

/// A change that waits for Enter in the strip.
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
            self.request_devices();
        }
    }

    /// Ask the player for its device list. One request at a time: while one is
    /// pending, a new one is dropped. The client ID is recorded, so a reply that
    /// arrives after the ID changed is not shown.
    pub(crate) fn request_devices(&mut self) {
        if self.spotify_devices_requested {
            return;
        }
        self.spotify_devices_client_id = self.spotify.client_id.clone();
        if self.ask_spotify(SpotifyAsk::Devices, SpotifyOp::Devices) {
            self.spotify_devices_requested = true;
        }
    }

    /// Ask the player to begin a login. One at a time: while a `BeginLogin` is
    /// pending, a new one would make a second PKCE session and void the first.
    pub(crate) fn request_login(&mut self) {
        if self.spotify_login_requested {
            return;
        }
        if self.ask_spotify(SpotifyAsk::BeginLogin, SpotifyOp::BeginLogin) {
            self.spotify_login_requested = true;
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
        let was_logged_in = self.spotify_state.login == SpotifyLogin::LoggedIn;
        self.spotify_state = state;
        let login = self.spotify_state.login.clone();
        // A login that just finished: the player's device list is now worth reading.
        if login == SpotifyLogin::LoggedIn && !was_logged_in {
            self.request_devices();
        }
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
        // Whatever the reply holds, the request is no longer pending.
        match done.tag {
            SpotifyAsk::BeginLogin => self.spotify_login_requested = false,
            SpotifyAsk::Devices => self.spotify_devices_requested = false,
            _ => {}
        }
        match (done.tag, done.result) {
            (SpotifyAsk::BeginLogin, Ok(SpotifyReply::AuthorizeUrl { url, listening, .. })) => {
                if self.modal == Modal::Settings {
                    self.open_login_panel(url, listening);
                } else {
                    // Settings closed while the player was starting the login:
                    // nothing shows it, so the player must not keep it pending.
                    self.ask_spotify(SpotifyAsk::CancelLogin, SpotifyOp::CancelLogin);
                }
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
            // A list for a client ID that was changed while it was pending is stale.
            (SpotifyAsk::Devices, Ok(SpotifyReply::Devices { devices }))
                if self.spotify.client_id == self.spotify_devices_client_id =>
            {
                self.spotify_devices = devices;
            }
            (SpotifyAsk::Devices, Ok(SpotifyReply::Devices { .. })) => {}
            // Devices is read when the tab opens; without a login it fails, and
            // the account row already says so.
            (SpotifyAsk::Devices, Err(e)) => tracing::debug!("spotify devices: {e}"),
            (_, Err(e)) => self.toasts.error(e.to_string()),
            (_, Ok(_)) => {}
        }
    }

    /// Enter on Account. Without a client ID nothing happens (the strip says
    /// so). While a login is pending or its panel is open, nothing happens.
    /// Logged in: ask before logging out. Otherwise start the login.
    pub(crate) fn spotify_account_enter(&mut self) {
        if !self.spotify.is_configured()
            || self.spotify_login.is_some()
            || self.spotify_login_requested
        {
            return;
        }
        if self.spotify_login_shown() == SpotifyLogin::LoggedIn {
            self.spotify_confirm = Some(SpotifyConfirm::Logout);
        } else {
            self.request_login();
        }
    }

    /// Enter on the strip's question: log out, or save the new client ID.
    pub(crate) fn confirm_spotify(&mut self) {
        match self.spotify_confirm.take() {
            Some(SpotifyConfirm::Logout) => {
                self.ask_spotify(SpotifyAsk::Logout, SpotifyOp::Logout);
            }
            Some(SpotifyConfirm::ClientId(id)) => {
                self.change_saved(&Row::ClientId, true, |app| app.set_client_id(id));
            }
            None => {}
        }
    }

    /// The URL came back from BeginLogin: show the panel, and open the browser
    /// when the setting and this terminal's environment say so (`self.browser`
    /// is `znicz_spotify::browser::open` in production).
    fn open_login_panel(&mut self, url: String, listening: bool) {
        self.spotify_state.login = SpotifyLogin::LoggingIn;
        let mut notice = None;
        let env = |key: &str| std::env::var(key).ok();
        if znicz_spotify::browser::should_open(self.open_browser, &env) {
            if let Err(e) = (self.browser)(&url) {
                notice = Some(format!("Could not open a browser: {e}. Use the URL below."));
            }
        }
        self.spotify_login = Some(LoginPanel {
            url,
            listening,
            paste: None,
            notice,
        });
    }

    /// Keys while the login panel is open. Returns false for keys the panel
    /// does not own.
    pub(crate) fn on_spotify_panel_key(&mut self, key: KeyEvent) -> bool {
        let pasting = self
            .spotify_login
            .as_ref()
            .is_some_and(|p| p.paste.is_some());
        if pasting {
            if key.code == KeyCode::Enter {
                self.submit_paste();
            } else if let Some(paste) = self.spotify_login.as_mut().and_then(|p| p.paste.as_mut()) {
                paste.text.on_key(key);
                paste.error = None;
            }
            return true;
        }
        match key.code {
            KeyCode::Char('c') => self.copy_login_url(),
            KeyCode::Enter => {
                if let Some(panel) = self.spotify_login.as_mut() {
                    panel.paste = Some(SpotifyPaste {
                        text: LineEdit::new(),
                        error: None,
                    });
                }
            }
            _ => return false,
        }
        true
    }

    /// Esc with the panel open: a paste prompt closes back to the panel;
    /// otherwise the login is cancelled and the panel closes.
    pub(crate) fn esc_spotify_panel(&mut self) {
        let Some(panel) = self.spotify_login.as_mut() else {
            return;
        };
        if panel.paste.take().is_some() {
            return;
        }
        self.spotify_login = None;
        self.ask_spotify(SpotifyAsk::CancelLogin, SpotifyOp::CancelLogin);
    }

    /// `c`: write the OSC 52 sequence through `self.clipboard`. Tells the user
    /// it was sent, not that it arrived: the terminal does not say.
    fn copy_login_url(&mut self) {
        let Some(url) = self.spotify_login.as_ref().map(|p| p.url.clone()) else {
            return;
        };
        match (self.clipboard)(&url) {
            Ok(()) => self
                .toasts
                .info("Sent the login URL to the terminal's clipboard"),
            Err(e) => self.toasts.error(format!("Could not copy the URL: {e}")),
        }
    }

    /// Enter in the paste prompt: send the pasted address. An empty address does nothing.
    fn submit_paste(&mut self) {
        let Some(paste) = self.spotify_login.as_mut().and_then(|p| p.paste.as_mut()) else {
            return;
        };
        let text = paste.text.as_str().trim().to_string();
        if text.is_empty() {
            return;
        }
        paste.error = None;
        self.ask_spotify(
            SpotifyAsk::CompleteLogin,
            SpotifyOp::CompleteLogin { redirect_url: text },
        );
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
    use crate::app::settings_ui::tests::press;
    use crate::spotify_worker::{Done, SpotifyAsk, SpotifyWorker};
    use znicz_spotify::browser::OpenBrowser;

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

    /// What the app handed to the clipboard and to the browser opener.
    #[derive(Clone, Default)]
    struct Recorded {
        copied: Arc<Mutex<Vec<String>>>,
        opened: Arc<Mutex<Vec<String>>>,
    }

    /// An app on the Spotify tab whose clipboard and browser are recorders.
    /// The host reports `LoggingIn`, as the player does while a panel is open,
    /// so a status poll during a test does not close the panel.
    fn panel_app(dir: &tempfile::TempDir, host: Arc<FakeHost>) -> (App, Recorded) {
        *host.state.lock().unwrap() = Some(SpotifyState {
            login: SpotifyLogin::LoggingIn,
            ..SpotifyState::default()
        });
        let mut app = app_with(dir, host);
        let recorded = Recorded::default();
        let copied = recorded.copied.clone();
        app.clipboard = Box::new(move |text| {
            copied.lock().unwrap().push(text.to_string());
            Ok(())
        });
        let opened = recorded.opened.clone();
        app.browser = Box::new(move |url| {
            opened.lock().unwrap().push(url.to_string());
            Ok(())
        });
        open_spotify_tab(&mut app);
        (app, recorded)
    }

    #[test]
    fn begin_login_opens_the_panel_with_the_url_and_the_listening_line() {
        let dir = tempfile::tempdir().unwrap();
        let host = Arc::new(FakeHost::default());
        *host.reply.lock().unwrap() = Some(SpotifyReply::AuthorizeUrl {
            url: "https://accounts.spotify.com/authorize?x=1".into(),
            redirect_uri: "http://127.0.0.1:8898/callback".into(),
            listening: true,
        });
        let (mut app, recorded) = panel_app(&dir, host.clone());
        app.open_browser = OpenBrowser::Always;
        app.ask_spotify(SpotifyAsk::BeginLogin, SpotifyOp::BeginLogin);
        wait_until(&mut app, |app| app.spotify_login.is_some());
        let panel = app.spotify_login.as_ref().unwrap();
        assert_eq!(panel.url, "https://accounts.spotify.com/authorize?x=1");
        assert!(panel.listening);
        assert!(panel.notice.is_none());
        assert_eq!(app.spotify_state.login, SpotifyLogin::LoggingIn);
        assert_eq!(
            recorded.opened.lock().unwrap().as_slice(),
            ["https://accounts.spotify.com/authorize?x=1"]
        );
    }

    #[test]
    fn a_failed_browser_open_is_one_line_and_the_panel_stays() {
        let dir = tempfile::tempdir().unwrap();
        let (mut app, _recorded) = panel_app(&dir, Arc::new(FakeHost::default()));
        app.open_browser = OpenBrowser::Always;
        app.browser = Box::new(|_| Err(std::io::Error::other("no opener")));
        app.open_login_panel("https://accounts.spotify.com/authorize?x=1".into(), false);
        let panel = app.spotify_login.as_ref().expect("the panel waits");
        assert_eq!(
            panel.notice.as_deref(),
            Some("Could not open a browser: no opener. Use the URL below.")
        );
        assert!(!panel.listening);
    }

    #[test]
    fn the_browser_rule_decides_whether_the_opener_runs() {
        let dir = tempfile::tempdir().unwrap();
        let (mut app, recorded) = panel_app(&dir, Arc::new(FakeHost::default()));
        app.open_browser = OpenBrowser::Never;
        app.open_login_panel("u".into(), true);
        assert!(recorded.opened.lock().unwrap().is_empty());
        assert!(app.spotify_login.as_ref().unwrap().notice.is_none());
    }

    #[test]
    fn enter_in_the_panel_opens_the_paste_prompt_and_a_pasted_address_is_sent() {
        let dir = tempfile::tempdir().unwrap();
        let host = Arc::new(FakeHost::default());
        let (mut app, _recorded) = panel_app(&dir, host.clone());
        app.spotify_login = Some(LoginPanel {
            url: "u".into(),
            listening: false,
            paste: None,
            notice: None,
        });
        press(&mut app, KeyCode::Enter);
        assert!(app.spotify_login.as_ref().unwrap().paste.is_some());
        for c in "code=x&state=y".chars() {
            press(&mut app, KeyCode::Char(c));
        }
        press(&mut app, KeyCode::Enter);
        wait_until(&mut app, |_| {
            host.ops
                .lock()
                .unwrap()
                .iter()
                .any(|op| matches!(op, SpotifyOp::CompleteLogin { .. }))
        });
        assert!(
            host.ops
                .lock()
                .unwrap()
                .contains(&SpotifyOp::CompleteLogin {
                    redirect_url: "code=x&state=y".into()
                })
        );
    }

    #[test]
    fn a_wrong_address_keeps_the_paste_prompt_open_with_the_reason() {
        let dir = tempfile::tempdir().unwrap();
        let host = Arc::new(FakeHost::default());
        let (mut app, _recorded) = panel_app(&dir, host);
        app.spotify_login = Some(LoginPanel {
            url: "u".into(),
            listening: false,
            paste: Some(SpotifyPaste {
                text: LineEdit::from_text("code=x&state=bad"),
                error: None,
            }),
            notice: None,
        });
        app.on_spotify_done(Done {
            tag: SpotifyAsk::CompleteLogin,
            result: Err(spotify_error(SpotifyErrorKind::WrongAddress)),
        });
        let paste = app.spotify_login.as_ref().unwrap().paste.as_ref().unwrap();
        assert_eq!(
            paste.error.as_deref(),
            Some("that address is not from this login")
        );
    }

    #[test]
    fn esc_in_the_paste_prompt_returns_to_the_panel_and_esc_in_the_panel_cancels() {
        let dir = tempfile::tempdir().unwrap();
        let host = Arc::new(FakeHost::default());
        let (mut app, _recorded) = panel_app(&dir, host.clone());
        app.spotify_login = Some(LoginPanel {
            url: "u".into(),
            listening: true,
            paste: Some(SpotifyPaste {
                text: LineEdit::new(),
                error: None,
            }),
            notice: None,
        });
        press(&mut app, KeyCode::Esc);
        assert!(app.spotify_login.is_some());
        assert!(app.spotify_login.as_ref().unwrap().paste.is_none());
        press(&mut app, KeyCode::Esc);
        assert!(app.spotify_login.is_none());
        wait_until(&mut app, |_| {
            host.ops.lock().unwrap().contains(&SpotifyOp::CancelLogin)
        });
        assert_eq!(
            app.modal,
            Modal::Settings,
            "Esc cancels the login, not Settings"
        );
    }

    #[test]
    fn c_hands_the_url_to_the_clipboard_writer_and_says_it_was_sent() {
        let dir = tempfile::tempdir().unwrap();
        let (mut app, recorded) = panel_app(&dir, Arc::new(FakeHost::default()));
        app.spotify_login = Some(LoginPanel {
            url: "https://x/y?z=1".into(),
            listening: true,
            paste: None,
            notice: None,
        });
        press(&mut app, KeyCode::Char('c'));
        assert_eq!(
            recorded.copied.lock().unwrap().as_slice(),
            ["https://x/y?z=1"]
        );
        assert!(
            settings_tests::toast_texts(&app)
                .iter()
                .any(|t| t.starts_with("Sent the login URL"))
        );
    }

    #[test]
    fn a_failed_copy_is_an_error_toast() {
        let dir = tempfile::tempdir().unwrap();
        let (mut app, _recorded) = panel_app(&dir, Arc::new(FakeHost::default()));
        app.clipboard = Box::new(|_| Err(std::io::Error::other("closed")));
        app.spotify_login = Some(LoginPanel {
            url: "u".into(),
            listening: true,
            paste: None,
            notice: None,
        });
        press(&mut app, KeyCode::Char('c'));
        assert!(
            settings_tests::toast_texts(&app)
                .iter()
                .any(|t| t.starts_with("Could not copy the URL"))
        );
    }

    #[test]
    fn keys_the_panel_does_not_own_fall_through_so_s_still_closes_settings() {
        let dir = tempfile::tempdir().unwrap();
        let (mut app, _recorded) = panel_app(&dir, Arc::new(FakeHost::default()));
        app.spotify_login = Some(LoginPanel {
            url: "u".into(),
            listening: true,
            paste: None,
            notice: None,
        });
        press(&mut app, KeyCode::Char('S'));
        assert_eq!(app.modal, Modal::None);
        assert!(app.spotify_login.is_none(), "the panel goes with Settings");
    }

    #[test]
    fn in_the_paste_prompt_every_key_types() {
        let dir = tempfile::tempdir().unwrap();
        let (mut app, _recorded) = panel_app(&dir, Arc::new(FakeHost::default()));
        app.spotify_login = Some(LoginPanel {
            url: "u".into(),
            listening: true,
            paste: Some(SpotifyPaste {
                text: LineEdit::new(),
                error: None,
            }),
            notice: None,
        });
        for c in "S?qc6".chars() {
            press(&mut app, KeyCode::Char(c));
        }
        let paste = app.spotify_login.as_ref().unwrap().paste.as_ref().unwrap();
        assert_eq!(paste.text.as_str(), "S?qc6");
        assert_eq!(app.modal, Modal::Settings);
        assert!(!app.should_quit);
    }

    #[test]
    fn the_footer_and_help_know_the_panel() {
        let dir = tempfile::tempdir().unwrap();
        let (mut app, _recorded) = panel_app(&dir, Arc::new(FakeHost::default()));
        app.spotify_login = Some(LoginPanel {
            url: "u".into(),
            listening: true,
            paste: None,
            notice: None,
        });
        assert_eq!(app.context(), crate::keys::Context::SettingsLogin);
        app.spotify_login.as_mut().unwrap().paste = Some(SpotifyPaste {
            text: LineEdit::new(),
            error: None,
        });
        assert_eq!(app.context(), crate::keys::Context::SettingsPrompt);
        let lines = crate::keys::SETTINGS;
        assert!(lines.iter().any(|b| b.keys == "1–6"));
        assert!(lines.iter().any(|b| b.keys == "Login: c"));
        assert!(lines.iter().any(|b| b.keys == "Login: Enter"));
        assert!(lines.iter().any(|b| b.keys == "Login: Esc"));
    }

    #[test]
    fn the_panel_is_dropped_when_settings_is_no_longer_on_screen() {
        let dir = tempfile::tempdir().unwrap();
        let (mut app, _recorded) = panel_app(&dir, Arc::new(FakeHost::default()));
        app.spotify_login = Some(LoginPanel {
            url: "u".into(),
            listening: true,
            paste: None,
            notice: None,
        });
        app.modal = Modal::Help;
        app.drop_removal_outside_settings();
        assert!(app.spotify_login.is_none());
    }

    #[test]
    fn a_login_that_arrives_after_settings_closed_is_cancelled_not_opened() {
        let dir = tempfile::tempdir().unwrap();
        let host = Arc::new(FakeHost::default());
        let (mut app, recorded) = panel_app(&dir, host.clone());
        app.modal = Modal::Help;
        app.on_spotify_done(Done {
            tag: SpotifyAsk::BeginLogin,
            result: Ok(SpotifyReply::AuthorizeUrl {
                url: "u".into(),
                redirect_uri: "http://127.0.0.1:8898/callback".into(),
                listening: true,
            }),
        });
        assert!(app.spotify_login.is_none(), "no panel without Settings");
        assert!(recorded.opened.lock().unwrap().is_empty());
        wait_until(&mut app, |_| {
            host.ops.lock().unwrap().contains(&SpotifyOp::CancelLogin)
        });
    }

    #[test]
    fn a_login_that_finishes_asks_the_player_for_the_device_list_once() {
        let dir = tempfile::tempdir().unwrap();
        let host = Arc::new(FakeHost::default());
        let mut app = app_with(&dir, host.clone());
        app.spotify_state.login = SpotifyLogin::LoggingIn;
        app.on_spotify_state(logged_in_state());
        wait_until(&mut app, |_| {
            host.ops.lock().unwrap().contains(&SpotifyOp::Devices)
        });
        let devices_asked = |host: &FakeHost| {
            host.ops
                .lock()
                .unwrap()
                .iter()
                .filter(|op| **op == SpotifyOp::Devices)
                .count()
        };
        let before = devices_asked(&host);
        // A later status that still says LoggedIn asks nothing more.
        app.on_spotify_state(logged_in_state());
        std::thread::sleep(std::time::Duration::from_millis(50));
        assert_eq!(devices_asked(&host), before);
    }

    #[test]
    fn a_begin_login_error_clears_the_pending_flag() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app_with(&dir, Arc::new(FakeHost::default()));
        app.spotify_login_requested = true;
        app.on_spotify_done(Done {
            tag: SpotifyAsk::BeginLogin,
            result: Err(spotify_error(SpotifyErrorKind::Network)),
        });
        assert!(!app.spotify_login_requested);
    }

    #[test]
    fn a_devices_reply_clears_the_pending_flag_even_when_it_is_not_a_list() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app_with(&dir, Arc::new(FakeHost::default()));
        app.spotify_devices_requested = true;
        app.spotify_devices_client_id = app.spotify.client_id.clone();
        app.on_spotify_done(Done {
            tag: SpotifyAsk::Devices,
            result: Ok(SpotifyReply::Done),
        });
        assert!(!app.spotify_devices_requested);
    }

    #[test]
    fn a_device_list_for_a_client_id_that_changed_meanwhile_is_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app_with(&dir, Arc::new(FakeHost::default()));
        app.spotify_devices_requested = true;
        app.spotify_devices_client_id = "0123456789abcdef0123456789abcdef".into();
        app.spotify.client_id = "fedcba9876543210fedcba9876543210".into();
        app.on_spotify_done(Done {
            tag: SpotifyAsk::Devices,
            result: Ok(SpotifyReply::Devices {
                devices: vec![SpotifyDevice {
                    id: "d1".into(),
                    name: "Kitchen".into(),
                    kind: "Speaker".into(),
                    is_active: false,
                    volume_percent: None,
                }],
            }),
        });
        assert!(app.spotify_devices.is_empty());
        assert!(!app.spotify_devices_requested);
    }

    #[test]
    fn closing_settings_with_a_login_pending_cancels_it_in_the_player() {
        let dir = tempfile::tempdir().unwrap();
        let host = Arc::new(FakeHost::default());
        let (mut app, _recorded) = panel_app(&dir, host.clone());
        app.spotify_login = Some(LoginPanel {
            url: "u".into(),
            listening: true,
            paste: None,
            notice: None,
        });
        press(&mut app, KeyCode::Char('S'));
        assert_eq!(app.modal, Modal::None);
        assert!(app.spotify_login.is_none());
        wait_until(&mut app, |_| {
            host.ops.lock().unwrap().contains(&SpotifyOp::CancelLogin)
        });
        // The player now says it is not logged in; reopening the tab shows that.
        *host.state.lock().unwrap() = Some(SpotifyState {
            login: SpotifyLogin::NotLoggedIn,
            ..SpotifyState::default()
        });
        press(&mut app, KeyCode::Char('S'));
        press(&mut app, KeyCode::Char('6'));
        wait_until(&mut app, |app| {
            app.spotify_state.login == SpotifyLogin::NotLoggedIn
        });
        assert_eq!(app.spotify_login_shown(), SpotifyLogin::NotLoggedIn);
    }
}
