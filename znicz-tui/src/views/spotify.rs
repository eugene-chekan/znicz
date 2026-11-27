//! Spotify mode's drawing: the browser in the list region, its search prompt,
//! Spotify's queue drawer and the Spotify Connect device list. Drawing reads
//! app state; it writes only the hit map and the browser's row offset.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, List, ListItem, ListState, Paragraph};
use znicz_core::SpotifyTrack;

use crate::app::App;
use crate::format;
use crate::hit::ListHit;
use crate::spotify_browse::{Row, TOP};
use crate::theme;
use crate::views;
use crate::views::window::visible_window;

const LOADING: &str = "Loading…";
const NOTHING_HERE: &str = "Nothing here";
const QUEUE_EMPTY: &str = "Spotify's queue is empty";
const NO_DEVICES: &str = "No Spotify devices: open Spotify on a device";

/// The browser in the list region: one column, drawn from the browser's levels.
/// Its bottom rows are the notice, the loading line and the search prompt.
pub fn render_browser(frame: &mut Frame, area: Rect, app: &mut App) {
    let title = format::truncate(&browser_title(app), views::title_width(area));
    let crumbs = app.spotify_browser.crumbs().join(" › ");
    let right = (!crumbs.is_empty()).then(|| format::truncate(&crumbs, views::title_width(area)));
    let block = views::pane_block(&title, true, right);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let at_top = app.spotify_browser.depth() == 1;
    let level = app.spotify_browser.level();
    let rows: Vec<Row> = if at_top { Vec::new() } else { level.rows() };
    let len = if at_top { TOP.len() } else { rows.len() };
    let cursor = level.cursor;
    let pending = level.pending > 0;

    let mut tail: Vec<Line<'static>> = Vec::new();
    if let Some(notice) = &level.notice {
        let text = format::truncate(notice, inner.width as usize);
        tail.push(Line::from(Span::styled(text, theme::warn())));
    }
    if pending && len > 0 {
        tail.push(views::placeholder(LOADING));
    }
    if let Some(edit) = &app.spotify_search {
        tail.push(views::prompt_line("/ ", edit));
    }
    let reserved = (tail.len() as u16).min(inner.height);
    let rows_area = Rect {
        height: inner.height - reserved,
        ..inner
    };
    let tail_top = rows_area.y + rows_area.height;
    for (i, line) in tail.into_iter().take(reserved as usize).enumerate() {
        let line_area = Rect {
            x: inner.x,
            y: tail_top + i as u16,
            width: inner.width,
            height: 1,
        };
        frame.render_widget(Paragraph::new(line), line_area);
    }

    if len == 0 {
        app.spotify_row_offset = 0;
        let text = if pending { LOADING } else { NOTHING_HERE };
        frame.render_widget(Paragraph::new(views::placeholder(text)), rows_area);
        return;
    }

    let width = rows_area.width as usize;
    let window = visible_window(
        len,
        Some(cursor),
        app.spotify_row_offset,
        rows_area.height as usize,
    );
    let items: Vec<ListItem> = window
        .clone()
        .map(|index| {
            if at_top {
                ListItem::new(Line::from(Span::styled(TOP[index], theme::text())))
            } else {
                row_item(&rows[index], width)
            }
        })
        .collect();
    // The visible rows are the list's first rows, so the selection is relative to the window.
    let mut list_state = ListState::default();
    list_state.select(
        Some(cursor)
            .filter(|c| window.contains(c))
            .map(|c| c - window.start),
    );
    let list = List::new(items).highlight_style(views::highlight(true));
    frame.render_stateful_widget(list, rows_area, &mut list_state);

    app.spotify_row_offset = window.start;
    app.hits.spotify_rows = Some(ListHit {
        inner: rows_area,
        offset: window.start,
        len,
    });
}

/// `Spotify · <user> · <device>`; the device is `Nothing playing` when none is.
fn browser_title(app: &App) -> String {
    let mut parts = vec!["Spotify".to_string()];
    if let Some(account) = &app.spotify_state.account {
        parts.push(account.user.clone());
    }
    let device = app
        .spotify_state
        .playback
        .as_ref()
        .and_then(|p| p.device.as_ref())
        .map_or("Nothing playing", |d| d.name.as_str());
    parts.push(device.to_string());
    parts.join(" · ")
}

/// One drawn row: a header in strong text, or an item's name with its subtitle dim after it.
fn row_item(row: &Row, width: usize) -> ListItem<'static> {
    match row {
        Row::Header(label) => {
            ListItem::new(Line::from(Span::styled(label.to_string(), theme::strong())))
        }
        Row::Item(item) => {
            let name = format::truncate(&item.name, width);
            let room = width.saturating_sub(name.chars().count());
            let mut spans = vec![Span::styled(name, theme::text())];
            if room > 2 && !item.subtitle.is_empty() {
                let subtitle = format!("  {}", item.subtitle);
                spans.push(Span::styled(
                    format::truncate(&subtitle, room),
                    theme::dim(),
                ));
            }
            ListItem::new(Line::from(spans))
        }
    }
}

