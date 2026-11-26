//! `znicz spotify login | logout | status`. Each command autostarts the
//! player like the other CLI commands, then talks to it over IPC. Only
//! `login` sends the `[spotify]` settings first; `status` and `logout` never
//! change them. The decisions and the wording are pure functions here; `run`
//! and `login` are the thin IO wrappers.

use std::io::BufRead;
use std::path::Path;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use clap::Subcommand;
use color_eyre::eyre::{bail, eyre};
use znicz_core::{
    IpcClient, SpotifyConfig, SpotifyErrorKind, SpotifyHost, SpotifyLogin, SpotifyOp, SpotifyReply,
    SpotifyState, ZniczError, valid_client_id,
};
use znicz_spotify::browser::{self, OpenBrowser};
use znicz_tui::ConfigProblem;

/// How long the login waits for the account name after `LoggedIn`. The
/// account read runs after the login is recorded, so it can lag.
pub const ACCOUNT_WAIT: Duration = Duration::from_secs(5);

/// How often the login polls the player.
const LOGIN_POLL: Duration = Duration::from_millis(500);

#[derive(Debug, Subcommand)]
pub enum SpotifyCmd {
    /// Log in to Spotify: opens a browser, or paste the address you were sent to
    Login,
    /// Log out of Spotify and delete the saved token
    Logout,
    /// Show the login, account, device and what is playing
    Status,
}

/// Whether a command sends the `[spotify]` settings to the player first. Only
/// login does: changing the client ID logs out, so `status` and `logout` must
/// never send them.
pub fn configure_before(command: &SpotifyCmd) -> bool {
    matches!(command, SpotifyCmd::Login)
}

/// Run one `znicz spotify` command against the player. `connect` autostarts
/// and connects to the player. `login` fails before anything is started or
/// sent when `config.toml` has a problem or the client ID is not usable.
/// `config_problem` is the text from `config_problem_text`, when the file
/// did not load.
pub fn run(
    command: SpotifyCmd,
    config: SpotifyConfig,
    config_problem: Option<String>,
    open: OpenBrowser,
    connect: impl FnOnce() -> color_eyre::Result<IpcClient>,
) -> color_eyre::Result<()> {
    if matches!(command, SpotifyCmd::Login) {
        if let Some(problem) = &config_problem {
            bail!("{problem}. Fix config.toml, then run `znicz spotify login` again.");
        }
        if let Err(message) = check_client_id(&config) {
            bail!("{message}");
        }
    }
    let host = connect()?;
    if configure_before(&command) {
        host.apply(SpotifyOp::Configure { config })
            .map_err(host_error)?;
    }
    match command {
        SpotifyCmd::Login => login(&host, open),
        SpotifyCmd::Logout => {
            host.apply(SpotifyOp::Logout).map_err(host_error)?;
            println!("Logged out of Spotify");
            Ok(())
        }
        SpotifyCmd::Status => {
            let state = host.try_state().map_err(host_error)?;
            for line in status_lines(&state, now_ms()) {
                println!("{line}");
            }
            Ok(())
        }
    }
}

fn login(host: &IpcClient, open: OpenBrowser) -> color_eyre::Result<()> {
    let state = host.try_state().map_err(host_error)?;
    if let Some(message) = already_logged_in_message(&state) {
        println!("{message}");
        return Ok(());
    }
    let SpotifyReply::AuthorizeUrl { url, listening, .. } =
        host.apply(SpotifyOp::BeginLogin).map_err(host_error)?
    else {
        bail!("the player sent an unexpected reply to the login");
    };
    println!("Open this URL in any browser and approve:\n{url}");
    println!("{}", login_intro(listening));
    let env = |key: &str| std::env::var(key).ok();
    if browser::should_open(open, &env) {
        if let Err(e) = browser::open(&url) {
            println!("Could not open a browser: {e}");
        }
    }

    let paste = host.clone();
    std::thread::Builder::new()
        .name("znicz-spotify-paste".into())
        .spawn(move || paste_lines(&paste))?;

    let mut logged_in_since: Option<Instant> = None;
    loop {
        let state = host.try_state().map_err(host_error)?;
        if state.login == SpotifyLogin::LoggedIn {
            logged_in_since.get_or_insert_with(Instant::now);
        }
        let waited = logged_in_since.map_or(Duration::ZERO, |since| since.elapsed());
        match login_wait(&state, waited) {
            LoginWait::Pending => std::thread::sleep(LOGIN_POLL),
            LoginWait::LoggedIn { user } => {
                println!("{}", logged_in_message(user.as_deref()));
                return Ok(());
            }
            LoginWait::Failed(reason) => bail!("{reason}"),
        }
    }
}

