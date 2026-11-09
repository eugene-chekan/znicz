//! Stage: the full-screen now-playing view — cover, details, signal path.

use std::sync::Arc;

use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;
use ratatui_image::picker::Picker;
use ratatui_image::{Resize, StatefulImage};
use znicz_core::{OutputInfo, PlaybackStatus, PlayerState, QueueItem, RepeatMode, TrackInfo};

use crate::app::{repeat_label, App};
use crate::cover::{pick_stream_cover, CoverKey, CoverReady};
use crate::format;
use crate::theme;
use crate::tui_config::CoverProtocol;

/// Column where signal values start (`  file    …`).
const LABEL: usize = 10;

pub fn render(frame: &mut Frame, area: Rect, app: &mut App, state: &PlayerState) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let font = app
        .picker
        .get_or_insert_with(Picker::halfblocks)
        .font_size();
    let layout = crate::layout::stage_layout(area, font.width, font.height, app.tui.show_cover);
    if let Some(cover) = layout.cover {
        render_cover(frame, cover, app, state);
    }
    let width = usize::from(layout.details.width);
    let title = format::truncate(&title_text(state), width);
    let lines = detail_lines(state, title, width, usize::from(layout.details.height));
    frame.render_widget(Paragraph::new(lines), layout.details);
}

/// What the title line says: the track (or ICY) title, or `Nothing playing`.
pub(crate) fn title_text(state: &PlayerState) -> String {
    state
        .current_track
        .as_ref()
        .map(|t| t.title.clone())
        .unwrap_or_else(|| "Nothing playing".to_string())
}

/// Every detail line for `rows` rows, `title` already fitted to `width`.
///
/// When rows run short, drops the path explanation, then the signal panel,
/// then the volume line; title, artist, and seek always stay.
pub(crate) fn detail_lines(
    state: &PlayerState,
    title: String,
    width: usize,
    rows: usize,
) -> Vec<Line<'static>> {
    let Some(track) = state.current_track.as_ref() else {
        return vec![
            Line::from(Span::styled(title, theme::strong())),
            Line::from(""),
            Line::from(Span::styled(
                "Enter a track in Browse (v) to start",
                theme::dim(),
            )),
        ];
    };

    let mut head = vec![Line::from(Span::styled(title, theme::strong()))];
    if track.url.is_some() {
        head.extend(stream_lines(state, track, width));
    } else {
        head.extend(file_lines(track, width));
    }
    head.push(Line::from(""));
    head.push(seek_line(state, width));

    let volume = volume_line(state);
    let (mut signal, mut explanation) = signal_lines(state);

    let needed = |signal: &[Line], explanation: &Option<Line>, volume: bool| {
        head.len()
            + usize::from(volume)
            + if signal.is_empty() {
                0
            } else {
                1 + signal.len() + usize::from(explanation.is_some())
            }
    };
    let mut with_volume = true;
    if needed(&signal, &explanation, with_volume) > rows {
        explanation = None;
    }
    if needed(&signal, &explanation, with_volume) > rows {
        signal.clear();
    }
    if needed(&signal, &explanation, with_volume) > rows {
        with_volume = false;
    }

    let mut out = head;
    if with_volume {
        out.push(volume);
    }
    if !signal.is_empty() {
        out.push(Line::from(""));
        out.extend(signal);
        out.extend(explanation);
    }
    out
}

fn file_lines(track: &TrackInfo, width: usize) -> Vec<Line<'static>> {
    let mut out = Vec::new();
    let artist = track
        .tags
        .artist
        .clone()
        .or_else(|| track.tags.album_artist.clone());
    if let Some(artist) = artist.filter(|a| !a.is_empty()) {
        out.push(Line::from(Span::styled(
            format::truncate(&artist, width),
            theme::subtitle(),
        )));
    }
    let mut parts = Vec::new();
    if let Some(album) = track.tags.album.clone().filter(|a| !a.is_empty()) {
        parts.push(album);
    }
    if let Some(year) = track.tags.year {
        parts.push(year.to_string());
    }
    if let Some(n) = track.tags.track_number {
        parts.push(format!("track {n}"));
    }
    if !parts.is_empty() {
        out.push(Line::from(Span::styled(
            format::truncate(&parts.join(" · "), width),
            theme::dim(),
        )));
    }
    out
}

/// Station name (from the queue row) and the stream URL, dim.
fn stream_lines(state: &PlayerState, track: &TrackInfo, width: usize) -> Vec<Line<'static>> {
    let mut out = Vec::new();
    let station = state
        .queue
        .get(state.queue_position)
        .and_then(|item| match item {
            QueueItem::Stream { name, .. } => Some(name.clone()),
            QueueItem::File { .. } => None,
        });
    if let Some(name) = station.filter(|n| *n != track.title) {
        out.push(Line::from(Span::styled(
            format::truncate(&name, width),
            theme::subtitle(),
        )));
    }
    if let Some(url) = track.url.as_deref() {
        out.push(Line::from(Span::styled(
            format::truncate(url, width),
            theme::dim(),
        )));
    }
    out
}

