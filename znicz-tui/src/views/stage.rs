//! Stage: the full-screen now-playing view — cover, details, signal path.

use std::sync::Arc;

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui_image::picker::Picker;
use ratatui_image::{Resize, StatefulImage};
use znicz_core::{PlaybackStatus, PlayerState, RepeatMode, TrackInfo};

use crate::app::{App, repeat_label};
use crate::cover::{CoverKey, CoverReady};
use crate::format;
use crate::stage_model::{SeekModel, StageModel, Tone, VolumeModel};
use crate::theme;
use crate::tui_config::{CoverProtocol, IconSet};

/// Column where signal values start (`  file    …`).
const LABEL: usize = 10;

pub fn render(frame: &mut Frame, area: Rect, app: &mut App, model: &StageModel) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let layout = stage_layout_for(area, app, app.shown_tui());
    if let Some(cover) = layout.cover {
        render_cover(frame, cover, app, model.cover.as_ref());
    }
    let width = usize::from(layout.details.width);
    let scroll = app
        .tui
        .marquee
        .then(|| (&mut app.stage_marquee, std::time::Instant::now()));
    let title = stage_title(&model.title, width, scroll);
    let lines = detail_lines(
        model,
        app.tui.icons,
        title,
        width,
        usize::from(layout.details.height),
    );
    frame.render_widget(Paragraph::new(lines), layout.details);
}

/// The Stage's model for the app's settings and the player's state.
pub(crate) fn model_for(app: &App, state: &PlayerState) -> StageModel {
    let shown = app.shown_tui();
    StageModel::from_player(
        state,
        &app.stations,
        shown.cover_search,
        shown.cover_protocol != CoverProtocol::Off,
    )
}

/// The title fitted to `width`: scrolled when a marquee is given and the
/// title overflows, else cut with an ellipsis.
pub(crate) fn stage_title(
    full: &str,
    width: usize,
    scroll: Option<(&mut crate::marquee::Marquee, std::time::Instant)>,
) -> String {
    match scroll {
        Some((marquee, now)) if full.chars().count() > width => {
            let offset = marquee.offset(full, width, now);
            crate::marquee::window(full, offset, width)
        }
        _ => format::truncate(full, width),
    }
}

/// The style for a model tone.
pub(crate) fn tone_style(tone: Tone) -> Style {
    match tone {
        Tone::Plain => theme::text(),
        Tone::Strong => theme::strong(),
        Tone::Dim => theme::dim(),
        Tone::Subtitle => theme::subtitle(),
        Tone::Good => theme::good(),
        Tone::Warn => theme::warn(),
    }
}

/// Every detail line for `rows` rows, `title` already fitted to `width`.
///
/// When rows run short, drops the path explanation, then the signal panel,
/// then the volume line; title, artist, and seek always stay.
pub(crate) fn detail_lines(
    model: &StageModel,
    icons: IconSet,
    title: String,
    width: usize,
    rows: usize,
) -> Vec<Line<'static>> {
    let Some(empty) = model.empty.as_ref() else {
        return playing_lines(model, icons, title, width, rows);
    };
    vec![
        Line::from(Span::styled(title, theme::strong())),
        Line::from(""),
        Line::from(Span::styled(empty.hint.clone(), theme::dim())),
    ]
}

