//! The Spotify service the player process hosts: a worker thread that polls
//! playback, and `SpotifyHost::apply` for every op. One `Client` behind a
//! mutex; one `SpotifyState` snapshot that front ends read.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, Weak};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use znicz_core::{SpotifyConfig, SpotifyHost, SpotifyLogin, SpotifyOp, SpotifyReply, SpotifyState};

use crate::api::Api;
use crate::client::{Client, Clock, Endpoints, system_clock};
use crate::error::{Result, SpotifyError};
use crate::listener::CallbackListener;

/// Poll every second while someone is watching...
const FAST_POLL: Duration = Duration::from_secs(1);
/// ...and every 15 s otherwise.
const SLOW_POLL: Duration = Duration::from_secs(15);
/// "Watching": a `Touch` or any op within this window.
const ACTIVE_WINDOW_MS: u64 = 60_000;
/// Devices are re-read at most this often.
const DEVICES_EVERY_MS: u64 = 15_000;
/// While a login waits for its callback, it is checked this often.
const LOGIN_CHECK: Duration = Duration::from_secs(1);
/// A user action waits for a 429 at most this long before retrying once.
const MAX_RETRY_WAIT_SECS: u64 = 10;

/// How long to wait before the next poll. `None`: not logged in, wait for a wake.
fn poll_interval(logged_in: bool, now_ms: u64, last_touch_ms: u64) -> Option<Duration> {
    if !logged_in {
        return None;
    }
    if now_ms.saturating_sub(last_touch_ms) < ACTIVE_WINDOW_MS {
        Some(FAST_POLL)
    } else {
        Some(SLOW_POLL)
    }
}

pub struct ServiceOptions {
    pub token_path: PathBuf,
    /// Called before an op that starts Spotify playback, so the player can
    /// pause its own engine.
    /// Runs on the caller's thread, inside `apply`. It must not take a lock the caller of `apply` holds (for example the player's), and should hold a `Weak` to the player rather than an `Arc`.
    pub on_start_playback: Box<dyn Fn() + Send + Sync>,
}

struct Shared {
    clock: Clock,
    client: Mutex<Client>,
    /// Bumped whenever the login ends, so a poll that started before a logout
    /// does not write its result into the cleared state.
    login_epoch: AtomicU64,
    state: Mutex<SpotifyState>,
    listener: Mutex<Option<CallbackListener>>,
    last_touch_ms: AtomicU64,
    last_devices_ms: AtomicU64,
    /// When the account was last read: a failed read is not retried every poll.
    last_account_ms: AtomicU64,
    /// No polls before this time (after a 429).
    backoff_until_ms: AtomicU64,
    /// The last poll error shown, so the same one is not repeated every second.
    last_poll_error: Mutex<Option<String>>,
    wake: (Mutex<bool>, Condvar),
    stop: AtomicBool,
    on_start_playback: Box<dyn Fn() + Send + Sync>,
}

pub struct SpotifyService {
    shared: Arc<Shared>,
    worker: Option<JoinHandle<()>>,
}

impl SpotifyService {
    pub fn start(config: SpotifyConfig, options: ServiceOptions) -> Self {
        Self::start_with(config, options, Endpoints::default(), system_clock())
    }

    pub(crate) fn start_with(
        config: SpotifyConfig,
        options: ServiceOptions,
        endpoints: Endpoints,
        clock: Clock,
    ) -> Self {
        let client = Client::new(config, options.token_path, endpoints, clock.clone());
        let state = SpotifyState {
            login: login_of(&client),
            ..SpotifyState::default()
        };
        let shared = Arc::new(Shared {
            clock,
            client: Mutex::new(client),
            login_epoch: AtomicU64::new(0),
            state: Mutex::new(state),
            listener: Mutex::new(None),
            last_touch_ms: AtomicU64::new(0),
            last_devices_ms: AtomicU64::new(0),
            last_account_ms: AtomicU64::new(0),
            backoff_until_ms: AtomicU64::new(0),
            last_poll_error: Mutex::new(None),
            wake: (Mutex::new(false), Condvar::new()),
            stop: AtomicBool::new(false),
            on_start_playback: options.on_start_playback,
        });
        let worker_shared = shared.clone();
        let worker = thread::Builder::new()
            .name("spotify".into())
            .spawn(move || worker_shared.run())
            .expect("spawn the Spotify thread");
        Self {
            shared,
            worker: Some(worker),
        }
    }

    /// Pause Spotify if it is playing: Znicz is about to play.
    /// Blocks for a whole Web API request (up to about 40 s: a refresh, a retry and a rate-limit wait), so never call it while holding the player's lock.
    pub fn pause_if_playing(&self) {
        if !self.shared.state.lock().unwrap().is_playing() {
            return;
        }
        if let Err(e) = self.shared.control(&SpotifyOp::Pause) {
            tracing::warn!(error = %e, "could not pause Spotify");
        }
    }

    /// The port the login listener uses (tests).
    #[cfg(test)]
    fn redirect_port(&self) -> u16 {
        self.shared.client.lock().unwrap().config().redirect_port
    }
}

impl Drop for SpotifyService {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::SeqCst);
        self.shared.wake();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

