//! The Spotify browser's App side: its keys, the loads and their replies, and
//! Enter, which plays. A third `impl App` beside `spotify_mode.rs`.

use crossterm::event::{KeyCode, KeyEvent};
use znicz_core::{SpotifyErrorKind, SpotifyOp, SpotifyReply, ZniczError};

use super::{App, HALF_PAGE, Modal, View};
use crate::line_edit::LineEdit;
use crate::spotify_browse::{BrowseAction, BrowseList, offset_of};
use crate::spotify_mode;
use crate::spotify_worker::{Done, SpotifyAsk};

impl App {
    /// Whether the browser owns the keys: Spotify mode, the Browse view, no modal,
    /// and no queue drawer open. The open drawer is read-only, so none of these
    /// keys reach the browser or the Znicz queue until it closes.
    pub(crate) fn in_spotify_browser(&self) -> bool {
        self.spotify_mode
            && self.modal == Modal::None
            && self.view == View::Browse
            && !self.queue_open
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
            KeyCode::Char('/') => self.spotify_search = Some(LineEdit::new()),
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
            BrowseAction::Play(op) => {
                self.spotify_send_play(op);
            }
            BrowseAction::Load {
                request, list, op, ..
            } => self.spotify_load(request, list, op),
            // The Search row opens the prompt, as `/` does.
            BrowseAction::OpenSearch => self.spotify_search = Some(LineEdit::new()),
            BrowseAction::Nothing => {}
        }
    }

    /// `h`: back one level. False at the top.
    pub(crate) fn spotify_back(&mut self) -> bool {
        self.spotify_browser.back()
    }

    /// The search prompt takes every key: it is typed text. Enter searches (an
    /// empty query sends nothing and keeps the prompt open); Esc closes it.
    pub(crate) fn on_spotify_search_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Enter => self.submit_search(),
            KeyCode::Esc => self.spotify_search = None,
            _ => {
                if let Some(prompt) = self.spotify_search.as_mut() {
                    prompt.on_key(key);
                }
            }
        }
    }

    /// Search for the prompt's text: one request per group, all at offset 0.
    fn submit_search(&mut self) {
        let Some(prompt) = self.spotify_search.as_ref() else {
            return;
        };
        let query = prompt.as_str().trim().to_string();
        if query.is_empty() {
            return;
        }
        self.spotify_search = None;
        for (request, list, op) in self.spotify_browser.start_search(&query) {
            self.spotify_load(request, list, op);
        }
    }

    /// `r`: the current list again from its first page.
    fn spotify_reload(&mut self) {
        if let Some((request, list, op)) = self.spotify_browser.reload() {
            self.spotify_load(request, list, op);
        }
    }

    /// A click on a browser row: the cursor moves there. Nothing plays or loads;
    /// Enter or `l` does that. True when the click was on the browser's rows.
    pub(crate) fn spotify_row_clicked(&mut self, column: u16, row: u16) -> bool {
        let Some(index) = self
            .hits
            .spotify_rows
            .and_then(|hit| hit.row_at(column, row))
        else {
            return false;
        };
        self.spotify_browser.select_row(index);
        true
    }

    /// Send a browse request for `list` at `request`. The offset is the op's.
    pub(crate) fn spotify_load(&mut self, request: u64, list: BrowseList, op: SpotifyOp) {
        let offset = offset_of(&op);
        if !self.ask_spotify(
            SpotifyAsk::Browse {
                request,
                list,
                offset,
            },
            op,
        ) {
            // The request never left, so no reply will settle the level: it is
            // not loading, and a reload can send again once a lane exists.
            self.spotify_browser.apply_failure(request);
            self.toasts.error(spotify_mode::NO_LANE);
        }
    }

    /// Send a playback op on the transport lane. False (with a toast) when that lane is not running.
    pub(crate) fn spotify_send_play(&mut self, op: SpotifyOp) -> bool {
        let sent = self.ask_spotify(SpotifyAsk::Transport, op);
        if !sent {
            self.toasts.error(spotify_mode::NO_LANE);
        }
        sent
    }

    /// A browse reply, routed to the browser by its request. An error that
    /// names the list (`unavailable`) becomes the level's notice; any other
    /// error is a toast. A reply for a list the user has left changes nothing.
    pub(crate) fn on_browse_done(&mut self, done: Done<SpotifyAsk>) {
        let SpotifyAsk::Browse {
            request,
            list,
            offset,
        } = done.tag
        else {
            return;
        };
        match done.result {
            Ok(SpotifyReply::Page { page }) => {
                self.spotify_browser
                    .apply_page(request, &list, offset, &page);
            }
            Err(ZniczError::Spotify {
                kind: SpotifyErrorKind::Unavailable,
                message,
            }) => {
                self.spotify_browser.apply_error(request, message);
            }
            Err(e) => {
                let text = match &e {
                    ZniczError::Spotify { kind, message } => {
                        spotify_mode::error_plan(*kind, message).text
                    }
                    other => other.to_string(),
                };
                if self.spotify_browser.apply_failure(request) {
                    self.toasts.error(text);
                }
            }
            // Any other reply to a browse request is not a list: it is settled, not shown.
            Ok(_) => {
                self.spotify_browser.apply_failure(request);
            }
        }
    }

    /// The tick: ask for the next page when the cursor is near the end of a list.
    pub(crate) fn request_more_rows(&mut self) {
        if !self.in_spotify_mode() {
            return;
        }
        if let Some((request, list, op)) = self.spotify_browser.next_page() {
            self.spotify_load(request, list, op);
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

    use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use crate::app::App;
    use crate::app::settings_ui::tests as settings_tests;
    use crate::app::settings_ui::tests::press;
    use crate::app::spotify_ui::tests::{
        FakeHost, settle, wait_for_count, wait_for_op, wait_until,
    };
    use crate::spotify_browse::{BrowseAction, PAGE, Row, SEARCH_LIMIT};
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
            position: None,
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
                next_offset: None,
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
        playlists_open_with(dir, browse, Arc::new(FakeHost::default()))
    }

    /// `playlists_open`, with the transport lane on a host the test keeps.
    fn playlists_open_with(
        dir: &tempfile::TempDir,
        browse: &Arc<FakeHost>,
        transport: Arc<FakeHost>,
    ) -> App {
        *browse.reply.lock().unwrap() = Some(page(vec![playlist(1)], 1));
        *browse.state.lock().unwrap() = Some(logged_in());
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
        let mut app = browse_app(&dir, browse.clone(), transport.clone());
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
        let mut app = browse_app(&dir, browse.clone(), Arc::new(FakeHost::default()));
        press(&mut app, KeyCode::Char('j'));
        let BrowseAction::Load { request, list, .. } = app.spotify_browser.enter() else {
            panic!("Playlists loads");
        };
        // A reply for the level's request lands: the positive control.
        app.on_spotify_done(Done {
            tag: SpotifyAsk::Browse {
                request,
                list: list.clone(),
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
                request,
                list,
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
        // Only the transport lane played: exactly one Play, on the transport host.
        assert_eq!(
            settle(&app, &[&*browse, &*transport])
                .iter()
                .filter(|op| matches!(op, SpotifyOp::Play { .. }))
                .count(),
            1
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
        assert_eq!(app.spotify_browser.level().pending(), 0);
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
        assert_eq!(app.spotify_browser.level().pending(), 0);
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
        let transport = Arc::new(FakeHost::default());
        let mut app = playlists_open_with(&dir, &browse, transport.clone());
        press(&mut app, KeyCode::Char('r'));
        // The reload asks for page zero again: wait for the second request to arrive.
        let asked = |op: &SpotifyOp| matches!(op, SpotifyOp::Playlists { offset: 0, .. });
        assert_eq!(wait_for_count(&browse, asked, 2), 2);
        let asks = settle(&app, &[&*browse, &*transport])
            .iter()
            .filter(|op| asked(op))
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
        let mut app = browse_app(&dir, browse.clone(), transport.clone());
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

    /// Answers each search with one item of the kind it asked for.
    struct KindsHost {
        inner: Arc<FakeHost>,
    }

    impl SpotifyHost for KindsHost {
        fn apply(&self, op: SpotifyOp) -> znicz_core::Result<SpotifyReply> {
            if let SpotifyOp::Search { kinds, .. } = &op {
                let kind = kinds[0];
                self.inner.ops.lock().unwrap().push(op.clone());
                let item = SpotifyItem {
                    position: None,
                    kind,
                    uri: format!("spotify:{}:found", kind.as_api()),
                    name: format!("{} found", kind.as_api()),
                    subtitle: "Miles Davis".into(),
                    image_url: None,
                    track: None,
                };
                return Ok(page(vec![item], 1));
            }
            self.inner.apply(op)
        }
        fn state(&self) -> SpotifyState {
            self.inner.state()
        }
    }

    fn type_text(app: &mut App, text: &str) {
        for c in text.chars() {
            press(app, KeyCode::Char(c));
        }
    }

    fn search_op(query: &str, kind: SpotifyKind) -> SpotifyOp {
        SpotifyOp::Search {
            query: query.into(),
            kinds: vec![kind],
            offset: 0,
            limit: SEARCH_LIMIT,
        }
    }

    #[test]
    fn slash_opens_the_prompt_and_typing_fills_it_without_searching() {
        let dir = tempfile::tempdir().unwrap();
        let browse = Arc::new(FakeHost::default());
        let transport = Arc::new(FakeHost::default());
        let mut app = browse_app(&dir, browse.clone(), transport.clone());
        press(&mut app, KeyCode::Char('/'));
        type_text(&mut app, "kind");
        assert_eq!(app.spotify_search.as_ref().unwrap().as_str(), "kind");
        assert!(settle(&app, &[&*browse, &*transport]).is_empty());
    }

    #[test]
    fn enter_searches_four_kinds_at_ten_each() {
        let dir = tempfile::tempdir().unwrap();
        let browse = Arc::new(FakeHost::default());
        let mut app = browse_app(&dir, browse.clone(), Arc::new(FakeHost::default()));
        press(&mut app, KeyCode::Char('/'));
        type_text(&mut app, "kind of blue");
        press(&mut app, KeyCode::Enter);
        for kind in [
            SpotifyKind::Track,
            SpotifyKind::Album,
            SpotifyKind::Artist,
            SpotifyKind::Playlist,
        ] {
            assert!(
                wait_for_op(&browse, |op| *op == search_op("kind of blue", kind)),
                "no search for {kind:?}"
            );
        }
        assert!(app.spotify_search.is_none());
        assert_eq!(app.spotify_browser.depth(), 2);
    }

    #[test]
    fn an_empty_search_sends_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let browse = Arc::new(FakeHost::default());
        let transport = Arc::new(FakeHost::default());
        let mut app = browse_app(&dir, browse.clone(), transport.clone());
        press(&mut app, KeyCode::Char('/'));
        type_text(&mut app, "   ");
        press(&mut app, KeyCode::Enter);
        assert!(settle(&app, &[&*browse, &*transport]).is_empty());
        assert!(app.spotify_search.is_some(), "the prompt stays open");
    }

    #[test]
    fn esc_closes_the_prompt_and_keeps_the_browser() {
        let dir = tempfile::tempdir().unwrap();
        let browse = Arc::new(FakeHost::default());
        let mut app = playlists_open(&dir, &browse);
        press(&mut app, KeyCode::Char('/'));
        type_text(&mut app, "x");
        press(&mut app, KeyCode::Esc);
        assert!(app.spotify_search.is_none());
        assert_eq!(app.spotify_browser.depth(), 2);
    }

    #[test]
    fn search_results_arrive_grouped() {
        let dir = tempfile::tempdir().unwrap();
        let inner = Arc::new(FakeHost::default());
        let browse = Arc::new(KindsHost {
            inner: inner.clone(),
        });
        let mut app = browse_app(&dir, browse, Arc::new(FakeHost::default()));
        press(&mut app, KeyCode::Char('/'));
        type_text(&mut app, "so what");
        press(&mut app, KeyCode::Enter);
        wait_until(&mut app, |app| {
            let level = app.spotify_browser.level();
            level.pending() == 0 && level.rows().len() == 8
        });
        let rows = app.spotify_browser.level().rows();
        let headers: Vec<&str> = rows
            .iter()
            .filter_map(|row| match row {
                Row::Header(label) => Some(*label),
                Row::Item(_) => None,
            })
            .collect();
        assert_eq!(headers, ["Tracks", "Albums", "Artists", "Playlists"]);
        assert!(matches!(rows[0], Row::Header("Tracks")));
    }

    /// One mouse press at a screen cell.
    fn click(app: &mut App, column: u16, row: u16) {
        app.on_mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column,
            row,
            modifiers: crossterm::event::KeyModifiers::NONE,
        });
    }

    /// Draw one frame, so the hit map is up to date.
    fn draw(app: &mut App) {
        let state = app.player.state();
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        terminal
            .draw(|frame| crate::views::render(frame, app, &state))
            .unwrap();
    }

    #[test]
    fn a_click_on_a_row_selects_it_and_does_not_play() {
        let dir = tempfile::tempdir().unwrap();
        let browse = Arc::new(FakeHost::default());
        let transport = Arc::new(FakeHost::default());
        let mut app = browse_app(&dir, browse.clone(), transport.clone());
        draw(&mut app);
        let hit = app.hits.spotify_rows.expect("the rows are hit-tested");
        // Liked songs is the third top entry.
        click(&mut app, hit.inner.x + 1, hit.inner.y + 2);
        assert_eq!(app.spotify_browser.level().cursor, 2);
        assert!(
            settle(&app, &[&*browse, &*transport]).is_empty(),
            "a click loads nothing"
        );
        assert!(
            settle(&app, &[&*browse, &*transport]).is_empty(),
            "a click plays nothing"
        );
    }

    #[test]
    fn the_wheel_moves_the_browser_cursor() {
        let dir = tempfile::tempdir().unwrap();
        let browse = Arc::new(FakeHost::default());
        let transport = Arc::new(FakeHost::default());
        let mut app = browse_app(&dir, browse, transport);
        app.on_mouse(MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: 0,
            row: 0,
            modifiers: crossterm::event::KeyModifiers::NONE,
        });
        assert_eq!(app.spotify_browser.level().cursor, 1);
    }

    #[test]
    fn a_click_on_a_device_selects_it_and_does_not_transfer() {
        let dir = tempfile::tempdir().unwrap();
        let browse = Arc::new(FakeHost::default());
        let transport = Arc::new(FakeHost::default());
        let mut app = browse_app(&dir, browse.clone(), transport.clone());
        app.spotify_state.devices = vec![
            znicz_core::SpotifyDevice {
                id: "k1".into(),
                name: "Kitchen".into(),
                kind: "Speaker".into(),
                is_active: false,
                volume_percent: None,
            },
            znicz_core::SpotifyDevice {
                id: "d1".into(),
                name: "Desk".into(),
                kind: "Computer".into(),
                is_active: false,
                volume_percent: None,
            },
        ];
        press(&mut app, KeyCode::Char(','));
        draw(&mut app);
        let hit = app
            .hits
            .overlay_list
            .expect("the device rows are hit-tested");
        click(&mut app, hit.inner.x + 1, hit.inner.y + 1);
        assert_eq!(app.spotify_devices_cursor.selected(2), Some(1));
        assert_eq!(app.modal, crate::app::Modal::SpotifyDevices);
        assert!(
            !settle(&app, &[&*browse, &*transport])
                .iter()
                .any(|op| matches!(op, SpotifyOp::Transfer { .. }))
        );
    }

    #[test]
    fn enter_on_the_search_row_opens_the_prompt_like_slash() {
        let dir = tempfile::tempdir().unwrap();
        let browse = Arc::new(FakeHost::default());
        let transport = Arc::new(FakeHost::default());
        let mut app = browse_app(&dir, browse.clone(), transport.clone());
        assert_eq!(app.spotify_browser.level().cursor, 0);
        press(&mut app, KeyCode::Enter);
        assert!(app.spotify_search.is_some());
        assert!(settle(&app, &[&*browse, &*transport]).is_empty());
    }

    #[test]
    fn a_failed_send_settles_the_level_and_a_later_reload_sends_again() {
        let dir = tempfile::tempdir().unwrap();
        let browse = Arc::new(FakeHost::default());
        *browse.reply.lock().unwrap() = Some(page(vec![playlist(1)], 1));
        let mut app = settings_tests::settings_app(&dir);
        app.spotify_state = logged_in();
        press(&mut app, KeyCode::Char('O'));
        assert!(app.in_spotify_mode());
        // No browse lane yet: Enter on Playlists cannot send.
        press(&mut app, KeyCode::Char('j'));
        press(&mut app, KeyCode::Enter);
        assert!(
            settings_tests::toast_texts(&app)
                .iter()
                .any(|t| t == spotify_mode::NO_LANE)
        );
        assert_eq!(app.spotify_browser.level().pending(), 0, "not left loading");
        // The lane comes up, and the reload sends the first page again.
        let mut app = app.with_spotify_worker(browse.clone()).unwrap();
        press(&mut app, KeyCode::Char('r'));
        assert!(wait_for_op(&browse, |op| matches!(
            op,
            SpotifyOp::Playlists { .. }
        )));
    }

    fn ctrl(app: &mut App, c: char) {
        app.on_key(crossterm::event::KeyEvent::new(
            KeyCode::Char(c),
            crossterm::event::KeyModifiers::CONTROL,
        ));
    }

    /// Spotify mode with thirty playlists open in the browser.
    fn thirty_playlists(dir: &tempfile::TempDir, browse: &Arc<FakeHost>) -> App {
        let items: Vec<SpotifyItem> = (1..=30).map(playlist).collect();
        *browse.reply.lock().unwrap() = Some(page(items, 30));
        *browse.state.lock().unwrap() = Some(logged_in());
        let mut app = browse_app(dir, browse.clone(), Arc::new(FakeHost::default()));
        press(&mut app, KeyCode::Char('j'));
        press(&mut app, KeyCode::Char('l'));
        wait_until(&mut app, |app| {
            app.spotify_browser.level().items.len() == 30
        });
        app
    }

    #[test]
    fn ctrl_d_and_ctrl_u_half_page_the_browser() {
        let dir = tempfile::tempdir().unwrap();
        let browse = Arc::new(FakeHost::default());
        let mut app = thirty_playlists(&dir, &browse);
        ctrl(&mut app, 'd');
        assert_eq!(app.spotify_browser.level().cursor, 10);
        ctrl(&mut app, 'd');
        assert_eq!(app.spotify_browser.level().cursor, 20);
        ctrl(&mut app, 'u');
        assert_eq!(app.spotify_browser.level().cursor, 10);
        ctrl(&mut app, 'u');
        ctrl(&mut app, 'u');
        assert_eq!(app.spotify_browser.level().cursor, 0, "clamped at the top");
    }

    #[test]
    fn ctrl_keys_are_text_in_the_search_prompt_and_do_not_move_the_browser() {
        let dir = tempfile::tempdir().unwrap();
        let browse = Arc::new(FakeHost::default());
        let mut app = thirty_playlists(&dir, &browse);
        press(&mut app, KeyCode::Char('/'));
        type_text(&mut app, "ab");
        ctrl(&mut app, 'd');
        ctrl(&mut app, 'u');
        assert_eq!(app.spotify_search.as_ref().unwrap().as_str(), "ab");
        assert_eq!(app.spotify_browser.level().cursor, 0);
    }

    #[test]
    fn enter_l_and_the_moves_do_nothing_while_the_spotify_drawer_is_open() {
        let dir = tempfile::tempdir().unwrap();
        let browse = Arc::new(FakeHost::default());
        let transport = Arc::new(FakeHost::default());
        let mut app = playlists_open_with(&dir, &browse, transport.clone());
        app.player
            .send_blocking(znicz_core::Command::QueueAdd(vec![
                znicz_core::QueueItem::file("/music/a.flac"),
                znicz_core::QueueItem::file("/music/b.flac"),
            ]))
            .unwrap();
        press(&mut app, KeyCode::Char(']'));
        assert!(app.queue_open);
        let before = settle(&app, &[&*browse, &*transport]);
        let browser_cursor = app.spotify_browser.level().cursor;
        for key in [KeyCode::Enter, KeyCode::Char('l')] {
            press(&mut app, key);
        }
        for key in ['j', 'k', 'g', 'G'] {
            press(&mut app, KeyCode::Char(key));
        }
        app.on_mouse(MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: 0,
            row: 0,
            modifiers: crossterm::event::KeyModifiers::NONE,
        });
        assert_eq!(
            settle(&app, &[&*browse, &*transport]),
            before,
            "no Play and no load after the drawer opened"
        );
        assert_eq!(
            app.queue_cursor.index(),
            0,
            "the Znicz queue cursor did not move"
        );
        assert_eq!(app.spotify_browser.level().cursor, browser_cursor);
    }
}