/// Spotify's queue, read-only: the playing track, then the ones after it.
pub fn render_queue(frame: &mut Frame, area: Rect, app: &mut App) {
    app.hits.close = views::close_button_rect(area);
    let block = views::pane_block("Spotify queue", false, None).title(views::close_title());
    let width = views::inner_width(area);
    let lines: Vec<Line<'static>> = match &app.spotify_queue {
        None => vec![views::placeholder(LOADING)],
        Some((None, next)) if next.is_empty() => vec![views::placeholder(QUEUE_EMPTY)],
        Some((current, next)) => {
            let mut lines = Vec::new();
            if let Some(track) = current {
                lines.push(queue_line("Now", track, width, theme::playing()));
            }
            lines.extend(
                next.iter()
                    .map(|track| queue_line("Next", track, width, theme::text())),
            );
            lines
        }
    };
    frame.render_widget(Paragraph::new(lines).block(block), area);
}

fn queue_line(label: &str, track: &SpotifyTrack, width: usize, style: Style) -> Line<'static> {
    let artist = track.artist_line();
    let text = if artist.is_empty() {
        track.title.clone()
    } else {
        format!("{} — {}", track.title, artist)
    };
    let text = format::truncate(&text, width.saturating_sub(5));
    Line::from(vec![
        Span::styled(format!("{label:<5}"), theme::dim()),
        Span::styled(text, style),
    ])
}

/// The Spotify Connect device list, over the list region. The active device is marked.
pub fn render_devices(frame: &mut Frame, list: Rect, app: &mut App) {
    let popup = crate::layout::list_modal_rect(list);
    app.hits.overlay = Some(popup);
    app.hits.close = views::close_button_rect(popup);
    frame.render_widget(Clear, popup);
    let block = views::pane_block("Spotify Connect", true, None).title(views::close_title());

    let devices = &app.spotify_state.devices;
    if devices.is_empty() {
        frame.render_widget(
            Paragraph::new(views::placeholder(NO_DEVICES)).block(block),
            popup,
        );
        return;
    }
    let inner = block.inner(popup);
    let width = views::inner_width(popup);
    let len = devices.len();
    let items: Vec<ListItem> = devices
        .iter()
        .map(|device| {
            let marker = if device.is_active { "● " } else { "  " };
            let kind = format!("  {}", device.kind);
            let room = width.saturating_sub(marker.chars().count() + kind.chars().count());
            Line::from(vec![
                Span::styled(marker, theme::playing()),
                Span::styled(format::truncate(&device.name, room), theme::text()),
                Span::styled(kind, theme::dim()),
            ])
            .into()
        })
        .collect();

    app.device_list_state
        .select(app.spotify_devices_cursor.selected(len));
    let widget = List::new(items)
        .block(block)
        .highlight_style(views::highlight(true));
    frame.render_stateful_widget(widget, popup, &mut app.device_list_state);
    app.hits.overlay_list = Some(ListHit {
        inner,
        offset: app.device_list_state.offset(),
        len,
    });
}

#[cfg(test)]
mod tests {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use znicz_core::{
        SpotifyAccount, SpotifyDevice, SpotifyItem, SpotifyKind, SpotifyLogin, SpotifyPage,
        SpotifyState, SpotifyTrack,
    };

    use crate::app::{App, Modal};
    use crate::line_edit::LineEdit;
    use crate::spotify_browse::BrowseList;

    /// The screen as one string per row, at 80 x 24.
    fn screen(app: &mut App) -> Vec<String> {
        let state = app.player.state();
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        terminal
            .draw(|frame| crate::views::render(frame, app, &state))
            .unwrap();
        let buffer = terminal.backend().buffer();
        let width = buffer.area.width as usize;
        buffer
            .content()
            .chunks(width)
            .map(|row| row.iter().map(|cell| cell.symbol()).collect())
            .collect()
    }

    fn shows(rows: &[String], text: &str) -> bool {
        rows.iter().any(|row| row.contains(text))
    }

    fn press(app: &mut App, code: KeyCode) {
        app.on_key(KeyEvent::new(code, KeyModifiers::NONE));
    }

    /// An App on a player that makes no sound, with the default config.
    fn plain_app() -> App {
        let (player, _thread) = znicz_core::spawn_player(znicz_core::AudioConfig::default());
        App::with_library(player, None)
    }

    /// Spotify mode on, logged in as Eugene, with nothing playing.
    fn mode() -> App {
        let mut app = plain_app();
        app.spotify_state = SpotifyState {
            login: SpotifyLogin::LoggedIn,
            account: Some(SpotifyAccount {
                user: "Eugene".into(),
                premium: Some(true),
            }),
            ..SpotifyState::default()
        };
        press(&mut app, KeyCode::Char('O'));
        assert!(app.in_spotify_mode());
        app
    }

    fn track(title: &str) -> SpotifyItem {
        SpotifyItem {
            position: None,
            kind: SpotifyKind::Track,
            uri: format!("spotify:track:{title}"),
            name: title.into(),
            subtitle: "Miles Davis".into(),
            image_url: None,
            track: Some(SpotifyTrack {
                uri: format!("spotify:track:{title}"),
                title: title.into(),
                duration_ms: 1_000,
                ..SpotifyTrack::default()
            }),
        }
    }