impl SpotifyHost for SpotifyService {
    fn apply(&self, op: SpotifyOp) -> znicz_core::Result<SpotifyReply> {
        let result = self.shared.apply(op);
        if let Err(e) = &result {
            if matches!(e, SpotifyError::Revoked) {
                self.shared.on_revoked();
            }
            self.shared.state.lock().unwrap().last_error = Some(e.to_string());
        }
        Ok(result?)
    }

    fn state(&self) -> SpotifyState {
        self.shared.state.lock().unwrap().clone()
    }
}

fn login_of(client: &Client) -> SpotifyLogin {
    if !client.config().is_configured() {
        SpotifyLogin::NotConfigured
    } else if client.login_pending() {
        SpotifyLogin::LoggingIn
    } else if client.is_logged_in() {
        SpotifyLogin::LoggedIn
    } else {
        SpotifyLogin::NotLoggedIn
    }
}

impl Shared {
    /// Never takes the client lock, which a poll may hold for a whole request.
    fn now_ms(&self) -> u64 {
        (self.clock)()
    }

    fn wake(&self) {
        let (flag, condvar) = &self.wake;
        *flag.lock().unwrap() = true;
        condvar.notify_all();
    }

    fn touch(&self) {
        self.last_touch_ms.store(self.now_ms(), Ordering::SeqCst);
    }

    fn set_login(&self, login: SpotifyLogin) {
        let mut state = self.state.lock().unwrap();
        if login != SpotifyLogin::LoggedIn {
            self.login_epoch.fetch_add(1, Ordering::SeqCst);
            state.account = None;
            state.playback = None;
            state.devices.clear();
        }
        state.login = login;
    }

    /// The refresh token was revoked: `Client` has removed the token.
    fn on_revoked(&self) {
        self.set_login(SpotifyLogin::NotLoggedIn);
        self.state.lock().unwrap().last_error = Some(SpotifyError::Revoked.to_string());
    }

    /// A login whose callback never came ends once its TTL has passed.
    fn expire_login(&self) {
        if self.state.lock().unwrap().login != SpotifyLogin::LoggingIn {
            return;
        }
        // Client before state, as everywhere; the listener is taken after both are released.
        let expired = {
            let mut client = self.client.lock().unwrap();
            let mut state = self.state.lock().unwrap();
            if !client.login_expired() || state.login != SpotifyLogin::LoggingIn {
                false
            } else {
                client.cancel_login();
                self.login_epoch.fetch_add(1, Ordering::SeqCst);
                state.account = None;
                state.playback = None;
                state.devices.clear();
                state.login = SpotifyLogin::Failed(SpotifyError::LoginExpired.to_string());
                true
            }
        };
        if expired {
            self.listener.lock().unwrap().take();
        }
    }

    fn apply(self: &Arc<Self>, op: SpotifyOp) -> Result<SpotifyReply> {
        self.touch();
        match op {
            SpotifyOp::Touch => {
                self.wake();
                Ok(SpotifyReply::Done)
            }
            SpotifyOp::Configure { config } => {
                let (login, pending) = {
                    let mut client = self.client.lock().unwrap();
                    client.set_config(config);
                    (login_of(&client), client.login_pending())
                };
                // A login still waiting for its callback keeps its listener.
                if !pending {
                    self.listener.lock().unwrap().take();
                }
                self.set_login(login);
                self.wake();
                Ok(SpotifyReply::Done)
            }
            SpotifyOp::BeginLogin => self.begin_login(),
            SpotifyOp::CompleteLogin { redirect_url } => {
                self.finish_login(&redirect_url)?;
                Ok(SpotifyReply::Done)
            }
            SpotifyOp::CancelLogin => {
                let login = {
                    let mut client = self.client.lock().unwrap();
                    client.cancel_login();
                    login_of(&client)
                };
                self.listener.lock().unwrap().take();
                self.set_login(login);
                Ok(SpotifyReply::Done)
            }
            SpotifyOp::Logout => {
                let result = self.client.lock().unwrap().logout();
                self.listener.lock().unwrap().take();
                let login = login_of(&self.client.lock().unwrap());
                self.set_login(login);
                result.map(|()| SpotifyReply::Done)
            }
            SpotifyOp::Search {
                query,
                kinds,
                offset,
                limit,
            } => self.page(|api| api.search(&query, &kinds, offset, limit)),
            SpotifyOp::Playlists { offset, limit } => self.page(|api| api.playlists(offset, limit)),
            SpotifyOp::LikedSongs { offset, limit } => {
                self.page(|api| api.liked_songs(offset, limit))
            }
            SpotifyOp::Albums { offset, limit } => self.page(|api| api.albums(offset, limit)),
            SpotifyOp::Tracks { uri, offset, limit } => {
                self.page(|api| api.tracks(&uri, offset, limit))
            }
            SpotifyOp::ArtistAlbums { uri, offset, limit } => {
                self.page(|api| api.artist_albums(&uri, offset, limit))
            }
            SpotifyOp::Devices => {
                let epoch = self.login_epoch.load(Ordering::SeqCst);
                let devices = self.client.lock().unwrap().call(|api| api.devices())?;
                let mut state = self.state.lock().unwrap();
                if self.login_epoch.load(Ordering::SeqCst) == epoch {
                    state.devices = devices.clone();
                }
                Ok(SpotifyReply::Devices { devices })
            }
            SpotifyOp::Queue => {
                let (current, next) = self.client.lock().unwrap().call(|api| api.queue())?;
                Ok(SpotifyReply::Queue { current, next })
            }
            control => {
                self.control(&control)?;
                Ok(SpotifyReply::Done)
            }
        }
    }

