//! Holds what a logged-in session needs: the settings, the token (in memory
//! and on disk), and a login in progress. Every API call goes through `call`,
//! which refreshes the token before it expires and once more on a 401.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use znicz_core::SpotifyConfig;

use crate::api::Api;
use crate::auth;
use crate::error::{Result, SpotifyError};
use crate::http::{ACCOUNTS_BASE, API_BASE, Http};
use crate::pkce;
use crate::redirect;
use crate::token::{self, Token};

/// How long a started login waits for its redirect.
pub(crate) const LOGIN_TTL_MS: u64 = 10 * 60 * 1000;

/// Unix time in milliseconds. Tests inject their own.
pub(crate) type Clock = Arc<dyn Fn() -> u64 + Send + Sync>;

pub(crate) fn system_clock() -> Clock {
    Arc::new(|| {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0)
    })
}

#[derive(Debug, Clone)]
pub(crate) struct Endpoints {
    pub api: String,
    pub accounts: String,
}

impl Default for Endpoints {
    fn default() -> Self {
        Self {
            api: API_BASE.into(),
            accounts: ACCOUNTS_BASE.into(),
        }
    }
}

/// A login that waits for its redirect.
struct LoginSession {
    verifier: String,
    state: String,
    started_ms: u64,
}

/// What `begin_login` hands back.
#[derive(Debug)]
pub(crate) struct BegunLogin {
    pub url: String,
}

pub(crate) struct Client {
    config: SpotifyConfig,
    token_path: PathBuf,
    endpoints: Endpoints,
    http: Http,
    clock: Clock,
    token: Option<Token>,
    login: Option<LoginSession>,
}

impl Client {
    pub(crate) fn new(
        config: SpotifyConfig,
        token_path: PathBuf,
        endpoints: Endpoints,
        clock: Clock,
    ) -> Self {
        let token = load_token_for(&token_path, &config.client_id);
        Self {
            config,
            token_path,
            endpoints,
            http: Http::new(),
            clock,
            token,
            login: None,
        }
    }

    pub(crate) fn now_ms(&self) -> u64 {
        (self.clock)()
    }

    pub(crate) fn config(&self) -> &SpotifyConfig {
        &self.config
    }

    /// A different client ID cannot use the old token. With `replace` the
    /// token is deleted (the user confirmed the switch). Without it nothing is
    /// deleted: the token stays on disk for a later switch back, and this app
    /// is simply not logged in.
    pub(crate) fn set_config(&mut self, config: SpotifyConfig, replace: bool) {
        if config.client_id != self.config.client_id {
            if replace {
                if let Err(e) = self.logout() {
                    tracing::warn!(error = %e, "could not remove the Spotify token");
                }
            } else {
                self.login = None;
                let kept = self
                    .token
                    .take()
                    .filter(|token| token.belongs_to(&config.client_id));
                self.token = kept.or_else(|| load_token_for(&self.token_path, &config.client_id));
            }
        }
        if config.redirect_port != self.config.redirect_port {
            self.login = None;
        }
        self.config = config;
    }

    pub(crate) fn is_logged_in(&self) -> bool {
        self.config.is_configured()
            && self
                .token
                .as_ref()
                .is_some_and(|token| token.belongs_to(&self.config.client_id))
    }

    pub(crate) fn login_pending(&self) -> bool {
        self.login
            .as_ref()
            .is_some_and(|login| self.now_ms().saturating_sub(login.started_ms) <= LOGIN_TTL_MS)
    }

    /// True only when a login is still held and has outlived its TTL. Unlike
    /// `!login_pending()`, a login that `complete_login` already took is not expired.
    pub(crate) fn login_expired(&self) -> bool {
        self.login
            .as_ref()
            .is_some_and(|login| self.now_ms().saturating_sub(login.started_ms) > LOGIN_TTL_MS)
    }

    pub(crate) fn begin_login(&mut self) -> Result<BegunLogin> {
        if !self.config.is_configured() {
            return Err(SpotifyError::NotConfigured);
        }
        let verifier = pkce::random_verifier();
        let state = pkce::random_state();
        let url = auth::authorize_url(
            &self.endpoints.accounts,
            &self.config.client_id,
            &self.config.redirect_uri(),
            &pkce::challenge(&verifier),
            &state,
        );
        self.login = Some(LoginSession {
            verifier,
            state: state.clone(),
            started_ms: self.now_ms(),
        });
        Ok(BegunLogin { url })
    }

