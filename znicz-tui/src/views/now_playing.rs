//! The transport: what is playing, how far in, and how it reaches the speakers.

use std::time::Duration;

use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;
use znicz_core::{PlaybackStatus, PlayerState, RepeatMode};

use crate::app::repeat_label;
use crate::format;
use crate::theme;

pub fn render_transport(frame: &mut Frame, area: Rect, state: &PlayerState, show_signal: bool) {
    if area.height == 0 || area.width == 0 {
        return;
    }

    let width = area.width as usize;

    if area.height >= 2 && show_signal {
        let chrome_area = Rect {
            x: area.x,
            y: area.y,
            width: area.width,
            height: 1,
        };
        let signal_area = Rect {
            x: area.x,
            y: area.y + 1,
            width: area.width,
            height: 1,
        };
        frame.render_widget(Paragraph::new(chrome_line(state, width)), chrome_area);
        frame.render_widget(Paragraph::new(signal_line(state, width)), signal_area);
    } else {
        frame.render_widget(Paragraph::new(chrome_line(state, width)), area);
    }
}

/// One-line transport chrome: symbol, title, seek, times, volume, repeat, shuffle.
fn chrome_line(state: &PlayerState, width: usize) -> Line<'static> {
    if width == 0 {
        return Line::from("");
    }

    let status_style = status_style(state.status);
    let symbol = status_symbol(state.status);
    let sym = format!("{symbol} ");
    let title = state
        .current_track
        .as_ref()
        .map(|t| t.title.as_str())
        .unwrap_or("Nothing playing");

    let total = state.current_track.as_ref().and_then(|t| t.duration);
    let times = format!(
        "{} / {}",
        format::duration(state.position),
        format::duration_opt(total)
    );
    let times_suffix = format!(" {times}");

    let subtitle = state
        .current_track
        .as_ref()
        .and_then(|t| t.artist_album())
        .filter(|s| !s.is_empty())
        .map(|s| format!(" {s}"));

    let volume = if state.muted {
        Some(" muted".to_string())
    } else {
        Some(format!(
            " {} {:>3.0}%",
            format::volume_bar(state.volume, 4),
            state.volume * 100.0
        ))
    };

    let mut toggles = String::new();
    if state.repeat != RepeatMode::Off {
        toggles.push_str(repeat_label(state.repeat));
    }
    if state.shuffle {
        if !toggles.is_empty() {
            toggles.push_str("  ");
        }
        toggles.push_str("shuffle");
    }
    let toggles = if toggles.is_empty() {
        None
    } else {
        Some(format!(" {toggles}"))
    };

    let mut include_subtitle = subtitle.is_some();
    let mut include_seek = true;
    let mut include_volume = volume.is_some();
    let mut include_toggles = toggles.is_some();

    let sym_len = sym.chars().count();
    let times_len = times_suffix.chars().count();

    loop {
        let mut middle_len = 0usize;
        if include_subtitle {
            middle_len += subtitle.as_ref().map(|s| s.chars().count()).unwrap_or(0);
        }
        if include_seek {
            middle_len += 1; // space before seek
            middle_len += seek_bar_width(20); // provisional, recalculated below
        }

        let mut right_len = 0usize;
        if include_volume {
            right_len += volume.as_ref().map(|s| s.chars().count()).unwrap_or(0);
        }
        if include_toggles {
            right_len += toggles.as_ref().map(|s| s.chars().count()).unwrap_or(0);
        }

        let overhead = sym_len + middle_len + right_len + times_len;
        let title_budget = width.saturating_sub(overhead);

        if title_budget >= 1 {
            break;
        }

        if include_toggles {
            include_toggles = false;
        } else if include_volume {
            include_volume = false;
        } else if include_subtitle {
            include_subtitle = false;
        } else if include_seek {
            include_seek = false;
        } else {
            break;
        }
    }

    // Recompute with actual seek width from whatever title space remains.
    let right_text = {
        let mut parts = String::new();
        if include_volume {
            parts.push_str(volume.as_deref().unwrap_or(""));
        }
        if include_toggles {
            parts.push_str(toggles.as_deref().unwrap_or(""));
        }
        parts
    };
    let right_len = right_text.chars().count();

    let fixed = sym_len + right_len + times_len;
    let mut middle_budget = width.saturating_sub(fixed);
    let truncated_title = {
        let sub_len = if include_subtitle {
            subtitle.as_ref().map(|s| s.chars().count()).unwrap_or(0)
        } else {
            0
        };
        let seek_space = if include_seek {
            middle_budget.saturating_sub(sub_len + 1).min(20)
        } else {
            0
        };
        let seek_len = if include_seek && seek_space > 0 {
            seek_space + 1
        } else {
            0
        };
        let title_w = middle_budget.saturating_sub(sub_len + seek_len);
        format::truncate(title, title_w.max(1))
    };

    let title_len = truncated_title.chars().count();
    middle_budget = middle_budget.saturating_sub(title_len);

    let mut spans = vec![
        Span::styled(sym, status_style),
        Span::styled(truncated_title, theme::title()),
    ];

    if include_subtitle {
        if let Some(sub) = &subtitle {
            let sub_len = sub.chars().count();
            if sub_len <= middle_budget {
                spans.push(Span::styled(sub.clone(), theme::subtitle()));
                middle_budget = middle_budget.saturating_sub(sub_len);
            }
        }
    }

    if include_seek && middle_budget > 1 {
        let bar_w = middle_budget.saturating_sub(1).min(40);
        if bar_w > 0 {
            spans.push(Span::raw(" "));
            middle_budget = middle_budget.saturating_sub(1);
            let (done, todo) = seek_bar_parts(state, bar_w);
            spans.push(Span::styled(done, theme::progress()));
            spans.push(Span::styled(todo, theme::progress_track()));
            middle_budget = middle_budget.saturating_sub(bar_w);
        }
    }

    if include_volume {
        if let Some(vol) = &volume {
            spans.push(Span::styled(
                vol.clone(),
                if state.muted {
                    theme::warn()
                } else {
                    theme::text()
                },
            ));
        }
    }

    if include_toggles {
        if let Some(tog) = &toggles {
            let repeat_style = if state.repeat == RepeatMode::Off {
                theme::toggle_off()
            } else {
                theme::toggle_on()
            };
            let shuffle_style = if state.shuffle {
                theme::toggle_on()
            } else {
                theme::toggle_off()
            };
            if tog.contains("shuffle") && state.shuffle {
                if let Some(repeat) = (state.repeat != RepeatMode::Off)
                    .then(|| repeat_label(state.repeat))
                {
                    spans.push(Span::styled(format!(" {repeat}  "), repeat_style));
                }
                spans.push(Span::styled("shuffle".to_string(), shuffle_style));
            } else {
                spans.push(Span::styled(tog.clone(), repeat_style));
            }
        }
    }

    spans.push(Span::styled(times_suffix, theme::text()));
    Line::from(spans)
}

