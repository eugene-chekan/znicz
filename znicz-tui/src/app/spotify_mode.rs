//! Spotify mode: `O` switches the keys, the footer and the status poll to
//! Spotify. A second `impl App` beside `spotify_ui.rs`.

use znicz_core::SpotifyLogin;

use super::{App, Modal};
use crate::spotify_mode;

impl App {
    /// Whether the keys, the footer and the status poll are Spotify's.
    pub(crate) fn in_spotify_mode(&self) -> bool {
        self.spotify_mode
    }

    /// `O`: enter or leave Spotify mode. Entering needs a login; without one the
    /// login line is toasted and nothing changes.
    pub(crate) fn toggle_spotify_mode(&mut self) {
        if self.in_spotify_mode() {
            self.leave_spotify_mode();
            return;
        }
        if self.spotify_state.login != SpotifyLogin::LoggedIn {
            self.toasts.info(spotify_mode::NOT_LOGGED_IN);
            return;
        }
        self.spotify_mode = true;
        // A stop from an old player is forgotten, and the next tick reads the status at once.
        self.spotify_unavailable = None;
        self.spotify_polled_at = None;
    }

    fn leave_spotify_mode(&mut self) {
        self.spotify_mode = false;
        self.spotify_browser.reset();
        self.spotify_search = None;
        if self.modal == Modal::SpotifyDevices {
            self.modal = Modal::None;
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use crossterm::event::KeyCode;
    use znicz_core::{SpotifyLogin, SpotifyState};

    use crate::app::settings_ui::tests as settings_tests;
    use crate::app::settings_ui::tests::press;
    use crate::app::{App, Modal, View};
    use crate::keys::{Context, context_hints};
    use crate::spotify_mode;

    fn logged_in(dir: &tempfile::TempDir) -> App {
        let mut app = settings_tests::settings_app(dir);
        app.spotify_state = SpotifyState {
            login: SpotifyLogin::LoggedIn,
            ..SpotifyState::default()
        };
        app
    }

    #[test]
    fn o_toggles_spotify_mode_from_browse_and_from_stage() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = logged_in(&dir);
        press(&mut app, KeyCode::Char('O'));
        assert!(app.in_spotify_mode());
        press(&mut app, KeyCode::Char('v'));
        assert_eq!(app.view, View::Stage);
        assert!(app.in_spotify_mode());
        press(&mut app, KeyCode::Char('O'));
        assert!(!app.in_spotify_mode());
    }

    #[test]
    fn o_without_login_toasts_the_login_line_and_stays_out() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = settings_tests::settings_app(&dir);
        press(&mut app, KeyCode::Char('O'));
        assert!(!app.in_spotify_mode());
        assert!(
            settings_tests::toast_texts(&app)
                .iter()
                .any(|t| t == spotify_mode::NOT_LOGGED_IN)
        );
    }

    #[test]
    fn v_and_i_still_switch_views_inside_the_mode() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = logged_in(&dir);
        press(&mut app, KeyCode::Char('O'));
        press(&mut app, KeyCode::Char('v'));
        assert_eq!(app.view, View::Stage);
        press(&mut app, KeyCode::Char('i'));
        assert_eq!(app.view, View::Browse);
        assert!(app.in_spotify_mode());
    }

    #[test]
    fn entering_the_mode_clears_the_unavailable_stop() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = logged_in(&dir);
        app.spotify_unavailable = Some("old".into());
        app.spotify_polled_at = Some(Instant::now());
        press(&mut app, KeyCode::Char('O'));
        assert!(app.spotify_unavailable.is_none());
        assert!(app.spotify_polled_at.is_none());
    }

    #[test]
    fn o_does_nothing_while_settings_is_open() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = logged_in(&dir);
        press(&mut app, KeyCode::Char('S'));
        press(&mut app, KeyCode::Char('O'));
        assert!(!app.in_spotify_mode());
        assert_eq!(app.modal, Modal::Settings);
    }

    #[test]
    fn the_context_is_spotify_browse_or_stage_in_the_mode() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = logged_in(&dir);
        press(&mut app, KeyCode::Char('O'));
        assert_eq!(app.context(), Context::SpotifyBrowse);
        press(&mut app, KeyCode::Char('v'));
        assert_eq!(app.context(), Context::SpotifyStage);
        press(&mut app, KeyCode::Char('O'));
        assert_eq!(app.context(), Context::Stage);
    }

    #[test]
    fn the_footer_ends_in_o_znicz_in_the_mode() {
        for context in [Context::SpotifyStage, Context::SpotifyBrowse] {
            assert_eq!(
                context_hints(context).last().unwrap().label,
                "O znicz",
                "{context:?}"
            );
        }
    }
}
