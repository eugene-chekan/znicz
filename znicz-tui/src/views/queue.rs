//! The queue: what plays next, with real track names.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{List, ListItem, ListState, Paragraph};
use znicz_core::PlayerState;

use crate::app::{App, Pane};
use crate::format;
use crate::theme;
use crate::views;
use crate::views::now_playing;

pub fn render(frame: &mut Frame, area: Rect, app: &App, state: &PlayerState) {
    let focused = app.pane == Pane::Queue;
    let width = views::inner_width(area);

    if state.queue.is_empty() {
        let block = views::pane_block("Queue", focused, None);
        let hint =
            views::placeholder("Queue is empty. Press 2 for the library, then a to add tracks.");
        frame.render_widget(Paragraph::new(hint).block(block), area);
        return;
    }

    // Ask the cache for every visible row; misses fill in on the next frame.
    let entries: Vec<Option<crate::meta::Entry>> =
        state.queue.iter().map(|path| app.meta.get(path)).collect();

    let index_width = state.queue.len().to_string().len().max(2);
    let items: Vec<ListItem> = state
        .queue
        .iter()
        .zip(entries.iter())
        .enumerate()
        .map(|(i, (path, entry))| {
            let is_playing = i == state.queue_position;

            let label = match entry {
                Some(entry) => entry.label(),
                // Fall back to the file name until the tags arrive.
                None => path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("?")
                    .to_string(),
            };
            let time = format::duration_opt(entry.as_ref().and_then(|e| e.duration));

            // number + marker + gap + time
            let fixed = index_width + 2 + 2 + time.chars().count() + 1;
            let label = format::truncate(&label, width.saturating_sub(fixed));

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

    let durations: Vec<Option<std::time::Duration>> = entries
        .iter()
        .map(|e| e.as_ref().and_then(|e| e.duration))
        .collect();
    let summary = match now_playing::total_duration(&durations) {
        Some(total) => format!("{} tracks · {}", state.queue.len(), format::duration(total)),
        None => format!("{} tracks", state.queue.len()),
    };

    let block = views::pane_block("Queue", focused, Some(summary));
    let list = List::new(items).block(block).highlight_style(if focused {
        theme::selected()
    } else {
        views::no_style()
    });

    let mut list_state = ListState::default();
    list_state.select(app.queue_cursor.selected(state.queue.len()));
    frame.render_stateful_widget(list, area, &mut list_state);
}
