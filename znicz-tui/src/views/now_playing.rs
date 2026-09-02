//! The transport: what is playing, how far in, and how it reaches the speakers.

use std::sync::Arc;
use std::time::Duration;

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;
use ratatui_image::picker::Picker;
use ratatui_image::{Resize, StatefulImage};
use znicz_core::{PlaybackStatus, PlayerState, RepeatMode};

use crate::app::{repeat_label, App};
use crate::cover::{pick_stream_cover, CoverKey, CoverReady};
use crate::format;
use crate::theme;
use crate::tui_config::CoverProtocol;

pub fn render_transport(
    frame: &mut Frame,
    area: Rect,
    app: &mut App,
    state: &PlayerState,
    show_signal: bool,
    with_cover: bool,
) {
    if area.height == 0 || area.width == 0 {
        return;
    }

    if !with_cover {
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
        return;
    }

    let picker = app.picker.get_or_insert_with(Picker::halfblocks);
    let font = picker.font_size();
    let cover_w = crate::layout::cover_width(area.height, font.width, font.height, area.width);
    let (cover, chrome) = crate::layout::cover_chrome_split(area, cover_w);
    render_cover(frame, cover, app, state);
    render_stacked_chrome(frame, chrome, state, show_signal);
}

fn cover_draw_label(ready: &CoverReady) -> String {
    match ready {
        CoverReady::Pending => "pending".into(),
        CoverReady::Logo => "logo".into(),
        CoverReady::Embedded(img) => format!("{:p}", Arc::as_ptr(img)),
    }
}

fn render_cover(frame: &mut Frame, area: Rect, app: &mut App, state: &PlayerState) {
    if area.width == 0 || area.height == 0 {
        return;
    }

    let use_logo_only = app.tui.cover_protocol == CoverProtocol::Off;
    let ready = match (use_logo_only, state.current_track.as_ref()) {
        (true, _) | (_, None) => CoverReady::Logo,
        (false, Some(track)) => {
            if let Some(p) = track.path.as_deref() {
                app.covers.get(CoverKey::File(p.to_path_buf()))
            } else {
                let icy_ready = match track.icy_stream_url.as_deref() {
                    Some(u) if u.starts_with("http://") || u.starts_with("https://") => {
                        app.covers.get(CoverKey::Url(u.to_string()))
                    }
                    _ => CoverReady::Logo,
                };
                let station_ready = app
                    .stations
                    .iter()
                    .find(|s| Some(s.url.as_str()) == track.url.as_deref())
                    .and_then(|s| s.art.clone())
                    .map(|p| app.covers.get(CoverKey::ImageFile(p)))
                    .unwrap_or(CoverReady::Logo);
                pick_stream_cover(icy_ready, station_ready)
            }
        }
    };

    let key = cover_draw_label(&ready);
    let draw_key = (key, area.width, area.height);
    if app.cover_draw_key.as_ref() != Some(&draw_key) {
        let image = match ready {
            CoverReady::Embedded(img) => img.as_ref().clone(),
            CoverReady::Pending | CoverReady::Logo => app.covers.logo_image().clone(),
        };
        let picker = app.picker.get_or_insert_with(Picker::halfblocks);
        let font = picker.font_size();
        let image =
            crate::cover::fill_cover_slot(&image, area.width, area.height, font.width, font.height);
        app.cover_image = Some(picker.new_resize_protocol(image));
        app.cover_draw_key = Some(draw_key);
    }

    if let Some(protocol) = app.cover_image.as_mut() {
        frame.render_stateful_widget(
            StatefulImage::new().resize(Resize::Fit(None)),
            area,
            protocol,
        );
    }
}

fn render_stacked_chrome(frame: &mut Frame, area: Rect, state: &PlayerState, show_signal: bool) {
    if area.width == 0 || area.height == 0 {
        return;
    }

    let width = area.width as usize;
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints(vec![Constraint::Length(1); area.height as usize])
        .split(area);

    for (i, row) in rows.iter().enumerate() {
        let line = match i {
            0 => title_row(state, width),
            1 => artist_album_row(state, width),
            2 => seek_row(state, width),
            3 if show_signal => signal_line(state, width),
            _ => Line::from(""),
        };
        frame.render_widget(Paragraph::new(line), *row);
    }
}

fn title_row(state: &PlayerState, width: usize) -> Line<'static> {
    if width == 0 {
        return Line::from("");
    }
    let status_style = status_style(state.status);
    let sym = status_marker(state.status);
    let title = state
        .current_track
        .as_ref()
        .map(|t| t.title.as_str())
        .unwrap_or("Nothing playing");
    let sym_len = sym.chars().count();
    if width <= sym_len {
        return Line::from(Span::styled(format::truncate(&sym, width), status_style));
    }
    let title_w = width.saturating_sub(sym_len);
    Line::from(vec![
        Span::styled(sym, status_style),
        Span::styled(format::truncate(title, title_w), theme::title()),
    ])
}

