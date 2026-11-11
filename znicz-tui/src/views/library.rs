//! The library pane: artist-first browse (columns / tree / paging), search, all-tracks.

use std::time::Duration;

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{List, ListItem, ListState, Paragraph};
use ratatui::Frame;
use znicz_library::{AlbumSummary, ArtistSummary, SearchHit, Track};

use crate::app::{App, Focus};
use crate::format;
use crate::hit::ListHit;
use crate::library_pane::{ColumnFocus, EffectiveLayout, LibraryPane, Mode, RowContext, TreeRow};
use crate::playing::PlayingRef;
use crate::theme;
use crate::views;

pub fn render(frame: &mut Frame, area: Rect, app: &mut App) {
    app.hits.library = None;
    app.hits.library_artists = None;
    app.hits.library_albums = None;
    app.hits.library_tracks = None;

    let folder_on_top = app.folder_prompt.is_some() && !app.library.has_no_music();
    let (prompt_area, list_area) = match app.library.is_typing() || folder_on_top {
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
            app.hits.search_prompt = Some(prompt_area);
        } else if let Some(edit) = app.folder_prompt.clone() {
            frame.render_widget(
                Paragraph::new(views::prompt_line("folder: ", &edit)),
                prompt_area,
            );
        }
    }

    let strip = crate::layout::strip_inner(list_area, app.queue_open);
    let focused = app.focus == Focus::Library && !app.modal.blocks_list_focus();

    match app.library.mode() {
        Mode::Search(_) | Mode::AllTracks => {
            render_single_list(frame, list_area, app, strip, focused);
        }
        Mode::Browse if app.library.artists().is_empty() => {
            render_empty(frame, list_area, "Library", focused, app);
        }
        Mode::Browse => match app.library.effective_layout(strip) {
            EffectiveLayout::Columns => render_columns(frame, list_area, app, focused),
            EffectiveLayout::Tree => render_tree(frame, list_area, app, strip, focused),
            EffectiveLayout::Paging => render_paging(frame, list_area, app, strip, focused),
        },
    }
}

fn render_columns(frame: &mut Frame, area: Rect, app: &mut App, focused: bool) {
    let playing = app.playing.clone();
    let parts = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(28),
            Constraint::Percentage(32),
            Constraint::Percentage(40),
        ])
        .split(area);

    let artist = app
        .library
        .selected_artist_index()
        .and_then(|i| app.library.artists().get(i))
        .map(|a| a.name.clone());
    let album = app
        .library
        .selected_album_index()
        .and_then(|i| app.library.albums().get(i))
        .map(|a| a.album.clone());
    let albums_title = column_title("Albums", artist.as_deref(), views::title_width(parts[1]));
    let tracks_title = column_title("Tracks", album.as_deref(), views::title_width(parts[2]));

    render_column_list(
        frame,
        parts[0],
        app,
        "Artists",
        ColumnFocus::Artists,
        focused && app.library.column_focus() == ColumnFocus::Artists,
        app.library
            .artists()
            .iter()
            .map(|a| {
                let strip = parts[0].width.saturating_sub(2) as usize;
                artist_row(a, strip, 0, playing.as_ref())
            })
            .collect(),
        app.library.artists().len(),
        app.library.selected_artist_index(),
    );
    render_column_list(
        frame,
        parts[1],
        app,
        &albums_title,
        ColumnFocus::Albums,
        focused && app.library.column_focus() == ColumnFocus::Albums,
        {
            let context = app.library.album_context();
            app.library
                .albums()
                .iter()
                .map(|a| {
                    let strip = parts[1].width.saturating_sub(2) as usize;
                    album_row(a, context, strip, 0, playing.as_ref())
                })
                .collect()
        },
        app.library.albums().len(),
        app.library.selected_album_index(),
    );
    render_column_list(
        frame,
        parts[2],
        app,
        &tracks_title,
        ColumnFocus::Tracks,
        focused && app.library.column_focus() == ColumnFocus::Tracks,
        {
            // Computed once: the numbering reads the whole track list.
            let context = app.library.track_context();
            app.library
                .tracks()
                .iter()
                .map(|t| {
                    let strip = parts[2].width.saturating_sub(2) as usize;
                    track_row(t, context, strip, 0, playing.as_ref())
                })
                .collect()
        },
        app.library.tracks().len(),
        app.library.selected_track_index(),
    );
}

/// `Albums · Miles`: a column title naming its parent selection.
fn column_title(name: &str, parent: Option<&str>, width: usize) -> String {
    match parent {
        Some(parent) => format::truncate(&format!("{name} · {parent}"), width),
        None => name.to_string(),
    }
}

