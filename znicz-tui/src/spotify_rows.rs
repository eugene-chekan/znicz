//! The Spotify rows of Settings, as pure functions: what the account row says,
//! which devices the choice offers, what a typed value must be, and the text of
//! the login panel. No IO, no App.

use znicz_core::{LOGIN_EXPIRED, SpotifyAccount, SpotifyDevice, SpotifyLogin, valid_client_id};
use znicz_spotify::browser::OpenBrowser;

use crate::settings::Row;

/// The lowest port the login callback may use; the top is 65535.
pub const PORT_MIN: u16 = 1024;
pub const OPEN_BROWSERS: [OpenBrowser; 3] =
    [OpenBrowser::Auto, OpenBrowser::Always, OpenBrowser::Never];
/// The strip's text when a row needs a client ID first.
pub const CLIENT_FIRST: &str = "Set a client ID first, in the Client ID row.";
pub const LOGOUT_QUESTION: &str = "Log out of Spotify? Enter logs out · Esc cancels";
pub const CLIENT_ID_QUESTION: &str =
    "Changing the client ID logs you out of Spotify. Enter confirms · Esc cancels";
/// Shown under the paste prompt when Spotify rejects the pasted address.
pub const WRONG_ADDRESS: &str = "that address is not from this login";
/// The Client ID row's hint while the player keeps another client ID.
pub const PLAYER_DIFFERS_HINT: &str = "player uses another ID";
/// The toast when the player refuses the saved client ID as another one.
pub const PLAYER_DIFFERS_TOAST: &str = "The player uses another Spotify client ID. Enter it again under Client ID to switch (this logs out).";
/// The strip's text on the rows the player does not apply while it keeps another ID.
pub const NOT_APPLIED: &str = "Not applied: the player uses another client ID.";

/// What the Spotify rows show, copied from the App each frame.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SpotifyView {
    /// A client ID is set.
    pub configured: bool,
    /// The first eight characters of the client ID, for the row. The full ID never reaches the row text.
    pub client_id_head: String,
    /// `[spotify] device`; empty is the active device.
    pub device: String,
    pub redirect_port: u16,
    pub open_browser: OpenBrowser,
    /// The login as the App shows it (see `App::spotify_login_shown`).
    pub login: SpotifyLogin,
    pub account: Option<SpotifyAccount>,
    /// The last device list from `SpotifyOp::Devices`.
    pub devices: Vec<SpotifyDevice>,
    /// A device list has arrived for the current client ID.
    pub devices_known: bool,
    /// The player does not answer Spotify requests: the reason.
    pub unavailable: Option<String>,
    /// The player refused the file's client ID as another one, so it keeps its own.
    pub player_differs: bool,
}

/// A failed login's reason, with the TUI's hint when the login expired.
fn failure_text(reason: &str) -> String {
    if reason == LOGIN_EXPIRED {
        format!("{LOGIN_EXPIRED}, press Enter to start again")
    } else {
        reason.to_string()
    }
}

/// The account row's text.
pub fn account_text(view: &SpotifyView) -> String {
    if let Some(reason) = &view.unavailable {
        return format!("Spotify is unavailable: {reason} (switch tab to retry)");
    }
    match &view.login {
        SpotifyLogin::NotConfigured => "Not configured".into(),
        SpotifyLogin::NotLoggedIn => "Not logged in".into(),
        SpotifyLogin::LoggingIn => "Logging in…".into(),
        SpotifyLogin::Failed(reason) => format!("Failed: {}", failure_text(reason)),
        SpotifyLogin::LoggedIn => match &view.account {
            None => "Logged in".into(),
            Some(account) => {
                let plan = match account.premium {
                    Some(true) => " · Premium",
                    Some(false) => " · not Premium (control unavailable)",
                    None => "",
                };
                format!("Logged in as {}{plan}", account.user)
            }
        },
    }
}

/// The Playback device choices: the active device (empty name) first, then
/// each device Spotify lists once, then the configured name when Spotify does
/// not list it (it is shown as offline).
pub fn device_choices(view: &SpotifyView) -> Vec<String> {
    let mut names = vec![String::new()];
    for device in &view.devices {
        if !names.contains(&device.name) {
            names.push(device.name.clone());
        }
    }
    if !view.device.is_empty() && !names.contains(&view.device) {
        names.push(view.device.clone());
    }
    names
}

/// A choice's text: the empty name is the active device.
pub fn device_value(name: &str) -> String {
    if name.is_empty() {
        "Active device".into()
    } else {
        name.into()
    }
}

/// The active device is always reachable; a named device must be listed.
pub fn device_online(view: &SpotifyView, name: &str) -> bool {
    name.is_empty() || view.devices.iter().any(|d| d.name == name)
}

