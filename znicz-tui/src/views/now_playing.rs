//! The transport: what is playing, how far in, and how it reaches the speakers.

use std::time::Duration;

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use znicz_core::{PlaybackStatus, PlayerState, RepeatMode};

use crate::app::{App, repeat_label};
use crate::format;
use crate::theme;
use crate::tui_config::IconSet;

pub fn render_transport(
    frame: &mut Frame,
    area: Rect,
    app: &mut App,
    state: &PlayerState,
    show_signal: bool,
) {
    if area.height == 0 || area.width == 0 {
        return;
    }

    let width = area.width as usize;
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints(vec![Constraint::Length(1); area.height as usize])
        .split(area);
    let last = rows.len().saturating_sub(1);
    let marquee_on = app.tui.marquee;
    for (i, row) in rows.iter().enumerate() {
        let line = if i == 0 {
            chrome_line(
                state,
                width,
                app.tui.icons,
                if marquee_on {
                    Some((&mut app.transport_marquee, std::time::Instant::now()))
                } else {
                    None
                },
            )
        } else if i == last {
            message_line(&app.toasts, app.library_status.scanning.as_ref(), width)
        } else if show_signal {
            signal_line(state, width)
        } else {
            Line::from("")
        };
        frame.render_widget(Paragraph::new(line), *row);
    }
}

/// The transport's message row: the newest live message (with `+N` for the
/// rest), else the unread-alerts marker, else blank. Always `width` wide.
pub(crate) fn message_line(
    toasts: &crate::toast::Toasts,
    scanning: Option<&znicz_core::ScanProgress>,
    width: usize,
) -> Line<'static> {
    if width == 0 {
        return Line::from("");
    }
    if let Some(toast) = toasts.current() {
        let (mark, style) = crate::views::level_mark(toast.level);
        let extra = toasts.live_extra();
        let suffix = if extra > 0 {
            format!(" +{extra}")
        } else {
            String::new()
        };
        let suffix_w = suffix.chars().count();
        let text = format::truncate(&toast.text, width.saturating_sub(2 + suffix_w));
        let pad = width.saturating_sub(2 + text.chars().count() + suffix_w);
        return Line::from(vec![
            Span::styled(format!("{mark} "), style),
            Span::styled(text, theme::strong()),
            Span::raw(" ".repeat(pad)),
            Span::styled(suffix, theme::dim()),
        ]);
    }
    if let Some(progress) = scanning {
        let home = dirs::home_dir();
        let text = crate::scan_status::progress_text(progress, home.as_deref(), width);
        let pad = width.saturating_sub(text.chars().count());
        return Line::from(vec![
            Span::styled(text, theme::progress()),
            Span::raw(" ".repeat(pad)),
        ]);
    }
    if let Some((count, level)) = toasts.unread_alerts() {
        let (mark, style) = crate::views::level_mark(level);
        let label = format::truncate(&format!("{mark} {count} unread · M"), width);
        let pad = width.saturating_sub(label.chars().count());
        return Line::from(vec![Span::raw(" ".repeat(pad)), Span::styled(label, style)]);
    }
    Line::from(" ".repeat(width))
}