    /// Finish the login from a pasted address or a callback query. A wrong
    /// address keeps the login open; any other outcome ends it.
    pub(crate) fn complete_login(&mut self, input: &str) -> Result<()> {
        let login = self.login.as_ref().ok_or(SpotifyError::NoLogin)?;
        if self.now_ms().saturating_sub(login.started_ms) > LOGIN_TTL_MS {
            self.login = None;
            return Err(SpotifyError::LoginExpired);
        }
        let code = match redirect::code_from(input, &login.state) {
            Err(SpotifyError::WrongAddress) => return Err(SpotifyError::WrongAddress),
            other => other,
        };
        let login = self.login.take().expect("checked above");
        let mut token = auth::exchange_code(
            &self.http,
            &self.endpoints.accounts,
            &self.config.client_id,
            &code?,
            &login.verifier,
            &self.config.redirect_uri(),
            self.now_ms() / 1000,
        )?;
        token.client_id = self.config.client_id.clone();
        if let Err(e) = token::save(&self.token_path, &token) {
            tracing::warn!(error = %e, "could not save the Spotify token");
        }
        self.token = Some(token);
        Ok(())
    }

    pub(crate) fn cancel_login(&mut self) {
        self.login = None;
    }

    pub(crate) fn logout(&mut self) -> Result<()> {
        self.token = None;
        self.login = None;
        token::remove(&self.token_path)
    }