#[allow(clippy::too_many_arguments)]
fn render_column_list(
    frame: &mut Frame,
    area: Rect,
    app: &mut App,
    title: &str,
    column: ColumnFocus,
    focused: bool,
    items: Vec<ListItem>,
    count: usize,
    selected: Option<usize>,
) {
    let summary = match column {
        ColumnFocus::Artists => format!("{count} artists"),
        ColumnFocus::Albums => format!("{count} albums"),
        ColumnFocus::Tracks => format!("{count} tracks"),
    };
    let block = views::pane_block(title, focused, Some(summary));
    let inner = block.inner(area);
    let list = List::new(items)
        .block(block)
        .highlight_style(views::highlight(focused));

    let mut state = ListState::default();
    state.select(selected);
    frame.render_stateful_widget(list, area, &mut state);

    let hit = ListHit {
        inner,
        offset: state.offset(),
        len: count,
    };
    match column {
        ColumnFocus::Artists => app.hits.library_artists = Some(hit),
        ColumnFocus::Albums => app.hits.library_albums = Some(hit),
        ColumnFocus::Tracks => app.hits.library_tracks = Some(hit),
    }
}

fn render_tree(frame: &mut Frame, area: Rect, app: &mut App, strip: usize, focused: bool) {
    let playing = app.playing.clone();
    let rows = app.library.tree_rows();
    let count = rows.len();
    // Resolve the selection once: asking per row rebuilds the tree for every
    // row and re-queries the library.
    let selected = app.library.selected_index(strip);
    let contexts = app.library.tree_row_contexts(&rows);
    let items: Vec<ListItem> = rows
        .iter()
        .zip(contexts)
        .enumerate()
        .map(|(i, (row, context))| {
            tree_row(
                app,
                row,
                context,
                strip,
                app.library.offset_for_row(i, selected),
                playing.as_ref(),
            )
        })
        .collect();
    let crumbs = tree_crumbs(&rows, selected);
    let parts: Vec<&str> = std::iter::once("Library")
        .chain(crumbs.iter().map(String::as_str))
        .collect();
    let title = views::breadcrumb(&parts, views::title_width(area));
    let summary = format!("{count} rows");
    let block = views::pane_block(&title, focused, Some(summary));
    let inner = block.inner(area);
    let list = List::new(items)
        .block(block)
        .highlight_style(views::highlight(focused));
    app.library_list_state
        .select(app.library.selected_index(strip));
    frame.render_stateful_widget(list, area, &mut app.library_list_state);
    app.hits.library = Some(ListHit {
        inner,
        offset: app.library_list_state.offset(),
        len: count,
    });
}

fn tree_row(
    app: &App,
    row: &TreeRow,
    context: RowContext<'_>,
    strip: usize,
    offset: usize,
    playing: Option<&PlayingRef>,
) -> ListItem<'static> {
    match row {
        TreeRow::Artist { artist, expanded } => {
            let mark = playing.is_some_and(|p| p.is_artist(artist));
            let marker = if *expanded { "▼ " } else { "▶ " };
            let middle = marked(
                format!("{marker}{}", LibraryPane::artist_middle(artist)),
                mark,
            );
            let right = format!("{} · artist", tracks_label(artist.track_count));
            list_item(&middle, &right, strip, offset, mark)
        }
        TreeRow::Album {
            album,
            expanded,
            depth,
            ..
        } => {
            let mark = playing.is_some_and(|p| p.is_album(album));
            let indent = "  ".repeat(*depth);
            let marker = if *expanded { "▼ " } else { "▶ " };
            let middle = marked(
                format!(
                    "{indent}{marker}{}",
                    LibraryPane::album_middle(album, context)
                ),
                mark,
            );
            let right = album_right(album);
            list_item(&middle, &right, strip, offset, mark)
        }
        TreeRow::Track {
            album_key,
            index,
            depth,
        } => {
            let indent = "  ".repeat(*depth);
            let track = app.library.tree_track_cache_get(album_key, *index);
            let mark = track.is_some_and(|t| playing.is_some_and(|p| p.is_track(t)));
            let lead = if mark { "▶ " } else { "" };
            let middle = match track {
                Some(t) => format!("{indent}{lead}{}", LibraryPane::track_middle(t, context)),
                None => format!("{indent}?"),
            };
            let right = track
                .map(|t| format::duration_opt(t.duration_secs.map(Duration::from_secs_f64)))
                .unwrap_or_default();
            list_item(&middle, &right, strip, offset, mark)
        }
    }
}

