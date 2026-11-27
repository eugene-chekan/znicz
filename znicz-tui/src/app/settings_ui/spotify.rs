//! The Spotify rows of the Settings modal: the `[spotify]` section as the file
//! holds it, the client ID, port and playback device rows, and what each one
//! sends to the player. A child of `settings_ui`, which keeps the save path.

use znicz_core::{SpotifyConfig, SpotifyOp};
use znicz_spotify::browser::OpenBrowser;

use super::{SliderPrompt, Stored};
use crate::app::App;
use crate::app::spotify_ui::SpotifyConfirm;
use crate::config_file::ConfigFile;
use crate::line_edit::LineEdit;
use crate::settings::{self, Row, Tab};
use crate::spotify_rows;
use crate::spotify_worker::SpotifyAsk;

/// The `[spotify]` section as `config.toml` has it now, and its browser setting.
/// A missing key is the default; a value of the wrong kind is an error.
pub(crate) fn read_spotify(file: &ConfigFile) -> Result<(SpotifyConfig, OpenBrowser), String> {
    let text = |key: &str| -> Result<Option<String>, String> {
        match file.get("spotify", key)? {
            None => Ok(None),
            Some(value) => value
                .as_str()
                .map(|found| Some(found.to_string()))
                .ok_or_else(|| format!("[spotify] {key} must be text")),
        }
    };
    let redirect_port = match file.get("spotify", "redirect_port")? {
        None => SpotifyConfig::DEFAULT_PORT,
        Some(value) => value
            .as_integer()
            .and_then(|n| u16::try_from(n).ok())
            .ok_or_else(|| "[spotify] redirect_port must be a port number".to_string())?,
    };
    let config = SpotifyConfig {
        client_id: text("client_id")?.unwrap_or_default().trim().to_string(),
        redirect_port,
        device: text("device")?.unwrap_or_default(),
    };
    let open_browser = text("open_browser")?
        .and_then(|value| OpenBrowser::parse(&value))
        .unwrap_or_default();
    Ok((config, open_browser))
}

impl App {
    /// Step the Playback device choice through the active device, the listed
    /// devices and the configured one. Without a client ID the row does nothing.
    pub(super) fn step_playback_device(&mut self, forward: bool, wrap: bool) -> bool {
        if !self.spotify.is_configured() {
            return false;
        }
        let view = self.spotify_view();
        let choices = spotify_rows::device_choices(&view);
        let new = settings::step(&choices, &self.spotify.device, forward, wrap);
        let changed = new != self.spotify.device;
        self.spotify.device = new;
        changed
    }

    /// Set the redirect port. Returns whether it changed.
    pub(super) fn set_redirect_port(&mut self, port: u16) -> bool {
        let changed = self.spotify.redirect_port != port;
        self.spotify.redirect_port = port;
        changed
    }

    /// Enter on the client ID or redirect port: a prompt with the current value.
    pub(super) fn open_text_prompt(&mut self, row: &Row) {
        let text = match row {
            Row::ClientId => self.spotify.client_id.clone(),
            Row::RedirectPort => self.spotify.redirect_port.to_string(),
            _ => return,
        };
        self.settings_prompt = Some(SliderPrompt {
            row: row.clone(),
            text: LineEdit::from_text(text),
            error: None,
        });
    }

    /// Enter in the port prompt: a number in range saves; anything else keeps the prompt open.
    pub(super) fn submit_port_prompt(&mut self) {
        let Some(prompt) = self.settings_prompt.as_mut() else {
            return;
        };
        match spotify_rows::parse_port(prompt.text.as_str()) {
            Err(range) => prompt.error = Some(range),
            Ok(port) => {
                self.settings_prompt = None;
                self.change_with(&Row::RedirectPort, |app| app.set_redirect_port(port));
            }
        }
    }

    /// Enter in the client ID prompt. The value is trimmed and checked; a
    /// refused value keeps the prompt open with the reason. Changing or clearing
    /// a set ID waits for Enter in the strip, because the player logs out.
    pub(super) fn submit_client_id_prompt(&mut self) {
        let Some(prompt) = self.settings_prompt.as_mut() else {
            return;
        };
        let text = prompt.text.as_str().trim().to_string();
        if let Some(problem) = spotify_rows::client_id_problem(&text) {
            prompt.error = Some(problem.into());
            return;
        }
        self.settings_prompt = None;
        if text == self.spotify.client_id {
            // The same ID again asks to switch when the player keeps another one.
            if self.spotify_player_differs {
                self.spotify_confirm = Some(SpotifyConfirm::ClientId(text));
            }
            return;
        }
        // A set ID asks first, whatever the login state: the player logs out
        // and deletes the token. The file is asked too, since it may hold an ID
        // this run never showed. Only a first ID saves directly.
        let file_id = self.file_client_id();
        let asks = [self.spotify.client_id.as_str(), file_id.as_str()]
            .iter()
            .any(|id| !id.is_empty() && *id != text.as_str());
        if asks {
            self.spotify_confirm = Some(SpotifyConfirm::ClientId(text));
            return;
        }
        self.change_with(&Row::ClientId, |app| app.set_client_id(text));
    }

    /// The confirmed client ID. It is saved and sent with the replace flag even
    /// when it is the ID the screen already shows: the player may still keep
    /// another one, and only the flag lets it switch.
    pub(super) fn replace_client_id(&mut self, id: String) {
        let before = self.settings_snapshot();
        self.set_client_id(id);
        self.after_setting_changed(&Row::ClientId);
        self.save_setting(&Row::ClientId, before, true);
    }

    /// Set the client ID. Returns whether it changed.
    pub(super) fn set_client_id(&mut self, id: String) -> bool {
        let changed = self.spotify.client_id != id;
        self.spotify.client_id = id;
        changed
    }

    /// The client ID `config.toml` has now, or empty when it cannot be read.
    pub(super) fn file_client_id(&self) -> String {
        self.config_file
            .as_ref()
            .and_then(|file| read_spotify(file).ok())
            .map(|(config, _)| config.client_id)
            .unwrap_or_default()
    }

