//! The output device picker.
//!
//! Worth its own pane in a player that cares about the signal path: the device
//! decides whether the file's own sample rate can be used at all. Opened from
//! Settings it picks the default output instead, with System default first.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, List, ListItem, Paragraph};
use znicz_core::PlayerState;

use crate::app::{App, DevicesMode, Modal};
use crate::format;
use crate::hit::ListHit;
use crate::theme;
use crate::views;

const DEFAULT_TITLE: &str = "Default output device";
const DEFAULT_HINT: &str = "Enter choose · Esc back";
const SYSTEM_DEFAULT: &str = "System default";

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
    let picking_default = app.devices_mode == DevicesMode::Default;

    if app.devices.is_empty() && !picking_default {
        let block = views::pane_block("Devices", focused, None).title(views::close_title());
        let hint = views::placeholder("No output devices found. Press R to look again.");
        frame.render_widget(Paragraph::new(hint).block(block), area);
        return;
    }

    // (in use, text) for each row, System default first when picking the default.
    let mut rows: Vec<(bool, String)> = Vec::new();
    if picking_default {
        rows.push((app.default_device.is_none(), SYSTEM_DEFAULT.to_string()));
    }
    for device in &app.devices {
        let in_use = if picking_default {
            app.default_device.as_deref() == Some(device.id.as_str())
        } else {
            state.device_id.as_deref() == Some(device.id.as_str())
                || (state.device_id.is_none() && device.is_default)
        };
        let tag = if device.is_default { " (default)" } else { "" };
        rows.push((in_use, format!("{}{tag}", device.name)));
    }
    let items: Vec<ListItem> = rows
        .into_iter()
        .map(|(in_use, text)| {
            let marker = if in_use { "▶ " } else { "  " };
            let label = format::truncate(&text, width.saturating_sub(4));
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
    let len = items.len();

    let block = if picking_default {
        views::pane_block(DEFAULT_TITLE, focused, Some(DEFAULT_HINT.to_string()))
    } else {
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
        views::pane_block("Devices", focused, Some(summary))
    }
    .title(views::close_title());
    let inner = block.inner(area);
    let list = List::new(items)
        .block(block)
        .highlight_style(views::highlight(focused));

    app.device_list_state
        .select(app.device_cursor.selected(len));
    frame.render_stateful_widget(list, area, &mut app.device_list_state);
    app.hits.overlay_list = Some(ListHit {
        inner,
        offset: app.device_list_state.offset(),
        len,
    });
}