fn playing_lines(
    model: &StageModel,
    icons: IconSet,
    title: String,
    width: usize,
    rows: usize,
) -> Vec<Line<'static>> {
    let mut head = vec![Line::from(Span::styled(title, theme::strong()))];
    head.extend(model.head.iter().map(|line| {
        Line::from(Span::styled(
            format::truncate(&line.text, width),
            tone_style(line.tone),
        ))
    }));
    head.push(Line::from(""));
    if let Some(seek) = model.seek.as_ref() {
        head.push(seek_line(seek, model.status, width));
    }

    let mut volume = model
        .volume
        .as_ref()
        .map(|v| volume_line(v, model.repeat, model.shuffle, icons));
    let (mut signal, mut explanation) = signal_lines(model, width);

    let needed = |signal: &[Line], explanation: &Option<Line>, volume: bool| {
        head.len()
            + usize::from(volume)
            + if signal.is_empty() {
                0
            } else {
                1 + signal.len() + usize::from(explanation.is_some())
            }
    };
    if needed(&signal, &explanation, volume.is_some()) > rows {
        explanation = None;
    }
    if needed(&signal, &explanation, volume.is_some()) > rows {
        signal.clear();
    }
    if needed(&signal, &explanation, volume.is_some()) > rows {
        volume = None;
    }

    let mut out = head;
    out.extend(volume);
    if !signal.is_empty() {
        out.push(Line::from(""));
        out.extend(signal);
        out.extend(explanation);
    }
    out
}

/// `▶ ━━━━●──── 3:32 / 11:04`, or `● live  3:32` for a stream.
fn seek_line(seek: &SeekModel, status: PlaybackStatus, width: usize) -> Line<'static> {
    let marker = crate::views::now_playing::status_marker(status);
    let marker_style = match status {
        PlaybackStatus::Playing => theme::good(),
        PlaybackStatus::Paused => theme::warn(),
        PlaybackStatus::Stopped => theme::dim(),
    };
    if let SeekModel::Live { position } = seek {
        return Line::from(vec![
            Span::styled(marker, marker_style),
            Span::styled("● live", theme::bad()),
            Span::styled(format!("  {}", format::duration(*position)), theme::text()),
        ]);
    }
    let times = format!(
        " {} / {}",
        format::duration(seek.position()),
        format::duration_opt(seek.total())
    );
    let bar_w = width
        .saturating_sub(marker.chars().count() + times.chars().count())
        .min(60);
    let (done, todo) = format::progress_bar(seek.ratio(), bar_w);
    Line::from(vec![
        Span::styled(marker, marker_style),
        Span::styled(done, theme::progress()),
        Span::styled(todo, theme::progress_track()),
        Span::styled(times, theme::text()),
    ])
}

/// `◢ ███████▁▁▁  70%   repeat all   shuffle` (a 10-cell volume bar). Muted:
/// `× ███████▁▁▁ muted`, with the bar dimmed.
fn volume_line(
    volume: &VolumeModel,
    repeat: RepeatMode,
    shuffle: bool,
    icons: IconSet,
) -> Line<'static> {
    let icon = format::volume_icon(icons, volume.volume, volume.muted);
    let bar = format::volume_bar(volume.volume, 10);
    let mut spans = if volume.muted {
        vec![
            Span::styled(format!("{icon} "), theme::warn()),
            Span::styled(bar, theme::dim()),
            Span::styled(" muted", theme::warn()),
        ]
    } else {
        vec![Span::styled(
            format!("{icon} {bar} {:>3.0}%", volume.volume * 100.0),
            theme::text(),
        )]
    };
    if repeat != RepeatMode::Off {
        spans.push(Span::styled(
            format!("   {}", repeat_label(repeat)),
            theme::toggle_on(),
        ));
    }
    if shuffle {
        spans.push(Span::styled("   shuffle", theme::toggle_on()));
    }
    Line::from(spans)
}

/// The signal panel and, separately, the path explanation (dropped first).
pub(crate) fn signal_lines(
    model: &StageModel,
    width: usize,
) -> (Vec<Line<'static>>, Option<Line<'static>>) {
    if model.signal.is_empty() {
        return (Vec::new(), None);
    }
    let mut lines = vec![Line::from(Span::styled("Signal", theme::strong()))];
    lines.extend(
        model
            .signal
            .iter()
            .map(|row| labelled(width, row.label, row.value.clone(), tone_style(row.tone))),
    );
    let explanation = model.explanation.as_ref().map(|text| {
        Line::from(vec![
            Span::raw(" ".repeat(LABEL)),
            Span::styled(
                format::truncate(text, width.saturating_sub(LABEL)),
                theme::dim(),
            ),
        ])
    });
    (lines, explanation)
}

