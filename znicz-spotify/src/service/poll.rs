//! The background poll: when Spotify is read, the backoff after a 429, and
//! the worker loop.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use znicz_core::SpotifyLogin;

use super::Shared;
use crate::error::SpotifyError;

/// Poll every second while someone is watching...
pub(super) const FAST_POLL: Duration = Duration::from_secs(1);
/// ...and every 15 s otherwise.
pub(super) const SLOW_POLL: Duration = Duration::from_secs(15);
/// "Watching": a `Touch` or any op within this window.
const ACTIVE_WINDOW_MS: u64 = 60_000;
/// Devices are re-read at most this often.
const DEVICES_EVERY_MS: u64 = 15_000;
/// While a login waits for its callback, it is checked this often.
const LOGIN_CHECK: Duration = Duration::from_secs(1);

/// How long to wait before the next poll. `None`: not logged in, wait for a wake.
pub(super) fn poll_interval(logged_in: bool, now_ms: u64, last_touch_ms: u64) -> Option<Duration> {
    if !logged_in {
        return None;
    }
    if now_ms.saturating_sub(last_touch_ms) < ACTIVE_WINDOW_MS {
        Some(FAST_POLL)
    } else {
        Some(SLOW_POLL)
    }
}

/// What the poll keeps between polls. The account and devices reads, the
/// backoff after a 429 and the last error shown are all poll-only.
#[derive(Default)]
pub(super) struct PollState {
    /// Devices are re-read at most every `DEVICES_EVERY_MS`.
    pub(super) last_devices_ms: AtomicU64,
    /// When the account was last read: a failed read is not retried every poll.
    pub(super) last_account_ms: AtomicU64,
    /// No polls before this time (after a 429).
    pub(super) backoff_until_ms: AtomicU64,
    /// The last poll error shown, so the same one is not repeated every second.
    pub(super) last_error: Mutex<Option<String>>,
}

impl Shared {
    pub(super) fn run(self: Arc<Self>) {
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
        if now < self.poll.backoff_until_ms.load(Ordering::SeqCst) {
            return;
        }
        let epoch = self.login_epoch.load(Ordering::SeqCst);
        let need_account = self.state.lock().unwrap().account.is_none();
        let account_due = now.saturating_sub(self.poll.last_account_ms.load(Ordering::SeqCst))
            >= DEVICES_EVERY_MS;
        if need_account && account_due {
            self.poll.last_account_ms.store(now, Ordering::SeqCst);
            self.refresh_account();
        }
        if self.stop.load(Ordering::SeqCst) {
            return;
        }
        let result = self.client.lock().unwrap().call(|api| api.playback(now));
        let result = result.and_then(|playback| {
            let due = now.saturating_sub(self.poll.last_devices_ms.load(Ordering::SeqCst))
                >= DEVICES_EVERY_MS;
            let devices = if due && !self.stop.load(Ordering::SeqCst) {
                self.poll.last_devices_ms.store(now, Ordering::SeqCst);
                Some(self.client.lock().unwrap().call(|api| api.devices())?)
            } else {
                None
            };
            Ok((playback, devices))
        });
        if self.login_epoch.load(Ordering::SeqCst) != epoch {
            *self.poll.last_error.lock().unwrap() = None;
            return;
        }
        match result {
            Ok((playback, devices)) => {
                // Never hold `poll.last_error` and `state` at once.
                let previous = self.poll.last_error.lock().unwrap().take();
                let current = self.update_if_current(epoch, |state| {
                    state.playback = playback;
                    if let Some(devices) = devices {
                        state.devices = devices;
                    }
                    if previous.is_some() && state.last_error == previous {
                        state.last_error = None;
                    }
                });
                if !current {
                    *self.poll.last_error.lock().unwrap() = None;
                }
            }
            Err(SpotifyError::Revoked) => self.on_revoked(),
            Err(SpotifyError::RateLimited { retry_after_secs }) => {
                let until = now.saturating_add(retry_after_secs.saturating_mul(1000));
                self.poll.backoff_until_ms.store(until, Ordering::SeqCst);
            }
            Err(e) => {
                let text = e.to_string();
                let changed = {
                    let mut last = self.poll.last_error.lock().unwrap();
                    if last.as_deref() != Some(text.as_str()) {
                        *last = Some(text.clone());
                        true
                    } else {
                        false
                    }
                };
                if changed {
                    tracing::warn!(error = %text, "Spotify poll failed");
                    self.update_if_current(epoch, |state| state.last_error = Some(text));
                }
            }
        }
    }
}