/// One-line transport chrome: symbol, title, subtitle, seek, times, volume, toggles.
fn chrome_line(
    state: &PlayerState,
    width: usize,
    icons: IconSet,
    marquee: Option<(&mut crate::marquee::Marquee, std::time::Instant)>,
) -> Line<'static> {
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

    let icon = format::volume_icon(icons, state.volume, state.muted);
    let bar = format::volume_bar(state.volume, 6);
    let volume = if state.muted {
        Some(format!(" {icon} {bar} muted"))
    } else {
        Some(format!(" {icon} {bar} {:>3.0}%", state.volume * 100.0))
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

    let volume_len = || {
        volume
            .as_ref()
            .map(|s| format::display_width(s))
            .unwrap_or(0)
    };
    let toggles_len = || {
        toggles
            .as_ref()
            .map(|s| format::display_width(s))
            .unwrap_or(0)
    };
    let subtitle_len = || {
        subtitle
            .as_ref()
            .map(|s| format::display_width(s))
            .unwrap_or(0)
    };

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
        subtitle
            .as_ref()
            .map(|s| format::display_width(s))
            .unwrap_or(0)
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
    let slot = middle_budget.saturating_sub(seek_prefix + seek_len);
    let subtitle_text = if include_subtitle {
        subtitle.clone().unwrap_or_default()
    } else {
        String::new()
    };
    let segment = format!("{title}{subtitle_text}");
    let overflows = segment.chars().count() > slot;
    if let (true, Some((marquee, now))) = (overflows, marquee) {
        let offset = marquee.offset(&segment, slot, now);
        let shown = crate::marquee::window(&segment, offset, slot);
        // Title style for the part that is still title, subtitle style after it.
        let title_left = title.chars().count().saturating_sub(offset);
        let title_part: String = shown.chars().take(title_left).collect();
        let sub_part: String = shown.chars().skip(title_left).collect();
        let pad = slot.saturating_sub(shown.chars().count());
        let mut spans = vec![
            Span::styled(sym, status_style),
            Span::styled(title_part, theme::title()),
            Span::styled(sub_part, theme::subtitle()),
            Span::raw(" ".repeat(pad)),
        ];
        push_tail(
            &mut spans,
            state,
            seek_len,
            times,
            include_volume,
            &volume,
            include_toggles,
            &toggles,
        );
        return Line::from(spans);
    }
    let truncated_title = format::truncate(title, title_w);
    let title_len = truncated_title.chars().count();
    middle_budget = middle_budget.saturating_sub(title_len);

    let mut spans = vec![
        Span::styled(sym, status_style),
        Span::styled(truncated_title, theme::title()),
    ];

    if include_subtitle {
        if let Some(sub) = &subtitle {
            let len = format::display_width(sub);
            if len <= middle_budget {
                spans.push(Span::styled(sub.clone(), theme::subtitle()));
                middle_budget = middle_budget.saturating_sub(len);
            }
        }
    }

    let seek_drawn = if seek_len > 0 && middle_budget >= seek_prefix + seek_len {
        seek_len
    } else {
        0
    };
    push_tail(
        &mut spans,
        state,
        seek_drawn,
        times,
        include_volume,
        &volume,
        include_toggles,
        &toggles,
    );

    Line::from(spans)
}

