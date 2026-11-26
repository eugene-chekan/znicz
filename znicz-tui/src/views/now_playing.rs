//! The transport: what is playing, how far in, and how it reaches the speakers.
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use znicz_core::{PlaybackStatus, RepeatMode};

use crate::app::{App, repeat_label};
use crate::format;
use crate::stage_model::{SeekModel, TransportModel};
use crate::theme;
use crate::tui_config::IconSet;
use crate::views::stage::tone_style;

pub fn render_transport(
    frame: &mut Frame,
    area: Rect,
    app: &mut App,
    model: &TransportModel,
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
                model,
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
            signal_line(model, width)
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
    model: &TransportModel,
    width: usize,
    icons: IconSet,
    marquee: Option<(&mut crate::marquee::Marquee, std::time::Instant)>,
) -> Line<'static> {
    if width == 0 {
        return Line::from("");
    }

    let status_style = status_style(model.status);
    let sym = status_marker(model.status);
    let title = model.title.as_str();

    let times = model.seek.as_ref().map_or_else(String::new, |seek| {
        format!(
            " {} / {}",
            format::duration(seek.position()),
            format::duration_opt(seek.total())
        )
    });

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

    let subtitle = model.subtitle.as_ref().map(|s| format!(" {s}"));

    let volume = model.volume.map(|v| {
        let icon = format::volume_icon(icons, v.volume, v.muted);
        let bar = format::volume_bar(v.volume, 6);
        if v.muted {
            format!(" {icon} {bar} muted")
        } else {
            format!(" {icon} {bar} {:>3.0}%", v.volume * 100.0)
        }
    });

    let mut toggle_text = String::new();
    if model.repeat != RepeatMode::Off {
        toggle_text.push_str(repeat_label(model.repeat));
    }
    if model.shuffle {
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
            model,
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
        model,
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
    model: &TransportModel,
    seek_len: usize,
    times: String,
    include_volume: bool,
    volume: &Option<String>,
    include_toggles: bool,
    toggles: &Option<String>,
) {
    if seek_len > 0 {
        spans.push(Span::raw(" "));
        let (done, todo) = seek_bar_parts(model, seek_len);
        spans.push(Span::styled(done, theme::progress()));
        spans.push(Span::styled(todo, theme::progress_track()));
    }

    spans.push(Span::styled(times, theme::text()));

    if include_volume {
        if let Some(vol) = &volume {
            spans.push(Span::styled(
                vol.clone(),
                if model.volume.is_some_and(|v| v.muted) {
                    theme::warn()
                } else {
                    theme::text()
                },
            ));
        }
    }

    if include_toggles {
        if let Some(tog) = &toggles {
            let repeat_style = if model.repeat == RepeatMode::Off {
                theme::toggle_off()
            } else {
                theme::toggle_on()
            };
            let shuffle_style = if model.shuffle {
                theme::toggle_on()
            } else {
                theme::toggle_off()
            };
            if model.shuffle && model.repeat != RepeatMode::Off {
                spans.push(Span::styled(
                    format!(" {}", repeat_label(model.repeat)),
                    repeat_style,
                ));
                spans.push(Span::raw("  "));
                spans.push(Span::styled("shuffle".to_string(), shuffle_style));
            } else if model.shuffle {
                spans.push(Span::styled(tog.clone(), shuffle_style));
            } else {
                spans.push(Span::styled(tog.clone(), repeat_style));
            }
        }
    }
}

fn seek_bar_parts(model: &TransportModel, width: usize) -> (String, String) {
    let ratio = model.seek.as_ref().map_or(0.0, SeekModel::ratio);
    format::progress_bar(ratio, width)
}