/// Artist (and album) names above the tree cursor, for the title.
fn tree_crumbs(rows: &[TreeRow], selected: Option<usize>) -> Vec<String> {
    let Some(index) = selected else {
        return Vec::new();
    };
    // A track row belongs to the nearest album row above it.
    let index = match rows.get(index) {
        Some(TreeRow::Track { .. }) => rows[..index]
            .iter()
            .rposition(|r| matches!(r, TreeRow::Album { .. }))
            .unwrap_or(index),
        _ => index,
    };
    match rows.get(index) {
        Some(TreeRow::Artist { artist, .. }) => vec![artist.name.clone()],
        Some(TreeRow::Album { artist, album, .. }) => vec![artist.clone(), album.album.clone()],
        _ => Vec::new(),
    }
}

fn render_paging(frame: &mut Frame, area: Rect, app: &mut App, strip: usize, focused: bool) {
    let playing = app.playing.clone();
    let artist = app
        .library
        .selected_artist_index()
        .and_then(|i| app.library.artists().get(i))
        .map(|a| a.name.clone())
        .unwrap_or_default();
    let album = app
        .library
        .selected_album_index()
        .and_then(|i| app.library.albums().get(i))
        .map(|a| a.album.clone())
        .unwrap_or_default();
    let parts: Vec<&str> = match app.library.paging_level() {
        ColumnFocus::Artists => vec!["Library"],
        ColumnFocus::Albums => vec!["Library", &artist],
        ColumnFocus::Tracks => vec!["Library", &artist, &album],
    };
    let title = views::breadcrumb(&parts, views::title_width(area));

    if app.library.is_empty(strip) {
        render_empty(frame, area, &title, focused, app);
        return;
    }

    let items: Vec<ListItem> = match app.library.paging_level() {
        ColumnFocus::Artists => app
            .library
            .artists()
            .iter()
            .enumerate()
            .map(|(i, a)| artist_row(a, strip, app.library.offset_for(i, strip), playing.as_ref()))
            .collect(),
        ColumnFocus::Albums => {
            let context = app.library.album_context();
            app.library
                .albums()
                .iter()
                .enumerate()
                .map(|(i, a)| {
                    album_row(
                        a,
                        context,
                        strip,
                        app.library.offset_for(i, strip),
                        playing.as_ref(),
                    )
                })
                .collect()
        }
        ColumnFocus::Tracks => {
            let context = app.library.track_context();
            app.library
                .tracks()
                .iter()
                .enumerate()
                .map(|(i, t)| {
                    track_row(
                        t,
                        context,
                        strip,
                        app.library.offset_for(i, strip),
                        playing.as_ref(),
                    )
                })
                .collect()
        }
    };
    let count = app.library.len(strip);
    let summary = match app.library.paging_level() {
        ColumnFocus::Artists => format!("{count} artists"),
        ColumnFocus::Albums => format!("{count} albums"),
        ColumnFocus::Tracks => format!("{count} tracks"),
    };
    finish_list(
        frame, area, app, &title, focused, items, count, summary, strip,
    );
}

fn render_single_list(frame: &mut Frame, area: Rect, app: &mut App, strip: usize, focused: bool) {
    let playing = app.playing.clone();
    let search;
    let parts: Vec<&str> = match app.library.mode() {
        Mode::AllTracks => vec!["Library", "all tracks"],
        Mode::Search(query) => {
            search = format!("search \"{query}\"");
            vec!["Library", &search]
        }
        Mode::Browse => vec!["Library"],
    };
    let title = views::breadcrumb(&parts, views::title_width(area));

    if app.library.is_empty(strip) {
        render_empty(frame, area, &title, focused, app);
        return;
    }

    let items: Vec<ListItem> = match app.library.mode() {
        Mode::Search(_) => app
            .library
            .search_hits()
            .iter()
            .enumerate()
            .map(|(i, hit)| {
                hit_row(
                    hit,
                    strip,
                    app.library.offset_for(i, strip),
                    playing.as_ref(),
                )
            })
            .collect(),
        Mode::AllTracks | Mode::Browse => app
            .library
            .tracks()
            .iter()
            .enumerate()
            .map(|(i, track)| {
                track_row(
                    track,
                    RowContext::Flat,
                    strip,
                    app.library.offset_for(i, strip),
                    playing.as_ref(),
                )
            })
            .collect(),
    };
    let count = app.library.len(strip);
    let summary = match app.library.mode() {
        Mode::Search(_) => search_summary(app.library.search_hits()),
        _ => format!("{count} tracks"),
    };
    finish_list(
        frame, area, app, &title, focused, items, count, summary, strip,
    );
}

