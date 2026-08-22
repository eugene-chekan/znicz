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
use ratatui::widgets::{Block, Borders, Tabs};
use znicz_core::PlayerState;

use crate::app::{App, Pane};
use crate::theme;

/// Below this height the signal-path line is dropped to keep the lists usable.
const COMPACT_HEIGHT: u16 = 20;
/// Below this, the tab bar goes too.
const TINY_HEIGHT: u16 = 12;

pub fn render(frame: &mut Frame, app: &App, state: &PlayerState) {
    let area = frame.area();
    let compact = area.height < COMPACT_HEIGHT;
    let tiny = area.height < TINY_HEIGHT;

    // Now Playing: borders, title, artist, seek bar, and the signal path.
    let header_height = if compact { 5 } else { 6 };
    let tabs_height = if tiny { 0 } else { 1 };

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(tabs_height),
            Constraint::Length(header_height),
            Constraint::Min(3),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .split(area);

    if !tiny {
        render_tabs(frame, chunks[0], app);
    }
    now_playing::render(frame, chunks[1], state, !compact);
    render_pane(frame, chunks[2], app, state);
    status::render_bar(frame, chunks[3], state);
    status::render_footer(frame, chunks[4], app);

    if app.show_help {
        help::render(frame, area);
    }
}

fn render_tabs(frame: &mut Frame, area: Rect, app: &App) {
    let titles: Vec<Line> = Pane::ALL
        .iter()
        .enumerate()
        .map(|(i, pane)| {
            Line::from(vec![
                Span::styled(format!("{} ", i + 1), theme::dim()),
                Span::raw(pane.title()),
            ])
        })
        .collect();

    let tabs = Tabs::new(titles)
        .select(app.pane.index())
        .style(theme::dim())
        .highlight_style(theme::title())
        .divider(Span::styled("│", theme::dim()));

    frame.render_widget(tabs, area);
}

fn render_pane(frame: &mut Frame, area: Rect, app: &App, state: &PlayerState) {
    match app.pane {
        Pane::Queue => queue::render(frame, area, app, state),
        Pane::Library => library::render(frame, area, app),
        Pane::Devices => devices::render(frame, area, app, state),
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
