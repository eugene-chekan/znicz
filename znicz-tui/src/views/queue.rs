//! The queue: what plays next, with real track names.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{List, ListItem, ListState, Paragraph};
use znicz_core::PlayerState;

use crate::app::{App, Focus};
use crate::format;
use crate::hit::ListHit;
use crate::theme;
use crate::views;
use crate::views::now_playing;
use crate::views::window::visible_window;

pub fn render(frame: &mut Frame, area: Rect, app: &mut App, state: &PlayerState) {
    app.hits.close = views::close_button_rect(area);
    let focused = app.focus == Focus::Queue && !app.modal.blocks_list_focus();
    let width = views::inner_width(area);

    if state.queue.is_empty() {
        let block = views::pane_block("Queue", focused, None).title(views::close_title());
        let hint = views::placeholder("Queue is empty. Add tracks from the library with a or A.");
        frame.render_widget(Paragraph::new(hint).block(block), area);
        return;
    }

    let len = state.queue.len();
    let (_, total) = queue_stats(app, &state.queue);
    let summary = match total {
        Some(total) => format!("{len} tracks · {}", format::duration(total)),
        None => format!("{len} tracks"),
    };
    let block = views::pane_block("Queue", focused, Some(summary)).title(views::close_title());
    let inner = block.inner(area);

    // Build rows only for what is on screen; the rest of a long queue is never touched.
    let selected = app.queue_cursor.selected(len);
    let window = visible_window(
        len,
        selected,
        app.queue_list_state.offset(),
        inner.height as usize,
    );

    let index_width = len.to_string().len().max(2);
    let items: Vec<ListItem> = state.queue[window.clone()]
        .iter()
        .enumerate()
        .map(|(row, item)| {
            let i = window.start + row;
            let is_playing = i == state.queue_position;

            let (label, time) = match item {
                znicz_core::QueueItem::Stream { name, .. } => {
                    (name.clone(), format::duration_opt(None))
                }
                znicz_core::QueueItem::File { path } => {
                    let entry = app.meta.get(path);
                    let label = match &entry {
                        Some(entry) => entry.label(),
                        // Fall back to the file name until the tags arrive.
                        None => path
                            .file_stem()
                            .and_then(|s| s.to_str())
                            .unwrap_or("?")
                            .to_string(),
                    };
                    let time = format::duration_opt(entry.and_then(|e| e.duration));
                    (label, time)
                }
            };

            // number + marker + gap + time
            let fixed = index_width + 2 + 2 + time.chars().count() + 1;
            let label = format::pan(
                &label,
                app.queue_offset_for(i, len),
                width.saturating_sub(fixed),
            );

            let marker = if is_playing {
                now_playing::status_symbol(state.status)
            } else {
                " "
            };
            let name_style = if is_playing {
                theme::playing()
            } else {
                theme::text()
            };

            let pad = width
                .saturating_sub(fixed + label.chars().count())
                .saturating_add(1);

            ListItem::new(Line::from(vec![
                Span::styled(
                    format!("{:>width$} ", i + 1, width = index_width),
                    theme::dim(),
                ),
                Span::styled(format!("{marker:<2}"), theme::playing()),
                Span::styled(label, name_style),
                Span::raw(" ".repeat(pad)),
                Span::styled(time, theme::dim()),
            ]))
        })
        .collect();

    // The visible rows are the list's first rows, so the selection is relative to the window.
    let mut list_state = ListState::default();
    list_state.select(
        selected
            .filter(|s| window.contains(s))
            .map(|s| s - window.start),
    );
    let list = List::new(items)
        .block(block)
        .highlight_style(views::highlight(focused));
    frame.render_stateful_widget(list, area, &mut list_state);

    *app.queue_list_state.offset_mut() = window.start;
    app.queue_list_state.select(selected);
    app.hits.queue = Some(ListHit {
        inner,
        offset: window.start,
        len,
    });
}

/// Duration of one queue entry, when it is known.
fn item_duration(app: &App, item: &znicz_core::QueueItem) -> Option<std::time::Duration> {
    match item {
        znicz_core::QueueItem::File { path } => app.meta.get(path).and_then(|e| e.duration),
        znicz_core::QueueItem::Stream { .. } => None,
    }
}

/// Widest time label in the queue, and the total length when every entry has one.
fn queue_stats(app: &App, queue: &[znicz_core::QueueItem]) -> (usize, Option<std::time::Duration>) {
    let durations: Vec<Option<std::time::Duration>> =
        queue.iter().map(|item| item_duration(app, item)).collect();
    let time_width = durations
        .iter()
        .map(|d| format::duration_width(*d))
        .max()
        .unwrap_or(0);
    (time_width, now_playing::total_duration(durations))
}

/// Width left for the title after pinning index, marker, and duration.
pub(crate) fn title_slot(app: &App, state: &PlayerState, width: usize) -> usize {
    if state.queue.is_empty() {
        return width;
    }
    let index_width = state.queue.len().to_string().len().max(2);
    let (time_width, _) = queue_stats(app, &state.queue);
    let fixed = index_width + 2 + 2 + time_width + 1;
    width.saturating_sub(fixed)
}
