//! The Spotify service the player process hosts: a worker thread that polls
//! playback, and `SpotifyHost::apply` for every op. One `Client` behind a
//! mutex; one `SpotifyState` snapshot that front ends read.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use znicz_core::{SpotifyConfig, SpotifyHost, SpotifyLogin, SpotifyOp, SpotifyReply, SpotifyState};

use self::control::{TRANSFER_BUDGET, TRANSFER_CHECK_EVERY};
use self::poll::PollState;
use crate::api::Api;
use crate::client::{Client, Clock, Endpoints, system_clock};
use crate::error::{Result, SpotifyError};
use crate::listener::CallbackListener;

mod control;
mod login;
mod poll;

#[cfg(test)]
mod tests;

pub struct ServiceOptions {
    pub token_path: PathBuf,
    /// Called inside `SpotifyHost::apply`, on the caller's thread, once Spotify
    /// has accepted a playback op that starts playback (the service was logged
    /// in when the op began), so the player can pause its own engine. A refused
    /// command does not call it, so Znicz keeps playing.
    ///
    /// It must not wait for a lock the caller of `apply` holds: the IPC server
    /// holds one per change request. Capture a cheap handle to the player (a
    /// clone of its `PlayerHandle`), not an `Arc` that keeps the service and
    /// the player alive together.
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
    poll: PollState,
    wake: (Mutex<bool>, Condvar),
    stop: AtomicBool,
    on_start_playback: Box<dyn Fn() + Send + Sync>,
    /// How long, and how far apart, a transfer reads playback for its device.
    transfer_check: (Duration, Duration),
}

pub struct SpotifyService {
    shared: Arc<Shared>,
    worker: Option<JoinHandle<()>>,
}

impl SpotifyService {
    pub fn start(config: SpotifyConfig, options: ServiceOptions) -> Self {
        Self::start_with(
            config,
            options,
            Endpoints::default(),
            system_clock(),
            (TRANSFER_BUDGET, TRANSFER_CHECK_EVERY),
        )
    }

    pub(crate) fn start_with(
        config: SpotifyConfig,
        options: ServiceOptions,
        endpoints: Endpoints,
        clock: Clock,
        transfer_check: (Duration, Duration),
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
            poll: PollState::default(),
            wake: (Mutex::new(false), Condvar::new()),
            stop: AtomicBool::new(false),
            on_start_playback: options.on_start_playback,
            transfer_check,
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

    /// Ends the data of the login that is running: the epoch moves on, so reads
    /// still in flight are dropped, and what was read for the login is cleared.
    /// `state` is the already locked state.
    fn end_login(&self, state: &mut SpotifyState, login: SpotifyLogin) {
        self.login_epoch.fetch_add(1, Ordering::SeqCst);
        state.account = None;
        state.playback = None;
        state.devices.clear();
        state.login = login;
    }

    fn set_login(&self, login: SpotifyLogin) {
        let mut state = self.state.lock().unwrap();
        if login == SpotifyLogin::LoggedIn {
            state.login = login;
        } else {
            self.end_login(&mut state, login);
        }
    }

    /// Runs `f` on the state only while the login epoch is still `epoch`, so a
    /// read that began before a logout cannot land after it. Returns whether it ran.
    fn update_if_current(&self, epoch: u64, f: impl FnOnce(&mut SpotifyState)) -> bool {
        let mut state = self.state.lock().unwrap();
        let current = self.login_epoch.load(Ordering::SeqCst) == epoch;
        if current {
            f(&mut state);
        }
        current
    }

    /// The refresh token was revoked: `Client` has removed the token.
    fn on_revoked(&self) {
        self.set_login(SpotifyLogin::NotLoggedIn);
        self.state.lock().unwrap().last_error = Some(SpotifyError::Revoked.to_string());
    }

    fn apply(self: &Arc<Self>, op: SpotifyOp) -> Result<SpotifyReply> {
        self.touch();
        match op {
            SpotifyOp::Touch => {
                self.wake();
                Ok(SpotifyReply::Done)
            }
            SpotifyOp::Configure {
                config,
                replace_client_id,
            } => {
                let (login, pending) = {
                    let mut client = self.client.lock().unwrap();
                    let current = client.config().client_id.clone();
                    if !current.is_empty() && config.client_id != current && !replace_client_id {
                        return Err(SpotifyError::ClientIdDiffers);
                    }
                    client.set_config(config, replace_client_id);
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
            SpotifyOp::CancelLogin => self.cancel_login(),
            SpotifyOp::Logout => self.logout(),
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
                self.update_if_current(epoch, |state| state.devices = devices.clone());
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

    fn refresh_account(&self) {
        let epoch = self.login_epoch.load(Ordering::SeqCst);
        let result = self.client.lock().unwrap().call(|api| api.me());
        match result {
            // A logout during the read must not be undone by it.
            Ok(account) => {
                self.update_if_current(epoch, |state| state.account = Some(account));
            }
            Err(SpotifyError::Revoked) => self.on_revoked(),
            Err(e) => tracing::debug!(error = %e, "could not read the Spotify account"),
        }
    }
}