/// `▶ ━━━━●──── 3:32 / 11:04`, or `● live  3:32` for a stream.
fn seek_line(state: &PlayerState, width: usize) -> Line<'static> {
    let marker = crate::views::now_playing::status_marker(state.status);
    let marker_style = match state.status {
        PlaybackStatus::Playing => theme::good(),
        PlaybackStatus::Paused => theme::warn(),
        PlaybackStatus::Stopped => theme::dim(),
    };
    let is_stream = state
        .current_track
        .as_ref()
        .is_some_and(|t| t.url.is_some());
    if is_stream {
        return Line::from(vec![
            Span::styled(marker, marker_style),
            Span::styled("● live", theme::bad()),
            Span::styled(
                format!("  {}", format::duration(state.position)),
                theme::text(),
            ),
        ]);
    }
    let total = state.current_track.as_ref().and_then(|t| t.duration);
    let times = format!(
        " {} / {}",
        format::duration(state.position),
        format::duration_opt(total)
    );
    let bar_w = width
        .saturating_sub(marker.chars().count() + times.chars().count())
        .min(60);
    let ratio = match total {
        Some(total) if !total.is_zero() => state.position.as_secs_f64() / total.as_secs_f64(),
        _ => 0.0,
    };
    let (done, todo) = format::progress_bar(ratio, bar_w);
    Line::from(vec![
        Span::styled(marker, marker_style),
        Span::styled(done, theme::progress()),
        Span::styled(todo, theme::progress_track()),
        Span::styled(times, theme::text()),
    ])
}

/// `▆▆▆▁ 70%   repeat all   shuffle`.
fn volume_line(state: &PlayerState) -> Line<'static> {
    let mut spans = if state.muted {
        vec![Span::styled("muted", theme::warn())]
    } else {
        vec![Span::styled(
            format!(
                "{} {:>3.0}%",
                format::volume_bar(state.volume, 4),
                state.volume * 100.0
            ),
            theme::text(),
        )]
    };
    if state.repeat != RepeatMode::Off {
        spans.push(Span::styled(
            format!("   {}", repeat_label(state.repeat)),
            theme::toggle_on(),
        ));
    }
    if state.shuffle {
        spans.push(Span::styled("   shuffle", theme::toggle_on()));
    }
    Line::from(spans)
}

/// The signal panel and, separately, the path explanation (dropped first).
pub(crate) fn signal_lines(state: &PlayerState) -> (Vec<Line<'static>>, Option<Line<'static>>) {
    let Some(track) = state.current_track.as_ref() else {
        return (Vec::new(), None);
    };
    let mut lines = vec![Line::from(Span::styled("Signal", theme::strong()))];
    let source_label = if track.url.is_some() {
        "stream"
    } else {
        "file"
    };
    lines.push(labelled(
        source_label,
        track.format_description(),
        theme::text(),
    ));
    let device = state
        .device_name
        .clone()
        .unwrap_or_else(|| "default device".to_string());
    let device = match state.output.as_ref() {
        Some(output) => format!("{device} · {}", device_stream(output)),
        None => format!("{device} · no stream open"),
    };
    lines.push(labelled("device", device, theme::text()));
    let Some(output) = state.output.as_ref() else {
        lines.push(labelled(
            "path",
            "nothing to compare until a stream opens".to_string(),
            theme::dim(),
        ));
        return (lines, None);
    };
    let explanation = if output.bit_perfect {
        lines.push(labelled("path", "● bit perfect".to_string(), theme::good()));
        "the device took the file's own rate and channels".to_string()
    } else {
        lines.push(labelled("path", "▲ resampled".to_string(), theme::warn()));
        format!(
            "file {} {} → device {} {}",
            format::khz(track.sample_rate),
            format::channel_name(track.channels),
            format::khz(output.sample_rate),
            format::channel_name(output.channels),
        )
    };
    let explanation = Line::from(vec![
        Span::raw(" ".repeat(LABEL)),
        Span::styled(explanation, theme::dim()),
    ]);
    (lines, Some(explanation))
}

fn labelled(label: &str, value: String, style: ratatui::style::Style) -> Line<'static> {
    Line::from(vec![
        Span::styled(
            format!("  {label:<width$}", width = LABEL - 2),
            theme::dim(),
        ),
        Span::styled(value, style),
    ])
}

fn device_stream(output: &OutputInfo) -> String {
    format!(
        "{} {} {}",
        format::khz(output.sample_rate),
        format::channel_name(output.channels),
        sample_format_label(&output.sample_format),
    )
}

fn sample_format_label(fmt: &str) -> &str {
    if fmt.is_empty() || fmt == "?" {
        "--"
    } else {
        fmt
    }
}

fn cover_draw_label(ready: &CoverReady) -> String {
    match ready {
        CoverReady::Pending => "pending".into(),
        CoverReady::Logo => "logo".into(),
        CoverReady::Embedded(img) => format!("{:p}", Arc::as_ptr(img)),
    }
}