/// Each non-empty line on stdin is a pasted address. Errors are printed and
/// the wait goes on; a wrong address is the usual one.
fn paste_lines(host: &IpcClient) {
    for line in std::io::stdin().lock().lines() {
        let Ok(line) = line else { break };
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Err(e) = host.apply(SpotifyOp::CompleteLogin {
            redirect_url: line.to_string(),
        }) {
            println!("{e}");
        }
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// The line for a player that has no Spotify service, or `None` for any
/// other error.
pub fn unavailable_message(error: &ZniczError) -> Option<&'static str> {
    matches!(
        error,
        ZniczError::Spotify {
            kind: SpotifyErrorKind::NoService,
            ..
        }
    )
    .then_some(
        "Spotify is unavailable: this player has no Spotify service (no data folder for the login token?)",
    )
}

/// An IPC error as the command's error: the no-service case gets its own line.
fn host_error(error: ZniczError) -> color_eyre::Report {
    match unavailable_message(&error) {
        Some(line) => eyre!(line),
        None => eyre!("{error}"),
    }
}

/// The line under the login URL.
pub fn login_intro(listening: bool) -> &'static str {
    if listening {
        "Waiting for the browser; or paste the address you were sent to."
    } else {
        "Paste the address you were sent to."
    }
}

/// What a login poll concludes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoginWait {
    Pending,
    LoggedIn { user: Option<String> },
    Failed(String),
}

/// The login's outcome from one state read. `account_wait` is how long the
/// login has been `LoggedIn` without an account name.
pub fn login_wait(state: &SpotifyState, account_wait: Duration) -> LoginWait {
    match &state.login {
        SpotifyLogin::LoggingIn => LoginWait::Pending,
        SpotifyLogin::LoggedIn => match &state.account {
            Some(account) => LoginWait::LoggedIn {
                user: Some(account.user.clone()),
            },
            None if account_wait < ACCOUNT_WAIT => LoginWait::Pending,
            None => LoginWait::LoggedIn { user: None },
        },
        SpotifyLogin::Failed(reason) => LoginWait::Failed(reason.replace(
            "press Enter to start again",
            "run `znicz spotify login` again",
        )),
        SpotifyLogin::NotLoggedIn | SpotifyLogin::NotConfigured => {
            LoginWait::Failed("the login was cancelled".into())
        }
    }
}

pub fn logged_in_message(user: Option<&str>) -> String {
    match user {
        Some(user) => format!("Logged in as {user}"),
        None => "Logged in to Spotify".into(),
    }
}

/// The line login prints when `config.toml` did not load: where the problem
/// is, in `file`.
pub fn config_problem_text(file: &Path, problem: &ConfigProblem) -> String {
    match problem {
        ConfigProblem::Parse {
            line,
            column,
            message,
        } => format!(
            "{} has an error (line {line}, column {column}): {message}",
            file.display()
        ),
        ConfigProblem::Unreadable { message } => {
            format!("{} could not be read: {message}", file.display())
        }
    }
}

/// The line for a login that is already done, or `None` when a login can
/// start.
pub fn already_logged_in_message(state: &SpotifyState) -> Option<String> {
    if state.login != SpotifyLogin::LoggedIn {
        return None;
    }
    let hint = "(run `znicz spotify logout` first to switch accounts).";
    Some(match &state.account {
        Some(account) => format!("Already logged in to Spotify as {} {hint}", account.user),
        None => format!("Already logged in to Spotify {hint}"),
    })
}

