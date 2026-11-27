//! Logging in: the browser URL, the callback listener, the paste path, and
//! how a login ends (finished, cancelled, logged out or expired).

use std::sync::{Arc, Weak};

use znicz_core::{SpotifyLogin, SpotifyReply};

use super::{Shared, login_of};
use crate::error::{Result, SpotifyError};
use crate::listener::CallbackListener;

impl Shared {
    pub(super) fn begin_login(self: &Arc<Self>) -> Result<SpotifyReply> {
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
    pub(super) fn finish_login(&self, input: &str) -> Result<()> {
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

    /// A login whose callback never came ends once its TTL has passed.
    pub(super) fn expire_login(&self) {
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
                self.end_login(
                    &mut state,
                    SpotifyLogin::Failed(SpotifyError::LoginExpired.to_string()),
                );
                true
            }
        };
        if expired {
            self.listener.lock().unwrap().take();
        }
    }

    /// Cancels the login in progress: its listener goes, and no login is shown.
    pub(super) fn cancel_login(&self) -> Result<SpotifyReply> {
        let login = {
            let mut client = self.client.lock().unwrap();
            client.cancel_login();
            login_of(&client)
        };
        self.listener.lock().unwrap().take();
        self.set_login(login);
        Ok(SpotifyReply::Done)
    }

    /// Forgets the token, here and on disk, and any login in progress.
    pub(super) fn logout(&self) -> Result<SpotifyReply> {
        let result = self.client.lock().unwrap().logout();
        self.listener.lock().unwrap().take();
        let login = login_of(&self.client.lock().unwrap());
        self.set_login(login);
        result.map(|()| SpotifyReply::Done)
    }
}
