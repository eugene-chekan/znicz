//! The help overlay, built from the keymap tables so it cannot go stale.

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;

use crate::app::App;
use crate::keys::{self, Binding};
use crate::theme;

const VERSION: &str = env!("CARGO_PKG_VERSION");
const VERSION_PREFIX: &str = " znicz ";
const CLOSE_HINT: &str = " any key closes ";

pub fn render(frame: &mut Frame, area: Rect, app: &mut App) {
    let popup = centered(86, 100, area);
    app.hits.overlay = Some(popup);
    app.hits.close = crate::views::close_button_rect(popup);

    let mut block = Block::default()
        .borders(Borders::ALL)
        .border_style(theme::border_active())
        .title(Span::styled(" Keys ", theme::title()))
        .title(crate::views::close_title())
        .title_bottom(Line::from(Span::styled(CLOSE_HINT, theme::dim())).right_aligned());
    if let Some(version) = version_title(popup.width) {
        block = block.title_bottom(version);
    }
    let inner = block.inner(popup);

    // Clear first, or the pane underneath shows through the overlay.
    frame.render_widget(Clear, popup);
    frame.render_widget(block, popup);

    // Two columns: the full keymap does not fit in one on a normal terminal.
    let mut left = Vec::new();
    section(&mut left, "Playback", keys::GLOBAL);

    let mut right = Vec::new();
    section(&mut right, "Lists", keys::NAVIGATION);
    section(&mut right, "Queue", keys::QUEUE);
    section(&mut right, "Library", keys::LIBRARY);
    section(&mut right, "Devices", keys::DEVICES);
    section(&mut right, "Playlists", keys::PLAYLISTS);
    section(&mut right, "Radio", keys::RADIO);

    // Below this width the columns would each be too narrow to read, so fall
    // back to a single scrolling-free list of the essentials.
    if inner.width < 60 {
        frame.render_widget(Paragraph::new(left).scroll((0, 0)), inner);
        return;
    }

    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(45), Constraint::Percentage(55)])
        .split(inner);

    frame.render_widget(Paragraph::new(left), columns[0]);
    frame.render_widget(Paragraph::new(right), columns[1]);
}

/// The bottom-left version title, or `None` when both titles cannot fit.
///
/// The close hint is functional, so it keeps the row when a tiny popup has no
/// room for the version.
fn version_title(popup_width: u16) -> Option<Line<'static>> {
    let label = format!("{VERSION_PREFIX}{VERSION} ");
    let budget = popup_width.saturating_sub(2) as usize;
    if label.chars().count() + CLOSE_HINT.chars().count() > budget {
        return None;
    }
    Some(Line::from(Span::styled(label, theme::dim())).left_aligned())
}

fn section(lines: &mut Vec<Line<'static>>, title: &str, bindings: &'static [Binding]) {
    lines.push(Line::from(Span::styled(title.to_string(), theme::strong())));
    for binding in bindings {
        lines.push(Line::from(vec![
            Span::raw(" "),
            Span::styled(format!("{:<16}", binding.keys), theme::key()),
            Span::styled(binding.action, theme::text()),
        ]));
    }
}

/// A box centred in `area`, sized as a percentage of it.
fn centered(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(area);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(vertical[1])[1]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_overlay_stays_inside_the_screen() {
        let screen = Rect::new(0, 0, 100, 40);
        let popup = centered(78, 80, screen);
        assert!(popup.x > 0 && popup.y > 0, "should be inset from the edges");
        assert!(popup.right() <= screen.right());
        assert!(popup.bottom() <= screen.bottom());
    }

    #[test]
    fn a_tiny_screen_does_not_produce_a_negative_box() {
        let popup = centered(78, 80, Rect::new(0, 0, 4, 3));
        assert!(popup.width <= 4 && popup.height <= 3);
    }

    #[test]
    fn the_version_title_names_the_package_version() {
        let title = version_title(80).expect("a normal popup has room");
        let text: String = title
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect();
        assert_eq!(text, format!(" znicz {} ", env!("CARGO_PKG_VERSION")));
    }

    #[test]
    fn a_narrow_popup_drops_the_version_and_keeps_the_close_hint() {
        assert!(version_title(30).is_none());
        assert!(version_title(80).is_some());
    }
}