/// Why login cannot start, or `Ok` when the client ID is usable.
pub fn check_client_id(config: &SpotifyConfig) -> Result<(), String> {
    if !config.is_configured() {
        return Err(format!(
            "Spotify is not configured. Register a free app at https://developer.spotify.com/dashboard, \
             add the redirect URI {}, then set client_id under [spotify] in config.toml \
             (find the file with `znicz config path`).",
            config.redirect_uri()
        ));
    }
    if !valid_client_id(&config.client_id) {
        return Err(
            "the client_id under [spotify] must be 32 hex characters; copy it from the app's page in the Spotify dashboard"
                .into(),
        );
    }
    Ok(())
}

/// `m:ss`, with minutes past an hour left to grow.
pub fn clock(ms: u64) -> String {
    let secs = ms / 1000;
    format!("{}:{:02}", secs / 60, secs % 60)
}

/// The lines `znicz spotify status` prints. `now_ms` is Unix time, for the
/// playback position.
pub fn status_lines(state: &SpotifyState, now_ms: u64) -> Vec<String> {
    let user = state
        .account
        .as_ref()
        .map_or("-", |account| account.user.as_str());
    let premium = match state.account.as_ref().and_then(|account| account.premium) {
        Some(true) => "yes",
        Some(false) => "no",
        None => "unknown (learned at the first play)",
    };
    let mut lines = vec![
        format!("Login: {}", login_text(&state.login)),
        format!("User: {user}"),
        format!("Premium: {premium}"),
        format!("Device: {}", device_text(state)),
        format!("Playing: {}", playing_text(state, now_ms)),
    ];
    if let Some(error) = &state.last_error {
        lines.push(format!("Last error: {error}"));
    }
    lines
}

fn login_text(login: &SpotifyLogin) -> String {
    match login {
        SpotifyLogin::NotConfigured => {
            "not configured (set client_id under [spotify] in config.toml)".into()
        }
        SpotifyLogin::NotLoggedIn => "not logged in".into(),
        SpotifyLogin::LoggingIn => "logging in".into(),
        SpotifyLogin::LoggedIn => "logged in".into(),
        SpotifyLogin::Failed(reason) => format!("failed: {reason}"),
    }
}

fn device_text(state: &SpotifyState) -> String {
    let device = state
        .playback
        .as_ref()
        .and_then(|playback| playback.device.as_ref())
        .or_else(|| state.devices.iter().find(|device| device.is_active));
    match device {
        Some(device) => format!("{} ({})", device.name, device.kind),
        None => "none".into(),
    }
}

