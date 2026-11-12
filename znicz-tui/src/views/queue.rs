//! The queue: what plays next, with real track names.

use std::time::Duration;

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

pub fn render(
    frame: &mut Frame,
    area: Rect,
    app: &mut App,
    state: &PlayerState,
    stats: &QueueStats,
) {
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
    let summary = match stats.total {
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

/// What the queue header and title need, gathered in one pass over the queue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct QueueStats {
    /// Widest time label, in columns.
    pub time_width: usize,
    /// Total length, when the queue is non-empty and every entry's length is known.
    pub total: Option<Duration>,
}

/// Stats for the queue as it is now, reading each entry's length once.
pub(crate) fn stats(app: &App, queue: &[znicz_core::QueueItem]) -> QueueStats {
    stats_of(queue.iter().map(|item| match item {
        znicz_core::QueueItem::File { path } => app.meta.get(path).and_then(|e| e.duration),
        znicz_core::QueueItem::Stream { .. } => None,
    }))
}

fn stats_of(durations: impl IntoIterator<Item = Option<Duration>>) -> QueueStats {
    let mut time_width = 0;
    let mut count = 0usize;
    let mut total = Duration::ZERO;
    let mut all_known = true;
    for d in durations {
        count += 1;
        time_width = time_width.max(format::duration_width(d));
        match d {
            Some(d) => total += d,
            None => all_known = false,
        }
    }
    QueueStats {
        time_width,
        total: (all_known && count > 0).then_some(total),
    }
}

/// Width left for the title after pinning index, marker, and duration.
pub(crate) fn title_slot(stats: &QueueStats, len: usize, width: usize) -> usize {
    if len == 0 {
        return width;
    }
    let index_width = len.to_string().len().max(2);
    let fixed = index_width + 2 + 2 + stats.time_width + 1;
    width.saturating_sub(fixed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stats_total_only_when_every_length_is_known() {
        let secs = |s| Some(Duration::from_secs(s));
        let known = stats_of([secs(60), secs(30)]);
        assert_eq!(known.total, Some(Duration::from_secs(90)));

        let partial = stats_of([secs(60), None]);
        assert_eq!(
            partial.total, None,
            "an unknown length makes the total a lie"
        );
        assert_eq!(
            partial.time_width, 4,
            "the unknown dash still takes a column"
        );

        assert_eq!(
            stats_of([]),
            QueueStats {
                time_width: 0,
                total: None
            }
        );
    }

    #[test]
    fn stats_time_width_is_the_widest_label() {
        let secs = |s| Some(Duration::from_secs(s));
        assert_eq!(stats_of([secs(59), secs(3600)]).time_width, 7);
        assert_eq!(stats_of([secs(59), None]).time_width, 4);
    }
}