    /// Run `f` with a fresh access token: refresh first when it is about to
    /// expire, and refresh once more and retry when the API answers 401.
    pub(crate) fn call<T>(&mut self, f: impl Fn(&Api<'_>) -> Result<T>) -> Result<T> {
        if !self.config.is_configured() {
            return Err(SpotifyError::NotConfigured);
        }
        let expired = match &self.token {
            None => return Err(SpotifyError::NotLoggedIn),
            Some(token) => token.needs_refresh(self.now_ms() / 1000),
        };
        if expired {
            self.refresh()?;
        }
        match self.run(&f) {
            Err(SpotifyError::Unauthorized) => {
                self.refresh()?;
                self.run(&f)
            }
            result => result,
        }
    }

    fn run<T>(&self, f: &impl Fn(&Api<'_>) -> Result<T>) -> Result<T> {
        let token = self.token.as_ref().ok_or(SpotifyError::NotLoggedIn)?;
        let api = Api {
            http: &self.http,
            base: &self.endpoints.api,
            token: &token.access_token,
        };
        f(&api)
    }

    fn refresh(&mut self) -> Result<()> {
        let (refresh_token, client_id) = match &self.token {
            Some(token) => (token.refresh_token.clone(), token.client_id.clone()),
            None => return Err(SpotifyError::NotLoggedIn),
        };
        match auth::refresh(
            &self.http,
            &self.endpoints.accounts,
            &self.config.client_id,
            &refresh_token,
            self.now_ms() / 1000,
        ) {
            Ok(mut token) => {
                token.client_id = client_id;
                // Keep the token in memory first: Spotify may have rotated the
                // refresh token, so a failed save must not drop it.
                self.token = Some(token);
                if let Some(token) = &self.token {
                    if let Err(e) = token::save(&self.token_path, token) {
                        tracing::warn!(error = %e, "could not save the refreshed Spotify token");
                    }
                }
                Ok(())
            }
            Err(SpotifyError::Revoked) => {
                if let Err(e) = self.logout() {
                    tracing::warn!(error = %e, "could not remove the Spotify token");
                }
                Err(SpotifyError::Revoked)
            }
            Err(e) => Err(e),
        }
    }
}

/// The saved token, when it exists, reads, and belongs to `client_id`. A token
/// for another ID stays on disk, but it is not loaded as this app's login.
fn load_token_for(path: &std::path::Path, client_id: &str) -> Option<Token> {
    token::load(path)
        .unwrap_or_else(|e| {
            tracing::warn!(error = %e, "could not read the Spotify token");
            None
        })
        .filter(|token| token.belongs_to(client_id))
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

    use super::*;
    use crate::http::test_server::{Response, serve};

    const CLIENT_ID: &str = "0123456789abcdef0123456789abcdef";

    struct Fixture {
        client: Client,
        now: Arc<AtomicU64>,
        token_path: PathBuf,
        _dir: tempfile::TempDir,
    }

    fn fixture(origin: &str, logged_in: Option<Token>) -> Fixture {
        fixture_with_config_id(origin, logged_in, CLIENT_ID)
    }

    fn fixture_with_config_id(origin: &str, logged_in: Option<Token>, client_id: &str) -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let token_path = dir.path().join("spotify_token.json");
        if let Some(token) = &logged_in {
            token::save(&token_path, token).unwrap();
        }
        let now = Arc::new(AtomicU64::new(1_000_000));
        let clock_now = now.clone();
        let clock: Clock = Arc::new(move || clock_now.load(Ordering::SeqCst));
        let config = SpotifyConfig {
            client_id: client_id.into(),
            ..SpotifyConfig::default()
        };
        let endpoints = Endpoints {
            api: format!("{origin}/v1"),
            accounts: origin.to_string(),
        };
        Fixture {
            client: Client::new(config, token_path.clone(), endpoints, clock),
            now,
            token_path,
            _dir: dir,
        }
    }

    fn token(expires_at: u64) -> Token {
        Token {
            access_token: "old".into(),
            refresh_token: "r".into(),
            expires_at,
            scope: String::new(),
            client_id: CLIENT_ID.into(),
        }
    }

    const OTHER_ID: &str = "fedcba9876543210fedcba9876543210";

    fn token_for(client_id: &str) -> Token {
        Token {
            client_id: client_id.into(),
            ..token(999_999)
        }
    }

    fn config_for(client_id: &str) -> SpotifyConfig {
        SpotifyConfig {
            client_id: client_id.into(),
            ..SpotifyConfig::default()
        }
    }

    fn token_reply() -> Response {
        Response::json(
            200,
            r#"{"access_token":"new","expires_in":3600,"refresh_token":"r2"}"#,
        )
    }

    fn state_of(url: &str) -> String {
        url.split("state=")
            .nth(1)
            .unwrap()
            .split('&')
            .next()
            .unwrap()
            .to_string()
    }

    #[test]
    fn login_needs_a_client_id() {
        let mut f = fixture("http://127.0.0.1:1", None);
        f.client.set_config(SpotifyConfig::default(), false);
        assert_eq!(
            f.client.begin_login().unwrap_err(),
            SpotifyError::NotConfigured
        );
    }

    #[test]
    fn a_pasted_address_finishes_the_login_and_saves_the_token() {
        let (origin, _) = serve(|_| token_reply());
        let mut f = fixture(&origin, None);
        assert!(!f.client.is_logged_in());
        let login = f.client.begin_login().unwrap();
        assert!(login.url.contains(&format!("client_id={CLIENT_ID}")));
        assert!(f.client.login_pending());
        let pasted = format!(
            "http://127.0.0.1:8898/callback?code=c&state={}",
            state_of(&login.url)
        );
        f.client.complete_login(&pasted).unwrap();
        assert!(f.client.is_logged_in());
        assert!(!f.client.login_pending());
        let saved = token::load(&f.token_path).unwrap().unwrap();
        assert_eq!(saved.access_token, "new");
        assert_eq!(saved.client_id, CLIENT_ID);
    }

    #[test]
    fn a_wrong_address_keeps_the_login_open() {
        let mut f = fixture("http://127.0.0.1:1", None);
        f.client.begin_login().unwrap();
        assert_eq!(
            f.client.complete_login("code=c&state=other").unwrap_err(),
            SpotifyError::WrongAddress
        );
        assert!(f.client.login_pending());
    }

    #[test]
    fn a_login_expires_after_ten_minutes() {
        let mut f = fixture("http://127.0.0.1:1", None);
        let login = f.client.begin_login().unwrap();
        f.now.fetch_add(10 * 60 * 1000 + 1, Ordering::SeqCst);
        assert!(!f.client.login_pending());
        let pasted = format!("code=c&state={}", state_of(&login.url));
        assert_eq!(
            f.client.complete_login(&pasted).unwrap_err(),
            SpotifyError::LoginExpired
        );
        assert_eq!(
            f.client.complete_login(&pasted).unwrap_err(),
            SpotifyError::NoLogin
        );
    }

    #[test]
    fn a_login_is_expired_only_past_its_ttl_and_while_held() {
        let mut f = fixture("http://127.0.0.1:1", None);
        f.client.begin_login().unwrap();
        assert!(!f.client.login_expired());
        f.now.fetch_add(LOGIN_TTL_MS + 1, Ordering::SeqCst);
        assert!(f.client.login_expired());
        f.client.cancel_login();
        assert!(!f.client.login_expired());
    }

    #[test]
    fn refreshes_before_the_token_expires() {
        let posts = Arc::new(AtomicUsize::new(0));
        let counter = posts.clone();
        let (origin, seen) = serve(move |request| {
            if request.path() == "/api/token" {
                counter.fetch_add(1, Ordering::SeqCst);
                token_reply()
            } else {
                Response::json(200, r#"{"id":"u","product":"premium"}"#)
            }
        });
        // 1_000_000 ms = 1_000 s; the token expires 30 s later, inside the margin.
        let mut f = fixture(&origin, Some(token(1_030)));
        f.client.call(|api| api.me()).unwrap();
        assert_eq!(posts.load(Ordering::SeqCst), 1);
        let me = seen
            .lock()
            .unwrap()
            .iter()
            .find(|r| r.path() == "/v1/me")
            .cloned()
            .unwrap();
        assert_eq!(me.header("authorization"), Some("Bearer new"));
        // A refresh keeps the client ID the token was issued for.
        let saved = token::load(&f.token_path).unwrap().unwrap();
        assert_eq!(saved.client_id, CLIENT_ID);
    }

    #[test]
    fn a_401_refreshes_once_and_retries() {
        let calls = Arc::new(Mutex::new(Vec::<String>::new()));
        let log = calls.clone();
        let (origin, _) = serve(move |request| {
            let auth = request.header("authorization").unwrap_or("").to_string();
            log.lock()
                .unwrap()
                .push(format!("{} {auth}", request.path()));
            match (request.path(), auth.as_str()) {
                ("/api/token", _) => token_reply(),
                ("/v1/me", "Bearer old") => Response::empty(401),
                _ => Response::json(200, r#"{"id":"u"}"#),
            }
        });
        let mut f = fixture(&origin, Some(token(999_999)));
        f.client.call(|api| api.me()).unwrap();
        assert_eq!(
            *calls.lock().unwrap(),
            vec!["/v1/me Bearer old", "/api/token ", "/v1/me Bearer new"]
        );
    }

    #[test]
    fn a_revoked_refresh_logs_out() {
        let (origin, _) = serve(|request| {
            if request.path() == "/api/token" {
                Response::json(400, r#"{"error":"invalid_grant"}"#)
            } else {
                Response::empty(401)
            }
        });
        let mut f = fixture(&origin, Some(token(999_999)));
        assert_eq!(
            f.client.call(|api| api.me()).unwrap_err(),
            SpotifyError::Revoked
        );
        assert!(!f.client.is_logged_in());
        assert!(!f.token_path.exists());
    }

    #[test]
    fn calls_need_a_login() {
        let mut f = fixture("http://127.0.0.1:1", None);
        assert_eq!(
            f.client.call(|api| api.me()).unwrap_err(),
            SpotifyError::NotLoggedIn
        );
    }

    #[test]
    fn a_new_client_id_logs_out() {
        let mut f = fixture("http://127.0.0.1:1", Some(token(999_999)));
        assert!(f.client.is_logged_in());
        f.client.set_config(config_for(OTHER_ID), true);
        assert!(!f.client.is_logged_in());
        assert!(!f.token_path.exists());
    }

    #[test]
    fn a_legacy_token_stays_logged_in_when_the_player_gets_its_first_id() {
        let legacy = Token {
            client_id: String::new(),
            ..token(999_999)
        };
        let mut f = fixture_with_config_id("http://127.0.0.1:1", Some(legacy), "");
        assert!(!f.client.is_logged_in());
        f.client.set_config(config_for(CLIENT_ID), false);
        assert!(f.client.is_logged_in());
        assert!(f.token_path.exists());
    }

    #[test]
    fn a_token_for_another_id_is_kept_on_disk_and_not_logged_in() {
        let mut f = fixture("http://127.0.0.1:1", Some(token_for(CLIENT_ID)));
        f.client.set_config(config_for(OTHER_ID), false);
        assert!(!f.client.is_logged_in());
        assert!(f.token_path.exists());
        f.client.set_config(config_for(CLIENT_ID), false);
        assert!(f.client.is_logged_in());
        assert!(f.token_path.exists());
    }

    #[test]
    fn replacing_the_id_deletes_the_token_of_the_old_one() {
        let mut f = fixture("http://127.0.0.1:1", Some(token_for(CLIENT_ID)));
        f.client.set_config(config_for(OTHER_ID), true);
        assert!(!f.client.is_logged_in());
        assert!(!f.token_path.exists());
    }

    #[test]
    fn a_token_for_another_id_loads_but_does_not_count_as_logged_in() {
        let f = fixture_with_config_id("http://127.0.0.1:1", Some(token_for(OTHER_ID)), CLIENT_ID);
        assert!(!f.client.is_logged_in());
        assert!(f.token_path.exists());
    }

    #[test]
    fn a_refresh_that_cannot_be_saved_still_keeps_the_new_token() {
        let (origin, _) = serve(|request| {
            let auth = request.header("authorization").unwrap_or("").to_string();
            match (request.path(), auth.as_str()) {
                ("/api/token", _) => token_reply(),
                ("/v1/me", "Bearer old") => Response::empty(401),
                _ => Response::json(200, r#"{"id":"u"}"#),
            }
        });
        let mut f = fixture(&origin, Some(token(999_999)));
        // A directory at the token path makes the save fail.
        std::fs::remove_file(&f.token_path).unwrap();
        std::fs::create_dir(&f.token_path).unwrap();
        f.client.call(|api| api.me()).unwrap();
        assert!(f.client.is_logged_in());
    }

    #[test]
    fn logout_removes_the_token() {
        let mut f = fixture("http://127.0.0.1:1", Some(token(999_999)));
        f.client.logout().unwrap();
        assert!(!f.client.is_logged_in());
        assert!(!f.token_path.exists());
    }
}