fn playing_text(state: &SpotifyState, now_ms: u64) -> String {
    let Some(playback) = &state.playback else {
        return "nothing".into();
    };
    let Some(track) = &playback.item else {
        return if playback.unsupported {
            "an item Znicz does not show (podcast or ad)".into()
        } else {
            "nothing".into()
        };
    };
    let status = if playback.is_playing {
        "playing"
    } else {
        "paused"
    };
    format!(
        "{} — {}, {} {} / {}",
        track.title,
        track.artist_line(),
        status,
        clock(playback.progress_at(now_ms)),
        clock(track.duration_ms)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use znicz_core::NO_SPOTIFY;
    use znicz_core::{
        SpotifyAccount, SpotifyDevice, SpotifyLogin, SpotifyPlayback, SpotifyState, SpotifyTrack,
    };

    fn track() -> SpotifyTrack {
        SpotifyTrack {
            title: "So What".into(),
            artists: vec!["Miles Davis".into()],
            duration_ms: 562_000,
            ..SpotifyTrack::default()
        }
    }

    fn kitchen() -> SpotifyDevice {
        SpotifyDevice {
            id: "d1".into(),
            name: "Kitchen speaker".into(),
            kind: "Speaker".into(),
            is_active: true,
            volume_percent: None,
        }
    }

    #[test]
    fn the_login_intro_depends_on_the_listener() {
        assert_eq!(
            login_intro(true),
            "Waiting for the browser; or paste the address you were sent to."
        );
        assert_eq!(login_intro(false), "Paste the address you were sent to.");
    }

    #[test]
    fn the_login_waits_while_logging_in() {
        let state = SpotifyState {
            login: SpotifyLogin::LoggingIn,
            ..SpotifyState::default()
        };
        assert_eq!(login_wait(&state, Duration::ZERO), LoginWait::Pending);
    }

    #[test]
    fn a_finished_login_names_the_user_once_known() {
        let known = SpotifyState {
            login: SpotifyLogin::LoggedIn,
            account: Some(SpotifyAccount {
                user: "Eugene".into(),
                premium: None,
            }),
            ..SpotifyState::default()
        };
        assert_eq!(
            login_wait(&known, Duration::ZERO),
            LoginWait::LoggedIn {
                user: Some("Eugene".into())
            }
        );

        // The account read follows the login; wait a little for it.
        let unknown_yet = SpotifyState {
            login: SpotifyLogin::LoggedIn,
            ..SpotifyState::default()
        };
        assert_eq!(
            login_wait(&unknown_yet, Duration::from_secs(1)),
            LoginWait::Pending
        );
        assert_eq!(
            login_wait(&unknown_yet, Duration::from_secs(5)),
            LoginWait::LoggedIn { user: None }
        );
        assert_eq!(logged_in_message(None), "Logged in to Spotify");
        assert_eq!(logged_in_message(Some("Eugene")), "Logged in as Eugene");
    }

    #[test]
    fn a_failed_or_cancelled_login_ends_with_a_reason() {
        let failed = SpotifyState {
            login: SpotifyLogin::Failed("login expired, press Enter to start again".into()),
            ..SpotifyState::default()
        };
        // The service's wording is for the TUI (Enter); the CLI says what to run.
        assert_eq!(
            login_wait(&failed, Duration::ZERO),
            LoginWait::Failed("login expired, run `znicz spotify login` again".into())
        );
        let cancelled = SpotifyState {
            login: SpotifyLogin::NotLoggedIn,
            ..SpotifyState::default()
        };
        assert_eq!(
            login_wait(&cancelled, Duration::ZERO),
            LoginWait::Failed("the login was cancelled".into())
        );
    }

    #[test]
    fn a_client_id_is_checked_before_anything_is_sent() {
        let empty = SpotifyConfig::default();
        let message = check_client_id(&empty).expect_err("not configured");
        assert!(message.contains("developer.spotify.com"), "{message}");
        assert!(
            message.contains("http://127.0.0.1:8898/callback"),
            "{message}"
        );
        assert!(message.contains("config.toml"), "{message}");

        let typo = SpotifyConfig {
            client_id: "not-hex".into(),
            ..SpotifyConfig::default()
        };
        assert!(
            check_client_id(&typo)
                .expect_err("bad id")
                .contains("32 hex")
        );

        let good = SpotifyConfig {
            client_id: "0123456789abcdef0123456789abcdef".into(),
            ..SpotifyConfig::default()
        };
        assert!(check_client_id(&good).is_ok());
    }

    #[test]
    fn the_clock_reads_minutes_and_seconds() {
        assert_eq!(clock(0), "0:00");
        assert_eq!(clock(62_000), "1:02");
        assert_eq!(clock(562_000), "9:22");
        assert_eq!(clock(3_600_000), "60:00");
    }

    #[test]
    fn status_of_an_unconfigured_player_says_so() {
        let lines = status_lines(&SpotifyState::default(), 0);
        assert_eq!(
            lines,
            vec![
                "Login: not configured (set client_id under [spotify] in config.toml)".to_string(),
                "User: -".to_string(),
                "Premium: unknown (learned at the first play)".to_string(),
                "Device: none".to_string(),
                "Playing: nothing".to_string(),
            ]
        );
    }

    #[test]
    fn status_shows_the_account_device_and_track() {
        let state = SpotifyState {
            login: SpotifyLogin::LoggedIn,
            account: Some(SpotifyAccount {
                user: "Eugene".into(),
                premium: Some(true),
            }),
            playback: Some(SpotifyPlayback {
                item: Some(track()),
                is_playing: true,
                progress_ms: 62_000,
                read_at_ms: 1_000,
                device: Some(kitchen()),
                ..SpotifyPlayback::default()
            }),
            last_error: Some("needs Spotify Premium".into()),
            ..SpotifyState::default()
        };
        let lines = status_lines(&state, 1_000);
        assert_eq!(lines[0], "Login: logged in");
        assert_eq!(lines[1], "User: Eugene");
        assert_eq!(lines[2], "Premium: yes");
        assert_eq!(lines[3], "Device: Kitchen speaker (Speaker)");
        assert_eq!(
            lines[4],
            "Playing: So What — Miles Davis, playing 1:02 / 9:22"
        );
        assert_eq!(lines[5], "Last error: needs Spotify Premium");
    }

    #[test]
    fn status_says_paused_and_not_premium() {
        let state = SpotifyState {
            login: SpotifyLogin::LoggedIn,
            account: Some(SpotifyAccount {
                user: "Eugene".into(),
                premium: Some(false),
            }),
            playback: Some(SpotifyPlayback {
                item: Some(track()),
                is_playing: false,
                progress_ms: 5_000,
                ..SpotifyPlayback::default()
            }),
            devices: vec![kitchen()],
            ..SpotifyState::default()
        };
        let lines = status_lines(&state, 99_999);
        assert_eq!(lines[2], "Premium: no");
        assert_eq!(lines[3], "Device: Kitchen speaker (Speaker)");
        assert_eq!(
            lines[4],
            "Playing: So What — Miles Davis, paused 0:05 / 9:22"
        );
    }

    #[test]
    fn an_unsupported_item_is_named_as_such() {
        let state = SpotifyState {
            playback: Some(SpotifyPlayback {
                unsupported: true,
                ..SpotifyPlayback::default()
            }),
            ..SpotifyState::default()
        };
        assert_eq!(
            status_lines(&state, 0)[4],
            "Playing: an item Znicz does not show (podcast or ad)"
        );
    }

    #[test]
    fn a_player_without_a_spotify_service_gets_a_clear_line() {
        let no_service = ZniczError::Spotify {
            kind: SpotifyErrorKind::NoService,
            message: NO_SPOTIFY.into(),
        };
        assert_eq!(
            unavailable_message(&no_service),
            Some(
                "Spotify is unavailable: this player has no Spotify service (no data folder for the login token?)"
            )
        );
        let other = ZniczError::Spotify {
            kind: SpotifyErrorKind::Other,
            message: NO_SPOTIFY.into(),
        };
        assert_eq!(unavailable_message(&other), None);
        assert_eq!(
            unavailable_message(&ZniczError::Player("the player went away".into())),
            None
        );
    }

    #[test]
    fn only_login_sends_the_settings() {
        assert!(configure_before(&SpotifyCmd::Login));
        assert!(!configure_before(&SpotifyCmd::Status));
        assert!(!configure_before(&SpotifyCmd::Logout));
    }

    #[test]
    fn a_second_login_says_so_and_names_the_user() {
        let known = SpotifyState {
            login: SpotifyLogin::LoggedIn,
            account: Some(SpotifyAccount {
                user: "Eugene".into(),
                premium: None,
            }),
            ..SpotifyState::default()
        };
        assert_eq!(
            already_logged_in_message(&known).as_deref(),
            Some(
                "Already logged in to Spotify as Eugene (run `znicz spotify logout` first to switch accounts)."
            )
        );

        let no_name = SpotifyState {
            login: SpotifyLogin::LoggedIn,
            ..SpotifyState::default()
        };
        assert_eq!(
            already_logged_in_message(&no_name).as_deref(),
            Some(
                "Already logged in to Spotify (run `znicz spotify logout` first to switch accounts)."
            )
        );

        let logged_out = SpotifyState {
            login: SpotifyLogin::NotLoggedIn,
            ..SpotifyState::default()
        };
        assert_eq!(already_logged_in_message(&logged_out), None);
    }

    #[test]
    fn a_config_problem_names_the_file_and_the_place() {
        let parse = ConfigProblem::Parse {
            line: 3,
            column: 5,
            message: "expected a value".into(),
        };
        assert_eq!(
            config_problem_text(Path::new("/home/x/config.toml"), &parse),
            "/home/x/config.toml has an error (line 3, column 5): expected a value"
        );
        let unreadable = ConfigProblem::Unreadable {
            message: "permission denied".into(),
        };
        assert_eq!(
            config_problem_text(Path::new("/c.toml"), &unreadable),
            "/c.toml could not be read: permission denied"
        );
    }
}
