//! The Messages list: every recent message, newest first, with its age.

use std::time::{Duration, Instant};

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, List, ListItem, Paragraph};

use crate::app::App;
use crate::format;
use crate::hit::ListHit;
use crate::theme;
use crate::toast::Entry;
use crate::views;

/// Mark, space, three-column age, two spaces.
const PREFIX: usize = 7;

pub fn render_modal(frame: &mut Frame, list: Rect, app: &mut App) {
    let popup = crate::layout::list_modal_rect(list);
    app.hits.overlay = Some(popup);
    app.hits.close = views::close_button_rect(popup);
    frame.render_widget(Clear, popup);

    let len = app.toasts.history_len();
    let block = views::pane_block("Messages", true, Some("C clear · Esc close".into()))
        .title(views::close_title());
    if len == 0 {
        let empty = views::placeholder("No messages yet");
        frame.render_widget(Paragraph::new(empty).block(block), popup);
        return;
    }

    let inner = block.inner(popup);
    let text_w = usize::from(inner.width).saturating_sub(PREFIX);
    let selected = app.message_cursor.selected(len);
    let longest = selected
        .and_then(|i| app.toasts.history().nth(i))
        .map_or(0, |e| e.text.chars().count());
    app.message_h_offset = app.message_h_offset.min(longest.saturating_sub(text_w));

    let now = Instant::now();
    let items: Vec<ListItem> = app
        .toasts
        .history()
        .enumerate()
        .map(|(i, entry)| {
            let offset = if Some(i) == selected {
                app.message_h_offset
            } else {
                0
            };
            message_row(entry, now, text_w, offset)
        })
        .collect();

    let list_widget = List::new(items)
        .block(block)
        .highlight_style(views::highlight(true));
    app.messages_list_state.select(selected);
    frame.render_stateful_widget(list_widget, popup, &mut app.messages_list_state);
    app.hits.overlay_list = Some(ListHit {
        inner,
        offset: app.messages_list_state.offset(),
        len,
    });
}

fn message_row(entry: &Entry, now: Instant, text_w: usize, offset: usize) -> ListItem<'static> {
    let (mark, style) = views::level_mark(entry.level);
    let age = short_age(now.saturating_duration_since(entry.at));
    let text_style = if entry.fresh {
        theme::strong()
    } else {
        theme::text()
    };
    ListItem::new(Line::from(vec![
        Span::styled(format!("{mark} "), style),
        Span::styled(format!("{age:>3}  "), theme::dim()),
        Span::styled(format::pan(&entry.text, offset, text_w), text_style),
    ]))
}

/// `12s`, `3m`, `2h`: seconds under a minute, minutes under an hour, then hours.
pub(crate) fn short_age(age: Duration) -> String {
    let secs = age.as_secs();
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        format!("{}m", secs / 60)
    } else {
        format!("{}h", secs / 3600)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ages_use_the_largest_whole_unit() {
        assert_eq!(short_age(Duration::from_secs(5)), "5s");
        assert_eq!(short_age(Duration::from_secs(59)), "59s");
        assert_eq!(short_age(Duration::from_secs(180)), "3m");
        assert_eq!(short_age(Duration::from_secs(7300)), "2h");
    }
}