fn labelled(width: usize, label: &str, value: String, style: Style) -> Line<'static> {
    Line::from(vec![
        Span::styled(
            format!("  {label:<width$}", width = LABEL - 2),
            theme::dim(),
        ),
        Span::styled(format::truncate(&value, width.saturating_sub(LABEL)), style),
    ])
}

fn cover_draw_label(ready: &CoverReady) -> String {
    match ready {
        CoverReady::Pending => "pending".into(),
        CoverReady::Logo => "logo".into(),
        CoverReady::Embedded(img) => format!("{:p}", Arc::as_ptr(img)),
    }
}

/// The cover question for the playing stream.
pub(crate) fn stream_query(
    state: &PlayerState,
    track: &TrackInfo,
    stations: &[znicz_core::Station],
    search: bool,
) -> znicz_core::StreamQuery {
    let stream_url = track.url.clone().unwrap_or_default();
    let station_name = match state.queue.get(state.queue_position) {
        Some(znicz_core::QueueItem::Stream { name, .. }) => name.clone(),
        _ => String::new(),
    };
    let station = stations.iter().find(|s| s.url == stream_url);
    let icy_title = Some(track.title.clone())
        .filter(|title| !title.trim().is_empty() && *title != station_name);
    znicz_core::StreamQuery {
        icy_title,
        icy_url: track.icy_stream_url.clone(),
        station_art: station.and_then(|s| s.art.clone()),
        search: search && station.is_none_or(|s| s.cover_search),
        stream_url,
        station_name,
    }
}

/// Show the last stream cover while the same stream's next cover is pending.
pub(crate) fn keep_previous(
    last: &mut Option<(String, CoverReady)>,
    stream_url: &str,
    ready: CoverReady,
) -> CoverReady {
    match &ready {
        CoverReady::Embedded(_) => {
            *last = Some((stream_url.to_string(), ready.clone()));
            ready
        }
        CoverReady::Pending => match last {
            Some((url, previous)) if url == stream_url => previous.clone(),
            _ => ready,
        },
        CoverReady::Logo => {
            *last = None;
            ready
        }
    }
}

/// The Stage layout for the given cover settings.
pub(crate) fn stage_layout_for(
    area: Rect,
    app: &mut App,
    tui: crate::TuiConfig,
) -> crate::layout::StageLayout {
    let font = app
        .picker
        .get_or_insert_with(Picker::halfblocks)
        .font_size();
    crate::layout::stage_layout(
        area,
        font.width,
        font.height,
        tui.show_cover,
        tui.cover_size,
    )
}