    /// After a Spotify row was written, read `[spotify]` back from the file and
    /// take that as the truth: the screen shows it, and the player gets it. A
    /// row the player needs is then sent. If the file holds an ID this run did
    /// not show, it was edited outside Znicz; the player still refuses a swap
    /// unless `replace_client_id` is set, and its refusal is toasted.
    /// Called only from `save_setting`, after the file was written.
    pub(super) fn sync_spotify_from_file(
        &mut self,
        file: &ConfigFile,
        row: &Row,
        replace_client_id: bool,
    ) {
        let (config, open_browser) = match read_spotify(file) {
            Ok(values) => values,
            Err(e) => {
                self.toasts.error(format!(
                    "Not sent to Spotify: could not read [spotify] from config.toml: {e}"
                ));
                return;
            }
        };
        self.spotify = config.clone();
        self.open_browser = open_browser;
        if !spotify_rows::sent_to_player(row) {
            return;
        }
        self.ask_spotify(
            SpotifyAsk::Configure,
            SpotifyOp::Configure {
                config,
                replace_client_id,
            },
        );
    }

    /// Enter on the strip's question: log out, or save the new client ID.
    pub(crate) fn confirm_spotify(&mut self) {
        match self.spotify_confirm.take() {
            Some(SpotifyConfirm::Logout) => {
                self.ask_spotify(SpotifyAsk::Logout, SpotifyOp::Logout);
            }
            Some(SpotifyConfirm::ClientId(id)) => self.replace_client_id(id),
            None => {}
        }
    }

    /// The value a Spotify row saves, as the config file takes it. None for any other row.
    pub(super) fn spotify_stored_value(&self, row: &Row) -> Option<Stored> {
        Some(match row {
            Row::ClientId if self.spotify.client_id.is_empty() => Stored::Remove,
            Row::ClientId => Stored::Text(self.spotify.client_id.clone()),
            Row::RedirectPort => Stored::Int(i64::from(self.spotify.redirect_port)),
            Row::OpenBrowser => Stored::Str(spotify_rows::open_browser_name(self.open_browser)),
            Row::PlaybackDevice if self.spotify.device.is_empty() => Stored::Remove,
            Row::PlaybackDevice => Stored::Text(self.spotify.device.clone()),
            _ => return None,
        })
    }
}