fn seek_bar_width(max: usize) -> usize {
    max.min(20)
}

fn seek_bar_parts(state: &PlayerState, width: usize) -> (String, String) {
    let total = state.current_track.as_ref().and_then(|t| t.duration);
    let ratio = match total {
        Some(total) if !total.is_zero() => state.position.as_secs_f64() / total.as_secs_f64(),
        _ => 0.0,
    };
    format::progress_bar(ratio, width)
}

/// File format on the left, the open device stream on the right, and whether
/// the two match.
fn signal_line(state: &PlayerState, width: usize) -> Line<'static> {
    let Some(track) = state.current_track.as_ref() else {
        return Line::from(Span::styled("—", theme::dim()));
    };

    let source = track.format_description();
    let Some(output) = state.output.as_ref() else {
        return Line::from(Span::styled(source, theme::text()));
    };

    let device = format!(
        "{} {}",
        format::khz(output.sample_rate),
        format::channel_name(output.channels),
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

    if state.status != PlaybackStatus::Stopped {
        let used: usize = spans.iter().map(|s| s.content.chars().count()).sum();
        if used + badge.chars().count() + 2 <= width {
            spans.push(Span::raw("  "));
            spans.push(Span::styled(badge, badge_style));
        }
    }

    Line::from(spans)
}

fn status_style(status: PlaybackStatus) -> ratatui::style::Style {
    match status {
        PlaybackStatus::Playing => theme::good(),
        PlaybackStatus::Paused => theme::warn(),
        PlaybackStatus::Stopped => theme::dim(),
    }
}

/// Symbol for the current transport state.
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
