//! The Update modal: the new release's notes and what to do about them.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Clear, Paragraph, Wrap};

use crate::app::{App, UpdateStep};
use crate::theme;
use crate::views;

const NOT_INSTALLED: &str = "This copy was not installed by the Znicz installer.";
const PLAYER_PLAYING: &str = "The player is playing. Restart it too? Playback resumes in a moment.";

pub fn render_modal(frame: &mut Frame, list: Rect, app: &mut App) {
    let popup = crate::layout::list_modal_rect(list);
    app.hits.overlay = Some(popup);
    app.hits.close = views::close_button_rect(popup);

    let title = match &app.update {
        Some(release) => format!("Update to {}", release.version),
        None => "Update".to_string(),
    };
    let mut block = views::pane_block(&title, true, None).title(views::close_title());
    if let Some(hint) = hint(&app.update_step) {
        block = block.title_bottom(Line::from(Span::styled(hint, theme::dim())).right_aligned());
    }
    let inner = block.inner(popup);
    frame.render_widget(Clear, popup);
    frame.render_widget(block, popup);

    let text = body(app);
    let rows = wrapped_rows(&text, inner.width);
    app.update_scroll = app
        .update_scroll
        .min(rows.saturating_sub(usize::from(inner.height)));
    let scroll = u16::try_from(app.update_scroll).unwrap_or(u16::MAX);
    frame.render_widget(
        Paragraph::new(text)
            .wrap(Wrap { trim: false })
            .scroll((scroll, 0)),
        inner,
    );
}

/// Bottom-right border text: what Enter and Esc do at this step.
fn hint(step: &UpdateStep) -> Option<&'static str> {
    match step {
        UpdateStep::Offer => Some(" Enter update · Esc cancel "),
        UpdateStep::NotInstallable | UpdateStep::Failed(_) => Some(" Esc close "),
        UpdateStep::Installing => None,
        UpdateStep::Done => Some(" Enter restart now · Esc later "),
        UpdateStep::PlayerPlaying => Some(" Enter restart player · Esc keep it playing "),
    }
}

/// The notes and the step's message, one line each.
fn body(app: &App) -> Text<'static> {
    let version = app.update.as_ref().map_or("", |r| r.version.as_str());
    let notes = app.update.as_ref().map_or("", |r| r.notes.as_str());
    let mut lines: Vec<String> = Vec::new();
    match &app.update_step {
        UpdateStep::Offer => lines.extend(notes.lines().map(str::to_string)),
        UpdateStep::NotInstallable => {
            lines.extend(notes.lines().map(str::to_string));
            lines.push(String::new());
            lines.push(NOT_INSTALLED.to_string());
            lines.push(app.update_hint.clone());
        }
        UpdateStep::Installing => lines.push(format!("Updating to {version}…")),
        UpdateStep::Failed(error) => {
            lines.extend(notes.lines().map(str::to_string));
            lines.push(String::new());
            lines.push(format!("Update failed: {error}"));
        }
        UpdateStep::Done => lines.push(format!("Updated to {version}. Restart Znicz to use it.")),
        UpdateStep::PlayerPlaying => lines.push(PLAYER_PLAYING.to_string()),
    }
    Text::from(lines.into_iter().map(Line::from).collect::<Vec<_>>())
}

/// Rows the text takes once wrapped to `width` columns (estimated by characters).
fn wrapped_rows(text: &Text, width: u16) -> usize {
    let width = usize::from(width.max(1));
    text.lines
        .iter()
        .map(|line| line.width().div_ceil(width).max(1))
        .sum()
}
