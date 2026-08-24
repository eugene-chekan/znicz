//! The output device picker.
//!
//! Worth its own pane in a player that cares about the signal path: the device
//! decides whether the file's own sample rate can be used at all.

use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, List, ListItem, ListState, Paragraph};
use ratatui::Frame;
use znicz_core::PlayerState;

use crate::app::{App, Modal};
use crate::format;
use crate::theme;
use crate::views;

pub fn render_modal(frame: &mut Frame, area: Rect, app: &App, state: &PlayerState) {
    let popup = centered_modal(area);
    frame.render_widget(Clear, popup);
    render(frame, popup, app, state);
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

pub fn render(frame: &mut Frame, area: Rect, app: &App, state: &PlayerState) {
    let focused = app.modal == Modal::Devices;
    let width = views::inner_width(area);

    if app.devices.is_empty() {
        let block = views::pane_block("Devices", focused, None);
        let hint = views::placeholder("No output devices found. Press R to look again.");
        frame.render_widget(Paragraph::new(hint).block(block), area);
        return;
    }

    let items: Vec<ListItem> = app
        .devices
        .iter()
        .map(|device| {
            let in_use = state.device_id.as_deref() == Some(device.id.as_str())
                || (state.device_id.is_none() && device.is_default);

            let marker = if in_use { "▶ " } else { "  " };
            let tag = if device.is_default { " (default)" } else { "" };
            let label = format::truncate(&format!("{}{tag}", device.name), width.saturating_sub(4));

            ListItem::new(Line::from(vec![
                Span::styled(marker, theme::playing()),
                Span::styled(
                    label,
                    if in_use {
                        theme::playing()
                    } else {
                        theme::text()
                    },
                ),
            ]))
        })
        .collect();

    // Show what the open stream actually settled on, not just the device name.
    let summary = match state.output.as_ref() {
        Some(output) => format!(
            "open: {} {} {}",
            format::khz(output.sample_rate),
            format::channel_name(output.channels),
            output.sample_format
        ),
        None => "no stream open".to_string(),
    };

    let list = List::new(items)
        .block(views::pane_block("Devices", focused, Some(summary)))
        .highlight_style(if focused {
            theme::selected()
        } else {
            views::no_style()
        });

    let mut list_state = ListState::default();
    list_state.select(app.device_cursor.selected(app.devices.len()));
    frame.render_stateful_widget(list, area, &mut list_state);
}