    fn page(
        &self,
        f: impl Fn(&Api<'_>) -> Result<znicz_core::SpotifyPage>,
    ) -> Result<SpotifyReply> {
        let page = self.client.lock().unwrap().call(f)?;
        Ok(SpotifyReply::Page { page })
    }

    fn begin_login(self: &Arc<Self>) -> Result<SpotifyReply> {
        let (url, port, redirect_uri) = {
            let mut client = self.client.lock().unwrap();
            let login = client.begin_login()?;
            let config = client.config();
            (login.url, config.redirect_port, config.redirect_uri())
        };
        // Before the listener starts: a fast callback must not be overwritten.
        self.set_login(SpotifyLogin::LoggingIn);
        // Release the port held by an earlier login's listener.
        self.listener.lock().unwrap().take();
        let weak: Weak<Shared> = Arc::downgrade(self);
        let started = CallbackListener::start(port, move |query| {
            let shared = weak.upgrade()?;
            match shared.finish_login(query) {
                Ok(()) => Some("Logged in to Spotify. You can close this tab.".into()),
                Err(SpotifyError::WrongAddress) => None,
                Err(SpotifyError::NoLogin) => {
                    Some("Already logged in to Spotify. You can close this tab.".into())
                }
                Err(e) => Some(format!("Spotify login failed: {e}")),
            }
        });
        let listening = match started {
            Ok(listener) => {
                *self.listener.lock().unwrap() = Some(listener);
                self.state.lock().unwrap().last_error = None;
                true
            }
            Err(e) => {
                tracing::warn!(port, error = %e, "login listener could not start; paste the address instead");
                self.state.lock().unwrap().last_error = Some(format!(
                    "could not listen on port {port} ({e}): paste the address from the browser instead"
                ));
                false
            }
        };
        Ok(SpotifyReply::AuthorizeUrl {
            url,
            redirect_uri,
            listening,
        })
    }

    /// Finish a login from a callback query or a pasted address.
    fn finish_login(&self, input: &str) -> Result<()> {
        let result = self.client.lock().unwrap().complete_login(input);
        match result {
            Ok(()) => {
                self.listener.lock().unwrap().take();
                self.set_login(SpotifyLogin::LoggedIn);
                self.state.lock().unwrap().last_error = None;
                self.refresh_account();
                self.wake();
                Ok(())
            }
            // Not a failed login: a wrong address, or a paste with no login to finish.
            Err(e @ (SpotifyError::WrongAddress | SpotifyError::NoLogin)) => Err(e),
            Err(e) => {
                self.listener.lock().unwrap().take();
                self.set_login(SpotifyLogin::Failed(e.to_string()));
                Err(e)
            }
        }
    }

    fn refresh_account(&self) {
        let epoch = self.login_epoch.load(Ordering::SeqCst);
        let result = self.client.lock().unwrap().call(|api| api.me());
        match result {
            Ok(account) => {
                let mut state = self.state.lock().unwrap();
                // A logout during the read must not be undone by it.
                if self.login_epoch.load(Ordering::SeqCst) == epoch {
                    state.account = Some(account);
                }
            }
            Err(SpotifyError::Revoked) => self.on_revoked(),
            Err(e) => tracing::debug!(error = %e, "could not read the Spotify account"),
        }
    }

    /// A playback command: pause Znicz first when it starts Spotify, fall
    /// back to the preferred device when none is active, and wait out one 429.
    fn control(&self, op: &SpotifyOp) -> Result<()> {
        // Only playback that can run pauses Znicz; a logged-out play fails below.
        let starts_here = op.starts_playback() && self.client.lock().unwrap().is_logged_in();
        if starts_here {
            (self.on_start_playback)();
        }
        let result = match self.control_once(op, None) {
            Err(SpotifyError::NoActiveDevice) => match self.preferred_device()? {
                Some(id) => self.control_once(op, Some(&id)),
                None => Err(SpotifyError::NoActiveDevice),
            },
            Err(SpotifyError::RateLimited { retry_after_secs })
                if retry_after_secs <= MAX_RETRY_WAIT_SECS =>
            {
                thread::sleep(Duration::from_secs(retry_after_secs));
                self.control_once(op, None)
            }
            other => other,
        };
        // Spotify no longer reports the plan, so a playback answer is how we learn it.
        match &result {
            Ok(()) => {
                self.set_premium(true);
                self.after_control(op);
                self.state.lock().unwrap().last_error = None;
                self.wake();
            }
            Err(SpotifyError::PremiumRequired) => self.set_premium(false),
            Err(SpotifyError::Revoked) => self.on_revoked(),
            Err(_) => {}
        }
        result
    }

    fn set_premium(&self, premium: bool) {
        if let Some(account) = self.state.lock().unwrap().account.as_mut() {
            account.premium = Some(premium);
        }
    }

    fn control_once(&self, op: &SpotifyOp, device: Option<&str>) -> Result<()> {
        let mut client = self.client.lock().unwrap();
        client.call(|api| match op {
            SpotifyOp::Play {
                context_uri,
                uris,
                offset,
            } => api.play(device, context_uri.as_deref(), uris, *offset),
            SpotifyOp::Pause => api.pause(device),
            SpotifyOp::Next => api.next(device),
            SpotifyOp::Previous => api.previous(device),
            SpotifyOp::Seek { position_ms } => api.seek(device, *position_ms),
            SpotifyOp::SetVolume { volume } => api.volume(device, *volume),
            SpotifyOp::SetRepeat { mode } => api.repeat(device, *mode),
            SpotifyOp::SetShuffle { on } => api.shuffle(device, *on),
            SpotifyOp::QueueAdd { uri } => api.queue_add(device, uri),
            SpotifyOp::Transfer { device_id, play } => api.transfer(device_id, *play),
            other => Err(SpotifyError::Parse(format!(
                "{other:?} is not a playback command"
            ))),
        })
    }

    /// The id of the device named in the settings, if it is online.
    fn preferred_device(&self) -> Result<Option<String>> {
        let epoch = self.login_epoch.load(Ordering::SeqCst);
        let mut client = self.client.lock().unwrap();
        let name = client.config().device.clone();
        if name.is_empty() {
            return Ok(None);
        }
        let devices = client.call(|api| api.devices())?;
        drop(client);
        let id = devices
            .iter()
            .find(|d| d.name == name)
            .map(|d| d.id.clone());
        let mut state = self.state.lock().unwrap();
        if self.login_epoch.load(Ordering::SeqCst) == epoch {
            state.devices = devices;
        }
        Ok(id)
    }

    /// Show the obvious effect at once, before the next poll confirms it.
    fn after_control(&self, op: &SpotifyOp) {
        let now = self.now_ms();
        let mut state = self.state.lock().unwrap();
        let Some(playback) = state.playback.as_mut() else {
            return;
        };
        match op {
            SpotifyOp::Play {
                context_uri, uris, ..
            } => {
                playback.is_playing = true;
                playback.read_at_ms = now;
                if context_uri.is_some() || !uris.is_empty() {
                    playback.progress_ms = 0;
                }
            }
            SpotifyOp::Transfer { play: true, .. } => {
                playback.is_playing = true;
                playback.read_at_ms = now;
            }
            SpotifyOp::Pause => {
                playback.progress_ms = playback.progress_at(now);
                playback.read_at_ms = now;
                playback.is_playing = false;
            }
            SpotifyOp::Seek { position_ms } => {
                playback.progress_ms = *position_ms;
                playback.read_at_ms = now;
            }
            SpotifyOp::SetRepeat { mode } => playback.repeat = *mode,
            SpotifyOp::SetShuffle { on } => playback.shuffle = *on,
            _ => {}
        }
    }

    fn run(self: Arc<Self>) {
        loop {
            let interval = {
                let client = self.client.lock().unwrap();
                poll_interval(
                    client.is_logged_in(),
                    client.now_ms(),
                    self.last_touch_ms.load(Ordering::SeqCst),
                )
            };
            let waiting_for_login = self.state.lock().unwrap().login == SpotifyLogin::LoggingIn;
            {
                let (flag, condvar) = &self.wake;
                let mut woken = flag.lock().unwrap();
                if !*woken {
                    let wait = match interval {
                        _ if waiting_for_login => LOGIN_CHECK,
                        Some(interval) => interval,
                        None => SLOW_POLL,
                    };
                    woken = condvar.wait_timeout(woken, wait).unwrap().0;
                }
                *woken = false;
            }
            if self.stop.load(Ordering::SeqCst) {
                return;
            }
            self.expire_login();
            if self.client.lock().unwrap().is_logged_in() {
                self.poll();
            }
        }
    }

    fn poll(&self) {
        let now = self.now_ms();
        if now < self.backoff_until_ms.load(Ordering::SeqCst) {
            return;
        }
        let epoch = self.login_epoch.load(Ordering::SeqCst);
        let need_account = self.state.lock().unwrap().account.is_none();
        let account_due =
            now.saturating_sub(self.last_account_ms.load(Ordering::SeqCst)) >= DEVICES_EVERY_MS;
        if need_account && account_due {
            self.last_account_ms.store(now, Ordering::SeqCst);
            self.refresh_account();
        }
        if self.stop.load(Ordering::SeqCst) {
            return;
        }
        let result = self.client.lock().unwrap().call(|api| api.playback(now));
        let result = result.and_then(|playback| {
            let due =
                now.saturating_sub(self.last_devices_ms.load(Ordering::SeqCst)) >= DEVICES_EVERY_MS;
            let devices = if due && !self.stop.load(Ordering::SeqCst) {
                self.last_devices_ms.store(now, Ordering::SeqCst);
                Some(self.client.lock().unwrap().call(|api| api.devices())?)
            } else {
                None
            };
            Ok((playback, devices))
        });
        if self.login_epoch.load(Ordering::SeqCst) != epoch {
            *self.last_poll_error.lock().unwrap() = None;
            return;
        }
        match result {
            Ok((playback, devices)) => {
                // Never hold `last_poll_error` and `state` at once.
                let previous = self.last_poll_error.lock().unwrap().take();
                let mut state = self.state.lock().unwrap();
                if self.login_epoch.load(Ordering::SeqCst) != epoch {
                    drop(state);
                    *self.last_poll_error.lock().unwrap() = None;
                    return;
                }
                state.playback = playback;
                if let Some(devices) = devices {
                    state.devices = devices;
                }
                if previous.is_some() && state.last_error == previous {
                    state.last_error = None;
                }
            }
            Err(SpotifyError::Revoked) => self.on_revoked(),
            Err(SpotifyError::RateLimited { retry_after_secs }) => {
                self.backoff_until_ms
                    .store(now + retry_after_secs * 1000, Ordering::SeqCst);
            }
            Err(e) => {
                let text = e.to_string();
                let changed = {
                    let mut last = self.last_poll_error.lock().unwrap();
                    if last.as_deref() != Some(text.as_str()) {
                        *last = Some(text.clone());
                        true
                    } else {
                        false
                    }
                };
                if changed {
                    tracing::warn!(error = %text, "Spotify poll failed");
                    self.state.lock().unwrap().last_error = Some(text);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicUsize;
    use std::time::Instant;

    use super::*;
    use crate::http::test_server::{Response, Seen, serve};
    use crate::token::{self, Token};

    const CLIENT_ID: &str = "0123456789abcdef0123456789abcdef";

    #[test]
    fn polls_fast_while_watched_and_not_at_all_when_logged_out() {
        assert_eq!(poll_interval(false, 0, 0), None);
        assert_eq!(poll_interval(true, 59_999, 0), Some(FAST_POLL));
        assert_eq!(poll_interval(true, 60_000, 0), Some(SLOW_POLL));
    }

    struct Fixture {
        service: SpotifyService,
        seen: Seen,
        starts: Arc<AtomicUsize>,
        _dir: tempfile::TempDir,
    }

    fn fixture<F>(logged_in: bool, device: &str, handler: F) -> Fixture
    where
        F: Fn(&crate::http::test_server::Request) -> Response + Send + 'static,
    {
        fixture_with_clock(logged_in, device, handler, system_clock())
    }

    fn fixture_with_clock<F>(logged_in: bool, device: &str, handler: F, clock: Clock) -> Fixture
    where
        F: Fn(&crate::http::test_server::Request) -> Response + Send + 'static,
    {
        let (origin, seen) = serve(handler);
        let dir = tempfile::tempdir().unwrap();
        let token_path = dir.path().join("spotify_token.json");
        if logged_in {
            let token = Token {
                access_token: "a".into(),
                refresh_token: "r".into(),
                expires_at: u64::MAX / 2,
                scope: String::new(),
            };
            token::save(&token_path, &token).unwrap();
        }
        let starts = Arc::new(AtomicUsize::new(0));
        let counter = starts.clone();
        let config = SpotifyConfig {
            client_id: CLIENT_ID.into(),
            redirect_port: free_port(),
            device: device.into(),
        };
        let options = ServiceOptions {
            token_path,
            on_start_playback: Box::new(move || {
                counter.fetch_add(1, Ordering::SeqCst);
            }),
        };
        let endpoints = Endpoints {
            api: format!("{origin}/v1"),
            accounts: origin,
        };
        let service = SpotifyService::start_with(config, options, endpoints, clock);
        Fixture {
            service,
            seen,
            starts,
            _dir: dir,
        }
    }

    fn free_port() -> u16 {
        std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port()
    }

    fn api_default(request: &crate::http::test_server::Request) -> Response {
        match request.path() {
            "/api/token" => Response::json(
                200,
                r#"{"access_token":"a","expires_in":3600,"refresh_token":"r"}"#,
            ),
            "/v1/me" => Response::json(
                200,
                r#"{"id":"u","display_name":"Eugene","product":"premium"}"#,
            ),
            "/v1/me/player" if request.method == "GET" => Response::json(
                200,
                r#"{"is_playing":true,"progress_ms":5,"currently_playing_type":"track",
                    "item":{"type":"track","uri":"spotify:track:t","name":"So What","duration_ms":9,
                            "artists":[],"album":{"name":"Kind of Blue","images":[]}}}"#,
            ),
            "/v1/me/player/devices" => Response::json(
                200,
                r#"{"devices":[{"id":"k1","name":"Kitchen","type":"Speaker","is_active":false}]}"#,
            ),
            _ => Response::empty(204),
        }
    }

    fn wait_for(service: &SpotifyService, what: impl Fn(&SpotifyState) -> bool) -> SpotifyState {
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            let state = service.state();
            if what(&state) || Instant::now() > deadline {
                return state;
            }
            thread::sleep(Duration::from_millis(20));
        }
    }

    fn requests(seen: &Seen) -> Vec<String> {
        seen.lock()
            .unwrap()
            .iter()
            .map(|r| format!("{} {}", r.method, r.target))
            .collect()
    }

    fn play() -> SpotifyOp {
        SpotifyOp::Play {
            context_uri: Some("spotify:album:a1".into()),
            uris: vec![],
            offset: None,
        }
    }

    #[test]
    fn starts_logged_out_without_a_token_and_refuses_control() {
        let f = fixture(false, "", api_default);
        assert_eq!(f.service.state().login, SpotifyLogin::NotLoggedIn);
        let error = f.service.apply(SpotifyOp::Pause).unwrap_err();
        assert_eq!(error.to_string(), "not logged in to Spotify");
    }

    #[test]
    fn play_pauses_znicz_first_and_calls_the_api() {
        let f = fixture(true, "", api_default);
        f.service.apply(play()).unwrap();
        assert_eq!(f.starts.load(Ordering::SeqCst), 1);
        assert!(requests(&f.seen).contains(&"PUT /v1/me/player/play".to_string()));
        f.service.apply(SpotifyOp::Pause).unwrap();
        assert_eq!(f.starts.load(Ordering::SeqCst), 1, "pause does not count");
    }

    #[test]
    fn no_active_device_falls_back_to_the_preferred_one() {
        let f = fixture(true, "Kitchen", |request| {
            if request.target == "/v1/me/player/play" {
                Response::json(
                    404,
                    r#"{"error":{"status":404,"message":"No active device found","reason":"NO_ACTIVE_DEVICE"}}"#,
                )
            } else {
                api_default(request)
            }
        });
        f.service.apply(play()).unwrap();
        let seen = requests(&f.seen);
        assert!(
            seen.contains(&"GET /v1/me/player/devices".to_string()),
            "{seen:?}"
        );
        assert!(
            seen.contains(&"PUT /v1/me/player/play?device_id=k1".to_string()),
            "{seen:?}"
        );
    }

    #[test]
    fn no_active_device_without_a_preference_is_reported() {
        let f = fixture(true, "", |request| {
            if request.path() == "/v1/me/player/play" {
                Response::json(404, r#"{"error":{"reason":"NO_ACTIVE_DEVICE"}}"#)
            } else {
                api_default(request)
            }
        });
        let error = f.service.apply(play()).unwrap_err();
        assert_eq!(error.to_string(), SpotifyError::NoActiveDevice.to_string());
        assert_eq!(f.service.state().last_error, Some(error.to_string()));
    }

    #[test]
    fn a_429_waits_and_retries_once() {
        let hits = Arc::new(AtomicUsize::new(0));
        let counter = hits.clone();
        let f = fixture(true, "", move |request| {
            if request.path() == "/v1/me/player/next" {
                if counter.fetch_add(1, Ordering::SeqCst) == 0 {
                    return Response::empty(429).with_header("Retry-After", "1");
                }
                return Response::empty(204);
            }
            api_default(request)
        });
        let started = Instant::now();
        f.service.apply(SpotifyOp::Next).unwrap();
        assert!(started.elapsed() >= Duration::from_millis(900));
        assert_eq!(hits.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn login_by_paste() {
        let f = fixture(false, "", api_default);
        let url = match f.service.apply(SpotifyOp::BeginLogin).unwrap() {
            SpotifyReply::AuthorizeUrl { url, .. } => url,
            other => panic!("{other:?}"),
        };
        assert_eq!(f.service.state().login, SpotifyLogin::LoggingIn);
        let state = url
            .split("state=")
            .nth(1)
            .unwrap()
            .split('&')
            .next()
            .unwrap();
        assert!(
            f.service
                .apply(SpotifyOp::CompleteLogin {
                    redirect_url: "code=x&state=nope".into()
                })
                .is_err()
        );
        assert_eq!(
            f.service.state().login,
            SpotifyLogin::LoggingIn,
            "a wrong paste keeps waiting"
        );
        f.service
            .apply(SpotifyOp::CompleteLogin {
                redirect_url: format!("code=x&state={state}"),
            })
            .unwrap();
        let state = f.service.state();
        assert_eq!(state.login, SpotifyLogin::LoggedIn);
        assert_eq!(state.account.unwrap().user, "Eugene");
        assert_eq!(state.last_error, None);
    }

    #[test]
    fn login_by_callback() {
        let f = fixture(false, "", api_default);
        let url = match f.service.apply(SpotifyOp::BeginLogin).unwrap() {
            SpotifyReply::AuthorizeUrl { url, .. } => url,
            other => panic!("{other:?}"),
        };
        let state = url
            .split("state=")
            .nth(1)
            .unwrap()
            .split('&')
            .next()
            .unwrap()
            .to_string();
        let port = f.service.redirect_port();
        let mut stream = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
        use std::io::{Read, Write};
        write!(
            stream,
            "GET /callback?code=x&state={state} HTTP/1.1\r\n\r\n"
        )
        .unwrap();
        let mut page = String::new();
        stream.read_to_string(&mut page).unwrap();
        assert!(page.contains("You can close this tab"), "{page}");
        let state = wait_for(&f.service, |s| s.login == SpotifyLogin::LoggedIn);
        assert_eq!(state.login, SpotifyLogin::LoggedIn);
    }

    #[test]
    fn touch_polls_playback_at_once() {
        let f = fixture(true, "", api_default);
        f.service.apply(SpotifyOp::Touch).unwrap();
        let state = wait_for(&f.service, |s| s.playback.is_some());
        let playback = state.playback.unwrap();
        assert!(playback.is_playing);
        assert_eq!(playback.item.unwrap().title, "So What");
        assert_eq!(state.devices.len(), 1);
    }

    #[test]
    fn pause_if_playing_only_pauses_a_playing_spotify() {
        let f = fixture(true, "", api_default);
        f.service.pause_if_playing();
        assert!(!requests(&f.seen).contains(&"PUT /v1/me/player/pause".to_string()));
        f.service.apply(SpotifyOp::Touch).unwrap();
        wait_for(&f.service, |s| s.is_playing());
        f.service.pause_if_playing();
        assert!(requests(&f.seen).contains(&"PUT /v1/me/player/pause".to_string()));
    }

    #[test]
    fn a_playback_answer_tells_whether_the_account_has_premium() {
        let f = fixture(true, "", |request| match request.path() {
            "/v1/me" => Response::json(200, r#"{"id":"u","display_name":"Eugene"}"#),
            "/v1/me/player/next" => Response::json(
                403,
                r#"{"error":{"status":403,"message":"Player command failed: Premium required","reason":"PREMIUM_REQUIRED"}}"#,
            ),
            _ => api_default(request),
        });
        f.service.apply(SpotifyOp::Touch).unwrap();
        let state = wait_for(&f.service, |s| s.account.is_some());
        assert_eq!(
            state.account.unwrap().premium,
            None,
            "unknown until a refusal"
        );
        f.service.apply(SpotifyOp::Next).unwrap_err();
        assert_eq!(f.service.state().account.unwrap().premium, Some(false));
        f.service.apply(SpotifyOp::Pause).unwrap();
        assert_eq!(f.service.state().account.unwrap().premium, Some(true));
    }

    #[test]
    fn logout_clears_the_state() {
        let f = fixture(true, "", api_default);
        f.service.apply(SpotifyOp::Touch).unwrap();
        wait_for(&f.service, |s| s.playback.is_some());
        f.service.apply(SpotifyOp::Logout).unwrap();
        let state = f.service.state();
        assert_eq!(state.login, SpotifyLogin::NotLoggedIn);
        assert_eq!(state.playback, None);
        assert_eq!(state.account, None);
    }

    #[test]
    fn configure_without_a_client_id_is_not_configured() {
        let f = fixture(true, "", api_default);
        f.service
            .apply(SpotifyOp::Configure {
                config: SpotifyConfig::default(),
            })
            .unwrap();
        assert_eq!(f.service.state().login, SpotifyLogin::NotConfigured);
        let error = f.service.apply(SpotifyOp::Devices).unwrap_err();
        assert_eq!(error.to_string(), SpotifyError::NotConfigured.to_string());
    }

    #[test]
    fn a_revoked_token_during_a_command_logs_out() {
        let f = fixture(true, "", |request| match request.path() {
            "/v1/me/player/next" => Response::empty(401),
            "/api/token" => Response::json(400, r#"{"error":"invalid_grant"}"#),
            _ => api_default(request),
        });
        f.service.apply(SpotifyOp::Next).unwrap_err();
        assert_eq!(f.service.state().login, SpotifyLogin::NotLoggedIn);
    }

    #[test]
    fn pasting_after_the_login_finished_keeps_it() {
        let f = fixture(true, "", api_default);
        assert!(
            f.service
                .apply(SpotifyOp::CompleteLogin {
                    redirect_url: "code=x&state=y".into()
                })
                .is_err()
        );
        assert_eq!(f.service.state().login, SpotifyLogin::LoggedIn);
    }

    #[test]
    fn logging_in_twice_does_not_report_a_busy_port() {
        let f = fixture(false, "", api_default);
        f.service.apply(SpotifyOp::BeginLogin).unwrap();
        f.service.apply(SpotifyOp::BeginLogin).unwrap();
        let state = f.service.state();
        let error = state.last_error.unwrap_or_default();
        assert!(!error.contains("busy"), "{error}");
    }

    #[test]
    fn begin_login_says_whether_it_listens() {
        let f = fixture(false, "", api_default);
        match f.service.apply(SpotifyOp::BeginLogin).unwrap() {
            SpotifyReply::AuthorizeUrl {
                redirect_uri,
                listening,
                ..
            } => {
                assert!(listening);
                assert!(redirect_uri.ends_with("/callback"), "{redirect_uri}");
            }
            other => panic!("{other:?}"),
        }

        let f = fixture(false, "", api_default);
        let port = f.service.redirect_port();
        let _busy = std::net::TcpListener::bind(("127.0.0.1", port)).unwrap();
        match f.service.apply(SpotifyOp::BeginLogin).unwrap() {
            SpotifyReply::AuthorizeUrl { listening, .. } => assert!(!listening),
            other => panic!("{other:?}"),
        }
        let error = f.service.state().last_error.unwrap_or_default();
        assert!(error.contains("could not listen"), "{error}");
    }

    #[test]
    fn configure_with_the_same_port_keeps_the_login_listener() {
        let f = fixture(false, "", api_default);
        let url = match f.service.apply(SpotifyOp::BeginLogin).unwrap() {
            SpotifyReply::AuthorizeUrl { url, .. } => url,
            other => panic!("{other:?}"),
        };
        let state = url
            .split("state=")
            .nth(1)
            .unwrap()
            .split('&')
            .next()
            .unwrap()
            .to_string();
        let port = f.service.redirect_port();
        f.service
            .apply(SpotifyOp::Configure {
                config: SpotifyConfig {
                    client_id: CLIENT_ID.into(),
                    redirect_port: port,
                    device: "Kitchen".into(),
                },
            })
            .unwrap();
        let mut stream = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
        use std::io::Write;
        write!(
            stream,
            "GET /callback?code=x&state={state} HTTP/1.1\r\n\r\n"
        )
        .unwrap();
        let state = wait_for(&f.service, |s| s.login == SpotifyLogin::LoggedIn);
        assert_eq!(state.login, SpotifyLogin::LoggedIn);
    }

    #[test]
    fn play_while_logged_out_does_not_pause_znicz() {
        let f = fixture(false, "", api_default);
        f.service.apply(play()).unwrap_err();
        assert_eq!(f.starts.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn pause_if_playing_after_a_revoke_logs_out() {
        let f = fixture(true, "", |request| match request.path() {
            "/v1/me/player/pause" => Response::empty(401),
            "/api/token" => Response::json(400, r#"{"error":"invalid_grant"}"#),
            _ => api_default(request),
        });
        f.service.apply(SpotifyOp::Touch).unwrap();
        wait_for(&f.service, |s| s.is_playing());
        f.service.pause_if_playing();
        assert_eq!(f.service.state().login, SpotifyLogin::NotLoggedIn);
    }

    #[test]
    fn an_abandoned_login_expires() {
        let now = Arc::new(AtomicU64::new(1_000_000));
        let clock_now = now.clone();
        let clock: Clock = Arc::new(move || clock_now.load(Ordering::SeqCst));
        let f = fixture_with_clock(false, "", api_default, clock);
        f.service.apply(SpotifyOp::BeginLogin).unwrap();
        assert_eq!(f.service.state().login, SpotifyLogin::LoggingIn);
        now.fetch_add(crate::client::LOGIN_TTL_MS + 1, Ordering::SeqCst);
        let state = wait_for(&f.service, |s| matches!(s.login, SpotifyLogin::Failed(_)));
        assert!(
            matches!(state.login, SpotifyLogin::Failed(_)),
            "{:?}",
            state.login
        );
        // The listener is dropped after the state changes; give it time to let go of the port.
        thread::sleep(Duration::from_millis(300));
        assert!(
            std::net::TcpStream::connect(("127.0.0.1", f.service.redirect_port())).is_err(),
            "the login listener still holds the port"
        );
    }

    #[test]
    fn a_failing_account_read_is_not_retried_every_poll() {
        let f = fixture(true, "", |request| match request.path() {
            "/v1/me" => Response::json(
                403,
                r#"{"error":{"status":403,"message":"not in user list"}}"#,
            ),
            _ => api_default(request),
        });
        for _ in 0..3 {
            f.service.apply(SpotifyOp::Touch).unwrap();
            thread::sleep(Duration::from_millis(1200));
        }
        let reads = requests(&f.seen)
            .iter()
            .filter(|r| r.as_str() == "GET /v1/me")
            .count();
        assert!(reads <= 2, "{reads} account reads");
    }

    #[test]
    fn play_marks_playback_playing_at_once() {
        let played = Arc::new(AtomicBool::new(false));
        let handler_played = played.clone();
        let f = fixture(true, "", move |request| match request.path() {
            "/v1/me/player/play" if request.method == "PUT" => {
                handler_played.store(true, Ordering::SeqCst);
                Response::empty(204)
            }
            "/v1/me/player" if request.method == "GET" => Response::json(
                200,
                if handler_played.load(Ordering::SeqCst) {
                    r#"{"is_playing":true,"progress_ms":5,"currently_playing_type":"track",
                    "item":{"type":"track","uri":"spotify:track:t","name":"So What","duration_ms":9,
                            "artists":[],"album":{"name":"Kind of Blue","images":[]}}}"#
                } else {
                    r#"{"is_playing":false,"progress_ms":5,"currently_playing_type":"track",
                    "item":{"type":"track","uri":"spotify:track:t","name":"So What","duration_ms":9,
                            "artists":[],"album":{"name":"Kind of Blue","images":[]}}}"#
                },
            ),
            _ => api_default(request),
        });
        f.service.apply(SpotifyOp::Touch).unwrap();
        wait_for(&f.service, |s| s.playback.is_some());
        assert!(!f.service.state().is_playing());
        f.service.apply(play()).unwrap();
        assert!(f.service.state().is_playing());
    }
}
