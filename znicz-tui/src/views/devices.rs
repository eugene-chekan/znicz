//! The output device picker.
//!
//! Worth its own pane in a player that cares about the signal path: the device
//! decides whether the file's own sample rate can be used at all.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, List, ListItem, Paragraph};
use znicz_core::PlayerState;

use crate::app::{App, Modal};
use crate::format;
use crate::hit::ListHit;
use crate::theme;
use crate::views;

pub fn render_modal(frame: &mut Frame, list: Rect, app: &mut App, state: &PlayerState) {
    let popup = crate::layout::list_modal_rect(list);
    app.hits.overlay = Some(popup);
    frame.render_widget(Clear, popup);
    render(frame, popup, app, state);
}

pub fn render(frame: &mut Frame, area: Rect, app: &mut App, state: &PlayerState) {
    app.hits.close = views::close_button_rect(area);
    let focused = app.modal == Modal::Devices;
    let width = views::inner_width(area);

    if app.devices.is_empty() {
        let block = views::pane_block("Devices", focused, None).title(views::close_title());
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

    let block = views::pane_block("Devices", focused, Some(summary)).title(views::close_title());
    let inner = block.inner(area);
    let list = List::new(items)
        .block(block)
        .highlight_style(views::highlight(focused));

    app.device_list_state
        .select(app.device_cursor.selected(app.devices.len()));
    frame.render_stateful_widget(list, area, &mut app.device_list_state);
    app.hits.overlay_list = Some(ListHit {
        inner,
        offset: app.device_list_state.offset(),
        len: app.devices.len(),
    });
}
