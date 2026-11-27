//! Playback commands: the device they go to, the preferred device fallback,
//! the pause of Znicz, the transfer check and the optimistic update.

use std::sync::atomic::Ordering;
use std::thread;
use std::time::{Duration, Instant};

use znicz_core::SpotifyOp;

use super::{Shared, SpotifyService};
use crate::error::{Result, SpotifyError};

/// A user action waits for a 429 at most this long before retrying once.
const MAX_RETRY_WAIT_SECS: u64 = 10;
/// After a transfer, Spotify is read for up to this long to see the device take over...
pub(super) const TRANSFER_BUDGET: Duration = Duration::from_secs(5);
/// ...this far apart.
pub(super) const TRANSFER_CHECK_EVERY: Duration = Duration::from_millis(500);

impl SpotifyService {
    /// Pause Spotify if it is playing: Znicz is about to play.
    ///
    /// Reads Spotify's playback first (one Web API request), so the decision
    /// does not depend on the last poll, which may be up to 15 s old. If that
    /// read fails for another reason, the last polled snapshot decides.
    ///
    /// Blocks for a whole Web API request (up to about 40 s: a refresh, a retry
    /// and a rate-limit wait). Never call it while holding the player's lock;
    /// call it on its own thread.
    pub fn pause_if_playing(&self) {
        let shared = &self.shared;
        let logged_in = shared.client.lock().unwrap().is_logged_in();
        if !logged_in {
            return;
        }
        let epoch = shared.login_epoch.load(Ordering::SeqCst);
        let now = shared.now_ms();
        let fresh = shared.client.lock().unwrap().call(|api| api.playback(now));
        let playing = match fresh {
            Ok(playback) => {
                let playing = playback.as_ref().is_some_and(|p| p.is_playing);
                shared.update_if_current(epoch, |state| state.playback = playback);
                playing
            }
            Err(SpotifyError::Revoked) => {
                shared.on_revoked();
                return;
            }
            Err(_) => shared.state.lock().unwrap().is_playing(),
        };
        if !playing {
            return;
        }
        if let Err(e) = shared.control(&SpotifyOp::Pause) {
            tracing::warn!(error = %e, "could not pause Spotify");
        }
    }
}

impl Shared {
    /// A playback command: fall back to the preferred device when none is
    /// active, wait out one 429, and pause Znicz once Spotify accepts a command
    /// that starts playback.
    pub(super) fn control(&self, op: &SpotifyOp) -> Result<()> {
        // Only playback that can run pauses Znicz; a logged-out play fails below.
        let starts_here = op.starts_playback() && self.client.lock().unwrap().is_logged_in();
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
        // Spotify accepts a transfer to a stale device too, so check that it took over.
        let result = match (result, op) {
            (Ok(()), SpotifyOp::Transfer { device_id, .. }) => self.verify_transfer(device_id),
            (result, _) => result,
        };
        // Spotify no longer reports the plan, so a playback answer is how we learn it.
        match &result {
            Ok(()) => {
                self.set_premium(true);
                if starts_here {
                    (self.on_start_playback)();
                }
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

    /// Read playback until `device_id` is the playing device, within the
    /// transfer check's time budget. A failed read is retried; a rate limit or
    /// a revoked login ends the check at once. If no read succeeded, the last
    /// read error is returned, since nothing was learned about the device.
    /// No lock is held while waiting.
    fn verify_transfer(&self, device_id: &str) -> Result<()> {
        let (budget, every) = self.transfer_check;
        let started = Instant::now();
        let mut read_ok = false;
        let mut last_error = None;
        let mut attempt = 0u32;
        loop {
            if attempt > 0 {
                if started.elapsed() >= budget {
                    break;
                }
                thread::sleep(every);
            }
            attempt += 1;
            let now = self.now_ms();
            let epoch = self.login_epoch.load(Ordering::SeqCst);
            let read = self.client.lock().unwrap().call(|api| api.playback(now));
            match read {
                Ok(playback) => {
                    read_ok = true;
                    let took = playback
                        .as_ref()
                        .and_then(|p| p.device.as_ref())
                        .is_some_and(|d| d.id == device_id);
                    if took {
                        self.update_if_current(epoch, |state| state.playback = playback);
                        return Ok(());
                    }
                }
                Err(e @ (SpotifyError::Revoked | SpotifyError::RateLimited { .. })) => {
                    return Err(e);
                }
                Err(e) => last_error = Some(e),
            }
        }
        if !read_ok {
            if let Some(e) = last_error {
                return Err(e);
            }
        }
        let name = self
            .state
            .lock()
            .unwrap()
            .devices
            .iter()
            .find(|d| d.id == device_id)
            .map(|d| d.name.clone())
            .unwrap_or_else(|| "the device".into());
        Err(SpotifyError::Unavailable(format!(
            "Spotify accepted the move, but {name} did not take over; it may be asleep, or its Spotify app may need a restart"
        )))
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
        self.update_if_current(epoch, |state| state.devices = devices);
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
}
