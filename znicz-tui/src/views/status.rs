//! The two bottom lines: player state, then messages or key hints.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use znicz_core::{PlaybackStatus, PlayerState};

use crate::app::{App, Focus, Modal, repeat_label};
use crate::format;
use crate::keys;
use crate::theme;
use crate::toast::Level;
use crate::views::now_playing::status_symbol;

/// Transport, volume, toggles and device, in one line.
pub fn render_bar(frame: &mut Frame, area: Rect, state: &PlayerState) {
    let status_style = match state.status {
        PlaybackStatus::Playing => theme::good(),
        PlaybackStatus::Paused => theme::warn(),
        PlaybackStatus::Stopped => theme::dim(),
    };

    let volume = if state.muted {
        Span::styled("  muted ", theme::warn())
    } else {
        Span::styled(
            format!(
                "  {} {:>3.0}%",
                format::volume_bar(state.volume, 8),
                state.volume * 100.0
            ),
            theme::text(),
        )
    };

    let device = state
        .device_name
        .as_deref()
        .or(state.device_id.as_deref())
        .unwrap_or("default device");

    let repeat_style = if state.repeat == znicz_core::RepeatMode::Off {
        theme::toggle_off()
    } else {
        theme::toggle_on()
    };
    let shuffle_style = if state.shuffle {
        theme::toggle_on()
    } else {
        theme::toggle_off()
    };

    let line = Line::from(vec![
        Span::styled(format!("{} ", status_symbol(state.status)), status_style),
        Span::styled(status_text(state.status), status_style),
        volume,
        Span::raw("  "),
        Span::styled(repeat_label(state.repeat), repeat_style),
        Span::raw("  "),
        Span::styled("shuffle", shuffle_style),
        Span::styled(format!("  {device}"), theme::dim()),
    ]);

    frame.render_widget(Paragraph::new(line), area);
}

/// A message if there is one, otherwise the hints for the focused pane.
pub fn render_footer(frame: &mut Frame, area: Rect, app: &App) {
    let line = match app.toasts.current() {
        Some(toast) => {
            let style = match toast.level {
                Level::Info => theme::text(),
                Level::Warn => theme::warn(),
                Level::Error => theme::bad(),
            };
            let prefix = match toast.level {
                Level::Info => "· ",
                Level::Warn => "! ",
                Level::Error => "✖ ",
            };
            Line::from(vec![
                Span::styled(prefix, style),
                Span::styled(
                    format::truncate(&toast.text, area.width.saturating_sub(3) as usize),
                    style,
                ),
            ])
        }
        None => Line::from(Span::styled(hints_for(app), theme::dim())),
    };

    frame.render_widget(Paragraph::new(line), area);
}

fn hints_for(app: &App) -> &'static str {
    if app.modal == Modal::Devices {
        keys::hints("Devices")
    } else if app.focus == Focus::Queue && app.queue_open {
        keys::hints("Queue")
    } else {
        keys::hints("Library")
    }
}

fn status_text(status: PlaybackStatus) -> &'static str {
    match status {
        PlaybackStatus::Playing => "playing",
        PlaybackStatus::Paused => "paused",
        PlaybackStatus::Stopped => "stopped",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_text_is_written_out_for_each_state() {
        assert_eq!(status_text(PlaybackStatus::Playing), "playing");
        assert_eq!(status_text(PlaybackStatus::Paused), "paused");
        assert_eq!(status_text(PlaybackStatus::Stopped), "stopped");
    }
}
