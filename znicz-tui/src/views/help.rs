//! The help overlay, built from the keymap tables so it cannot go stale.

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;

use crate::keys::{self, Binding};
use crate::theme;

pub fn render(frame: &mut Frame, area: Rect) {
    let popup = centered(86, 88, area);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(theme::border_active())
        .title(Span::styled(" Keys ", theme::title()))
        .title_bottom(Line::from(Span::styled(" any key closes ", theme::dim())).right_aligned());
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

fn section(lines: &mut Vec<Line<'static>>, title: &str, bindings: &'static [Binding]) {
    lines.push(Line::from(Span::styled(title.to_string(), theme::strong())));
    for binding in bindings {
        lines.push(Line::from(vec![
            Span::raw(" "),
            Span::styled(format!("{:<16}", binding.keys), theme::key()),
            Span::styled(binding.action, theme::text()),
        ]));
    }
    lines.push(Line::from(""));
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
}