/// Why a typed client ID is refused, or `None` when it is accepted. The text
/// is trimmed first. Empty is accepted: it clears the ID.
pub fn client_id_problem(text: &str) -> Option<&'static str> {
    let text = text.trim();
    (!text.is_empty() && !valid_client_id(text)).then_some("32 hex characters")
}

/// A typed port, or the range text the strip shows.
pub fn parse_port(text: &str) -> std::result::Result<u16, String> {
    match text.trim().parse::<u16>() {
        Ok(port) if port >= PORT_MIN => Ok(port),
        _ => Err("1024–65535".into()),
    }
}

pub fn open_browser_name(value: OpenBrowser) -> &'static str {
    match value {
        OpenBrowser::Auto => "auto",
        OpenBrowser::Always => "always",
        OpenBrowser::Never => "never",
    }
}

/// The URI the Spotify app must register.
pub fn redirect_uri(port: u16) -> String {
    format!("http://127.0.0.1:{port}/callback")
}

/// The panel's three lines. The URL line carries the keys.
pub fn login_panel_lines(url: &str, listening: bool) -> Vec<String> {
    let waiting = if listening {
        "Waiting for the browser; or paste the address you were sent to."
    } else {
        "Paste the address you were sent to."
    };
    vec![
        "Open this URL in any browser and approve:".into(),
        format!("{url}   (c copy · Enter paste)"),
        waiting.into(),
    ]
}

/// Rows whose value the player must hear about. The browser setting is only
/// read by the front end.
pub fn sent_to_player(row: &Row) -> bool {
    matches!(row, Row::ClientId | Row::RedirectPort | Row::PlaybackDevice)
}

/// A finished login, for the toast.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoginOutcome {
    Success(String),
    Failure(String),
}