fn render_empty(frame: &mut Frame, area: Rect, title: &str, focused: bool, app: &App) {
    let block = views::pane_block(title, focused, None);
    frame.render_widget(
        Paragraph::new(empty_body(app, views::inner_width(area))).block(block),
        area,
    );
}

fn empty_body(app: &App, width: usize) -> Vec<Line<'static>> {
    if app.library.has_no_music() {
        if let Some(edit) = &app.folder_prompt {
            return vec![
                Line::from("No music yet. Which folder holds your music?"),
                Line::from(""),
                views::prompt_line("folder: ", edit),
                Line::from(""),
                views::placeholder("Tab completes · Enter scans · Esc skips"),
            ];
        }
        if let Some(progress) = &app.library_status.scanning {
            let home = dirs::home_dir();
            return vec![views::placeholder(&crate::scan_status::progress_text(
                progress,
                home.as_deref(),
                width,
            ))];
        }
    }
    let notice = app
        .library
        .notice()
        .unwrap_or("Nothing here. Press / to search, or Esc to go back.");
    vec![views::placeholder(notice)]
}

#[allow(clippy::too_many_arguments)]
fn finish_list(
    frame: &mut Frame,
    area: Rect,
    app: &mut App,
    title: &str,
    focused: bool,
    items: Vec<ListItem>,
    count: usize,
    summary: String,
    strip: usize,
) {
    let block = views::pane_block(title, focused, Some(summary));
    let inner = block.inner(area);
    let list = List::new(items)
        .block(block)
        .highlight_style(views::highlight(focused));
    app.library_list_state
        .select(app.library.selected_index(strip));
    frame.render_stateful_widget(list, area, &mut app.library_list_state);
    app.hits.library = Some(ListHit {
        inner,
        offset: app.library_list_state.offset(),
        len: count,
    });
}

fn search_summary(hits: &[SearchHit]) -> String {
    let mut artists = 0usize;
    let mut albums = 0usize;
    let mut tracks = 0usize;
    for hit in hits {
        match hit {
            SearchHit::Artist(_) => artists += 1,
            SearchHit::Album(_) => albums += 1,
            SearchHit::Track(_) => tracks += 1,
        }
    }
    format!("{artists} artists · {albums} albums · {tracks} tracks")
}

fn hit_row(
    hit: &SearchHit,
    strip: usize,
    offset: usize,
    playing: Option<&PlayingRef>,
) -> ListItem<'static> {
    match hit {
        SearchHit::Artist(artist) => {
            let mark = playing.is_some_and(|p| p.is_artist(artist));
            let right = format!("{} · artist", tracks_label(artist.track_count));
            let middle = marked(LibraryPane::artist_middle(artist), mark);
            list_item(&middle, &right, strip, offset, mark)
        }
        SearchHit::Album(album) => {
            let mark = playing.is_some_and(|p| p.is_album(album));
            let right = format!("{} · album", album_right(album));
            let middle = marked(LibraryPane::album_middle(album, RowContext::Flat), mark);
            list_item(&middle, &right, strip, offset, mark)
        }
        SearchHit::Track(track) => track_row(track, RowContext::Flat, strip, offset, playing),
    }
}

fn artist_row(
    artist: &ArtistSummary,
    strip: usize,
    offset: usize,
    playing: Option<&PlayingRef>,
) -> ListItem<'static> {
    let mark = playing.is_some_and(|p| p.is_artist(artist));
    let right = format!("{} · artist", tracks_label(artist.track_count));
    let middle = marked(LibraryPane::artist_middle(artist), mark);
    list_item(&middle, &right, strip, offset, mark)
}

fn album_row(
    album: &AlbumSummary,
    context: RowContext<'_>,
    strip: usize,
    offset: usize,
    playing: Option<&PlayingRef>,
) -> ListItem<'static> {
    let mark = playing.is_some_and(|p| p.is_album(album));
    list_item(
        &marked(LibraryPane::album_middle(album, context), mark),
        &album_right(album),
        strip,
        offset,
        mark,
    )
}

