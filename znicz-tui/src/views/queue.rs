//! The queue: what plays next, with real track names.

use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{List, ListItem, Paragraph};
use ratatui::Frame;
use znicz_core::PlayerState;

use crate::app::{App, Focus};
use crate::format;
use crate::hit::ListHit;
use crate::theme;
use crate::views;
use crate::views::now_playing;

pub fn render(frame: &mut Frame, area: Rect, app: &mut App, state: &PlayerState) {
    let focused = app.focus == Focus::Queue && !app.modal.blocks_list_focus();
    let width = views::inner_width(area);

    if state.queue.is_empty() {
        let block = views::pane_block("Queue", focused, None);
        let hint = views::placeholder("Queue is empty. Add tracks from the library with a or A.");
        frame.render_widget(Paragraph::new(hint).block(block), area);
        return;
    }

    let index_width = state.queue.len().to_string().len().max(2);
    let items: Vec<ListItem> = state
        .queue
        .iter()
        .enumerate()
        .map(|(i, item)| {
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
                app.queue_offset_for(i, state.queue.len()),
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

    let durations: Vec<Option<std::time::Duration>> = state
        .queue
        .iter()
        .map(|item| match item {
            znicz_core::QueueItem::File { path } => app.meta.get(path).and_then(|e| e.duration),
            znicz_core::QueueItem::Stream { .. } => None,
        })
        .collect();
    let summary = match now_playing::total_duration(&durations) {
        Some(total) => format!("{} tracks · {}", state.queue.len(), format::duration(total)),
        None => format!("{} tracks", state.queue.len()),
    };

    let block = views::pane_block("Queue", focused, Some(summary));
    let inner = block.inner(area);
    let list = List::new(items).block(block).highlight_style(if focused {
        theme::selected()
    } else {
        views::no_style()
    });

    app.queue_list_state
        .select(app.queue_cursor.selected(state.queue.len()));
    frame.render_stateful_widget(list, area, &mut app.queue_list_state);
    app.hits.queue = Some(ListHit {
        inner,
        offset: app.queue_list_state.offset(),
        len: state.queue.len(),
    });
}

/// Width left for the title after pinning index, marker, and duration.
pub(crate) fn title_slot(app: &App, state: &PlayerState, width: usize) -> usize {
    if state.queue.is_empty() {
        return width;
    }
    let index_width = state.queue.len().to_string().len().max(2);
    let fixed = state
        .queue
        .iter()
        .map(|item| {
            let time = match item {
                znicz_core::QueueItem::Stream { .. } => format::duration_opt(None),
                znicz_core::QueueItem::File { path } => {
                    format::duration_opt(app.meta.get(path).and_then(|e| e.duration))
                }
            };
            index_width + 2 + 2 + time.chars().count() + 1
        })
        .max()
        .unwrap_or(0);
    width.saturating_sub(fixed)
}