/// Seek bar, times, volume, and toggles: everything right of the title.
#[allow(clippy::too_many_arguments)]
fn push_tail(
    spans: &mut Vec<Span<'static>>,
    state: &PlayerState,
    seek_len: usize,
    times: String,
    include_volume: bool,
    volume: &Option<String>,
    include_toggles: bool,
    toggles: &Option<String>,
) {
    if seek_len > 0 {
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

    fn text(line: &Line) -> String {
        line.spans.iter().map(|s| s.content.as_ref()).collect()
    }

    #[test]
    fn the_message_row_shows_the_newest_live_message() {
        let mut toasts = crate::toast::Toasts::new();
        toasts.info("first");
        toasts.success("added 4 tracks");
        let row = text(&message_line(&toasts, None, 40));
        assert!(row.starts_with("● added 4 tracks"), "{row:?}");
        assert!(row.trim_end().ends_with("+1"), "{row:?}");
        assert_eq!(row.chars().count(), 40);
    }

    #[test]
    fn the_message_row_shows_unread_alerts_once_messages_expire() {
        let mut toasts = crate::toast::Toasts::new();
        toasts.warn("stream dropped");
        toasts.error("device refused");
        toasts.expire_live_for_test();
        let row = text(&message_line(&toasts, None, 40));
        assert!(row.trim_end().ends_with("x 2 unread · M"), "{row:?}");
    }

    #[test]
    fn the_message_row_is_blank_with_nothing_to_say() {
        let toasts = crate::toast::Toasts::new();
        assert_eq!(text(&message_line(&toasts, None, 40)).trim(), "");
    }

    #[test]
    fn a_long_message_is_cut_to_the_row() {
        let mut toasts = crate::toast::Toasts::new();
        toasts.error("player error: playlist had no playable files at all");
        let row = text(&message_line(&toasts, None, 20));
        assert_eq!(row.chars().count(), 20);
        assert!(row.ends_with('…'), "{row:?}");
    }

    #[test]
    fn scan_progress_shows_when_no_message_is_live() {
        let toasts = crate::toast::Toasts::new();
        let progress = znicz_core::ScanProgress {
            folder: std::path::PathBuf::from("/srv/music"),
            seen: 12,
            added: 3,
            updated: 0,
        };
        let row = text(&message_line(&toasts, Some(&progress), 60));
        assert!(
            row.starts_with("⟳ scanning /srv/music · 12 files · 3 new"),
            "{row:?}"
        );
        assert_eq!(row.chars().count(), 60);
    }

    #[test]
    fn a_live_message_beats_scan_progress() {
        let mut toasts = crate::toast::Toasts::new();
        toasts.info("added 4 tracks");
        let progress = znicz_core::ScanProgress::default();
        let row = text(&message_line(&toasts, Some(&progress), 60));
        assert!(row.starts_with("● added 4 tracks"), "{row:?}");
    }

    fn long_state() -> PlayerState {
        PlayerState {
            status: PlaybackStatus::Playing,
            current_track: Some(znicz_core::TrackInfo {
                path: None,
                url: None,
                icy_stream_url: None,
                title: "A Very Long Title That Will Never Fit In This Row".into(),
                codec: "FLAC".into(),
                sample_rate: 44_100,
                channels: 2,
                bits_per_sample: Some(16),
                bitrate_kbps: None,
                duration: Some(Duration::from_secs(200)),
                tags: znicz_core::TrackTags::default(),
            }),
            ..PlayerState::default()
        }
    }

    #[test]
    fn without_a_marquee_a_long_title_is_cut() {
        // At 100 columns the title gets 35 (the seek bar takes up to 40).
        let row = text(&chrome_line(&long_state(), 100, IconSet::Unicode, None));
        assert!(row.contains('…'), "{row:?}");
    }

    #[test]
    fn with_a_marquee_a_long_title_scrolls() {
        let state = long_state();
        let mut m = crate::marquee::Marquee::default();
        let t0 = std::time::Instant::now();
        let start = text(&chrome_line(
            &state,
            100,
            IconSet::Unicode,
            Some((&mut m, t0)),
        ));
        assert!(start.contains("A Very"), "{start:?}");
        assert!(!start.contains('…'), "{start:?}");
        let later = text(&chrome_line(
            &state,
            100,
            IconSet::Unicode,
            Some((&mut m, t0 + Duration::from_millis(2_000 + 5 * 200))),
        ));
        assert!(!later.contains("A Very"), "moved 5 columns: {later:?}");
        assert!(later.contains("y Long Title"), "{later:?}");
    }

    fn line_width(line: &Line) -> usize {
        line.spans.iter().map(|s| s.content.chars().count()).sum()
    }

    #[test]
    fn an_emoji_volume_is_measured_in_cells_not_chars() {
        // Two cells for the speaker emoji: at 33 the volume segment only
        // fits if it is counted by display width (it needs 34 cells).
        let state = PlayerState {
            volume: 1.0,
            ..long_state()
        };
        let state = PlayerState {
            current_track: state.current_track.map(|mut t| {
                t.title = "Song".into();
                t
            }),
            ..state
        };
        let narrow = chrome_line(&state, 33, IconSet::Emoji, None);
        let t = text(&narrow);
        assert!(!t.contains('🔊'), "{t:?}");
        assert!(line_width(&narrow) <= 33, "{t:?}");

        let wide = text(&chrome_line(&state, 100, IconSet::Emoji, None));
        assert!(wide.contains('🔊'), "{wide:?}");
    }

    #[test]
    fn a_scrolling_title_fills_the_row_exactly() {
        // Long enough to overflow even at 140 columns.
        let mut state = long_state();
        if let Some(track) = state.current_track.as_mut() {
            track.title = "Very Long Title ".repeat(12);
        }
        for width in [80, 100, 140] {
            let mut m = crate::marquee::Marquee::default();
            let t0 = std::time::Instant::now();
            let line = chrome_line(&state, width, IconSet::Unicode, Some((&mut m, t0)));
            assert_eq!(line_width(&line), width, "scrolling at {width}");
            let plain = chrome_line(&state, width, IconSet::Unicode, None);
            assert!(line_width(&plain) <= width, "normal at {width}");
        }
    }
}
