//! The Spotify browser's App side: its keys, the loads and their replies, and
//! Enter, which plays. A third `impl App` beside `spotify_mode.rs`.

use crossterm::event::{KeyCode, KeyEvent};
use znicz_core::{SpotifyErrorKind, SpotifyOp, SpotifyReply, ZniczError};

use super::{App, HALF_PAGE, Modal, View};
use crate::spotify_browse::{BrowseAction, BrowseList, offset_of};
use crate::spotify_mode;
use crate::spotify_worker::{Done, SpotifyAsk};

impl App {
    /// Whether the browser owns the keys: Spotify mode, the Browse view, no modal.
    pub(crate) fn in_spotify_browser(&self) -> bool {
        self.spotify_mode && self.modal == Modal::None && self.view == View::Browse
    }

    /// The browser's keys. True when the key was the browser's.
    pub(crate) fn spotify_browser_key(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Char('j') | KeyCode::Down => self.step(1),
            KeyCode::Char('k') | KeyCode::Up => self.step(-1),
            KeyCode::Char('g') | KeyCode::Home => self.go_first(),
            KeyCode::Char('G') | KeyCode::End => self.go_last(),
            KeyCode::PageDown => self.page(HALF_PAGE * 2),
            KeyCode::PageUp => self.page(-HALF_PAGE * 2),
            KeyCode::Enter => self.spotify_enter(false),
            KeyCode::Char('l') => self.spotify_enter(true),
            KeyCode::Char('h') => {
                self.spotify_back();
            }
            KeyCode::Char('r') => self.spotify_reload(),
            // One column: Tab has nowhere to go.
            KeyCode::Tab | KeyCode::BackTab => {}
            _ => return false,
        }
        true
    }

    /// Enter (`open` false) or `l` (`open` true) at the cursor.
    pub(crate) fn spotify_enter(&mut self, open: bool) {
        let action = if open {
            self.spotify_browser.open()
        } else {
            self.spotify_browser.enter()
        };
        match action {
            BrowseAction::Play(op) => self.spotify_send_play(op),
            BrowseAction::Load {
                generation,
                list,
                op,
                ..
            } => self.spotify_load(generation, list, op),
            // The Search row opens the prompt (Task 12).
            BrowseAction::OpenSearch | BrowseAction::Nothing => {}
        }
    }

    /// `h`: back one level. False at the top.
    pub(crate) fn spotify_back(&mut self) -> bool {
        self.spotify_browser.back()
    }

    /// `r`: the current list again from its first page.
    fn spotify_reload(&mut self) {
        if let Some((generation, list, op)) = self.spotify_browser.reload() {
            self.spotify_load(generation, list, op);
        }
    }

    /// Send a browse request for `list` at `generation`. The offset is the op's.
    pub(crate) fn spotify_load(&mut self, generation: u64, list: BrowseList, op: SpotifyOp) {
        let offset = offset_of(&op);
        if !self.ask_spotify(
            SpotifyAsk::Browse {
                generation,
                list,
                offset,
            },
            op,
        ) {
            self.toasts.error(spotify_mode::NO_LANE);
        }
    }

    /// Send a playback op on the transport lane.
    pub(crate) fn spotify_send_play(&mut self, op: SpotifyOp) {
        if !self.ask_spotify(SpotifyAsk::Transport, op) {
            self.toasts.error(spotify_mode::NO_LANE);
        }
    }

    /// A browse reply, routed to the browser by its generation. An error that
    /// names the list (`unavailable`) becomes the level's notice; any other
    /// error is a toast. A reply for a list the user has left changes nothing.
    pub(crate) fn on_browse_done(&mut self, done: Done<SpotifyAsk>) {
        let SpotifyAsk::Browse {
            generation,
            list,
            offset,
        } = done.tag
        else {
            return;
        };
        match done.result {
            Ok(SpotifyReply::Page { page }) => {
                self.spotify_browser
                    .apply_page(generation, &list, offset, &page);
            }
            Err(ZniczError::Spotify {
                kind: SpotifyErrorKind::Unavailable,
                message,
            }) => {
                self.spotify_browser.apply_error(generation, &list, message);
            }
            Err(e) => {
                let text = match &e {
                    ZniczError::Spotify { kind, message } => {
                        spotify_mode::error_plan(*kind, message).text
                    }
                    other => other.to_string(),
                };
                if self.spotify_browser.apply_failure(generation, &list) {
                    self.toasts.error(text);
                }
            }
            // Any other reply to a browse request is not a list: it is settled, not shown.
            Ok(_) => {
                self.spotify_browser.apply_failure(generation, &list);
            }
        }
    }

    /// The tick: ask for the next page when the cursor is near the end of a list.
    pub(crate) fn request_more_rows(&mut self) {
        if !self.in_spotify_mode() {
            return;
        }
        if let Some((generation, list, op)) = self.spotify_browser.next_page() {
            self.spotify_load(generation, list, op);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use crossterm::event::KeyCode;
    use znicz_core::{
        SpotifyErrorKind, SpotifyHost, SpotifyItem, SpotifyKind, SpotifyLogin, SpotifyOp,
        SpotifyPage, SpotifyReply, SpotifyState, ZniczError,
    };

    use crate::app::App;
    use crate::app::settings_ui::tests as settings_tests;
    use crate::app::settings_ui::tests::press;
    use crate::app::spotify_ui::tests::{FakeHost, sent_ops, wait_for_op, wait_until};
    use crate::spotify_browse::{BrowseList, PAGE};
    use crate::spotify_mode;
    use crate::spotify_worker::{Done, SpotifyAsk};

    /// Answers the ops `matches` picks with one error, and the rest as `inner` does.
    struct FailOn {
        inner: Arc<FakeHost>,
        matches: fn(&SpotifyOp) -> bool,
        kind: SpotifyErrorKind,
        message: &'static str,
    }

    impl SpotifyHost for FailOn {
        fn apply(&self, op: SpotifyOp) -> znicz_core::Result<SpotifyReply> {
            if (self.matches)(&op) {
                self.inner.ops.lock().unwrap().push(op);
                return Err(ZniczError::Spotify {
                    kind: self.kind,
                    message: self.message.into(),
                });
            }
            self.inner.apply(op)
        }
        fn state(&self) -> SpotifyState {
            self.inner.state()
        }
    }

    fn logged_in() -> SpotifyState {
        SpotifyState {
            login: SpotifyLogin::LoggedIn,
            ..SpotifyState::default()
        }
    }

    fn playlist(n: u32) -> SpotifyItem {
        SpotifyItem {
            kind: SpotifyKind::Playlist,
            uri: format!("spotify:playlist:{n}"),
            name: format!("Playlist {n}"),
            subtitle: "Eugene".into(),
            image_url: None,
            track: None,
        }
    }

    fn page(items: Vec<SpotifyItem>, total: u32) -> SpotifyReply {
        SpotifyReply::Page {
            page: SpotifyPage {
                items,
                offset: 0,
                total,
            },
        }
    }

    /// Spotify mode on, logged in, with the browse and transport lanes on the given hosts.
    fn browse_app(
        dir: &tempfile::TempDir,
        browse: Arc<dyn SpotifyHost>,
        transport: Arc<dyn SpotifyHost>,
    ) -> App {
        let mut app = settings_tests::settings_app(dir)
            .with_spotify_worker(browse.clone())
            .unwrap()
            .with_spotify_transport(transport)
            .unwrap();
        app.spotify_main = Some(browse);
        app.spotify_state = logged_in();
        press(&mut app, KeyCode::Char('O'));
        assert!(app.in_spotify_mode());
        app
    }

    /// Spotify mode with one playlist on the browse host's page, and the top's Playlists open.
    fn playlists_open(dir: &tempfile::TempDir, browse: &Arc<FakeHost>) -> App {
        *browse.reply.lock().unwrap() = Some(page(vec![playlist(1)], 1));
        *browse.state.lock().unwrap() = Some(logged_in());
        let transport = Arc::new(FakeHost::default());
        let mut app = browse_app(dir, browse.clone(), transport);
        press(&mut app, KeyCode::Char('j'));
        press(&mut app, KeyCode::Char('l'));
        assert!(wait_for_op(browse, |op| matches!(
            op,
            SpotifyOp::Playlists { .. }
        )));
        wait_until(&mut app, |app| app.spotify_browser.level().items.len() == 1);
        app
    }

    #[test]
    fn entering_playlists_sends_the_first_page_and_fills_the_list_when_it_arrives() {
        let dir = tempfile::tempdir().unwrap();
        let browse = Arc::new(FakeHost::default());
        *browse.reply.lock().unwrap() = Some(page(vec![playlist(1), playlist(2)], 2));
        let transport = Arc::new(FakeHost::default());
        let mut app = browse_app(&dir, browse.clone(), transport);
        press(&mut app, KeyCode::Char('j'));
        press(&mut app, KeyCode::Char('l'));
        assert!(wait_for_op(&browse, |op| *op
            == SpotifyOp::Playlists {
                offset: 0,
                limit: PAGE
            }));
        wait_until(&mut app, |app| app.spotify_browser.level().items.len() == 2);
    }

    #[test]
    fn a_late_browse_reply_after_leaving_is_dropped_in_the_app() {
        let dir = tempfile::tempdir().unwrap();
        let browse = Arc::new(FakeHost::default());
        let mut app = playlists_open(&dir, &browse);
        let current = app.spotify_browser.generation();
        // A reply for the current generation lands: the positive control.
        app.on_spotify_done(Done {
            tag: SpotifyAsk::Browse {
                generation: current,
                list: BrowseList::Playlists,
                offset: 0,
            },
            result: Ok(page(vec![playlist(7), playlist(8)], 2)),
        });
        assert!(
            app.spotify_browser
                .level()
                .items
                .iter()
                .any(|item| item.name == "Playlist 7")
        );
        // Leave the mode and come back: the Playlists level is gone.
        press(&mut app, KeyCode::Char('O'));
        press(&mut app, KeyCode::Char('O'));
        assert!(app.in_spotify_mode());
        app.on_spotify_done(Done {
            tag: SpotifyAsk::Browse {
                generation: current,
                list: BrowseList::Playlists,
                offset: 0,
            },
            result: Ok(page(vec![playlist(9)], 1)),
        });
        assert!(app.spotify_browser.level().items.is_empty());
        assert_eq!(app.spotify_browser.depth(), 1);
    }

    #[test]
    fn enter_plays_a_playlist_from_the_top_and_sends_play_on_the_transport_lane() {
        let dir = tempfile::tempdir().unwrap();
        let browse = Arc::new(FakeHost::default());
        let transport = Arc::new(FakeHost::default());
        *browse.reply.lock().unwrap() = Some(page(vec![playlist(1)], 1));
        let mut app = browse_app(&dir, browse.clone(), transport.clone());
        press(&mut app, KeyCode::Char('j'));
        press(&mut app, KeyCode::Char('l'));
        wait_until(&mut app, |app| app.spotify_browser.level().items.len() == 1);
        press(&mut app, KeyCode::Enter);
        assert!(wait_for_op(&transport, |op| *op
            == SpotifyOp::Play {
                context_uri: Some("spotify:playlist:1".into()),
                uris: Vec::new(),
                offset: None,
            }));
        assert!(
            sent_ops(&browse)
                .iter()
                .all(|op| !matches!(op, SpotifyOp::Play { .. }))
        );
    }

    #[test]
    fn l_on_a_playlist_opens_its_tracks_and_an_unavailable_reply_shows_a_notice() {
        const NOTICE: &str = "Spotify only lists the tracks of playlists you own or collaborate on; Enter still plays it";
        let dir = tempfile::tempdir().unwrap();
        let inner = Arc::new(FakeHost::default());
        *inner.reply.lock().unwrap() = Some(page(vec![playlist(1)], 1));
        let browse = Arc::new(FailOn {
            inner: inner.clone(),
            matches: |op| matches!(op, SpotifyOp::Tracks { .. }),
            kind: SpotifyErrorKind::Unavailable,
            message: NOTICE,
        });
        let mut app = browse_app(&dir, browse, Arc::new(FakeHost::default()));
        press(&mut app, KeyCode::Char('j'));
        press(&mut app, KeyCode::Char('l'));
        wait_until(&mut app, |app| app.spotify_browser.level().items.len() == 1);
        press(&mut app, KeyCode::Char('l'));
        assert!(wait_for_op(&inner, |op| *op
            == SpotifyOp::Tracks {
                uri: "spotify:playlist:1".into(),
                offset: 0,
                limit: PAGE,
            }));
        wait_until(&mut app, |app| app.spotify_browser.level().notice.is_some());
        assert_eq!(app.spotify_browser.level().notice.as_deref(), Some(NOTICE));
        assert_eq!(app.spotify_browser.level().pending, 0);
    }

    #[test]
    fn h_goes_back_one_level_and_never_past_the_top() {
        let dir = tempfile::tempdir().unwrap();
        let browse = Arc::new(FakeHost::default());
        let mut app = playlists_open(&dir, &browse);
        assert_eq!(app.spotify_browser.depth(), 2);
        press(&mut app, KeyCode::Char('h'));
        assert_eq!(app.spotify_browser.depth(), 1);
        press(&mut app, KeyCode::Char('h'));
        assert_eq!(app.spotify_browser.depth(), 1);
    }

    #[test]
    fn a_browse_error_other_than_unavailable_is_a_toast_and_the_list_stays() {
        let dir = tempfile::tempdir().unwrap();
        let inner = Arc::new(FakeHost::default());
        *inner.reply.lock().unwrap() = Some(page(vec![playlist(1)], 1));
        let browse = Arc::new(FailOn {
            inner: inner.clone(),
            matches: |op| matches!(op, SpotifyOp::Playlists { .. }),
            kind: SpotifyErrorKind::Network,
            message: "timed out",
        });
        let mut app = browse_app(&dir, browse, Arc::new(FakeHost::default()));
        press(&mut app, KeyCode::Char('j'));
        press(&mut app, KeyCode::Char('l'));
        assert!(wait_for_op(&inner, |op| matches!(
            op,
            SpotifyOp::Playlists { .. }
        )));
        wait_until(&mut app, |app| {
            settings_tests::toast_texts(app)
                .iter()
                .any(|t| t == spotify_mode::NETWORK_TEXT)
        });
        assert_eq!(app.spotify_browser.level().pending, 0);
        assert!(app.spotify_browser.level().notice.is_none());
    }

    #[test]
    fn a_transport_error_is_toasted_with_its_own_message() {
        const PREMIUM: &str = "Spotify playback needs Spotify Premium";
        let dir = tempfile::tempdir().unwrap();
        let browse = Arc::new(FakeHost::default());
        *browse.reply.lock().unwrap() = Some(page(vec![playlist(1)], 1));
        let transport = Arc::new(FailOn {
            inner: Arc::new(FakeHost::default()),
            matches: |op| matches!(op, SpotifyOp::Play { .. }),
            kind: SpotifyErrorKind::PremiumRequired,
            message: PREMIUM,
        });
        let mut app = browse_app(&dir, browse, transport);
        press(&mut app, KeyCode::Char('j'));
        press(&mut app, KeyCode::Char('l'));
        wait_until(&mut app, |app| app.spotify_browser.level().items.len() == 1);
        press(&mut app, KeyCode::Enter);
        wait_until(&mut app, |app| {
            settings_tests::toast_texts(app)
                .iter()
                .any(|t| t == PREMIUM)
        });
    }

    #[test]
    fn r_reloads_the_list_from_page_zero() {
        let dir = tempfile::tempdir().unwrap();
        let browse = Arc::new(FakeHost::default());
        let mut app = playlists_open(&dir, &browse);
        press(&mut app, KeyCode::Char('r'));
        let asks = sent_ops(&browse)
            .iter()
            .filter(|op| matches!(op, SpotifyOp::Playlists { offset: 0, .. }))
            .count();
        assert_eq!(asks, 2);
        assert_eq!(app.spotify_browser.depth(), 2);
    }

    #[test]
    fn the_next_page_is_asked_for_near_the_end() {
        let dir = tempfile::tempdir().unwrap();
        let browse = Arc::new(FakeHost::default());
        let fifty: Vec<SpotifyItem> = (1..=50).map(playlist).collect();
        *browse.reply.lock().unwrap() = Some(page(fifty, 100));
        let transport = Arc::new(FakeHost::default());
        let mut app = browse_app(&dir, browse.clone(), transport);
        press(&mut app, KeyCode::Char('j'));
        press(&mut app, KeyCode::Char('l'));
        wait_until(&mut app, |app| {
            app.spotify_browser.level().items.len() == 50
        });
        press(&mut app, KeyCode::Char('G'));
        app.request_more_rows();
        assert!(wait_for_op(&browse, |op| *op
            == SpotifyOp::Playlists {
                offset: 50,
                limit: PAGE,
            }));
    }

    #[test]
    fn q_and_capital_s_still_work_in_spotify_mode() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = browse_app(
            &dir,
            Arc::new(FakeHost::default()),
            Arc::new(FakeHost::default()),
        );
        press(&mut app, KeyCode::Char('S'));
        assert_eq!(app.modal, crate::app::Modal::Settings);
        press(&mut app, KeyCode::Esc);
        press(&mut app, KeyCode::Char('q'));
        assert!(app.should_quit);
    }
}