/// File format on the left, the open device stream on the right, and the
/// path badge when it fits.
fn signal_line(model: &TransportModel, width: usize) -> Line<'static> {
    let Some((source, device)) = model.signal.as_ref() else {
        return Line::from(Span::styled("—", theme::dim()));
    };
    if device.is_empty() {
        return Line::from(Span::styled(source.clone(), theme::text()));
    }

    let mut spans = vec![
        Span::styled(source.clone(), theme::text()),
        Span::styled(" → ", theme::dim()),
        Span::styled(device.clone(), theme::text()),
    ];
    if let Some((badge, tone)) = model.path_badge.as_ref() {
        let used: usize = spans.iter().map(|s| s.content.chars().count()).sum();
        if used + badge.chars().count() + 2 <= width {
            spans.push(Span::raw("  "));
            spans.push(Span::styled(badge.clone(), tone_style(*tone)));
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

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::views::stage::tests::{at_fixed_time, playing, stream};
    use znicz_core::PlayerState;

    fn transport_of(state: &PlayerState) -> TransportModel {
        TransportModel::from_player(state)
    }

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
        let row = text(&chrome_line(
            &transport_of(&long_state()),
            100,
            IconSet::Unicode,
            None,
        ));
        assert!(row.contains('…'), "{row:?}");
    }

    #[test]
    fn with_a_marquee_a_long_title_scrolls() {
        let state = long_state();
        let mut m = crate::marquee::Marquee::default();
        let t0 = std::time::Instant::now();
        let start = text(&chrome_line(
            &transport_of(&state),
            100,
            IconSet::Unicode,
            Some((&mut m, t0)),
        ));
        assert!(start.contains("A Very"), "{start:?}");
        assert!(!start.contains('…'), "{start:?}");
        let later = text(&chrome_line(
            &transport_of(&state),
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
        let narrow = chrome_line(&transport_of(&state), 33, IconSet::Emoji, None);
        let t = text(&narrow);
        assert!(!t.contains('🔊'), "{t:?}");
        assert!(line_width(&narrow) <= 33, "{t:?}");

        let wide = text(&chrome_line(
            &transport_of(&state),
            100,
            IconSet::Emoji,
            None,
        ));
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
            let line = chrome_line(
                &transport_of(&state),
                width,
                IconSet::Unicode,
                Some((&mut m, t0)),
            );
            assert_eq!(line_width(&line), width, "scrolling at {width}");
            let plain = chrome_line(&transport_of(&state), width, IconSet::Unicode, None);
            assert!(line_width(&plain) <= width, "normal at {width}");
        }
    }

    /// Draws the transport into a `TestBackend` and returns each row's symbols.
    /// `toast` is a message to show on the message row.
    fn transport_rows(
        state: &PlayerState,
        width: u16,
        height: u16,
        show_signal: bool,
        toast: Option<&str>,
    ) -> Vec<String> {
        let mut app = crate::app::test_app();
        if let Some(text) = toast {
            app.toasts.info(text);
        }
        let model = transport_of(state);
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| render_transport(frame, frame.area(), &mut app, &model, show_signal))
            .unwrap();
        terminal
            .backend()
            .buffer()
            .content()
            .chunks(usize::from(width))
            .map(|row| row.iter().map(|cell| cell.symbol()).collect())
            .collect()
    }

    const GOLDEN_WIDE_TRANSPORT: &[&str] = &[
        "▶ Sour Times Portishead — Dummy ━━━━━━━━━━━━━─────────────────────────── 1:23 / 4:11 ◢ ████▎▁  70%  ",
        "FLAC 96 kHz 24-bit 2882 kbps stereo → 48 kHz stereo  ▲ resampled                                    ",
        "FLAC 96 kHz 24-bit 2882 kbps stereo → 48 kHz stereo  ▲ resampled                                    ",
        "                                                                                                    ",
    ];
    const GOLDEN_NARROW_TRANSPORT: &[&str] = &[
        "▶ Sour Times Portishe ━━━━──────── 1:23 / 4:11",
        "FLAC 96 kHz 24-bit 2882 kbps stereo → 48 kHz s",
        "                                              ",
    ];
    const GOLDEN_TRANSPORT_WITHOUT_SIGNAL: &[&str] = &[
        "▶ Sour Times Portishead — Dummy ━━━━━━━━━━━━━─────────────────────────── 1:23 / 4:11 ◢ ████▎▁  70%  ",
        "                                                                                                    ",
        "                                                                                                    ",
    ];
    const GOLDEN_STREAM_TRANSPORT: &[&str] = &[
        "▶ Artist - Song ──────────────────────────────────────── 1:23 / — ◢ ████▎▁  70%                     ",
        "MP3 44.1 kHz 128 kbps stereo                                                                        ",
        "                                                                                                    ",
    ];
    const GOLDEN_EMPTY_TRANSPORT: &[&str] = &[
        "■ Nothing playing ──────────────────────────────────────── 0:00 / — ◢ ██████ 100%                   ",
        "—                                                                                                   ",
        "                                                                                                    ",
    ];
    const GOLDEN_TRANSPORT_WITH_A_MESSAGE: &[&str] = &[
        "▶ Sour Times Portishead — Dummy ━━━━━━━━━━━━━─────────────────────────── 1:23 / 4:11 ◢ ████▎▁  70%  ",
        "FLAC 96 kHz 24-bit 2882 kbps stereo → 48 kHz stereo  ▲ resampled                                    ",
        "● hello                                                                                             ",
    ];

    #[test]
    fn golden_wide_transport() {
        let state = at_fixed_time(playing(false));
        assert_eq!(
            transport_rows(&state, 100, 4, true, None),
            GOLDEN_WIDE_TRANSPORT
        );
    }

    #[test]
    fn golden_narrow_transport() {
        let state = at_fixed_time(playing(false));
        assert_eq!(
            transport_rows(&state, 46, 3, true, None),
            GOLDEN_NARROW_TRANSPORT
        );
    }

    #[test]
    fn golden_transport_without_signal() {
        let state = at_fixed_time(playing(false));
        assert_eq!(
            transport_rows(&state, 100, 3, false, None),
            GOLDEN_TRANSPORT_WITHOUT_SIGNAL
        );
    }

    #[test]
    fn golden_stream_transport() {
        let state = at_fixed_time(stream());
        assert_eq!(
            transport_rows(&state, 100, 3, true, None),
            GOLDEN_STREAM_TRANSPORT
        );
    }

    #[test]
    fn golden_empty_transport() {
        assert_eq!(
            transport_rows(&PlayerState::default(), 100, 3, true, None),
            GOLDEN_EMPTY_TRANSPORT
        );
    }

    #[test]
    fn golden_transport_with_a_message() {
        let state = at_fixed_time(playing(false));
        assert_eq!(
            transport_rows(&state, 100, 3, true, Some("hello")),
            GOLDEN_TRANSPORT_WITH_A_MESSAGE
        );
    }
}