/// Whether a Spotify row goes back to its old value when it is not written:
/// the screen must not show a client ID or device the player does not have.
pub(super) fn restores_when_unsaved(row: &Row) -> bool {
    row.tab() == Tab::Spotify
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use crossterm::event::KeyCode;
    use znicz_core::{SpotifyConfig, SpotifyDevice, SpotifyLogin, SpotifyOp};

    use super::*;
    use crate::app::settings_ui::tests::{
        click, press, saved, screen_buffer, screen_rows, select, settings_app, toast_texts,
        type_text,
    };
    use crate::app::spotify_ui::tests::{settle, wait_for_op};
    use crate::app::{App, Modal, SettingsValues};
    use crate::config_file::ConfigProblem;
    use crate::spotify_worker::SpotifyWorker;

    // --- Spotify rows: saves, Configure ---

    fn spotify_app(
        dir: &tempfile::TempDir,
        host: Arc<crate::app::spotify_ui::tests::FakeHost>,
    ) -> App {
        let mut app = settings_app(dir).with_settings_values(SettingsValues {
            spotify: SpotifyConfig {
                client_id: "0123456789abcdef0123456789abcdef".into(),
                ..SpotifyConfig::default()
            },
            ..SettingsValues::default()
        });
        app.spotify_worker = Some(SpotifyWorker::spawn(host.clone()).unwrap());
        app.spotify_main = Some(host);
        app
    }

    fn one_device(name: &str) -> Vec<SpotifyDevice> {
        vec![SpotifyDevice {
            id: "d1".into(),
            name: name.into(),
            kind: "Speaker".into(),
            is_active: false,
            volume_percent: None,
        }]
    }

    #[test]
    fn enter_steps_the_open_browser_setting_and_saves_it_without_sending_it() {
        let dir = tempfile::tempdir().unwrap();
        let host = Arc::new(crate::app::spotify_ui::tests::FakeHost::default());
        let mut app = spotify_app(&dir, host.clone());
        press(&mut app, KeyCode::Char('S'));
        press(&mut app, KeyCode::Char('6'));
        select(&mut app, Row::OpenBrowser);
        press(&mut app, KeyCode::Enter);
        assert_eq!(
            saved(&dir)["spotify"]["open_browser"].as_str(),
            Some("always")
        );
        assert!(
            settle(&app, &[&*host])
                .iter()
                .all(|op| !matches!(op, SpotifyOp::Configure { .. }))
        );
    }

    #[test]
    fn stepping_the_device_saves_it_and_sends_configure() {
        let dir = tempfile::tempdir().unwrap();
        let host = Arc::new(crate::app::spotify_ui::tests::FakeHost::default());
        let mut app = spotify_app(&dir, host.clone());
        app.spotify_state.devices = one_device("Kitchen");
        press(&mut app, KeyCode::Char('S'));
        press(&mut app, KeyCode::Char('6'));
        select(&mut app, Row::PlaybackDevice);
        press(&mut app, KeyCode::Enter);
        assert_eq!(saved(&dir)["spotify"]["device"].as_str(), Some("Kitchen"));
        assert!(
            wait_for_op(&host, |op| matches!(
                op,
                SpotifyOp::Configure { config, .. } if config.device == "Kitchen"
            )),
            "{:?}",
            settle(&app, &[&*host])
        );
    }

    #[test]
    fn choosing_the_active_device_removes_the_key() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("config.toml"),
            "[spotify]\ndevice = \"Kitchen\"\n",
        )
        .unwrap();
        let host = Arc::new(crate::app::spotify_ui::tests::FakeHost::default());
        let mut app = settings_app(&dir).with_settings_values(SettingsValues {
            spotify: SpotifyConfig {
                client_id: "0123456789abcdef0123456789abcdef".into(),
                device: "Kitchen".into(),
                ..SpotifyConfig::default()
            },
            ..SettingsValues::default()
        });
        app.spotify_worker = Some(SpotifyWorker::spawn(host.clone()).unwrap());
        app.spotify_state.devices = one_device("Kitchen");
        press(&mut app, KeyCode::Char('S'));
        press(&mut app, KeyCode::Char('6'));
        select(&mut app, Row::PlaybackDevice);
        press(&mut app, KeyCode::Left);
        assert!(saved(&dir)["spotify"].get("device").is_none());
        assert!(app.spotify.device.is_empty());
    }

    #[test]
    fn a_port_typed_in_the_prompt_is_saved_and_sent() {
        let dir = tempfile::tempdir().unwrap();
        let host = Arc::new(crate::app::spotify_ui::tests::FakeHost::default());
        let mut app = spotify_app(&dir, host.clone());
        press(&mut app, KeyCode::Char('S'));
        press(&mut app, KeyCode::Char('6'));
        select(&mut app, Row::RedirectPort);
        press(&mut app, KeyCode::Enter);
        // Replace the default 8898 with 9001.
        for _ in 0..4 {
            press(&mut app, KeyCode::Backspace);
        }
        type_text(&mut app, "9001");
        press(&mut app, KeyCode::Enter);
        assert_eq!(
            saved(&dir)["spotify"]["redirect_port"].as_integer(),
            Some(9001)
        );
        assert!(
            wait_for_op(&host, |op| matches!(
                op,
                SpotifyOp::Configure { config, .. } if config.redirect_port == 9001
            )),
            "{:?}",
            settle(&app, &[&*host])
        );
    }

    #[test]
    fn a_port_out_of_range_keeps_the_prompt_open() {
        let dir = tempfile::tempdir().unwrap();
        let host = Arc::new(crate::app::spotify_ui::tests::FakeHost::default());
        let mut app = spotify_app(&dir, host);
        press(&mut app, KeyCode::Char('S'));
        press(&mut app, KeyCode::Char('6'));
        select(&mut app, Row::RedirectPort);
        press(&mut app, KeyCode::Enter);
        for _ in 0..4 {
            press(&mut app, KeyCode::Backspace);
        }
        type_text(&mut app, "80");
        press(&mut app, KeyCode::Enter);
        let prompt = app.settings_prompt.as_ref().expect("still open");
        assert_eq!(prompt.error.as_deref(), Some("1024–65535"));
        assert!(!dir.path().join("config.toml").exists());
    }

    #[test]
    fn a_broken_config_is_not_written_and_not_sent_and_the_row_goes_back() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "[spotify\nclient_id = 1\n").unwrap();
        let host = Arc::new(crate::app::spotify_ui::tests::FakeHost::default());
        let mut app = settings_app(&dir)
            .with_config_file(ConfigFile::new(&path))
            .with_config_problem(ConfigProblem::Parse {
                message: "bad".into(),
                line: 1,
                column: 1,
            })
            .with_settings_values(SettingsValues {
                spotify: SpotifyConfig {
                    client_id: "0123456789abcdef0123456789abcdef".into(),
                    ..SpotifyConfig::default()
                },
                ..SettingsValues::default()
            });
        app.spotify_worker = Some(SpotifyWorker::spawn(host.clone()).unwrap());
        press(&mut app, KeyCode::Char('S'));
        press(&mut app, KeyCode::Char('6'));
        select(&mut app, Row::OpenBrowser);
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.open_browser, OpenBrowser::Auto);
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "[spotify\nclient_id = 1\n"
        );
        assert!(
            !settle(&app, &[&*host])
                .iter()
                .any(|op| matches!(op, SpotifyOp::Configure { .. }))
        );
        assert!(toast_texts(&app).iter().any(|t| t.starts_with("Not saved")));
    }

    #[test]
    fn a_broken_config_restores_a_port_and_sends_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "[spotify\nclient_id = 1\n").unwrap();
        let host = Arc::new(crate::app::spotify_ui::tests::FakeHost::default());
        let mut app = settings_app(&dir)
            .with_config_file(ConfigFile::new(&path))
            .with_config_problem(ConfigProblem::Parse {
                message: "bad".into(),
                line: 1,
                column: 1,
            });
        app.spotify_worker = Some(SpotifyWorker::spawn(host.clone()).unwrap());
        press(&mut app, KeyCode::Char('S'));
        press(&mut app, KeyCode::Char('6'));
        select(&mut app, Row::RedirectPort);
        press(&mut app, KeyCode::Enter);
        for _ in 0..4 {
            press(&mut app, KeyCode::Backspace);
        }
        type_text(&mut app, "9001");
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.spotify.redirect_port, 8898);
        assert!(
            !settle(&app, &[&*host])
                .iter()
                .any(|op| matches!(op, SpotifyOp::Configure { .. }))
        );
    }

    /// The App as a start on a broken `config.toml` leaves it: the defaults in
    /// memory and the problem recorded. The file on disk has been fixed since.
    fn started_on_a_broken_file_now_fixed(
        dir: &tempfile::TempDir,
        host: Arc<crate::app::spotify_ui::tests::FakeHost>,
    ) -> App {
        std::fs::write(
            dir.path().join("config.toml"),
            format!("[spotify]\nclient_id = \"{ID}\"\n"),
        )
        .unwrap();
        let mut app = settings_app(dir).with_config_problem(ConfigProblem::Parse {
            message: "bad".into(),
            line: 1,
            column: 1,
        });
        app.spotify_worker = Some(SpotifyWorker::spawn(host.clone()).unwrap());
        app.spotify_main = Some(host);
        app
    }

    const FIXED_NOTICE: &str = "config.toml is fixed; restart Znicz to load all settings";

    #[test]
    fn showing_the_spotify_tab_reads_the_fixed_file_again() {
        let dir = tempfile::tempdir().unwrap();
        let host = Arc::new(crate::app::spotify_ui::tests::FakeHost::default());
        let mut app = started_on_a_broken_file_now_fixed(&dir, host);
        assert!(app.spotify.client_id.is_empty());
        assert_eq!(
            crate::spotify_rows::account_text(&app.spotify_view()),
            "Not configured"
        );

        press(&mut app, KeyCode::Char('S'));
        press(&mut app, KeyCode::Char('6'));

        let view = app.spotify_view();
        assert_eq!(view.client_id_head, &ID[..8]);
        assert_ne!(crate::spotify_rows::account_text(&view), "Not configured");
        assert!(
            app.config_problem.is_some(),
            "the problem stays until a restart"
        );
    }

    #[test]
    fn the_fixed_file_is_announced_once_per_run() {
        let dir = tempfile::tempdir().unwrap();
        let host = Arc::new(crate::app::spotify_ui::tests::FakeHost::default());
        let mut app = started_on_a_broken_file_now_fixed(&dir, host);
        let count = |app: &App| {
            toast_texts(app)
                .iter()
                .filter(|t| t.as_str() == FIXED_NOTICE)
                .count()
        };

        press(&mut app, KeyCode::Char('S'));
        press(&mut app, KeyCode::Char('6'));
        assert_eq!(count(&app), 1);

        press(&mut app, KeyCode::Char('S'));
        press(&mut app, KeyCode::Char('S'));
        assert_eq!(count(&app), 1);
    }

    #[test]
    fn after_the_file_is_fixed_a_port_change_is_still_refused_and_nothing_is_sent() {
        let dir = tempfile::tempdir().unwrap();
        let host = Arc::new(crate::app::spotify_ui::tests::FakeHost::default());
        let mut app = started_on_a_broken_file_now_fixed(&dir, host.clone());
        press(&mut app, KeyCode::Char('S'));
        press(&mut app, KeyCode::Char('6'));
        select(&mut app, Row::RedirectPort);
        press(&mut app, KeyCode::Enter);
        for _ in 0..4 {
            press(&mut app, KeyCode::Backspace);
        }
        type_text(&mut app, "9001");
        press(&mut app, KeyCode::Enter);

        assert_eq!(app.spotify.redirect_port, SpotifyConfig::DEFAULT_PORT);
        assert!(
            !settle(&app, &[&*host])
                .iter()
                .any(|op| matches!(op, SpotifyOp::Configure { .. }))
        );
    }

    #[test]
    fn the_first_save_creates_the_file_and_sends_what_the_player_already_has_plus_the_change() {
        // No config file yet: the app and the player both run on the defaults.
        let dir = tempfile::tempdir().unwrap();
        let host = Arc::new(crate::app::spotify_ui::tests::FakeHost::default());
        let mut app = settings_app(&dir);
        app.spotify_worker = Some(SpotifyWorker::spawn(host.clone()).unwrap());
        assert!(!dir.path().join("config.toml").exists());
        press(&mut app, KeyCode::Char('S'));
        press(&mut app, KeyCode::Char('6'));
        select(&mut app, Row::RedirectPort);
        press(&mut app, KeyCode::Enter);
        for _ in 0..4 {
            press(&mut app, KeyCode::Backspace);
        }
        type_text(&mut app, "9001");
        press(&mut app, KeyCode::Enter);
        assert_eq!(
            saved(&dir)["spotify"]["redirect_port"].as_integer(),
            Some(9001)
        );
        assert!(
            wait_for_op(&host, |op| matches!(
                op,
                SpotifyOp::Configure { config, .. }
                    if config.redirect_port == 9001 && config.client_id.is_empty() && config.device.is_empty()
            )),
            "{:?}",
            settle(&app, &[&*host])
        );
    }

    #[test]
    fn without_a_config_file_path_a_spotify_row_goes_back_and_nothing_is_sent() {
        let dir = tempfile::tempdir().unwrap();
        let host = Arc::new(crate::app::spotify_ui::tests::FakeHost::default());
        let mut app = spotify_app(&dir, host.clone());
        app.config_file = None;
        app.spotify_state.devices = one_device("Kitchen");
        press(&mut app, KeyCode::Char('S'));
        press(&mut app, KeyCode::Char('6'));
        select(&mut app, Row::PlaybackDevice);
        press(&mut app, KeyCode::Enter);
        assert!(app.spotify.device.is_empty());
        assert!(
            !settle(&app, &[&*host])
                .iter()
                .any(|op| matches!(op, SpotifyOp::Configure { .. }))
        );
        assert!(toast_texts(&app).iter().any(|t| t.starts_with("Not saved")));
    }

    /// Every Spotify change a user can make, one after the other.
    fn change_every_spotify_row(app: &mut App) {
        press(app, KeyCode::Char('S'));
        press(app, KeyCode::Char('6'));
        select(app, Row::OpenBrowser);
        press(app, KeyCode::Enter);
        select(app, Row::PlaybackDevice);
        press(app, KeyCode::Enter);
        select(app, Row::RedirectPort);
        press(app, KeyCode::Enter);
        for _ in 0..4 {
            press(app, KeyCode::Backspace);
        }
        type_text(app, "9001");
        press(app, KeyCode::Enter);
        select(app, Row::ClientId);
        press(app, KeyCode::Enter);
        for _ in 0..ID.len() {
            press(app, KeyCode::Backspace);
        }
        type_text(app, NEW_ID);
        press(app, KeyCode::Enter);
        // A set client ID always asks first, so the change waits for Enter.
        press(app, KeyCode::Enter);
    }

    fn kitchen() -> Vec<SpotifyDevice> {
        vec![SpotifyDevice {
            id: "d1".into(),
            name: "Kitchen".into(),
            kind: "Speaker".into(),
            is_active: false,
            volume_percent: None,
        }]
    }

    #[test]
    fn a_broken_config_never_changes_the_player_through_any_spotify_row() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "[spotify\n").unwrap();
        let host = Arc::new(crate::app::spotify_ui::tests::FakeHost::default());
        let mut app = spotify_app(&dir, host.clone())
            .with_config_file(ConfigFile::new(&path))
            .with_config_problem(ConfigProblem::Parse {
                message: "bad".into(),
                line: 1,
                column: 1,
            });
        app.spotify_state.devices = kitchen();
        change_every_spotify_row(&mut app);
        assert!(
            !settle(&app, &[&*host])
                .iter()
                .any(|op| matches!(op, SpotifyOp::Configure { .. })),
            "nothing reaches the player"
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "[spotify\n");
        assert_eq!(app.spotify.redirect_port, SpotifyConfig::DEFAULT_PORT);
        assert_eq!(
            app.spotify.client_id, ID,
            "the row shows what the player has"
        );
        assert!(app.spotify.device.is_empty());
        assert_eq!(app.open_browser, OpenBrowser::Auto);
    }

    #[test]
    fn a_clean_config_sends_one_configure_per_player_side_change_and_never_an_empty_id_unasked() {
        let dir = tempfile::tempdir().unwrap();
        // The file holds the ID the app shows, as a clean config does.
        std::fs::write(
            dir.path().join("config.toml"),
            format!("[spotify]\nclient_id = \"{ID}\"\n"),
        )
        .unwrap();
        let host = Arc::new(crate::app::spotify_ui::tests::FakeHost::default());
        let mut app = spotify_app(&dir, host.clone());
        app.spotify_state.devices = kitchen();
        change_every_spotify_row(&mut app);
        assert!(
            wait_for_op(&host, |op| matches!(
                op,
                SpotifyOp::Configure { config, .. } if config.client_id == NEW_ID
            )),
            "the last change never arrived: {:?}",
            settle(&app, &[&*host])
        );
        let configures: Vec<_> = settle(&app, &[&*host])
            .into_iter()
            .filter_map(|op| match op {
                SpotifyOp::Configure { config, .. } => Some(config),
                _ => None,
            })
            .collect();
        // Device, port, client ID: three saves. The browser setting is not sent.
        assert_eq!(configures.len(), 3, "{configures:?}");
        assert!(
            configures.iter().all(|c| !c.client_id.is_empty()),
            "an ID was never blanked"
        );
        assert_eq!(configures.last().unwrap().client_id, NEW_ID);
        assert_eq!(configures.last().unwrap().redirect_port, 9001);
        assert_eq!(configures.last().unwrap().device, "Kitchen");
    }

    // --- Spotify rows: client ID, logout question, Account ---

    const ID: &str = "0123456789abcdef0123456789abcdef";
    const NEW_ID: &str = "fedcba9876543210fedcba9876543210";

    #[test]
    fn a_client_id_that_is_not_32_hex_keeps_the_prompt_open_with_why() {
        let dir = tempfile::tempdir().unwrap();
        let host = Arc::new(crate::app::spotify_ui::tests::FakeHost::default());
        let mut app = spotify_app(&dir, host);
        press(&mut app, KeyCode::Char('S'));
        press(&mut app, KeyCode::Char('6'));
        select(&mut app, Row::ClientId);
        press(&mut app, KeyCode::Enter);
        for _ in 0..ID.len() {
            press(&mut app, KeyCode::Backspace);
        }
        type_text(&mut app, "xyz");
        press(&mut app, KeyCode::Enter);
        let prompt = app.settings_prompt.as_ref().expect("still open");
        assert_eq!(prompt.error.as_deref(), Some("32 hex characters"));
        assert_eq!(app.spotify.client_id, ID);
        assert!(!dir.path().join("config.toml").exists());
    }

    #[test]
    fn clearing_the_client_id_while_logged_in_asks_first_too() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("config.toml"),
            format!("[spotify]\nclient_id = \"{ID}\"\n"),
        )
        .unwrap();
        let host = Arc::new(crate::app::spotify_ui::tests::FakeHost::default());
        let mut app = spotify_app(&dir, host.clone());
        app.spotify_state.login = SpotifyLogin::LoggedIn;
        press(&mut app, KeyCode::Char('S'));
        press(&mut app, KeyCode::Char('6'));
        select(&mut app, Row::ClientId);
        press(&mut app, KeyCode::Enter);
        for _ in 0..ID.len() {
            press(&mut app, KeyCode::Backspace);
        }
        press(&mut app, KeyCode::Enter);
        assert_eq!(
            app.spotify_confirm,
            Some(SpotifyConfirm::ClientId(String::new()))
        );
        assert_eq!(app.spotify.client_id, ID);
        assert!(
            saved(&dir)["spotify"].get("client_id").is_some(),
            "the file still has the ID"
        );
        assert!(
            !settle(&app, &[&*host])
                .iter()
                .any(|op| matches!(op, SpotifyOp::Configure { .. }))
        );
    }

    #[test]
    fn an_empty_client_id_clears_the_key() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("config.toml"),
            format!("[spotify]\nclient_id = \"{ID}\"\n"),
        )
        .unwrap();
        let host = Arc::new(crate::app::spotify_ui::tests::FakeHost::default());
        let mut app = spotify_app(&dir, host.clone());
        press(&mut app, KeyCode::Char('S'));
        press(&mut app, KeyCode::Char('6'));
        select(&mut app, Row::ClientId);
        press(&mut app, KeyCode::Enter);
        for _ in 0..ID.len() {
            press(&mut app, KeyCode::Backspace);
        }
        press(&mut app, KeyCode::Enter);
        // Clearing a set ID asks first, whatever the login state is.
        assert_eq!(
            app.spotify_confirm,
            Some(SpotifyConfirm::ClientId(String::new()))
        );
        assert_eq!(app.spotify.client_id, ID, "nothing changes before Enter");
        assert!(
            saved(&dir)["spotify"].get("client_id").is_some(),
            "the file still has the ID"
        );
        assert!(
            !settle(&app, &[&*host])
                .iter()
                .any(|op| matches!(op, SpotifyOp::Configure { .. }))
        );
        press(&mut app, KeyCode::Enter);
        assert!(app.settings_prompt.is_none());
        assert!(app.spotify.client_id.is_empty());
        assert!(saved(&dir)["spotify"].get("client_id").is_none());
        assert!(
            wait_for_op(&host, |op| matches!(
                op,
                SpotifyOp::Configure { config, .. } if config.client_id.is_empty()
            )),
            "{:?}",
            settle(&app, &[&*host])
        );
    }

    #[test]
    fn changing_a_set_client_id_asks_first_even_before_the_login_is_known() {
        let dir = tempfile::tempdir().unwrap();
        let host = Arc::new(crate::app::spotify_ui::tests::FakeHost::default());
        let mut app = spotify_app(&dir, host.clone());
        // No poll has run: the login state is still the default.
        assert_eq!(app.spotify_state.login, SpotifyLogin::NotConfigured);
        press(&mut app, KeyCode::Char('S'));
        press(&mut app, KeyCode::Char('6'));
        select(&mut app, Row::ClientId);
        press(&mut app, KeyCode::Enter);
        for _ in 0..ID.len() {
            press(&mut app, KeyCode::Backspace);
        }
        type_text(&mut app, NEW_ID);
        press(&mut app, KeyCode::Enter);
        assert_eq!(
            app.spotify_confirm,
            Some(SpotifyConfirm::ClientId(NEW_ID.into()))
        );
        assert_eq!(app.spotify.client_id, ID);
        assert!(!dir.path().join("config.toml").exists());
        assert!(
            !settle(&app, &[&*host])
                .iter()
                .any(|op| matches!(op, SpotifyOp::Configure { .. }))
        );
    }

    #[test]
    fn setting_the_first_client_id_saves_without_a_question() {
        let dir = tempfile::tempdir().unwrap();
        let host = Arc::new(crate::app::spotify_ui::tests::FakeHost::default());
        let mut app = settings_app(&dir).with_settings_values(SettingsValues::default());
        app.spotify_worker = Some(SpotifyWorker::spawn(host.clone()).unwrap());
        press(&mut app, KeyCode::Char('S'));
        press(&mut app, KeyCode::Char('6'));
        select(&mut app, Row::ClientId);
        press(&mut app, KeyCode::Enter);
        type_text(&mut app, ID);
        press(&mut app, KeyCode::Enter);
        assert!(app.spotify_confirm.is_none());
        assert_eq!(app.spotify.client_id, ID);
        assert_eq!(saved(&dir)["spotify"]["client_id"].as_str(), Some(ID));
        assert!(
            wait_for_op(&host, |op| matches!(
                op,
                SpotifyOp::Configure { config, .. } if config.client_id == ID
            )),
            "{:?}",
            settle(&app, &[&*host])
        );
    }

    #[test]
    fn a_new_client_id_while_logged_in_asks_first_and_says_it_logs_out() {
        let dir = tempfile::tempdir().unwrap();
        let host = Arc::new(crate::app::spotify_ui::tests::FakeHost::default());
        let mut app = spotify_app(&dir, host.clone());
        app.spotify_state.login = SpotifyLogin::LoggedIn;
        press(&mut app, KeyCode::Char('S'));
        press(&mut app, KeyCode::Char('6'));
        select(&mut app, Row::ClientId);
        press(&mut app, KeyCode::Enter);
        for _ in 0..ID.len() {
            press(&mut app, KeyCode::Backspace);
        }
        type_text(&mut app, NEW_ID);
        press(&mut app, KeyCode::Enter);
        assert_eq!(
            app.spotify_confirm,
            Some(SpotifyConfirm::ClientId(NEW_ID.into()))
        );
        assert_eq!(app.spotify.client_id, ID, "nothing changes before Enter");
        assert!(
            !dir.path().join("config.toml").exists(),
            "nothing is written before Enter"
        );
        assert!(
            !settle(&app, &[&*host])
                .iter()
                .any(|op| matches!(op, SpotifyOp::Configure { .. }))
        );
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.spotify.client_id, NEW_ID);
        assert_eq!(saved(&dir)["spotify"]["client_id"].as_str(), Some(NEW_ID));
        assert!(
            wait_for_op(&host, |op| matches!(
                op,
                SpotifyOp::Configure { config, .. } if config.client_id == NEW_ID
            )),
            "{:?}",
            settle(&app, &[&*host])
        );
    }

    #[test]
    fn a_port_save_sends_the_client_id_the_file_has_not_the_stale_copy() {
        let dir = tempfile::tempdir().unwrap();
        let host = Arc::new(crate::app::spotify_ui::tests::FakeHost::default());
        let mut app = settings_app(&dir);
        app.spotify_worker = Some(SpotifyWorker::spawn(host.clone()).unwrap());
        // The TUI started with no ID; the file gained one since.
        std::fs::write(
            dir.path().join("config.toml"),
            format!("[spotify]\nclient_id = \"{ID}\"\n"),
        )
        .unwrap();
        press(&mut app, KeyCode::Char('S'));
        press(&mut app, KeyCode::Char('6'));
        select(&mut app, Row::RedirectPort);
        press(&mut app, KeyCode::Enter);
        for _ in 0..4 {
            press(&mut app, KeyCode::Backspace);
        }
        type_text(&mut app, "9001");
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.spotify.client_id, ID, "the screen shows the file");
        assert!(
            wait_for_op(&host, |op| matches!(
                op,
                SpotifyOp::Configure { config, replace_client_id: false }
                    if config.client_id == ID && config.redirect_port == 9001
            )),
            "{:?}",
            settle(&app, &[&*host])
        );
    }

    #[test]
    fn entering_a_client_id_asks_when_the_file_already_has_one() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("config.toml"),
            format!("[spotify]\nclient_id = \"{ID}\"\n"),
        )
        .unwrap();
        let host = Arc::new(crate::app::spotify_ui::tests::FakeHost::default());
        let mut app = settings_app(&dir);
        app.spotify_worker = Some(SpotifyWorker::spawn(host.clone()).unwrap());
        press(&mut app, KeyCode::Char('S'));
        press(&mut app, KeyCode::Char('6'));
        select(&mut app, Row::ClientId);
        press(&mut app, KeyCode::Enter);
        type_text(&mut app, NEW_ID);
        press(&mut app, KeyCode::Enter);
        assert_eq!(
            app.spotify_confirm,
            Some(SpotifyConfirm::ClientId(NEW_ID.into()))
        );
        assert!(app.spotify.client_id.is_empty());
        assert_eq!(saved(&dir)["spotify"]["client_id"].as_str(), Some(ID));
        assert!(
            !settle(&app, &[&*host])
                .iter()
                .any(|op| matches!(op, SpotifyOp::Configure { .. }))
        );
    }

    #[test]
    fn a_confirmed_client_id_change_sends_the_replace_flag() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("config.toml"),
            format!("[spotify]\nclient_id = \"{ID}\"\n"),
        )
        .unwrap();
        let host = Arc::new(crate::app::spotify_ui::tests::FakeHost::default());
        let mut app = spotify_app(&dir, host.clone());
        press(&mut app, KeyCode::Char('S'));
        press(&mut app, KeyCode::Char('6'));
        select(&mut app, Row::ClientId);
        press(&mut app, KeyCode::Enter);
        for _ in 0..ID.len() {
            press(&mut app, KeyCode::Backspace);
        }
        type_text(&mut app, NEW_ID);
        press(&mut app, KeyCode::Enter);
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.spotify.client_id, NEW_ID);
        assert!(
            wait_for_op(&host, |op| matches!(
                op,
                SpotifyOp::Configure { config, replace_client_id: true }
                    if config.client_id == NEW_ID
            )),
            "{:?}",
            settle(&app, &[&*host])
        );
    }

    /// Refuse the player's `Configure`s as a different client ID, or accept them.
    fn refuse_configures(host: &crate::app::spotify_ui::tests::FakeHost, refuse: bool) {
        *host.configure_refusal.lock().unwrap() =
            refuse.then_some(znicz_core::SpotifyErrorKind::ClientIdDiffers);
    }

    /// Type `port` into the redirect port prompt and submit it.
    fn change_port(app: &mut App, port: &str) {
        select(app, Row::RedirectPort);
        press(app, KeyCode::Enter);
        for _ in 0..app.spotify.redirect_port.to_string().len() {
            press(app, KeyCode::Backspace);
        }
        type_text(app, port);
        press(app, KeyCode::Enter);
    }

    /// Poll the Spotify worker until `done` holds, as the UI tick does.
    fn wait_for_player(app: &mut App, done: impl Fn(&App) -> bool) {
        for _ in 0..500 {
            app.poll_spotify();
            if done(app) {
                return;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        panic!("the player never answered the way the test needs");
    }

    /// A Settings tab with the file holding `ID`, and a port change the player
    /// refuses as another client ID.
    fn app_whose_player_refused_the_port(
        dir: &tempfile::TempDir,
        host: Arc<crate::app::spotify_ui::tests::FakeHost>,
    ) -> App {
        std::fs::write(
            dir.path().join("config.toml"),
            format!("[spotify]\nclient_id = \"{ID}\"\n"),
        )
        .unwrap();
        refuse_configures(&host, true);
        let mut app = spotify_app(dir, host);
        press(&mut app, KeyCode::Char('S'));
        press(&mut app, KeyCode::Char('6'));
        change_port(&mut app, "8899");
        wait_for_player(&mut app, |app| app.spotify_player_differs);
        app
    }

    #[test]
    fn a_refused_configure_says_the_player_uses_another_id() {
        let dir = tempfile::tempdir().unwrap();
        let host = Arc::new(crate::app::spotify_ui::tests::FakeHost::default());
        let mut app = app_whose_player_refused_the_port(&dir, host);
        assert!(toast_texts(&app).contains(&spotify_rows::PLAYER_DIFFERS_TOAST.to_string()));
        assert!(
            screen_rows(&mut app)
                .iter()
                .any(|row| row.contains(spotify_rows::PLAYER_DIFFERS_HINT)),
            "the Client ID row shows the hint"
        );
    }

    #[test]
    fn the_same_id_again_asks_and_confirming_sends_the_replace_flag() {
        let dir = tempfile::tempdir().unwrap();
        let host = Arc::new(crate::app::spotify_ui::tests::FakeHost::default());
        let mut app = app_whose_player_refused_the_port(&dir, host.clone());
        select(&mut app, Row::ClientId);
        press(&mut app, KeyCode::Enter);
        press(&mut app, KeyCode::Enter);
        assert_eq!(
            app.spotify_confirm,
            Some(SpotifyConfirm::ClientId(ID.into()))
        );
        refuse_configures(&host, false);
        press(&mut app, KeyCode::Enter);
        assert!(
            wait_for_op(&host, |op| matches!(
                op,
                SpotifyOp::Configure { config, replace_client_id: true }
                    if config.client_id == ID
            )),
            "{:?}",
            settle(&app, &[&*host])
        );
    }

    #[test]
    fn a_successful_configure_clears_the_hint() {
        let dir = tempfile::tempdir().unwrap();
        let host = Arc::new(crate::app::spotify_ui::tests::FakeHost::default());
        let mut app = app_whose_player_refused_the_port(&dir, host.clone());
        refuse_configures(&host, false);
        change_port(&mut app, "8900");
        wait_for_player(&mut app, |app| !app.spotify_player_differs);
        assert!(
            !screen_rows(&mut app)
                .iter()
                .any(|row| row.contains(spotify_rows::PLAYER_DIFFERS_HINT))
        );
    }

    #[test]
    fn the_strip_says_the_redirect_port_and_device_are_not_applied() {
        let dir = tempfile::tempdir().unwrap();
        let host = Arc::new(crate::app::spotify_ui::tests::FakeHost::default());
        let mut app = app_whose_player_refused_the_port(&dir, host);
        select(&mut app, Row::RedirectPort);
        assert!(
            screen_rows(&mut app)
                .iter()
                .any(|row| row.contains(spotify_rows::NOT_APPLIED))
        );
    }

    #[test]
    fn esc_cancels_a_pending_client_id_change() {
        let dir = tempfile::tempdir().unwrap();
        let host = Arc::new(crate::app::spotify_ui::tests::FakeHost::default());
        let mut app = spotify_app(&dir, host);
        app.spotify_state.login = SpotifyLogin::LoggedIn;
        press(&mut app, KeyCode::Char('S'));
        press(&mut app, KeyCode::Char('6'));
        app.spotify_confirm = Some(SpotifyConfirm::ClientId(NEW_ID.into()));
        press(&mut app, KeyCode::Esc);
        assert!(app.spotify_confirm.is_none());
        assert_eq!(app.spotify.client_id, ID);
        assert!(
            app.modal == Modal::Settings,
            "Esc only cancels the question"
        );
    }

    #[test]
    fn enter_on_account_without_a_client_id_does_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let host = Arc::new(crate::app::spotify_ui::tests::FakeHost::default());
        let mut app = settings_app(&dir);
        app.spotify_worker = Some(SpotifyWorker::spawn(host.clone()).unwrap());
        press(&mut app, KeyCode::Char('S'));
        press(&mut app, KeyCode::Char('6'));
        select(&mut app, Row::Account);
        press(&mut app, KeyCode::Enter);
        assert!(app.spotify_login.is_none());
        assert!(app.spotify_confirm.is_none());
        assert!(
            settle(&app, &[&*host])
                .iter()
                .all(|op| !matches!(op, SpotifyOp::BeginLogin))
        );
    }

    #[test]
    fn enter_on_account_when_logged_out_begins_a_login() {
        let dir = tempfile::tempdir().unwrap();
        let host = Arc::new(crate::app::spotify_ui::tests::FakeHost::default());
        let mut app = spotify_app(&dir, host.clone());
        app.spotify_state.login = SpotifyLogin::NotLoggedIn;
        press(&mut app, KeyCode::Char('S'));
        press(&mut app, KeyCode::Char('6'));
        select(&mut app, Row::Account);
        press(&mut app, KeyCode::Enter);
        assert!(wait_for_op(&host, |op| *op == SpotifyOp::BeginLogin));
    }

    #[test]
    fn enter_on_account_when_logged_in_asks_then_logs_out() {
        let dir = tempfile::tempdir().unwrap();
        let host = Arc::new(crate::app::spotify_ui::tests::FakeHost::default());
        let mut app = spotify_app(&dir, host.clone());
        app.spotify_state.login = SpotifyLogin::LoggedIn;
        press(&mut app, KeyCode::Char('S'));
        press(&mut app, KeyCode::Char('6'));
        select(&mut app, Row::Account);
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.spotify_confirm, Some(SpotifyConfirm::Logout));
        assert!(!settle(&app, &[&*host]).contains(&SpotifyOp::Logout));
        press(&mut app, KeyCode::Enter);
        assert!(wait_for_op(&host, |op| *op == SpotifyOp::Logout));
    }

    // --- Spotify tab: drawing ---

    #[test]
    fn the_spotify_tab_shows_its_five_rows_and_the_tab_label() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = settings_app(&dir);
        press(&mut app, KeyCode::Char('S'));
        press(&mut app, KeyCode::Char('6'));
        let screen = screen_rows(&mut app).join("\n");
        for text in [
            "Client ID",
            "Redirect port",
            "Open browser",
            "Account",
            "Playback device",
            "Spotify",
        ] {
            assert!(screen.contains(text), "{text} missing:\n{screen}");
        }
    }

    #[test]
    fn without_a_client_id_the_strip_points_at_it() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = settings_app(&dir);
        press(&mut app, KeyCode::Char('S'));
        press(&mut app, KeyCode::Char('6'));
        select(&mut app, Row::Account);
        let screen = screen_rows(&mut app).join("\n");
        assert!(screen.contains("Set a client ID first"), "{screen}");
    }

    #[test]
    fn the_login_panel_shows_the_url_and_the_keys_in_the_list_area() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = settings_app(&dir);
        press(&mut app, KeyCode::Char('S'));
        press(&mut app, KeyCode::Char('6'));
        app.spotify_login = Some(crate::app::LoginPanel {
            url: "https://accounts.spotify.com/authorize?client_id=0123".into(),
            listening: true,
            paste: None,
            notice: None,
        });
        let screen = screen_rows(&mut app).join("\n");
        assert!(
            screen.contains("Open this URL in any browser and approve:"),
            "{screen}"
        );
        assert!(
            screen.contains("accounts.spotify.com/authorize"),
            "{screen}"
        );
        assert!(screen.contains("Waiting for the browser"), "{screen}");
    }

    #[test]
    fn the_panel_draws_in_a_small_terminal_without_panicking() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = settings_app(&dir);
        press(&mut app, KeyCode::Char('S'));
        press(&mut app, KeyCode::Char('6'));
        app.spotify_login = Some(crate::app::LoginPanel {
            url: "https://accounts.spotify.com/authorize?".repeat(8),
            listening: false,
            paste: None,
            notice: Some("Could not open a browser: no opener.".into()),
        });
        let _ = screen_buffer(&mut app);
    }

    #[test]
    fn a_long_url_wraps_in_the_panel_and_none_of_it_is_lost() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = settings_app(&dir);
        press(&mut app, KeyCode::Char('S'));
        press(&mut app, KeyCode::Char('6'));
        let url = format!(
            "https://accounts.spotify.com/authorize?{}",
            "a1b2c3d4".repeat(12)
        );
        app.spotify_login = Some(crate::app::LoginPanel {
            url,
            listening: true,
            paste: None,
            notice: None,
        });
        let screen = screen_rows(&mut app);
        // The URL is one word longer than the popup is wide: it must run over
        // several rows (broken at the width), not be cut at the first.
        let rows_with_url = screen
            .iter()
            .filter(|row| row.contains("a1b2c3d4a1b2c3d4"))
            .count();
        assert!(rows_with_url >= 2, "{}", screen.join("\n"));
    }

    #[test]
    fn the_paste_prompt_shows_in_the_strip_with_the_reason_when_refused() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = settings_app(&dir);
        press(&mut app, KeyCode::Char('S'));
        press(&mut app, KeyCode::Char('6'));
        app.spotify_login = Some(crate::app::LoginPanel {
            url: "u".into(),
            listening: true,
            paste: Some(crate::app::SpotifyPaste {
                text: crate::line_edit::LineEdit::from_text("code=x"),
                error: Some("that address is not from this login".into()),
            }),
            notice: None,
        });
        let screen = screen_rows(&mut app).join("\n");
        assert!(screen.contains("Address: code=x"), "{screen}");
        assert!(
            screen.contains("that address is not from this login"),
            "{screen}"
        );
    }

    #[test]
    fn the_logout_question_and_the_client_id_question_show_in_the_strip() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = settings_app(&dir);
        press(&mut app, KeyCode::Char('S'));
        press(&mut app, KeyCode::Char('6'));
        app.spotify_confirm = Some(crate::app::SpotifyConfirm::Logout);
        assert!(
            screen_rows(&mut app)
                .join("\n")
                .contains("Log out of Spotify?")
        );
        app.spotify_confirm = Some(crate::app::SpotifyConfirm::ClientId("x".into()));
        assert!(
            screen_rows(&mut app)
                .join("\n")
                .contains("Changing the client ID logs you out")
        );
    }

    #[test]
    fn two_enters_on_account_before_the_reply_send_one_begin_login() {
        let dir = tempfile::tempdir().unwrap();
        let host = Arc::new(crate::app::spotify_ui::tests::FakeHost::default());
        let mut app = spotify_app(&dir, host.clone());
        app.spotify_state.login = SpotifyLogin::NotLoggedIn;
        press(&mut app, KeyCode::Char('S'));
        press(&mut app, KeyCode::Char('6'));
        select(&mut app, Row::Account);
        press(&mut app, KeyCode::Enter);
        press(&mut app, KeyCode::Enter);
        assert!(wait_for_op(&host, |op| *op == SpotifyOp::BeginLogin));
        let begins = settle(&app, &[&*host])
            .iter()
            .filter(|op| **op == SpotifyOp::BeginLogin)
            .count();
        assert_eq!(begins, 1);
    }

    #[test]
    fn three_switches_to_the_spotify_tab_while_devices_is_pending_send_one_request() {
        let dir = tempfile::tempdir().unwrap();
        let host = Arc::new(crate::app::spotify_ui::tests::FakeHost::default());
        let mut app = spotify_app(&dir, host.clone());
        press(&mut app, KeyCode::Char('S'));
        for _ in 0..3 {
            press(&mut app, KeyCode::Char('6'));
            press(&mut app, KeyCode::Char('1'));
        }
        assert!(wait_for_op(&host, |op| *op == SpotifyOp::Devices));
        let devices = settle(&app, &[&*host])
            .iter()
            .filter(|op| **op == SpotifyOp::Devices)
            .count();
        assert_eq!(devices, 1);
    }

    #[test]
    fn space_under_a_pending_client_id_question_opens_no_prompt_and_keeps_the_question() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = settings_app(&dir);
        press(&mut app, KeyCode::Char('S'));
        press(&mut app, KeyCode::Char('6'));
        select(&mut app, Row::ClientId);
        app.spotify_confirm = Some(SpotifyConfirm::ClientId("x".into()));
        press(&mut app, KeyCode::Char(' '));
        assert!(app.settings_prompt.is_none());
        assert_eq!(
            app.spotify_confirm,
            Some(SpotifyConfirm::ClientId("x".into()))
        );
    }

    #[test]
    fn a_login_panel_takes_the_list_clicks_but_the_tab_bar_stays_clickable() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = settings_app(&dir);
        press(&mut app, KeyCode::Char('S'));
        press(&mut app, KeyCode::Char('6'));
        app.spotify_login = Some(crate::app::LoginPanel {
            url: "u".into(),
            listening: true,
            paste: None,
            notice: None,
        });
        let _ = screen_rows(&mut app);
        assert!(
            app.hits.overlay_list.is_none(),
            "the rows under the panel are not clickable"
        );
        let (rect, _) = app
            .hits
            .settings_tabs
            .iter()
            .copied()
            .find(|(_, tab)| *tab == Tab::Audio)
            .expect("Audio has a label");
        click(&mut app, rect.x, rect.y);
        assert_eq!(app.settings_tab, Tab::Audio);
    }
}
