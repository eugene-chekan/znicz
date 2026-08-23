//! Drawing. Each pane lives in its own module; this one lays out the frame.

mod devices;
mod help;
mod library;
mod now_playing;
mod queue;
mod status;

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use znicz_core::PlayerState;

use crate::app::{App, Modal};
use crate::format;
use crate::theme;
use crate::toast::Level;

/// Below this height the signal-path line is dropped to keep the lists usable.
const COMPACT_HEIGHT: u16 = 20;

pub fn render(frame: &mut Frame, app: &mut App, state: &PlayerState) {
    let area = frame.area();
    let compact = area.height < COMPACT_HEIGHT;
    let transport = if compact { 1 } else { 2 };

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(3),
            Constraint::Length(transport),
            Constraint::Length(1),
        ])
        .split(area);

    let list = chunks[0];
    app.list_width = list.width;
    app.title_slot = crate::layout::strip_inner(list, app.queue_open).saturating_sub(8);
    app.library.clamp_pan(app.title_slot);

    library::render(frame, list, app);

    match crate::layout::drawer(list, app.queue_open) {
        crate::layout::Drawer::Overlay(rect) | crate::layout::Drawer::Sheet(rect) => {
            frame.render_widget(Clear, rect);
            queue::render(frame, rect, app, state);
        }
        crate::layout::Drawer::Closed => {}
    }

    now_playing::render_transport(frame, chunks[1], state, !compact);
    status::render_footer(frame, chunks[2], app);

    match app.modal {
        Modal::Help => help::render(frame, area),
        Modal::Devices => devices::render_modal(frame, area, app, state),
        Modal::None => {}
    }

    render_toasts(frame, list, app);
}

fn render_toasts(frame: &mut Frame, list: Rect, app: &App) {
    let shown = app.toasts.visible();
    let max_width = ((list.width as usize) * 40 / 100).clamp(8, 40) as u16;
    let areas = crate::layout::toast_areas(list, shown.len() as u16, max_width);
    for (toast, area) in shown.iter().zip(areas) {
        frame.render_widget(Clear, area);
        let style = match toast.level {
            Level::Info => theme::text(),
            Level::Warn => theme::warn(),
            Level::Error => theme::bad(),
        };
        let text = format::truncate(&toast.text, area.width as usize);
        frame.render_widget(Paragraph::new(Span::styled(text, style)), area);
    }
}

/// Border for a pane, highlighted when it has focus.
pub(crate) fn pane_block(title: &str, focused: bool, right: Option<String>) -> Block<'static> {
    let style = if focused {
        theme::border_active()
    } else {
        theme::border_idle()
    };

    let mut block = Block::default()
        .borders(Borders::ALL)
        .border_style(style)
        .title(Span::styled(
            format!(" {title} "),
            if focused {
                theme::title()
            } else {
                theme::dim()
            },
        ));

    if let Some(right) = right {
        block = block.title_bottom(
            Line::from(Span::styled(format!(" {right} "), theme::dim())).right_aligned(),
        );
    }
    block
}

/// A row that fills a pane which has nothing to show.
pub(crate) fn placeholder(text: &str) -> Line<'static> {
    Line::from(Span::styled(text.to_string(), theme::dim()))
}

/// Width available inside a bordered pane.
pub(crate) fn inner_width(area: Rect) -> usize {
    area.width.saturating_sub(2) as usize
}

pub(crate) fn no_style() -> Style {
    Style::default()
}
