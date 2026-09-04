//! Saved M3U playlists: pick one to play or add, or write the queue out.

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, List, ListItem, Paragraph};
use ratatui::Frame;

use crate::app::{App, Modal, PlaylistPrompt};
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
    app.hits.close = views::close_button_rect(area);
    let focused = app.modal == Modal::Playlists;
    let prompting = app.playlist_prompt.is_some();
    let hint = if prompting {
        "← → move · Enter confirm · Esc cancel"
    } else {
        "Enter play · a add · n new · e edit · c copy · d delete · Esc close"
    };
    let block = views::pane_block("Playlists", focused, Some(hint.to_string()))
        .title(views::close_title());
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

    if let (Some(rect), Some(prompt)) = (prompt_area, &app.playlist_prompt) {
        let (prefix, edit) = match prompt {
            PlaylistPrompt::Save(edit) => ("save: ", edit),
            PlaylistPrompt::Rename(edit) => ("rename: ", edit),
            PlaylistPrompt::Copy(edit) => ("copy: ", edit),
        };
        frame.render_widget(Paragraph::new(views::prompt_line(prefix, edit)), rect);
    }

    if app.playlists.is_empty() {
        let hint = views::placeholder("(empty)  —  n to save the queue");
        frame.render_widget(Paragraph::new(hint), list_area);
        return;
    }

    let width = list_area.width as usize;
    let items: Vec<ListItem> = app
        .playlists
        .iter()
        .map(|name| {
            let label = format::truncate(name, width);
            ListItem::new(Line::from(Span::styled(label, theme::text())))
        })
        .collect();

    let list = List::new(items).highlight_style(if focused {
        theme::selected()
    } else {
        views::no_style()
    });

    app.playlist_list_state
        .select(app.playlist_cursor.selected(app.playlists.len()));
    frame.render_stateful_widget(list, list_area, &mut app.playlist_list_state);
    app.hits.overlay_list = Some(ListHit {
        inner: list_area,
        offset: app.playlist_list_state.offset(),
        len: app.playlists.len(),
    });
}
