//! Saved radio stations: pick one to play, or add, edit, copy, and delete.

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, List, ListItem, Paragraph};
use ratatui::Frame;

use crate::app::{App, Modal, RadioPrompt, StationField};
use crate::format;
use crate::hit::ListHit;
use crate::theme;
use crate::views;

pub fn render_modal(frame: &mut Frame, area: Rect, app: &mut App) {
    let popup = centered_modal(area);
    app.hits.overlay = Some(popup);
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

pub fn render(frame: &mut Frame, area: Rect, app: &mut App) {
    let focused = app.modal == Modal::Radio;
    let form_rows = match &app.radio_prompt {
        Some(RadioPrompt::Form { .. }) => 3,
        Some(RadioPrompt::Copy(_)) => 1,
        None => 0,
    };
    let hint = if form_rows > 0 {
        if form_rows == 3 {
            "Tab field · ← → move · Enter confirm · Esc cancel"
        } else {
            "← → move · Enter confirm · Esc cancel"
        }
    } else {
        "Enter play · a add · n new · e edit · c copy · d delete · Esc close"
    };
    let block = views::pane_block("Radio", focused, Some(hint.to_string()));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let (prompt_area, list_area) = if form_rows > 0 {
        let parts = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(form_rows), Constraint::Min(1)])
            .split(inner);
        (Some(parts[0]), parts[1])
    } else {
        (None, inner)
    };

    if let (Some(rect), Some(prompt)) = (prompt_area, &app.radio_prompt) {
        match prompt {
            RadioPrompt::Form {
                name,
                url,
                art,
                field,
                ..
            } => {
                let rows = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([
                        Constraint::Length(1),
                        Constraint::Length(1),
                        Constraint::Length(1),
                    ])
                    .split(rect);
                let name_line = if *field == StationField::Name {
                    views::prompt_line("name: ", name)
                } else {
                    views::prompt_line_idle("name: ", name.as_str())
                };
                let url_line = if *field == StationField::Url {
                    views::prompt_line("url: ", url)
                } else {
                    views::prompt_line_idle("url: ", url.as_str())
                };
                let art_line = if *field == StationField::Art {
                    views::prompt_line("art: ", art)
                } else {
                    views::prompt_line_idle("art: ", art.as_str())
                };
                frame.render_widget(Paragraph::new(name_line), rows[0]);
                frame.render_widget(Paragraph::new(url_line), rows[1]);
                frame.render_widget(Paragraph::new(art_line), rows[2]);
            }
            RadioPrompt::Copy(edit) => {
                frame.render_widget(Paragraph::new(views::prompt_line("copy: ", edit)), rect);
            }
        }
    }

    if app.stations.is_empty() {
        let hint = views::placeholder("(empty)  —  n to add a station");
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

    app.station_list_state.select(app.station_cursor.selected(app.stations.len()));
    frame.render_stateful_widget(list, list_area, &mut app.station_list_state);
    app.hits.overlay_list = Some(ListHit {
        inner: list_area,
        offset: app.station_list_state.offset(),
        len: app.stations.len(),
    });
}
