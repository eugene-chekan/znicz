//! Full signal-path overlay.
//!
//! The transport line is a one-line summary. This modal is where the rest
//! lives: device sample format, device name, and a plain-English path.

use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;
use znicz_core::{OutputInfo, PlayerState, TrackInfo};

use crate::format;
use crate::theme;

pub fn render(frame: &mut Frame, area: Rect, state: &PlayerState) {
    let content = lines(state);
    let height = (content.len() as u16)
        .saturating_add(2)
        .clamp(3, area.height.max(1));
    let width = ((area.width as u32 * 62 / 100).max(44) as u16)
        .min(area.width)
        .max(1);
    let popup = centered(area, width, height);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(theme::border_active())
        .title(Span::styled(" Signal ", theme::title()))
        .title_bottom(Line::from(Span::styled(" i / Esc close ", theme::dim())).right_aligned());

    frame.render_widget(Clear, popup);
    frame.render_widget(Paragraph::new(content).block(block), popup);
}

pub(crate) fn lines(state: &PlayerState) -> Vec<Line<'static>> {
    let Some(track) = state.current_track.as_ref() else {
        return vec![
            Line::from(Span::styled("No file is playing.", theme::text())),
            Line::from(""),
            Line::from(Span::styled(
                "Open a track, then press i to inspect the path.",
                theme::dim(),
            )),
        ];
    };

    let mut out = Vec::new();
    heading(&mut out, "File");
    indented(&mut out, track.format_description(), theme::text());
    if let Some(title) = track.tags.title.as_deref().filter(|s| !s.is_empty()) {
        indented(&mut out, title.to_string(), theme::title());
    } else {
        indented(&mut out, track.title.clone(), theme::title());
    }
    if let Some(summary) = track.artist_album() {
        indented(&mut out, summary, theme::subtitle());
    }

    out.push(Line::from(""));
    heading(&mut out, "Device");
    indented(
        &mut out,
        state
            .device_name
            .clone()
            .unwrap_or_else(|| "default device".to_string()),
        theme::text(),
    );
    match state.output.as_ref() {
        Some(output) => indented(&mut out, device_stream(output), theme::text()),
        None => indented(&mut out, "no stream open".to_string(), theme::dim()),
    }

    out.push(Line::from(""));
    heading(&mut out, "Path");
    match state.output.as_ref() {
        Some(output) => path_lines(&mut out, track, output),
        None => indented(
            &mut out,
            "nothing to compare until a stream opens".to_string(),
            theme::dim(),
        ),
    }

    out
}

fn heading(lines: &mut Vec<Line<'static>>, title: &str) {
    lines.push(Line::from(Span::styled(title.to_string(), theme::strong())));
}

fn indented(lines: &mut Vec<Line<'static>>, text: String, style: ratatui::style::Style) {
    lines.push(Line::from(vec![Span::raw("  "), Span::styled(text, style)]));
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

fn path_lines(lines: &mut Vec<Line<'static>>, track: &TrackInfo, output: &OutputInfo) {
    if output.bit_perfect {
        indented(lines, "● bit perfect".to_string(), theme::good());
        indented(
            lines,
            "the device took the file's own rate and channels".to_string(),
            theme::dim(),
        );
    } else {
        indented(lines, "▲ resampled".to_string(), theme::warn());
        indented(
            lines,
            format!(
                "file {} {} → device {} {}",
                format::khz(track.sample_rate),
                format::channel_name(track.channels),
                format::khz(output.sample_rate),
                format::channel_name(output.channels),
            ),
            theme::text(),
        );
    }
}

fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(area.width);
    let height = height.min(area.height);
    Rect {
        x: area.x + (area.width.saturating_sub(width)) / 2,
        y: area.y + (area.height.saturating_sub(height)) / 2,
        width,
        height,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::time::Duration;

    use znicz_core::{OutputInfo, PlaybackStatus, PlayerState, TrackInfo, TrackTags};

    fn dump(state: &PlayerState) -> String {
        lines(state)
            .into_iter()
            .map(|line| {
                line.spans
                    .into_iter()
                    .map(|span| span.content.to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn playing(bit_perfect: bool) -> PlayerState {
        PlayerState {
            status: PlaybackStatus::Playing,
            current_track: Some(TrackInfo {
                path: PathBuf::from("/music/sour-times.flac"),
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

    #[test]
    fn idle_state_explains_there_is_nothing_to_inspect() {
        let text = dump(&PlayerState::default());
        assert!(text.contains("No file is playing"), "{text}");
        assert!(!text.contains("f32"), "no stream means no sample format");
    }

    #[test]
    fn playing_state_shows_the_device_sample_format() {
        let text = dump(&playing(true));
        assert!(text.contains("f32"), "{text}");
        assert!(text.contains("FLAC"), "{text}");
        assert!(text.contains("96 kHz"), "{text}");
        assert!(text.contains("24-bit"), "{text}");
        assert!(text.contains("Topping E30 II"), "{text}");
        assert!(text.contains("bit perfect"), "{text}");
        assert!(text.contains("Sour Times"), "{text}");
    }

    #[test]
    fn resampled_path_shows_both_rates() {
        let text = dump(&playing(false));
        assert!(text.contains("resampled"), "{text}");
        assert!(text.contains("48 kHz"), "{text}");
        assert!(text.contains("96 kHz"), "{text}");
        assert!(text.contains("f32"), "device format still belongs here");
    }

    #[test]
    fn a_missing_sample_format_is_a_dash_not_a_question_mark() {
        let mut state = playing(true);
        if let Some(output) = state.output.as_mut() {
            output.sample_format = "?".into();
        }
        let text = dump(&state);
        assert!(text.contains("--"), "{text}");
        assert!(!text.contains('?'), "{text}");
    }

    #[test]
    fn a_tiny_popup_stays_inside_the_screen() {
        let popup = centered(Rect::new(0, 0, 10, 3), 44, 14);
        assert!(popup.width <= 10 && popup.height <= 3);
        assert_eq!(popup.x, 0);
        assert_eq!(popup.y, 0);
    }
}