/// The toast for a login that has finished. `None` while it runs, and for a
/// login that was cancelled (`NotLoggedIn`).
pub fn login_outcome(
    login: &SpotifyLogin,
    account: Option<&SpotifyAccount>,
) -> Option<LoginOutcome> {
    match login {
        SpotifyLogin::LoggedIn => {
            let text = match account {
                Some(account) => format!("Logged in as {}", account.user),
                None => "Logged in to Spotify".to_string(),
            };
            Some(LoginOutcome::Success(text))
        }
        SpotifyLogin::Failed(reason) => Some(LoginOutcome::Failure(format!(
            "Spotify login failed: {}",
            failure_text(reason)
        ))),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_player_side_rows_are_sent_to_the_player() {
        assert!(sent_to_player(&Row::ClientId));
        assert!(sent_to_player(&Row::RedirectPort));
        assert!(sent_to_player(&Row::PlaybackDevice));
        assert!(!sent_to_player(&Row::OpenBrowser));
        assert!(!sent_to_player(&Row::Account));
        assert!(!sent_to_player(&Row::ShowCover));
    }

    fn device(name: &str) -> SpotifyDevice {
        SpotifyDevice {
            id: format!("id-{name}"),
            name: name.into(),
            kind: "Computer".into(),
            is_active: false,
            volume_percent: None,
        }
    }

    fn account(premium: Option<bool>) -> SpotifyAccount {
        SpotifyAccount {
            user: "Eugene".into(),
            premium,
        }
    }

    fn view(login: SpotifyLogin) -> SpotifyView {
        SpotifyView {
            configured: true,
            login,
            ..SpotifyView::default()
        }
    }

    #[test]
    fn the_account_row_says_each_login_state() {
        assert_eq!(
            account_text(&view(SpotifyLogin::NotConfigured)),
            "Not configured"
        );
        assert_eq!(
            account_text(&view(SpotifyLogin::NotLoggedIn)),
            "Not logged in"
        );
        assert_eq!(account_text(&view(SpotifyLogin::LoggingIn)), "Logging in…");
        assert_eq!(
            account_text(&view(SpotifyLogin::Failed("bad redirect".into()))),
            "Failed: bad redirect"
        );
    }

    #[test]
    fn an_expired_login_says_to_press_enter_to_start_again() {
        let expired = SpotifyLogin::Failed(LOGIN_EXPIRED.into());
        assert_eq!(
            account_text(&view(expired.clone())),
            "Failed: login expired, press Enter to start again"
        );
        assert_eq!(
            login_outcome(&expired, None),
            Some(LoginOutcome::Failure(
                "Spotify login failed: login expired, press Enter to start again".into()
            ))
        );
    }

    #[test]
    fn a_logged_in_account_says_what_its_plan_is_once_known() {
        let mut v = view(SpotifyLogin::LoggedIn);
        v.account = Some(account(Some(true)));
        assert_eq!(account_text(&v), "Logged in as Eugene · Premium");
        v.account = Some(account(Some(false)));
        assert_eq!(
            account_text(&v),
            "Logged in as Eugene · not Premium (control unavailable)"
        );
        v.account = Some(account(None));
        assert_eq!(account_text(&v), "Logged in as Eugene");
        v.account = None;
        assert_eq!(account_text(&v), "Logged in");
    }

    #[test]
    fn an_unavailable_player_wins_over_the_login_state() {
        let mut v = view(SpotifyLogin::LoggedIn);
        v.unavailable = Some("the running player has no Spotify service".into());
        assert_eq!(
            account_text(&v),
            "Spotify is unavailable: the running player has no Spotify service (switch tab to retry)"
        );
    }

    #[test]
    fn device_choices_start_with_the_active_device_and_keep_an_offline_name() {
        let mut v = view(SpotifyLogin::LoggedIn);
        v.devices = vec![device("Laptop"), device("Kitchen"), device("Laptop")];
        assert_eq!(device_choices(&v), ["", "Laptop", "Kitchen"]);
        v.device = "Garage".into();
        assert_eq!(device_choices(&v), ["", "Laptop", "Kitchen", "Garage"]);
        assert!(!device_online(&v, "Garage"));
        assert!(device_online(&v, "Kitchen"));
        assert!(device_online(&v, ""));
    }

    #[test]
    fn the_active_device_is_named_in_words() {
        assert_eq!(device_value(""), "Active device");
        assert_eq!(device_value("Kitchen"), "Kitchen");
    }

    #[test]
    fn a_client_id_is_32_hex_characters_or_empty() {
        assert_eq!(client_id_problem(""), None);
        assert_eq!(client_id_problem("   "), None);
        assert_eq!(client_id_problem("0123456789abcdef0123456789ABCDEF"), None);
        assert_eq!(
            client_id_problem("  0123456789abcdef0123456789abcdef \n"),
            None
        );
        assert_eq!(
            client_id_problem("0123456789abcdef0123456789abcde"),
            Some("32 hex characters")
        );
        assert_eq!(
            client_id_problem("0123456789abcdef0123456789abcdefa"),
            Some("32 hex characters")
        );
        assert_eq!(
            client_id_problem("0123456789abcdef0123456789abcdeg"),
            Some("32 hex characters")
        );
    }

    #[test]
    fn a_port_is_1024_to_65535() {
        assert_eq!(parse_port("8898"), Ok(8898));
        assert_eq!(parse_port(" 1024 "), Ok(1024));
        assert_eq!(parse_port("65535"), Ok(65535));
        assert_eq!(parse_port("1023"), Err("1024–65535".to_string()));
        assert_eq!(parse_port("0"), Err("1024–65535".to_string()));
        assert_eq!(parse_port("65536"), Err("1024–65535".to_string()));
        assert_eq!(parse_port("http"), Err("1024–65535".to_string()));
        assert_eq!(parse_port(""), Err("1024–65535".to_string()));
    }

    #[test]
    fn the_redirect_uri_is_the_one_the_app_needs() {
        assert_eq!(redirect_uri(8898), "http://127.0.0.1:8898/callback");
    }

    #[test]
    fn open_browser_names_and_order() {
        assert_eq!(
            OPEN_BROWSERS.map(open_browser_name),
            ["auto", "always", "never"]
        );
    }

    #[test]
    fn the_login_panel_says_what_to_do_in_each_case() {
        let url = "https://accounts.spotify.com/authorize?client_id=x";
        let listening = login_panel_lines(url, true);
        assert_eq!(listening.len(), 3);
        assert_eq!(listening[0], "Open this URL in any browser and approve:");
        assert_eq!(listening[1], format!("{url}   (c copy · Enter paste)"));
        assert_eq!(
            listening[2],
            "Waiting for the browser; or paste the address you were sent to."
        );
        let pasting = login_panel_lines(url, false);
        assert_eq!(pasting[2], "Paste the address you were sent to.");
    }

    #[test]
    fn login_outcomes_only_for_finished_logins() {
        assert_eq!(
            login_outcome(&SpotifyLogin::LoggedIn, Some(&account(None))),
            Some(LoginOutcome::Success("Logged in as Eugene".into()))
        );
        // Before the account name is known, the toast names the service.
        assert_eq!(
            login_outcome(&SpotifyLogin::LoggedIn, None),
            Some(LoginOutcome::Success("Logged in to Spotify".into()))
        );
        assert_eq!(
            login_outcome(&SpotifyLogin::Failed("no".into()), None),
            Some(LoginOutcome::Failure("Spotify login failed: no".into()))
        );
        assert_eq!(login_outcome(&SpotifyLogin::LoggingIn, None), None);
        assert_eq!(login_outcome(&SpotifyLogin::NotLoggedIn, None), None);
    }
}
