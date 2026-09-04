//! The hint line at the bottom of the frame.

use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::app::{App, Focus, Modal};
use crate::footer_hits;
use crate::keys;
use crate::theme;

/// Key hints for the focused pane. Toasts never replace this line.
pub fn render_footer(frame: &mut Frame, area: Rect, app: &mut App) {
    let text = hints_for(app);
    app.hits.footer_hints = footer_hits::layout_footer_hits(area, text);
    let line = Line::from(Span::styled(text, theme::dim()));
    frame.render_widget(Paragraph::new(line), area);
}

fn hints_for(app: &App) -> &'static str {
    if app.library.is_typing() || app.playlist_prompt.is_some() || app.radio_prompt.is_some() {
        return "type · ← → · Enter · Esc cancel";
    }
    let pane = match app.modal {
        Modal::Devices => "Devices",
        Modal::Inspector => "Inspector",
        Modal::Playlists => "Playlists",
        Modal::Radio => "Radio",
        _ => match app.focus {
            Focus::Queue => "Queue",
            Focus::Library => "Library",
        },
    };
    keys::hints(pane)
}
