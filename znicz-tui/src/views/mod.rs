//! Drawing. Each pane lives in its own module; this one lays out the frame.

mod devices;
mod help;
mod inspector;
mod library;
mod now_playing;
mod playlists;
mod queue;
mod radio;
mod status;

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;
use znicz_core::PlayerState;

use crate::app::{App, Modal};
use crate::format;
use crate::hit::HitMap;
use crate::line_edit::LineEdit;
use crate::theme;
use crate::toast::Level;

pub fn render(frame: &mut Frame, app: &mut App, state: &PlayerState) {
    app.hits = HitMap::default();
    let area = frame.area();
    let show_cover = app.tui.show_cover;
    let transport = crate::layout::transport_height(area.height, show_cover);
    let compact = area.height < crate::layout::COMPACT_HEIGHT;

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
    app.list_height = list.height;
    app.hits.queue_toggle = list.width.checked_sub(1).map(|w| Rect {
        x: list.x + w,
        y: list.y,
        width: 1,
        height: list.height,
    });
    let lib_w = crate::layout::strip_width(list, app.queue_open);
    app.hits.library_pane = Some(Rect {
        x: list.x,
        y: list.y,
        width: lib_w,
        height: list.height,
    });
    app.title_slot = library::title_slot(
        &app.library,
        crate::layout::strip_inner(list, app.queue_open),
    );
    app.library.clamp_pan(
        app.title_slot,
        crate::layout::strip_inner(list, app.queue_open),
    );

    library::render(frame, list, app);

    match crate::layout::drawer(list, app.queue_open) {
        crate::layout::Drawer::Overlay(rect) | crate::layout::Drawer::Sheet(rect) => {
            frame.render_widget(Clear, rect);
            app.queue_title_slot = queue::title_slot(app, state, crate::views::inner_width(rect));
            app.clamp_queue_pan();
            queue::render(frame, rect, app, state);
        }
        crate::layout::Drawer::Closed => {}
    }

    let with_cover = crate::layout::cover_enabled(area.height, show_cover);
    now_playing::render_transport(frame, chunks[1], app, state, !compact, with_cover);
    status::render_footer(frame, chunks[2], app);

    match app.modal {
        Modal::Help => help::render(frame, area, app),
        Modal::Devices => devices::render_modal(frame, area, app, state),
        Modal::Inspector => inspector::render(frame, area, app, state),
        Modal::Playlists => playlists::render_modal(frame, area, app),
        Modal::Radio => radio::render_modal(frame, area, app),
        Modal::None => {}
    }

    render_toasts(frame, list, app);
}

fn render_toasts(frame: &mut Frame, list: Rect, app: &App) {
    let shown = app.toasts.visible();
    if shown.is_empty() {
        return;
    }
    let boxed = crate::layout::toast_boxed(list);
    let line_width = shown
        .iter()
        .map(|toast| crate::layout::toast_width_for(&toast.text, boxed))
        .max()
        .unwrap_or(1);
    let areas = crate::layout::toast_areas(list, shown.len() as u16, line_width);
    for (toast, area) in shown.iter().zip(areas) {
        frame.render_widget(Clear, area);
        let (marker, style) = toast_mark(toast.level);
        let inner_w = if boxed {
            inner_width(area)
        } else {
            area.width as usize
        };
        let text_w = inner_w.saturating_sub(crate::layout::TOAST_MARK_AND_SPACE as usize);
        let text = format::truncate(&toast.text, text_w);
        let line = Line::from(vec![
            Span::styled(format!("{marker} "), style),
            Span::styled(text, theme::strong()),
        ]);
        if boxed {
            let block = Block::default().borders(Borders::ALL).border_style(style);
            frame.render_widget(Paragraph::new(line).block(block), area);
        } else {
            frame.render_widget(Paragraph::new(line), area);
        }
    }
}

fn toast_mark(level: Level) -> (&'static str, Style) {
    match level {
        Level::Info => ("●", theme::info()),
        Level::Success => ("●", theme::good()),
        Level::Warn => ("▲", theme::warn()),
        Level::Error => ("x", theme::bad()),
    }
}

/// Top-right cell for the close control (`X`), inside the border row.
pub(crate) fn close_button_rect(area: Rect) -> Option<Rect> {
    if area.width < 3 || area.height < 1 {
        return None;
    }
    Some(Rect {
        x: area.x + area.width - 2,
        y: area.y,
        width: 1,
        height: 1,
    })
}

pub(crate) fn close_title() -> Line<'static> {
    // Single "X", right-aligned into the titles strip (inside the borders). That
    // lands on `area.x + width - 2`, matching `close_button_rect`. A padded
    // " X " put the glyph one cell left of the hit.
    Line::from(Span::styled("X", theme::key())).right_aligned()
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

/// A one-line prompt: prefix, typed text, and a block caret at the cursor.
pub(crate) fn prompt_line(prefix: &str, edit: &LineEdit) -> Line<'static> {
    let (before, after) = edit.split_at_cursor();
    Line::from(vec![
        Span::styled(prefix.to_string(), theme::key()),
        Span::styled(before.to_string(), theme::strong()),
        Span::styled("█", theme::progress()),
        Span::styled(after.to_string(), theme::strong()),
    ])
}

/// The same prompt row without a caret, for the unfocused field of a two-line form.
pub(crate) fn prompt_line_idle(prefix: &str, text: &str) -> Line<'static> {
    Line::from(vec![
        Span::styled(prefix.to_string(), theme::dim()),
        Span::styled(text.to_string(), theme::strong()),
    ])
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