pub(crate) fn render_cover(frame: &mut Frame, area: Rect, app: &mut App, state: &PlayerState) {
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
                let aa_ready = match track
                    .url
                    .as_deref()
                    .and_then(znicz_core::parse_audioaddict_channel)
                {
                    Some((network, channel)) => {
                        app.covers.get(CoverKey::AudioAddict { network, channel })
                    }
                    None => CoverReady::Logo,
                };
                let station_ready = app
                    .stations
                    .iter()
                    .find(|s| Some(s.url.as_str()) == track.url.as_deref())
                    .and_then(|s| s.art.clone())
                    .map(|p| app.covers.get(CoverKey::ImageFile(p)))
                    .unwrap_or(CoverReady::Logo);
                pick_stream_cover(icy_ready, aa_ready, station_ready)
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::time::Duration;
    use znicz_core::TrackTags;

    fn text(lines: &[Line]) -> String {
        lines
            .iter()
            .map(|l| {
                l.spans
                    .iter()
                    .map(|s| s.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn playing(bit_perfect: bool) -> PlayerState {
        PlayerState {
            status: PlaybackStatus::Playing,
            current_track: Some(TrackInfo {
                path: Some(PathBuf::from("/music/sour-times.flac")),
                url: None,
                icy_stream_url: None,
                title: "Sour Times".into(),
                codec: "FLAC".into(),
                sample_rate: 96_000,
                channels: 2,
                bits_per_sample: Some(24),
                bitrate_kbps: Some(2882),
                duration: Some(Duration::from_secs(251)),
                tags: TrackTags {
                    title: Some("Sour Times".into()),
                    artist: Some("Portishead".into()),
                    album: Some("Dummy".into()),
                    year: Some(1994),
                    track_number: Some(3),
                    ..TrackTags::default()
                },
            }),
            device_name: Some("Topping E30 II".into()),
            output: Some(OutputInfo {
                sample_rate: if bit_perfect { 96_000 } else { 48_000 },
                channels: 2,
                sample_format: "f32".into(),
                bit_perfect,
            }),
            ..PlayerState::default()
        }
    }

    fn stream() -> PlayerState {
        PlayerState {
            status: PlaybackStatus::Playing,
            current_track: Some(TrackInfo {
                path: None,
                url: Some("https://example.com/stream".into()),
                icy_stream_url: None,
                title: "Artist - Song".into(),
                codec: "MP3".into(),
                sample_rate: 44_100,
                channels: 2,
                bits_per_sample: None,
                bitrate_kbps: Some(128),
                duration: None,
                tags: TrackTags::default(),
            }),
            queue: vec![QueueItem::Stream {
                name: "Jazz Radio".into(),
                url: "https://example.com/stream".into(),
            }],
            ..PlayerState::default()
        }
    }

    #[test]
    fn a_file_shows_artist_album_seek_and_signal() {
        let t = text(&detail_lines(&playing(true), "Sour Times".into(), 70, 30));
        for want in [
            "Sour Times",
            "Portishead",
            "Dummy · 1994 · track 3",
            "Signal",
            "FLAC",
            "Topping E30 II",
            "f32",
            "bit perfect",
        ] {
            assert!(t.contains(want), "missing {want:?}:\n{t}");
        }
    }

    #[test]
    fn resampled_explains_both_rates_and_a_missing_format_is_a_dash() {
        let mut state = playing(false);
        state.output.as_mut().unwrap().sample_format = "?".into();
        let t = text(&detail_lines(&state, "x".into(), 70, 30));
        assert!(
            t.contains("resampled") && t.contains("48 kHz") && t.contains("96 kHz"),
            "{t}"
        );
        assert!(t.contains("--") && !t.contains('?'), "{t}");
    }

    #[test]
    fn a_stream_shows_station_url_and_live() {
        let t = text(&detail_lines(&stream(), "Artist - Song".into(), 70, 30));
        assert!(t.contains("Jazz Radio"), "{t}");
        assert!(t.contains("https://example.com/stream"), "{t}");
        assert!(t.contains("● live"), "{t}");
        assert!(t.contains("stream "), "signal says stream:\n{t}");
    }

    #[test]
    fn nothing_playing_says_so_without_a_signal_panel() {
        let t = text(&detail_lines(
            &PlayerState::default(),
            "Nothing playing".into(),
            70,
            30,
        ));
        assert!(
            t.contains("Nothing playing") && t.contains("Browse (v)"),
            "{t}"
        );
        assert!(!t.contains("Signal"), "{t}");
    }

    #[test]
    fn short_rows_drop_explanation_then_signal_then_volume() {
        let state = playing(true);
        let all = detail_lines(&state, "x".into(), 70, 30).len();
        let t = text(&detail_lines(&state, "x".into(), 70, all - 1));
        assert!(!t.contains("own rate") && t.contains("Signal"), "{t}");
        let t = text(&detail_lines(&state, "x".into(), 70, 7));
        assert!(!t.contains("Signal") && t.contains('%'), "{t}");
        let t = text(&detail_lines(&state, "x".into(), 70, 4));
        assert!(!t.contains('%'), "{t}");
        assert!(
            t.contains("x") && t.contains("Portishead") && t.contains("/ "),
            "title, artist, seek stay:\n{t}"
        );
    }
}
