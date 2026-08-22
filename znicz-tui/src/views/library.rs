//! The library pane: albums, album tracks, and search results.

use std::time::Duration;

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{List, ListItem, ListState, Paragraph};
use znicz_library::{AlbumSummary, Track};

use crate::app::{App, Pane};
use crate::format;
use crate::library_pane::Mode;
use crate::theme;
use crate::views;

pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    // The search prompt takes a line off the top of the pane when open.
    let (prompt_area, list_area) = match app.library.is_typing() {
        true => {
            let parts = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Length(1), Constraint::Min(3)])
                .split(area);
            (Some(parts[0]), parts[1])
        }
        false => (None, area),
    };

    if let Some(prompt_area) = prompt_area {
        let query = app.library.input().unwrap_or("");
        let prompt = Line::from(vec![
            Span::styled("search: ", theme::key()),
            Span::styled(query.to_string(), theme::strong()),
            // A block cursor, since the terminal cursor stays hidden.
            Span::styled("█", theme::progress()),
        ]);
        frame.render_widget(Paragraph::new(prompt), prompt_area);
    }

    let focused = app.pane == Pane::Library;
    let width = views::inner_width(list_area);
    let title = match app.library.mode() {
        Mode::Albums => "Library".to_string(),
        Mode::AllTracks => "Library / all tracks".to_string(),
        Mode::Album(album) => format!("Library / {}", format::truncate(album, 40)),
        Mode::Search(query) => format!("Library / search \"{}\"", format::truncate(query, 30)),
    };

    if app.library.is_empty() {
        let notice = app
            .library
            .notice()
            .unwrap_or("Nothing here. Press / to search, or Esc to go back.");
        let block = views::pane_block(&title, focused, None);
        frame.render_widget(
            Paragraph::new(views::placeholder(notice)).block(block),
            list_area,
        );
        return;
    }

    let items: Vec<ListItem> = match app.library.mode() {
        Mode::Albums => app
            .library
            .albums()
            .iter()
            .map(|album| album_row(album, width))
            .collect(),
        _ => app
            .library
            .tracks()
            .iter()
            .map(|track| track_row(track, width))
            .collect(),
    };

    let count = app.library.len();
    let summary = match app.library.mode() {
        Mode::Albums => format!("{count} albums"),
        _ => format!("{count} tracks"),
    };

    let list = List::new(items)
        .block(views::pane_block(&title, focused, Some(summary)))
        .highlight_style(if focused {
            theme::selected()
        } else {
            views::no_style()
        });

    let mut state = ListState::default();
    state.select(app.library.selected_index());
    frame.render_stateful_widget(list, list_area, &mut state);
}

fn album_row(album: &AlbumSummary, width: usize) -> ListItem<'static> {
    let year = album.year.map(|y| format!(" ({y})")).unwrap_or_default();
    let artist = album.album_artist.as_deref().unwrap_or("Unknown artist");

    let right = match album.total_secs {
        Some(secs) => format!(
            "{} · {}",
            tracks_label(album.track_count),
            format::duration(Duration::from_secs_f64(secs))
        ),
        None => tracks_label(album.track_count),
    };

    let left = format!("{}{year} — {artist}", album.album);
    let left = format::truncate(&left, width.saturating_sub(right.chars().count() + 2));
    let pad = width.saturating_sub(left.chars().count() + right.chars().count());

    ListItem::new(Line::from(vec![
        Span::styled(left, theme::text()),
        Span::raw(" ".repeat(pad)),
        Span::styled(right, theme::dim()),
    ]))
}

fn track_row(track: &Track, width: usize) -> ListItem<'static> {
    let number = track
        .track_number
        .map(|n| format!("{n:>2} "))
        .unwrap_or_else(|| "   ".to_string());
    let time = format::duration_opt(track.duration_secs.map(Duration::from_secs_f64));

    let label = match track.artist.as_deref() {
        Some(artist) => format!("{} — {artist}", track.title),
        None => track.title.clone(),
    };

    let fixed = number.chars().count() + time.chars().count() + 2;
    let label = format::truncate(&label, width.saturating_sub(fixed));
    let pad = width
        .saturating_sub(fixed + label.chars().count())
        .saturating_add(1);

    ListItem::new(Line::from(vec![
        Span::styled(number, theme::dim()),
        Span::styled(label, theme::text()),
        Span::raw(" ".repeat(pad)),
        Span::styled(time, theme::dim()),
    ]))
}

fn tracks_label(count: u32) -> String {
    if count == 1 {
        "1 track".to_string()
    } else {
        format!("{count} tracks")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_track_is_not_pluralised() {
        assert_eq!(tracks_label(1), "1 track");
        assert_eq!(tracks_label(0), "0 tracks");
        assert_eq!(tracks_label(12), "12 tracks");
    }
}
