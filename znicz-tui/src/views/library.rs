//! The library pane: albums, album tracks, and search results.

use std::time::Duration;

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{List, ListItem, ListState, Paragraph};
use ratatui::Frame;
use znicz_library::{AlbumSummary, Track};

use crate::app::{App, Focus};
use crate::format;
use crate::library_pane::{LibraryPane, Mode};
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
        if let Some(edit) = app.library.prompt() {
            frame.render_widget(
                Paragraph::new(views::prompt_line("search: ", edit)),
                prompt_area,
            );
        }
    }

    let focused = app.focus == Focus::Library && !app.modal.blocks_list_focus();
    let strip = crate::layout::strip_inner(list_area, app.queue_open);
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
            .enumerate()
            .map(|(i, album)| album_row(album, strip, app.library.offset_for(i)))
            .collect(),
        _ => app
            .library
            .tracks()
            .iter()
            .enumerate()
            .map(|(i, track)| track_row(track, strip, app.library.offset_for(i)))
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

fn album_row(album: &AlbumSummary, strip: usize, offset: usize) -> ListItem<'static> {
    let right = album_right(album);
    let fixed = album_fixed(album);
    let middle = format::pan(
        &LibraryPane::album_middle(album),
        offset,
        strip.saturating_sub(fixed),
    );
    let pad = strip.saturating_sub(middle.chars().count() + right.chars().count());

    ListItem::new(Line::from(vec![
        Span::styled(middle, theme::text()),
        Span::raw(" ".repeat(pad)),
        Span::styled(right, theme::dim()),
    ]))
}

fn track_row(track: &Track, strip: usize, offset: usize) -> ListItem<'static> {
    let number = track_number(track);
    let time = format::duration_opt(track.duration_secs.map(Duration::from_secs_f64));
    let fixed = track_fixed(track);
    let label = format::pan(
        &LibraryPane::track_middle(track),
        offset,
        strip.saturating_sub(fixed),
    );
    let pad = strip
        .saturating_sub(fixed + label.chars().count())
        .saturating_add(1);

    ListItem::new(Line::from(vec![
        Span::styled(number, theme::dim()),
        Span::styled(label, theme::text()),
        Span::raw(" ".repeat(pad)),
        Span::styled(time, theme::dim()),
    ]))
}

fn album_right(album: &AlbumSummary) -> String {
    match album.total_secs {
        Some(secs) => format!(
            "{} · {}",
            tracks_label(album.track_count),
            format::duration(Duration::from_secs_f64(secs))
        ),
        None => tracks_label(album.track_count),
    }
}

fn album_fixed(album: &AlbumSummary) -> usize {
    album_right(album).chars().count() + 2
}

fn track_number(track: &Track) -> String {
    track
        .track_number
        .map(|n| format!("{n:>2} "))
        .unwrap_or_else(|| "   ".to_string())
}

fn track_fixed(track: &Track) -> usize {
    let time = format::duration_opt(track.duration_secs.map(Duration::from_secs_f64));
    track_number(track).chars().count() + time.chars().count() + 2
}

/// Width available for the title column after reserving fixed right-side columns.
pub(crate) fn title_slot(pane: &LibraryPane, strip: usize) -> usize {
    let fixed = match pane.mode() {
        Mode::Albums => pane.albums().iter().map(album_fixed).max().unwrap_or(0),
        _ => pane.tracks().iter().map(track_fixed).max().unwrap_or(0),
    };
    strip.saturating_sub(fixed)
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

    #[test]
    fn title_slot_matches_strip_minus_max_fixed_for_albums() {
        let album = AlbumSummary {
            album: "Long Album Title".to_string(),
            album_artist: Some("Artist".to_string()),
            year: Some(2020),
            track_count: 12,
            total_secs: Some(3600.0),
        };
        let mut pane = LibraryPane::new(None);
        pane.inject_albums_for_test(vec![album.clone()]);
        let strip: usize = 60;
        assert_eq!(
            title_slot(&pane, strip),
            strip.saturating_sub(album_fixed(&album))
        );
    }

    #[test]
    fn title_slot_matches_strip_minus_max_fixed_for_tracks() {
        use std::path::PathBuf;

        let track = Track {
            id: 1,
            path: PathBuf::from("/music/track.flac"),
            title: "A Very Long Track Title Indeed".to_string(),
            artist: Some("Performer".to_string()),
            album: Some("Album".to_string()),
            album_artist: None,
            genre: None,
            year: None,
            track_number: Some(3),
            disc_number: None,
            codec: None,
            sample_rate: None,
            channels: None,
            bits_per_sample: None,
            duration_secs: Some(245.0),
        };
        let mut pane = LibraryPane::new(None);
        pane.inject_tracks_for_test(vec![track.clone()]);
        let strip: usize = 55;
        assert_eq!(
            title_slot(&pane, strip),
            strip.saturating_sub(track_fixed(&track))
        );
    }
}
