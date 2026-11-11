//! The hint line at the bottom of the frame.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::app::App;
use crate::footer_hits::{self, FooterLabel, PlacedHint, SEPARATOR};
use crate::keys::{self, Hint};
use crate::theme;

/// Key hints for the current context. Toasts never replace this line.
pub fn render_footer(frame: &mut Frame, area: Rect, app: &mut App) {
    let context = app.context();
    let unread = app.toasts.unread_alerts().map(|(count, _)| count);
    let labels: Vec<FooterLabel> = keys::context_hints(context)
        .iter()
        .map(|hint| display_label(*hint, unread))
        .collect();
    let more = keys::more_hint(context).map(FooterLabel::from);
    let placed = footer_hits::layout_footer(area, &labels, more);
    app.hits.footer_hints = footer_hits::footer_hits(&placed);
    frame.render_widget(Paragraph::new(footer_line(area, &placed)), area);
}

/// The hint as drawn: `M messages` carries the unread-alerts count.
fn display_label(hint: Hint, unread: Option<usize>) -> FooterLabel {
    let mut label = FooterLabel::from(hint);
    if hint == keys::MESSAGES_OPEN {
        if let Some(count) = unread {
            label.text = format!("{} ({count})", hint.label);
        }
    }
    label
}

/// Hints at their placed columns: ` · ` between neighbours, spaces before `? more`.
fn footer_line(area: Rect, placed: &[PlacedHint]) -> Line<'static> {
    let separator_width = SEPARATOR.chars().count() as u16;
    let mut spans = Vec::new();
    let mut x = area.x;
    for item in placed {
        let gap = item.rect.x.saturating_sub(x);
        if x > area.x && gap == separator_width {
            spans.push(Span::styled(SEPARATOR, theme::dim()));
        } else if gap > 0 {
            spans.push(Span::raw(" ".repeat(usize::from(gap))));
        }
        let (key, action) = item
            .label
            .text
            .split_once(' ')
            .unwrap_or((item.label.text.as_str(), ""));
        spans.push(Span::styled(key.to_string(), theme::key()));
        if !action.is_empty() {
            spans.push(Span::styled(format!(" {action}"), theme::dim()));
        }
        x = item.rect.x + item.rect.width;
    }
    Line::from(spans)
}