fn artist_album_row(state: &PlayerState, width: usize) -> Line<'static> {
    let text = state
        .current_track
        .as_ref()
        .and_then(|t| t.artist_album())
        .filter(|s| !s.is_empty())
        .unwrap_or_default();
    if text.is_empty() || width == 0 {
        return Line::from("");
    }
    Line::from(Span::styled(
        format::truncate(&text, width),
        theme::subtitle(),
    ))
}

fn seek_row(state: &PlayerState, width: usize) -> Line<'static> {
    if width == 0 {
        return Line::from("");
    }

    let total = state.current_track.as_ref().and_then(|t| t.duration);
    let times = format!(
        " {} / {}",
        format::duration(state.position),
        format::duration_opt(total)
    );
    let times_len = times.chars().count();

    let volume = if state.muted {
        Some(" muted".to_string())
    } else {
        Some(format!(
            " {} {:>3.0}%",
            format::volume_bar(state.volume, 4),
            state.volume * 100.0
        ))
    };

    let mut toggle_text = String::new();
    if state.repeat != RepeatMode::Off {
        toggle_text.push_str(repeat_label(state.repeat));
    }
    if state.shuffle {
        if !toggle_text.is_empty() {
            toggle_text.push_str("  ");
        }
        toggle_text.push_str("shuffle");
    }
    let toggles = if toggle_text.is_empty() {
        None
    } else {
        Some(format!(" {toggle_text}"))
    };

    let mut include_volume = volume.is_some();
    let mut include_toggles = toggles.is_some();
    let mut include_seek = true;

    let volume_len = || volume.as_ref().map(|s| s.chars().count()).unwrap_or(0);
    let toggles_len = || toggles.as_ref().map(|s| s.chars().count()).unwrap_or(0);

    loop {
        let tail = if include_volume { volume_len() } else { 0 }
            + if include_toggles { toggles_len() } else { 0 };
        let middle_extra = if include_seek { 1 + 4 } else { 0 };
        if times_len + tail + middle_extra <= width {
            break;
        }
        if include_toggles {
            include_toggles = false;
        } else if include_volume {
            include_volume = false;
        } else if include_seek {
            include_seek = false;
        } else {
            break;
        }
    }

    let tail_len = if include_volume { volume_len() } else { 0 }
        + if include_toggles { toggles_len() } else { 0 };
    let mut middle_budget = width.saturating_sub(times_len + tail_len);

    let seek_len = if include_seek && middle_budget > 1 {
        (middle_budget.saturating_sub(1)).min(40)
    } else {
        0
    };

    let mut spans = Vec::new();
    if seek_len > 0 {
        spans.push(Span::raw(" "));
        let (done, todo) = seek_bar_parts(state, seek_len);
        spans.push(Span::styled(done, theme::progress()));
        spans.push(Span::styled(todo, theme::progress_track()));
        middle_budget = middle_budget.saturating_sub(1 + seek_len);
    }
    let _ = middle_budget;

    spans.push(Span::styled(times, theme::text()));

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
            if state.shuffle && state.repeat != RepeatMode::Off {
                spans.push(Span::styled(
                    format!(" {}", repeat_label(state.repeat)),
                    repeat_style,
                ));
                spans.push(Span::raw("  "));
                spans.push(Span::styled("shuffle".to_string(), shuffle_style));
            } else if state.shuffle {
                spans.push(Span::styled(tog.clone(), shuffle_style));
            } else {
                spans.push(Span::styled(tog.clone(), repeat_style));
            }
        }
    }

    Line::from(spans)
}

