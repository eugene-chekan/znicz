//! Saved radio stations: pick one to play, or add, rename, and delete.

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, List, ListItem, ListState, Paragraph};
use ratatui::Frame;

use crate::app::{App, Modal, RadioPrompt};
use crate::format;
use crate::theme;
use crate::views;

pub fn render_modal(frame: &mut Frame, area: Rect, app: &App) {
    let popup = centered_modal(area);
    frame.render_widget(Clear, popup);
    render(frame, popup, app);
}

fn centered_modal(area: Rect) -> Rect {
    let width = ((area.width as u32 * 70 / 100).max(40) as u16).min(area.width);
    let height = ((area.height as u32 * 70 / 100).max(8) as u16).min(area.height);
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    Rect {
        x,
        y,
        width,
        height,
    }
}

pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    let focused = app.modal == Modal::Radio;
    let prompting = app.radio_prompt.is_some();
    let hint = if prompting {
        "Enter confirm · Esc cancel"
    } else {
        "Enter play · a add · w rename · c URL · d delete · Esc close"
    };
    let block = views::pane_block("Radio", focused, Some(hint.to_string()));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let (prompt_area, list_area) = if prompting {
        let parts = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(1), Constraint::Min(1)])
            .split(inner);
        (Some(parts[0]), parts[1])
    } else {
        (None, inner)
    };

    if let (Some(rect), Some(prompt)) = (prompt_area, &app.radio_prompt) {
        let (prefix, text) = match prompt {
            RadioPrompt::AddName(s) => ("name: ", s.as_str()),
            RadioPrompt::AddUrl { buffer, .. } => ("url: ", buffer.as_str()),
            RadioPrompt::Rename(s) => ("rename: ", s.as_str()),
            RadioPrompt::ChangeUrl(s) => ("url: ", s.as_str()),
        };
        let line = Line::from(vec![
            Span::styled(prefix, theme::key()),
            Span::styled(text.to_string(), theme::strong()),
            Span::styled("█", theme::progress()),
        ]);
        frame.render_widget(Paragraph::new(line), rect);
    }

    if app.stations.is_empty() {
        let hint = views::placeholder("(empty)  —  a to add a station");
        frame.render_widget(Paragraph::new(hint), list_area);
        return;
    }

    let width = list_area.width as usize;
    let items: Vec<ListItem> = app
        .stations
        .iter()
        .map(|station| {
            let label = format::truncate(&station.name, width);
            ListItem::new(Line::from(Span::styled(label, theme::text())))
        })
        .collect();

    let list = List::new(items).highlight_style(if focused {
        theme::selected()
    } else {
        views::no_style()
    });

    let mut list_state = ListState::default();
    list_state.select(app.station_cursor.selected(app.stations.len()));
    frame.render_stateful_widget(list, list_area, &mut list_state);
}
