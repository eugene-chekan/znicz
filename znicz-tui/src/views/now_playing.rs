//! The header: what is playing, how far in, and how it reaches the speakers.

use std::time::Duration;

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use znicz_core::{PlaybackStatus, PlayerState};

use crate::format;
use crate::theme;
use crate::views;

pub fn render(frame: &mut Frame, area: Rect, state: &PlayerState, show_signal: bool) {
    let track = state.current_track.as_ref();

    let title = match track {
        Some(track) => track.title.clone(),
        None => "Nothing playing".to_string(),
    };
    let subtitle = track
        .and_then(|t| t.artist_album())
        .unwrap_or_else(|| "—".to_string());

    let width = views::inner_width(area);
    let mut lines = vec![
        Line::from(Span::styled(
            format::truncate(&title, width),
            theme::title(),
        )),
        Line::from(Span::styled(
            format::truncate(&subtitle, width),
            theme::subtitle(),
        )),
        seek_line(state, width),
    ];

    if show_signal {
        lines.push(signal_line(state, width));
    }

    let position = format::position_of(state.queue_position, state.queue.len());
    let block = views::pane_block("Now Playing", false, Some(format!("track {position}")));

    frame.render_widget(Paragraph::new(lines).block(block), area);
}

/// The seek bar with elapsed and total time on the right.
fn seek_line(state: &PlayerState, width: usize) -> Line<'static> {
    let total = state.current_track.as_ref().and_then(|t| t.duration);
    let times = format!(
        "{} / {}",
        format::duration(state.position),
        format::duration_opt(total)
    );

    // Leave room for the times plus a space.
    let bar_width = width.saturating_sub(times.chars().count() + 2);
    let ratio = match total {
        Some(total) if !total.is_zero() => state.position.as_secs_f64() / total.as_secs_f64(),
        _ => 0.0,
    };

    let (done, todo) = format::progress_bar(ratio, bar_width);
    Line::from(vec![
        Span::styled(done, theme::progress()),
        Span::styled(todo, theme::progress_track()),
        Span::raw(" "),
        Span::styled(times, theme::text()),
    ])
}

/// File format on the left, the open device stream on the right, and whether
/// the two match.
///
/// This is the line that matters for an audiophile player: if the device would
/// not take the file's own rate, Znicz has to resample, and that is a change to
/// the audio the user should know about rather than guess at.
fn signal_line(state: &PlayerState, width: usize) -> Line<'static> {
    let Some(track) = state.current_track.as_ref() else {
        return Line::from(Span::styled("—", theme::dim()));
    };

    let source = track.format_description();
    let Some(output) = state.output.as_ref() else {
        return Line::from(Span::styled(source, theme::text()));
    };

    let device = format!(
        "{} {} {}",
        format::khz(output.sample_rate),
        format::channel_name(output.channels),
        output.sample_format
    );

    let (badge, badge_style) = if output.bit_perfect {
        ("● bit perfect", theme::good())
    } else {
        ("▲ resampled", theme::warn())
    };

    let mut spans = vec![
        Span::styled(source, theme::text()),
        Span::styled(" → ", theme::dim()),
        Span::styled(device, theme::text()),
    ];

    // Only claim a signal path while audio is actually flowing.
    if state.status != PlaybackStatus::Stopped {
        let used: usize = spans.iter().map(|s| s.content.chars().count()).sum();
        if used + badge.chars().count() + 2 <= width {
            spans.push(Span::raw("  "));
            spans.push(Span::styled(badge, badge_style));
        }
    }

    Line::from(spans)
}

/// Symbol for the current transport state, used here and in the status bar.
pub fn status_symbol(status: PlaybackStatus) -> &'static str {
    match status {
        PlaybackStatus::Playing => "▶",
        PlaybackStatus::Paused => "❚❚",
        PlaybackStatus::Stopped => "■",
    }
}

/// Total time of everything queued, when every entry's length is known.
pub fn total_duration(durations: &[Option<Duration>]) -> Option<Duration> {
    if durations.is_empty() || durations.iter().any(|d| d.is_none()) {
        return None;
    }
    Some(durations.iter().flatten().sum())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transport_symbols_differ_per_state() {
        assert_eq!(status_symbol(PlaybackStatus::Playing), "▶");
        assert_eq!(status_symbol(PlaybackStatus::Paused), "❚❚");
        assert_eq!(status_symbol(PlaybackStatus::Stopped), "■");
    }

    #[test]
    fn queue_length_needs_every_track_measured() {
        let known = [Some(Duration::from_secs(60)), Some(Duration::from_secs(30))];
        assert_eq!(total_duration(&known), Some(Duration::from_secs(90)));

        let partial = [Some(Duration::from_secs(60)), None];
        assert_eq!(
            total_duration(&partial),
            None,
            "an unknown length makes the total a lie"
        );
        assert_eq!(total_duration(&[]), None);
    }
}
