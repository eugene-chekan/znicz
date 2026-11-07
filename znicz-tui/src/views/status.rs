//! The hint line at the bottom of the frame.

use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::app::App;
use crate::footer_hits::{self, PlacedHint, SEPARATOR};
use crate::keys;
use crate::theme;

/// Key hints for the current context. Toasts never replace this line.
pub fn render_footer(frame: &mut Frame, area: Rect, app: &mut App) {
    let context = app.context();
    let placed =
        footer_hits::layout_footer(area, keys::context_hints(context), keys::more_hint(context));
    app.hits.footer_hints = footer_hits::footer_hits(&placed);
    frame.render_widget(Paragraph::new(footer_line(area, &placed)), area);
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
            .hint
            .label
            .split_once(' ')
            .unwrap_or((item.hint.label, ""));
        spans.push(Span::styled(key, theme::key()));
        if !action.is_empty() {
            spans.push(Span::styled(format!(" {action}"), theme::dim()));
        }
        x = item.rect.x + item.rect.width;
    }
    Line::from(spans)
}