fn track_row(
    track: &Track,
    context: RowContext<'_>,
    strip: usize,
    offset: usize,
    playing: Option<&PlayingRef>,
) -> ListItem<'static> {
    let mark = playing.is_some_and(|p| p.is_track(track));
    let middle = LibraryPane::track_middle(track, context);
    // The number now leads the title, so this column is only the playing marker.
    let marker = if mark { " ▶ " } else { TRACK_MARKER_BLANK };
    let time = format::duration_opt(track.duration_secs.map(Duration::from_secs_f64));
    let right = format!("{marker}{time}");
    list_item(&middle, &right, strip, offset, mark)
}

fn list_item(
    middle: &str,
    right: &str,
    strip: usize,
    offset: usize,
    playing: bool,
) -> ListItem<'static> {
    let right_w = right.chars().count();
    let middle_w = strip.saturating_sub(right_w.saturating_add(1));
    let shown = format::pan(middle, offset, middle_w);
    let pad = middle_w.saturating_sub(shown.chars().count());
    let (middle_style, right_style) = if playing {
        (theme::playing(), theme::playing())
    } else {
        (Style::default(), theme::dim())
    };
    ListItem::new(Line::from(vec![
        Span::styled(shown, middle_style),
        Span::raw(" ".repeat(pad)),
        Span::styled(format!(" {right}"), right_style),
    ]))
}

/// The row's name with ` ♪` appended when it stands for what is playing.
fn marked(name: String, mark: bool) -> String {
    if mark {
        format!("{name} ♪")
    } else {
        name
    }
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

fn artist_fixed(artist: &ArtistSummary) -> usize {
    format!("{} · artist", tracks_label(artist.track_count))
        .chars()
        .count()
        + 2
}

fn search_album_fixed(album: &AlbumSummary) -> usize {
    format!("{} · album", album_right(album)).chars().count() + 2
}

/// Three cells before a track's duration: the playing marker, or blanks.
const TRACK_MARKER_BLANK: &str = "   ";

fn track_fixed(track: &Track) -> usize {
    let time = format::duration_opt(track.duration_secs.map(Duration::from_secs_f64));
    TRACK_MARKER_BLANK.len() + time.chars().count() + 2
}

/// Width available for the title column after reserving fixed right-side columns.
pub(crate) fn title_slot(pane: &LibraryPane, strip: usize) -> usize {
    let fixed = match pane.mode() {
        Mode::Browse => match pane.effective_layout(strip) {
            EffectiveLayout::Tree => 12,
            EffectiveLayout::Columns => match pane.column_focus() {
                ColumnFocus::Artists => pane.artists().iter().map(artist_fixed).max().unwrap_or(0),
                ColumnFocus::Albums => pane.albums().iter().map(album_fixed).max().unwrap_or(0),
                ColumnFocus::Tracks => pane.tracks().iter().map(track_fixed).max().unwrap_or(0),
            },
            EffectiveLayout::Paging => match pane.paging_level() {
                ColumnFocus::Artists => pane.artists().iter().map(artist_fixed).max().unwrap_or(0),
                ColumnFocus::Albums => pane.albums().iter().map(album_fixed).max().unwrap_or(0),
                ColumnFocus::Tracks => pane.tracks().iter().map(track_fixed).max().unwrap_or(0),
            },
        },
        Mode::Search(_) => pane
            .search_hits()
            .iter()
            .map(|hit| match hit {
                SearchHit::Artist(artist) => artist_fixed(artist),
                SearchHit::Album(album) => search_album_fixed(album),
                SearchHit::Track(track) => track_fixed(track),
            })
            .max()
            .unwrap_or(0),
        Mode::AllTracks => pane.tracks().iter().map(track_fixed).max().unwrap_or(0),
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
    fn search_summary_counts_each_kind() {
        let hits = vec![
            SearchHit::Artist(ArtistSummary {
                name: "A".into(),
                track_count: 1,
            }),
            SearchHit::Album(AlbumSummary {
                album: "B".into(),
                album_artist: None,
                year: None,
                track_count: 2,
                total_secs: None,
            }),
            SearchHit::Track(Track {
                id: 1,
                path: std::path::PathBuf::from("/t.flac"),
                title: "T".into(),
                artist: None,
                album: None,
                album_artist: None,
                genre: None,
                year: None,
                track_number: None,
                disc_number: None,
                codec: None,
                sample_rate: None,
                channels: None,
                bits_per_sample: None,
                duration_secs: None,
            }),
        ];
        assert_eq!(search_summary(&hits), "1 artists · 1 albums · 1 tracks");
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
        let strip: usize = 40; // paging
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
        let strip: usize = 40;
        assert_eq!(
            title_slot(&pane, strip),
            strip.saturating_sub(track_fixed(&track))
        );
    }
}
