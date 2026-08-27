//! The hint line at the bottom of the frame.

use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::app::{App, Focus, Modal};
use crate::keys;
use crate::theme;

/// Key hints for the focused pane. Toasts never replace this line.
pub fn render_footer(frame: &mut Frame, area: Rect, app: &App) {
    let line = Line::from(Span::styled(hints_for(app), theme::dim()));
    frame.render_widget(Paragraph::new(line), area);
}

fn hints_for(app: &App) -> &'static str {
    let pane = match app.modal {
        Modal::Devices => "Devices",
        Modal::Inspector => "Inspector",
        Modal::Playlists => "Playlists",
        _ => match app.focus {
            Focus::Queue => "Queue",
            Focus::Library => "Library",
        },
    };
    keys::hints(pane)
}