    fn page(items: Vec<SpotifyItem>) -> SpotifyPage {
        let total = items.len() as u32;
        SpotifyPage {
            next_offset: None,
            items,
            offset: 0,
            total,
        }
    }

    fn device(name: &str, is_active: bool) -> SpotifyDevice {
        SpotifyDevice {
            id: name.to_lowercase(),
            name: name.into(),
            kind: "Computer".into(),
            is_active,
            volume_percent: Some(40),
        }
    }

    #[test]
    fn the_browser_shows_the_four_top_entries_with_the_cursor_on_the_first() {
        let mut app = mode();
        let rows = screen(&mut app);
        for label in ["Search", "Playlists", "Liked songs", "Albums"] {
            assert!(shows(&rows, label), "{label} is drawn");
        }
        assert!(shows(&rows, "Spotify · Eugene"), "the title names the user");
    }

    #[test]
    fn an_empty_list_while_loading_says_loading() {
        let mut app = mode();
        // Playlists: one step down, then enter. The request is pending.
        app.spotify_browser.step(1);
        app.spotify_browser.enter();
        assert!(shows(&screen(&mut app), "Loading…"));
    }

    #[test]
    fn an_empty_list_with_nothing_pending_says_nothing_here() {
        let mut app = mode();
        app.spotify_browser.step(1);
        app.spotify_browser.enter();
        let generation = app.spotify_browser.generation();
        app.spotify_browser
            .apply_page(generation, &BrowseList::Playlists, 0, &page(Vec::new()));
        assert!(shows(&screen(&mut app), "Nothing here"));
    }

    #[test]
    fn an_unavailable_playlist_notice_is_drawn_under_the_rows() {
        let mut app = mode();
        app.spotify_browser.step(1);
        app.spotify_browser.enter();
        let generation = app.spotify_browser.generation();
        app.spotify_browser.apply_error(
            generation,
            &BrowseList::Playlists,
            "This playlist's tracks are not available to you".into(),
        );
        assert!(shows(
            &screen(&mut app),
            "This playlist's tracks are not available to you"
        ));
    }

    #[test]
    fn search_results_are_drawn_under_their_group_headers() {
        let mut app = mode();
        let requests = app.spotify_browser.start_search("so what");
        let (generation, list, _) = requests[0].clone();
        app.spotify_browser
            .apply_page(generation, &list, 0, &page(vec![track("So What")]));
        let rows = screen(&mut app);
        assert!(shows(&rows, "Tracks"), "the Tracks group has a header");
        assert!(shows(&rows, "So What"), "the track is drawn");
    }

    #[test]
    fn the_search_prompt_is_drawn_when_open() {
        let mut app = mode();
        app.spotify_search = Some(LineEdit::from_text("so what"));
        assert!(shows(&screen(&mut app), "/ so what"));
    }

    #[test]
    fn the_queue_drawer_shows_the_playing_track_then_the_next_ones() {
        let mut app = mode();
        app.queue_open = true;
        app.spotify_queue = Some((
            Some(SpotifyTrack {
                title: "So What".into(),
                artists: vec!["Miles Davis".into()],
                ..SpotifyTrack::default()
            }),
            vec![SpotifyTrack {
                title: "Blue in Green".into(),
                ..SpotifyTrack::default()
            }],
        ));
        let rows = screen(&mut app);
        assert!(shows(&rows, "Spotify queue"));
        assert!(shows(&rows, "Now"));
        assert!(shows(&rows, "So What — Miles Davis"));
        assert!(shows(&rows, "Next"));
        assert!(shows(&rows, "Blue in Green"));
    }

    #[test]
    fn the_queue_drawer_says_empty_when_spotify_has_nothing_queued() {
        let mut app = mode();
        app.queue_open = true;
        app.spotify_queue = Some((None, Vec::new()));
        assert!(shows(&screen(&mut app), "Spotify's queue is empty"));
    }

    #[test]
    fn the_queue_drawer_says_loading_before_the_first_reply() {
        let mut app = mode();
        app.queue_open = true;
        assert!(shows(&screen(&mut app), "Loading…"));
    }

    #[test]
    fn the_device_list_says_so_when_there_are_no_devices() {
        let mut app = mode();
        app.modal = Modal::SpotifyDevices;
        assert!(shows(
            &screen(&mut app),
            "No Spotify devices: open Spotify on a device"
        ));
    }

    #[test]
    fn the_device_list_marks_the_active_device() {
        let mut app = mode();
        app.spotify_state.devices = vec![device("Kitchen", false), device("Desk", true)];
        app.modal = Modal::SpotifyDevices;
        let rows = screen(&mut app);
        assert!(shows(&rows, "Spotify Connect"));
        assert!(shows(&rows, "● Desk"), "the active device is marked");
        assert!(shows(&rows, "Kitchen"));
    }

    #[test]
    fn the_browser_is_not_drawn_in_znicz_mode() {
        let mut app = plain_app();
        let rows = screen(&mut app);
        assert!(!shows(&rows, "Spotify ·"));
    }
}