/// One-line transport chrome: symbol, title, subtitle, seek, times, volume, toggles.
fn chrome_line(state: &PlayerState, width: usize) -> Line<'static> {
    if width == 0 {
        return Line::from("");
    }

    let status_style = status_style(state.status);
    let sym = status_marker(state.status);
    let title = state
        .current_track
        .as_ref()
        .map(|t| t.title.as_str())
        .unwrap_or("Nothing playing");

    let total = state.current_track.as_ref().and_then(|t| t.duration);
    let times = format!(
        " {} / {}",
        format::duration(state.position),
        format::duration_opt(total)
    );

    let sym_len = sym.chars().count();
    let times_len = times.chars().count();

    // Times and play symbol are never dropped.
    if width <= sym_len {
        return Line::from(Span::styled(format::truncate(&sym, width), status_style));
    }
    if width <= sym_len + times_len {
        return Line::from(vec![
            Span::styled(format::truncate(&sym, sym_len.min(width)), status_style),
            Span::styled(
                format::truncate(&times, width.saturating_sub(sym_len)),
                theme::text(),
            ),
        ]);
    }

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

    let mut toggle_text = String::new();
    if state.repeat != RepeatMode::Off {
        toggle_text.push_str(repeat_label(state.repeat));
    }
    if state.shuffle {
        if !toggle_text.is_empty() {
            toggle_text.push_str("  ");
        }
        toggle_text.push_str("shuffle");
    }
    let toggles = if toggle_text.is_empty() {
        None
    } else {
        Some(format!(" {toggle_text}"))
    };

    let mut include_volume = volume.is_some();
    let mut include_toggles = toggles.is_some();
    let mut include_subtitle = subtitle.is_some();
    let mut include_seek = true;

    let volume_len = || volume.as_ref().map(|s| s.chars().count()).unwrap_or(0);
    let toggles_len = || toggles.as_ref().map(|s| s.chars().count()).unwrap_or(0);
    let subtitle_len = || subtitle.as_ref().map(|s| s.chars().count()).unwrap_or(0);

    loop {
        let tail = if include_volume { volume_len() } else { 0 }
            + if include_toggles { toggles_len() } else { 0 };
        let middle_extra = if include_subtitle { subtitle_len() } else { 0 }
            + if include_seek { 1 + 4 } else { 0 }; // space + minimal seek bar

        if sym_len + times_len + tail + middle_extra <= width {
            break;
        }

        if include_toggles {
            include_toggles = false;
        } else if include_volume {
            include_volume = false;
        } else if include_seek {
            include_seek = false;
        } else if include_subtitle {
            include_subtitle = false;
        } else {
            break;
        }
    }

    let tail_len = if include_volume { volume_len() } else { 0 }
        + if include_toggles { toggles_len() } else { 0 };
    let mut middle_budget = width.saturating_sub(sym_len + times_len + tail_len);

    let sub_len = if include_subtitle {
        subtitle.as_ref().map(|s| s.chars().count()).unwrap_or(0)
    } else {
        0
    };
    let seek_len = if include_seek && middle_budget > sub_len + 1 {
        (middle_budget.saturating_sub(sub_len + 1)).min(40)
    } else {
        0
    };
    let seek_prefix = if seek_len > 0 { 1 } else { 0 };
    let title_w = middle_budget.saturating_sub(sub_len + seek_prefix + seek_len);
    let truncated_title = format::truncate(title, title_w);
    let title_len = truncated_title.chars().count();
    middle_budget = middle_budget.saturating_sub(title_len);

    let mut spans = vec![
        Span::styled(sym, status_style),
        Span::styled(truncated_title, theme::title()),
    ];

    if include_subtitle {
        if let Some(sub) = &subtitle {
            let len = sub.chars().count();
            if len <= middle_budget {
                spans.push(Span::styled(sub.clone(), theme::subtitle()));
                middle_budget = middle_budget.saturating_sub(len);
            }
        }
    }

    if seek_len > 0 && middle_budget >= seek_prefix + seek_len {
        spans.push(Span::raw(" "));
        let (done, todo) = seek_bar_parts(state, seek_len);
        spans.push(Span::styled(done, theme::progress()));
        spans.push(Span::styled(todo, theme::progress_track()));
    }

    spans.push(Span::styled(times, theme::text()));

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
            if state.shuffle && state.repeat != RepeatMode::Off {
                spans.push(Span::styled(
                    format!(" {}", repeat_label(state.repeat)),
                    repeat_style,
                ));
                spans.push(Span::raw("  "));
                spans.push(Span::styled("shuffle".to_string(), shuffle_style));
            } else if state.shuffle {
                spans.push(Span::styled(tog.clone(), shuffle_style));
            } else {
                spans.push(Span::styled(tog.clone(), repeat_style));
            }
        }
    }

    Line::from(spans)
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

/// Play and pause glyphs are different widths; pad so the title does not jump.
pub fn status_marker(status: PlaybackStatus) -> String {
    format!("{:<2}", status_symbol(status))
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
    fn play_and_pause_markers_occupy_the_same_columns() {
        let play = status_marker(PlaybackStatus::Playing);
        let pause = status_marker(PlaybackStatus::Paused);
        let stop = status_marker(PlaybackStatus::Stopped);
        assert_eq!(play.chars().count(), pause.chars().count());
        assert_eq!(play.chars().count(), stop.chars().count());
        assert_ne!(play.trim(), pause.trim());
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