pub(crate) fn render_cover(frame: &mut Frame, area: Rect, app: &mut App, cover: Option<&CoverKey>) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let ready = match cover {
        None => {
            app.last_stream_cover = None;
            CoverReady::Logo
        }
        Some(CoverKey::File(path)) => {
            app.last_stream_cover = None;
            app.covers.get(CoverKey::File(path.clone()))
        }
        Some(CoverKey::Url(url)) => {
            app.last_stream_cover = None;
            app.covers.get(CoverKey::Url(url.clone()))
        }
        Some(CoverKey::Stream(query)) => {
            let stream_url = query.stream_url.clone();
            let ready = app.covers.get(CoverKey::Stream(query.clone()));
            keep_previous(&mut app.last_stream_cover, &stream_url, ready)
        }
    };

    let key = cover_draw_label(&ready);
    let draw_key = (key, area.width, area.height);
    // Under an overlay, keep the cover already drawn (see `App::stage_frozen`).
    let frozen = app.stage_frozen.is_some() && app.cover_image.is_some();
    if !frozen && app.cover_draw_key.as_ref() != Some(&draw_key) {
        let image = match ready {
            CoverReady::Embedded(img) => img.as_ref().clone(),
            CoverReady::Pending | CoverReady::Logo => app.covers.logo_image().clone(),
        };
        let picker = app.picker.get_or_insert_with(Picker::halfblocks);
        let font = picker.font_size();
        let background = crate::cover::slot_background(picker.capabilities());
        let image = crate::cover::fill_cover_slot(
            &image,
            area.width,
            area.height,
            font.width,
            font.height,
            background,
            crate::cover::slot_is_see_through(picker.protocol_type()),
        );
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
pub(crate) mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::time::Duration;
    use znicz_core::{OutputInfo, QueueItem, TrackTags};

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

    /// The Stage model of a player state, with covers on.
    fn model(state: &PlayerState) -> StageModel {
        StageModel::from_player(state, &[], true, true)
    }

    /// The Stage's detail lines for a player state.
    fn detail_of(
        state: &PlayerState,
        icons: IconSet,
        title: String,
        width: usize,
        rows: usize,
    ) -> Vec<Line<'static>> {
        detail_lines(&model(state), icons, title, width, rows)
    }

    pub(crate) fn playing(bit_perfect: bool) -> PlayerState {
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

    pub(crate) fn stream() -> PlayerState {
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
            }]
            .into(),
            ..PlayerState::default()
        }
    }

    #[test]
    fn a_long_stage_title_scrolls_with_the_marquee() {
        let full = "A Very Long Title That Will Never Fit In This Row";
        let mut m = crate::marquee::Marquee::default();
        let t0 = std::time::Instant::now();
        assert_eq!(
            stage_title(full, 20, Some((&mut m, t0))),
            "A Very Long Title Th"
        );
        let later = t0 + Duration::from_millis(2_000 + 5 * 200);
        assert_eq!(
            stage_title(full, 20, Some((&mut m, later))),
            "y Long Title That Wi"
        );
        assert!(stage_title(full, 20, None).ends_with('…'));
    }

    #[test]
    fn narrow_signal_values_are_cut_to_the_width() {
        let width = 24;
        let (lines, explanation) = signal_lines(&model(&playing(false)), width);
        for line in lines.iter().chain(explanation.iter()) {
            let w: usize = line.spans.iter().map(|s| s.content.chars().count()).sum();
            assert!(
                w <= width,
                "{w} > {width}: {}",
                text(std::slice::from_ref(line))
            );
        }
        let t = text(&lines);
        assert!(t.contains('…'), "{t}");
        assert!(text(&[explanation.unwrap()]).ends_with('…'));
    }

    #[test]
    fn a_file_shows_artist_album_seek_and_signal() {
        let t = text(&detail_of(
            &playing(true),
            IconSet::Unicode,
            "Sour Times".into(),
            70,
            30,
        ));
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
        let t = text(&detail_of(&state, IconSet::Unicode, "x".into(), 70, 30));
        assert!(
            t.contains("resampled") && t.contains("48 kHz") && t.contains("96 kHz"),
            "{t}"
        );
        assert!(t.contains("--") && !t.contains('?'), "{t}");
    }

    #[test]
    fn a_stream_shows_station_url_and_live() {
        let t = text(&detail_of(
            &stream(),
            IconSet::Unicode,
            "Artist - Song".into(),
            70,
            30,
        ));
        assert!(t.contains("Jazz Radio"), "{t}");
        assert!(t.contains("https://example.com/stream"), "{t}");
        assert!(t.contains("● live"), "{t}");
        assert!(t.contains("stream "), "signal says stream:\n{t}");
    }

    fn stream_titled(title: &str) -> PlayerState {
        let mut state = stream();
        state.current_track.as_mut().unwrap().title = title.into();
        state
    }

    fn station(cover_search: bool) -> znicz_core::Station {
        znicz_core::Station {
            name: "Jazz Radio".into(),
            url: "https://example.com/stream".into(),
            art: Some("/art/jazz.png".into()),
            cover_search,
        }
    }

    #[test]
    fn a_stream_query_carries_the_icy_title_station_and_switches() {
        let state = stream_titled("Serotonal - Memory Leaves");
        let track = state.current_track.as_ref().unwrap();
        let q = stream_query(&state, track, &[station(true)], true);
        assert_eq!(q.station_name, "Jazz Radio");
        assert_eq!(q.icy_title.as_deref(), Some("Serotonal - Memory Leaves"));
        assert_eq!(
            q.station_art.as_deref(),
            Some(std::path::Path::new("/art/jazz.png"))
        );
        assert!(q.search);

        assert!(!stream_query(&state, track, &[station(false)], true).search);
        assert!(!stream_query(&state, track, &[station(true)], false).search);
    }

    #[test]
    fn a_title_that_is_the_station_name_is_no_icy_title() {
        let state = stream_titled("Jazz Radio");
        let track = state.current_track.as_ref().unwrap();
        assert_eq!(stream_query(&state, track, &[], true).icy_title, None);
    }

    #[test]
    fn an_empty_title_is_no_icy_title() {
        let state = stream_titled("");
        let track = state.current_track.as_ref().unwrap();
        assert_eq!(stream_query(&state, track, &[], true).icy_title, None);
        let state = stream_titled("   ");
        let track = state.current_track.as_ref().unwrap();
        assert_eq!(stream_query(&state, track, &[], true).icy_title, None);
    }

    #[test]
    fn the_previous_stream_cover_shows_while_the_next_is_pending() {
        let img = CoverReady::Embedded(Arc::new(image::DynamicImage::new_rgb8(1, 1)));
        let mut last = None;
        let url = "http://radio/stream";

        assert_eq!(keep_previous(&mut last, url, img.clone()), img);
        assert_eq!(keep_previous(&mut last, url, CoverReady::Pending), img);
        assert_eq!(
            keep_previous(&mut last, "http://other/stream", CoverReady::Pending),
            CoverReady::Pending,
            "a new station starts from the logo"
        );
        assert_eq!(
            keep_previous(&mut last, url, CoverReady::Logo),
            CoverReady::Logo
        );
    }

    #[test]
    fn nothing_playing_says_so_without_a_signal_panel() {
        let t = text(&detail_of(
            &PlayerState::default(),
            IconSet::Unicode,
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
        let all = detail_of(&state, IconSet::Unicode, "x".into(), 70, 30).len();
        let t = text(&detail_of(
            &state,
            IconSet::Unicode,
            "x".into(),
            70,
            all - 1,
        ));
        assert!(!t.contains("own rate") && t.contains("Signal"), "{t}");
        let t = text(&detail_of(&state, IconSet::Unicode, "x".into(), 70, 7));
        assert!(!t.contains("Signal") && t.contains('%'), "{t}");
        let t = text(&detail_of(&state, IconSet::Unicode, "x".into(), 70, 4));
        assert!(!t.contains('%'), "{t}");
        assert!(
            t.contains("x") && t.contains("Portishead") && t.contains("/ "),
            "title, artist, seek stay:\n{t}"
        );
    }

    #[test]
    fn the_volume_line_leads_with_the_icon_then_the_bar() {
        let volume = VolumeModel {
            volume: 0.5,
            muted: false,
        };
        let t = text(&[volume_line(
            &volume,
            RepeatMode::Off,
            false,
            IconSet::Unicode,
        )]);
        assert!(t.starts_with("◢ █████▁▁▁▁▁ "), "{t:?}");
        assert!(t.trim_end().ends_with(" 50%"), "{t:?}");
    }

    #[test]
    fn a_muted_volume_line_keeps_the_bar_and_says_muted() {
        let volume = VolumeModel {
            volume: 0.5,
            muted: true,
        };
        let t = text(&[volume_line(
            &volume,
            RepeatMode::Off,
            false,
            IconSet::Unicode,
        )]);
        assert!(t.starts_with("× █████▁▁▁▁▁ muted"), "{t:?}");
        assert!(!t.contains('%'), "{t:?}");
    }

    /// The playback position and volume the goldens draw at, so the seek bar
    /// and the volume line are fixed.
    pub(crate) fn at_fixed_time(mut state: PlayerState) -> PlayerState {
        state.position = Duration::from_secs(83);
        state.volume = 0.7;
        state
    }

    /// Draws the Stage into a `TestBackend` and returns each row's symbols.
    fn stage_rows(state: &PlayerState, width: u16, height: u16, show_cover: bool) -> Vec<String> {
        let mut app = crate::app::test_app();
        app.tui.show_cover = show_cover;
        app.tui.cover_protocol = if show_cover {
            CoverProtocol::Auto
        } else {
            CoverProtocol::Off
        };
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height)).unwrap();
        let model = model_for(&app, state);
        terminal
            .draw(|frame| render(frame, frame.area(), &mut app, &model))
            .unwrap();
        terminal
            .backend()
            .buffer()
            .content()
            .chunks(usize::from(width))
            .map(|row| row.iter().map(|cell| cell.symbol()).collect())
            .collect()
    }

    const GOLDEN_WIDE_FILE: &[&str] = &[
        "                                                  Sour Times                                        ",
        "                      ▄▄▄                         Portishead                                        ",
        "                      ▄▄▄▄▄                       Dummy · 1994 · track 3                            ",
        "                      ▄▄ ▄▄▄▄                                                                       ",
        "                    ▄▄▄    ▄▄▄                    ▶ ━━━━━━━━━━━━──────────────────────── 1:23 / 4:11",
        "                   ▄▄▄      ▄▄▄                   ◢ ███████▁▁▁  70%                                 ",
        "                  ▄▄▄ ▀▀▀▀▀  ▄▄                                                                     ",
        "                 ▄▄▄  ▄  ▀▀▀▀ ▄▄                  Signal                                            ",
        "            ▄▄▄▄▄▄▀     ▄▄▄▄▄ ▀▄▄▄▄▄▄               file    FLAC 96 kHz 24-bit 2882 kbps stereo     ",
        "           ▄▄▀▀▀▀▀▀▀  ▄▄▄▄   ▀▀▀▀▀▀▀▄▄              device  Topping E30 II · 48 kHz stereo f32      ",
        "           ▀▀▀▄▄▄▄▄▄▀▀▀▀▀▀▀▀▀▄▄▄▄▄▄▀▀▀              path    ▲ resampled                             ",
        "           ▀▀▄▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▄▀▀                      file 96 kHz stereo → device 48 kHz ster…",
        "           ▀▀▄ ▄▄▄▄▄▄▄▄▄▀▄▄▄▄▄▄▄▄▄ ▄▀▀                                                              ",
        "            ▀▄▄▄▄▄     ▄▄▄     ▄▄▄▄▄▀                                                               ",
        "            ▀▄▀▀                 ▀▀▄▀                                                               ",
        "            ▀▀▄▀                 ▀▄▀▀                                                               ",
        "             ▀▄▀▀               ▀▀▄▀▀                                                               ",
        "             ▀▄▀▀               ▀▀▄▀                                                                ",
        "             ▀▀▄▀▀             ▀▀▄▀▀                                                                ",
        "             ▀▀▄▄▄▀▀▀▀▀▀▀▀▀▀▀▀▀▄▄▄▀▀                                                                ",
        "              ▀▄▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▄▀                                                                 ",
        "              ▀▀▀▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▀▀▀                                                                 ",
        "                ▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀                                                                   ",
        "                                                                                                    ",
    ];
    const GOLDEN_WIDE_BIT_PERFECT: &[&str] = &[
        "                                                  Sour Times                                        ",
        "                      ▄▄▄                         Portishead                                        ",
        "                      ▄▄▄▄▄                       Dummy · 1994 · track 3                            ",
        "                      ▄▄ ▄▄▄▄                                                                       ",
        "                    ▄▄▄    ▄▄▄                    ▶ ━━━━━━━━━━━━──────────────────────── 1:23 / 4:11",
        "                   ▄▄▄      ▄▄▄                   ◢ ███████▁▁▁  70%                                 ",
        "                  ▄▄▄ ▀▀▀▀▀  ▄▄                                                                     ",
        "                 ▄▄▄  ▄  ▀▀▀▀ ▄▄                  Signal                                            ",
        "            ▄▄▄▄▄▄▀     ▄▄▄▄▄ ▀▄▄▄▄▄▄               file    FLAC 96 kHz 24-bit 2882 kbps stereo     ",
        "           ▄▄▀▀▀▀▀▀▀  ▄▄▄▄   ▀▀▀▀▀▀▀▄▄              device  Topping E30 II · 96 kHz stereo f32      ",
        "           ▀▀▀▄▄▄▄▄▄▀▀▀▀▀▀▀▀▀▄▄▄▄▄▄▀▀▀              path    ● bit perfect                           ",
        "           ▀▀▄▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▄▀▀                      the device took the file's own rate and…",
        "           ▀▀▄ ▄▄▄▄▄▄▄▄▄▀▄▄▄▄▄▄▄▄▄ ▄▀▀                                                              ",
        "            ▀▄▄▄▄▄     ▄▄▄     ▄▄▄▄▄▀                                                               ",
        "            ▀▄▀▀                 ▀▀▄▀                                                               ",
        "            ▀▀▄▀                 ▀▄▀▀                                                               ",
        "             ▀▄▀▀               ▀▀▄▀▀                                                               ",
        "             ▀▄▀▀               ▀▀▄▀                                                                ",
        "             ▀▀▄▀▀             ▀▀▄▀▀                                                                ",
        "             ▀▀▄▄▄▀▀▀▀▀▀▀▀▀▀▀▀▀▄▄▄▀▀                                                                ",
        "              ▀▄▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▄▀                                                                 ",
        "              ▀▀▀▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▀▀▀                                                                 ",
        "                ▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀                                                                   ",
        "                                                                                                    ",
    ];
    const GOLDEN_NARROW_FILE: &[&str] = &[
        "           ▄▄                                   ",
        "           ▄▄▄▄                                 ",
        "         ▄▄▄▀▄▄▄                                ",
        "        ▄▄▄▀▀▀▀▄▄                               ",
        "     ▄▄▄▀▀▀▄▄▄▄▀▀▄▄▄                            ",
        "     ▀▀▀▀▀▀▄▄▄▀▀▀▀▀▀                            ",
        "      ▀▄▄▄▄▄▄▄▄▄▄▄▀▀                            ",
        "      ▀▀▀   ▄   ▀▀▀                             ",
        "      ▀▀▀       ▀▀▀                             ",
        "      ▀▄▀▀▀▀▀▀▀▀▀▄▀                             ",
        "       ▀▄▄▀▀▀▀▀▄▄▀                              ",
        "        ▀▀▀▀▀▀▀▀▀                               ",
        "                                                ",
        " Sour Times                                     ",
        " Portishead                                     ",
        " Dummy · 1994 · track 3                         ",
        "                                                ",
        " ▶ ━━━━━━━━━━╾───────────────────── 1:23 / 4:11 ",
        " ◢ ███████▁▁▁  70%                              ",
        "                                                ",
        " Signal                                         ",
        "   file    FLAC 96 kHz 24-bit 2882 kbps stereo  ",
        "   device  Topping E30 II · 48 kHz stereo f32   ",
        "   path    ▲ resampled                          ",
    ];
    const GOLDEN_NO_COVER: &[&str] = &[
        "                                                                                                    ",
        "          Sour Times                                                                                ",
        "          Portishead                                                                                ",
        "          Dummy · 1994 · track 3                                                                    ",
        "                                                                                                    ",
        "          ▶ ━━━━━━━━━━━━━━━━━━━━──────────────────────────────────────── 1:23 / 4:11                ",
        "          ◢ ███████▁▁▁  70%                                                                         ",
        "                                                                                                    ",
        "          Signal                                                                                    ",
        "            file    FLAC 96 kHz 24-bit 2882 kbps stereo                                             ",
        "            device  Topping E30 II · 48 kHz stereo f32                                              ",
        "            path    ▲ resampled                                                                     ",
        "                    file 96 kHz stereo → device 48 kHz stereo                                       ",
        "                                                                                                    ",
        "                                                                                                    ",
        "                                                                                                    ",
        "                                                                                                    ",
        "                                                                                                    ",
        "                                                                                                    ",
        "                                                                                                    ",
        "                                                                                                    ",
        "                                                                                                    ",
        "                                                                                                    ",
        "                                                                                                    ",
    ];
    const GOLDEN_STREAM: &[&str] = &[
        "                                                                                                    ",
        "          Artist - Song                                                                             ",
        "          Jazz Radio                                                                                ",
        "          https://example.com/stream                                                                ",
        "                                                                                                    ",
        "          ▶ ● live  1:23                                                                            ",
        "          ◢ ███████▁▁▁  70%                                                                         ",
        "                                                                                                    ",
        "          Signal                                                                                    ",
        "            stream  MP3 44.1 kHz 128 kbps stereo                                                    ",
        "            device  default device · no stream open                                                 ",
        "            path    nothing to compare until a stream opens                                         ",
        "                                                                                                    ",
        "                                                                                                    ",
        "                                                                                                    ",
        "                                                                                                    ",
        "                                                                                                    ",
        "                                                                                                    ",
        "                                                                                                    ",
        "                                                                                                    ",
        "                                                                                                    ",
        "                                                                                                    ",
        "                                                                                                    ",
        "                                                                                                    ",
    ];
    const GOLDEN_NOTHING_PLAYING: &[&str] = &[
        "                                                                                                    ",
        "          Nothing playing                                                                           ",
        "                                                                                                    ",
        "          Enter a track in Browse (v) to start                                                      ",
        "                                                                                                    ",
        "                                                                                                    ",
        "                                                                                                    ",
        "                                                                                                    ",
        "                                                                                                    ",
        "                                                                                                    ",
        "                                                                                                    ",
        "                                                                                                    ",
        "                                                                                                    ",
        "                                                                                                    ",
        "                                                                                                    ",
        "                                                                                                    ",
        "                                                                                                    ",
        "                                                                                                    ",
        "                                                                                                    ",
        "                                                                                                    ",
        "                                                                                                    ",
        "                                                                                                    ",
        "                                                                                                    ",
        "                                                                                                    ",
    ];

    #[test]
    fn golden_wide_file_stage() {
        let state = at_fixed_time(playing(false));
        assert_eq!(stage_rows(&state, 100, 24, true), GOLDEN_WIDE_FILE);
    }

    #[test]
    fn golden_wide_bit_perfect_stage() {
        let state = at_fixed_time(playing(true));
        assert_eq!(stage_rows(&state, 100, 24, true), GOLDEN_WIDE_BIT_PERFECT);
    }

    #[test]
    fn golden_narrow_file_stage() {
        let state = at_fixed_time(playing(false));
        assert_eq!(stage_rows(&state, 48, 24, true), GOLDEN_NARROW_FILE);
    }

    #[test]
    fn golden_no_cover_stage() {
        let state = at_fixed_time(playing(false));
        assert_eq!(stage_rows(&state, 100, 24, false), GOLDEN_NO_COVER);
    }

    #[test]
    fn golden_stream_stage() {
        let state = at_fixed_time(stream());
        assert_eq!(stage_rows(&state, 100, 24, false), GOLDEN_STREAM);
    }

    #[test]
    fn golden_nothing_playing_stage() {
        let state = PlayerState::default();
        assert_eq!(stage_rows(&state, 100, 24, false), GOLDEN_NOTHING_PLAYING);
    }
}
