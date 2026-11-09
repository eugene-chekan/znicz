# Stage Mode Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A full-screen Stage view (`v` / `i`) with a large cover, track details, seek, volume, and the signal path; the inspector modal folds into it; long now-playing titles scroll (marquee); Browse's transport drops the cover.

**Architecture:** `App` gains `view: View { Browse, Stage }`. A new `views/stage.rs` draws Stage over the list region and the transport region (minus the message row), using a pure `layout::stage_layout` and pure line builders; the cover drawing moves there from `now_playing.rs`. A pure `marquee.rs` turns elapsed time into a scroll offset; a small `Marquee` holder resets on text or width change.

**Tech Stack:** Rust 2021, ratatui 0.30, ratatui-image 11, crossterm 0.28, cargo test.

**Spec:** [`docs/superpowers/specs/2026-11-09-stage-mode-design.md`](../specs/2026-11-09-stage-mode-design.md)

**Branch:** `feat/stage-mode` — every task starts with `git branch --show-current` and stops if it is anything else.

**Commands:** `cargo test -p znicz-tui [--lib|--test <name>] <filter>`; lint `cargo clippy --workspace --all-targets -- -D warnings`; format `cargo fmt --all -- --check`.

Every commit message ends with a blank line then `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.

---

### Task 1: Commit the plan

- [x] **Step 1:** `git add docs/superpowers/plans/2026-11-09-stage-mode.md && git commit -m "Add stage mode implementation plan"`

---

### Task 2: Marquee module and `marquee` config

**Files:**
- Create: `znicz-tui/src/marquee.rs`
- Modify: `znicz-tui/src/lib.rs`, `znicz-tui/src/tui_config.rs`, `znicz/src/main.rs`

- [ ] **Step 1: Write the module with its failing tests**

Create `znicz-tui/src/marquee.rs`:

```rust
//! Scrolling text that is wider than its slot: hold, scroll, hold, jump back.
//!
//! The offset is a pure function of elapsed time, so it does not drift with
//! how often frames are drawn, and tests can pick any moment.

use std::time::{Duration, Instant};

/// Pause at the start and at the end of each pass.
pub const HOLD: Duration = Duration::from_secs(2);
/// Time per column moved; matches the TUI's redraw tick.
pub const STEP: Duration = Duration::from_millis(200);

/// Characters to skip from the start of a `len`-character text shown in
/// `width` columns, `elapsed` after it first appeared.
pub fn marquee_offset(len: usize, width: usize, elapsed: Duration) -> usize {
    if width == 0 || len <= width {
        return 0;
    }
    let max = len - width;
    let travel = STEP * max as u32;
    let cycle = HOLD + travel + HOLD;
    let t = Duration::from_nanos((elapsed.as_nanos() % cycle.as_nanos()) as u64);
    if t < HOLD {
        0
    } else {
        let moved = ((t - HOLD).as_millis() / STEP.as_millis()) as usize;
        moved.min(max)
    }
}

/// `width` characters of `text` starting at `offset` (no ellipsis).
pub fn window(text: &str, offset: usize, width: usize) -> String {
    text.chars().skip(offset).take(width).collect()
}

/// When the current text first showed at its width.
#[derive(Debug, Default)]
pub struct Marquee {
    shown: Option<(String, usize)>,
    started: Option<Instant>,
}

impl Marquee {
    /// The scroll offset for `text` in `width` columns at `now`. A different
    /// text or width starts a new pass.
    pub fn offset(&mut self, text: &str, width: usize, now: Instant) -> usize {
        let same = self
            .shown
            .as_ref()
            .is_some_and(|(t, w)| t == text && *w == width);
        if !same {
            self.shown = Some((text.to_string(), width));
            self.started = Some(now);
        }
        let elapsed = now.saturating_duration_since(self.started.unwrap_or(now));
        marquee_offset(text.chars().count(), width, elapsed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    #[test]
    fn text_that_fits_never_moves() {
        assert_eq!(marquee_offset(10, 10, ms(9_000)), 0);
        assert_eq!(marquee_offset(3, 10, ms(9_000)), 0);
        assert_eq!(marquee_offset(30, 0, ms(9_000)), 0);
    }

    #[test]
    fn it_holds_then_moves_one_column_per_step() {
        // 15 characters in 10 columns: max offset 5.
        assert_eq!(marquee_offset(15, 10, ms(0)), 0);
        assert_eq!(marquee_offset(15, 10, ms(1_999)), 0);
        assert_eq!(marquee_offset(15, 10, ms(2_000)), 0);
        assert_eq!(marquee_offset(15, 10, ms(2_200)), 1);
        assert_eq!(marquee_offset(15, 10, ms(2_600)), 3);
    }

    #[test]
    fn it_holds_at_the_end_then_jumps_back() {
        // Travel 5 × 200 ms = 1 s; cycle = 2 s + 1 s + 2 s = 5 s.
        assert_eq!(marquee_offset(15, 10, ms(3_000)), 5);
        assert_eq!(marquee_offset(15, 10, ms(4_900)), 5);
        assert_eq!(marquee_offset(15, 10, ms(5_000)), 0);
        assert_eq!(marquee_offset(15, 10, ms(7_200)), 1);
    }

    #[test]
    fn window_takes_a_slice_without_ellipsis() {
        assert_eq!(window("abcdefgh", 2, 4), "cdef");
        assert_eq!(window("żółw", 1, 2), "ół");
    }

    #[test]
    fn a_new_text_or_width_starts_over() {
        let mut m = Marquee::default();
        let t0 = Instant::now();
        let long = "a".repeat(30);
        assert_eq!(m.offset(&long, 10, t0), 0);
        assert_eq!(m.offset(&long, 10, t0 + ms(2_600)), 3);
        assert_eq!(m.offset(&long, 12, t0 + ms(2_600)), 0, "width change");
        let other = "b".repeat(30);
        assert_eq!(m.offset(&other, 12, t0 + ms(9_000)), 0, "text change");
    }
}
```

Add `pub mod marquee;` to `znicz-tui/src/lib.rs`.

- [ ] **Step 2: Run to verify** — `cargo test -p znicz-tui --lib marquee` (write the tests first with `todo!()` bodies to see them fail), then PASS.

- [ ] **Step 3: Config**

`znicz-tui/src/tui_config.rs`: add to `TuiConfig`

```rust
    /// Scroll now-playing titles that do not fit (otherwise cut with `…`).
    pub marquee: bool,
```

and `marquee: true,` in `Default`. Add a test:

```rust
    #[test]
    fn marquee_is_on_by_default() {
        assert!(TuiConfig::default().marquee);
    }
```

`znicz/src/main.rs`: `TuiSection` gains `marquee: Option<bool>,`; `tui_config` sets `marquee: config.tui.marquee.unwrap_or(true),`.

- [ ] **Step 4:** `cargo test -p znicz-tui -p znicz`, clippy, fmt; commit `Add the marquee scroller and its setting`.

---

### Task 3: `layout::stage_layout`

**Files:**
- Modify: `znicz-tui/src/layout.rs`

- [ ] **Step 1: Write the failing tests** (in `layout.rs` tests)

```rust
    // Halfblocks report 10 × 20 px cells: a square cover is 2 columns per row.
    const FW: u16 = 10;
    const FH: u16 = 20;

    #[test]
    fn a_wide_stage_puts_the_cover_left_at_full_height() {
        let area = Rect::new(0, 0, 120, 40);
        let s = stage_layout(area, FW, FH, true);
        let cover = s.cover.expect("cover");
        assert_eq!((cover.x, cover.y, cover.height), (0, 0, 40));
        assert_eq!(cover.width, 60, "80 wanted, capped at half the width");
        assert_eq!(s.details.x, 62, "one blank column, then details");
        assert_eq!(s.details.width, 58);
    }

    #[test]
    fn a_narrow_stage_stacks_the_cover_on_top() {
        let area = Rect::new(0, 0, 60, 30);
        let s = stage_layout(area, FW, FH, true);
        let cover = s.cover.expect("cover");
        assert_eq!(cover.y, 0);
        assert_eq!(cover.height, 15, "half the height");
        assert!(cover.x > 0, "centred");
        assert_eq!(s.details.y, 16);
        assert_eq!(s.details.height, 14);
    }

    #[test]
    fn a_short_wide_stage_also_stacks() {
        let s = stage_layout(Rect::new(0, 0, 100, 12), FW, FH, true);
        let cover = s.cover.expect("cover");
        assert!(cover.height <= 6, "{cover:?}");
    }

    #[test]
    fn a_tiny_stage_or_show_cover_off_has_no_cover() {
        assert!(stage_layout(Rect::new(0, 0, 50, 7), FW, FH, true).cover.is_none());
        let s = stage_layout(Rect::new(0, 0, 120, 40), FW, FH, false);
        assert!(s.cover.is_none());
        assert!(s.details.width <= 80, "details centred, at most 80 wide");
        assert!(s.details.x > 0);
    }
```

- [ ] **Step 2: Run** — compile error (`stage_layout`).

- [ ] **Step 3: Implement** (in `layout.rs`, after `cover_width`)

```rust
/// Stage needs this much room for the side-by-side layout.
pub const STAGE_WIDE_MIN_W: u16 = 70;
pub const STAGE_WIDE_MIN_H: u16 = 14;
/// Below this many rows Stage shows no cover.
const STAGE_COVER_MIN_H: u16 = 8;
/// Details never get wider than this when centred without a cover.
const STAGE_DETAILS_MAX_W: u16 = 80;

/// Where Stage draws the cover (if any) and the details.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StageLayout {
    pub cover: Option<Rect>,
    pub details: Rect,
}

pub fn stage_layout(area: Rect, font_w: u16, font_h: u16, show_cover: bool) -> StageLayout {
    if !show_cover || area.height < STAGE_COVER_MIN_H || area.width < 4 {
        let width = area.width.saturating_sub(2).min(STAGE_DETAILS_MAX_W);
        return StageLayout {
            cover: None,
            details: Rect {
                x: area.x + (area.width - width) / 2,
                y: area.y + area.height.min(1),
                width,
                height: area.height.saturating_sub(1),
            },
        };
    }
    if area.width >= STAGE_WIDE_MIN_W && area.height >= STAGE_WIDE_MIN_H {
        let cover_w = cover_width(area.height, font_w, font_h, area.width);
        let gap = 2;
        return StageLayout {
            cover: Some(Rect {
                x: area.x,
                y: area.y,
                width: cover_w,
                height: area.height,
            }),
            details: Rect {
                x: area.x + cover_w + gap,
                y: area.y,
                width: area.width.saturating_sub(cover_w + gap),
                height: area.height,
            },
        };
    }
    let cover_h = area.height / 2;
    let cover_w = cover_width(cover_h, font_w, font_h, area.width);
    StageLayout {
        cover: Some(Rect {
            x: area.x + (area.width - cover_w) / 2,
            y: area.y,
            width: cover_w,
            height: cover_h,
        }),
        details: Rect {
            x: area.x + 1,
            y: area.y + cover_h + 1,
            width: area.width.saturating_sub(2),
            height: area.height - cover_h - 1,
        },
    }
}
```

(`cover_width` caps at half the width; the wide test's `details.x == 62` follows from `cover_w = 60` plus the 2-column gap. If `cover_width` rounds differently, adjust the expected numbers to the function's actual output and keep the relations: cover ≤ half, details right of the gap.)

- [ ] **Step 4:** run, clippy, fmt; commit `Lay out the stage view`.

---

### Task 4: Stage view behind `v`

**Files:**
- Create: `znicz-tui/src/views/stage.rs`
- Modify: `znicz-tui/src/app.rs`, `znicz-tui/src/views/mod.rs`, `znicz-tui/src/views/now_playing.rs`, `znicz-tui/src/lib.rs`
- Test: `stage.rs` unit tests, `znicz-tui/tests/stage.rs` (new)

- [ ] **Step 1: Write `stage.rs` line builders with failing tests**

Create `znicz-tui/src/views/stage.rs`:

```rust
//! Stage: the full-screen now-playing view — cover, details, signal path.

use std::sync::Arc;
use std::time::Duration;

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
    let font = app.picker.get_or_insert_with(Picker::halfblocks).font_size();
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
    let station = state.queue.get(state.queue_position).and_then(|item| match item {
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
            Span::styled(format!("  {}", format::duration(state.position)), theme::text()),
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
    let source_label = if track.url.is_some() { "stream" } else { "file" };
    lines.push(labelled(source_label, track.format_description(), theme::text()));
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
        Span::styled(format!("  {label:<width$}", width = LABEL - 2), theme::dim()),
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
```

Then **move** `cover_draw_label` and `render_cover` from `views/now_playing.rs` into `stage.rs` unchanged, making `render_cover` `pub(crate)`; in `now_playing.rs` call `crate::views::stage::render_cover(...)` and delete the moved functions and now-unused imports. (The Browse transport still draws a cover until Task 6.) Make `status_marker` in `now_playing.rs` reachable as `crate::views::now_playing::status_marker` (it is already `pub`).

Unit tests at the bottom of `stage.rs` (port the inspector's fixtures):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use znicz_core::{TrackTags};

    fn text(lines: &[Line]) -> String {
        lines
            .iter()
            .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect::<String>())
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
        for want in ["Sour Times", "Portishead", "Dummy · 1994 · track 3", "Signal", "FLAC", "Topping E30 II", "f32", "bit perfect"] {
            assert!(t.contains(want), "missing {want:?}:\n{t}");
        }
    }

    #[test]
    fn resampled_explains_both_rates_and_a_missing_format_is_a_dash() {
        let mut state = playing(false);
        state.output.as_mut().unwrap().sample_format = "?".into();
        let t = text(&detail_lines(&state, "x".into(), 70, 30));
        assert!(t.contains("resampled") && t.contains("48 kHz") && t.contains("96 kHz"), "{t}");
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
        let t = text(&detail_lines(&PlayerState::default(), "Nothing playing".into(), 70, 30));
        assert!(t.contains("Nothing playing") && t.contains("Browse (v)"), "{t}");
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
        assert!(t.contains("x") && t.contains("Portishead") && t.contains("/ "), "title, artist, seek stay:\n{t}");
    }
}
```

(Row counts in the last test follow from the layout: title, artist, album, blank, seek = 5 head rows; adjust `7` / `4` if the head differs, keeping the order of what is dropped.)

Add `pub mod stage;` (or `mod stage;` with `pub(crate)` items) in `views/mod.rs`.

Run `cargo test -p znicz-tui --lib stage` — red first (write tests before the builders, or with `todo!()`), then green.

- [ ] **Step 2: `View` and `v`**

`app.rs`:

```rust
/// What fills the screen above the footer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum View {
    /// Library, queue drawer, transport.
    #[default]
    Browse,
    /// Full-screen now playing.
    Stage,
}
```

- `App`: `pub view: View,` initialised `View::Browse`. Export `View` from `lib.rs` (`pub use app::{…, View};`).
- `fn toggle_stage(&mut self) { self.view = match self.view { View::Browse => View::Stage, View::Stage => View::Browse }; }`
- `on_global_key`: `KeyCode::Char('v') => self.toggle_stage(),`.
- In Stage, library keys do nothing:
  - `on_key` focus dispatch: before `match self.focus { … }` add `if self.view == View::Stage && self.focus == Focus::Library { return; }`.
  - `step`, `page`, `go_first`, `go_last`: the final library branch only when `self.view == View::Browse`.
  - `library_horizontal` and `library_toggle_tree_expand`: return early (false) when `self.view != View::Browse`.
  - `on_wheel` `Modal::None` → `Focus::Library` arm: only when `self.view == View::Browse`.
  - `wants_first_run_prompt`: require `self.view == View::Browse`.
- `on_esc`: replace the queue/library branches with

```rust
        } else if self.queue_open && (self.focus == Focus::Queue || self.view == View::Stage) {
            self.close_queue();
        } else if self.view == View::Stage {
            self.view = View::Browse;
        } else {
            self.library.back();
        }
```

- [ ] **Step 3: Draw Stage**

`views/mod.rs` `render`: after computing `chunks`, `list`, and the hit/width bookkeeping, branch:

```rust
    if app.view == crate::app::View::Stage {
        let transport = chunks[1];
        let message_row = Rect {
            y: transport.y + transport.height.saturating_sub(1),
            height: transport.height.min(1),
            ..transport
        };
        let stage_area = Rect {
            height: list.height + transport.height.saturating_sub(1),
            ..list
        };
        stage::render(frame, stage_area, app, state);
        let scanning = app.library_status.scanning.clone();
        frame.render_widget(
            Paragraph::new(now_playing::message_line(
                &app.toasts,
                scanning.as_ref(),
                usize::from(message_row.width),
            )),
            message_row,
        );
    } else {
        // existing: title_slot / clamp_pan / library::render
        // existing: now_playing::render_transport(…)
    }
```

Keep the queue drawer, footer, and modal drawing after the branch so they work in both views (the drawer still uses `list`). Move the library-only lines (`title_slot`, `clamp_pan`, `library::render`, `render_transport`) into the `else` branch.

- [ ] **Step 4: Integration tests**

Create `znicz-tui/tests/stage.rs`:

```rust
//! The Stage view: toggling, keys, and drawing.

use std::path::PathBuf;
use std::time::Duration;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::backend::TestBackend;
use ratatui::Terminal;
use znicz_core::{spawn_player, AudioConfig, OutputInfo, PlaybackStatus, PlayerState, TrackInfo, TrackTags};
use znicz_tui::{views, App, View};

fn app() -> App {
    let (player, _thread) = spawn_player(AudioConfig::default());
    App::with_library(player, None)
}

fn press(app: &mut App, c: char) {
    app.on_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
}

fn draw_with(app: &mut App, state: &PlayerState, w: u16, h: u16) -> String {
    let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
    terminal.draw(|f| views::render(f, app, state)).unwrap();
    let buf = terminal.backend().buffer();
    let mut out = String::new();
    for y in 0..buf.area().height {
        for x in 0..buf.area().width {
            out.push_str(buf[(x, y)].symbol());
        }
        out.push('\n');
    }
    out
}

fn playing() -> PlayerState {
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
                artist: Some("Portishead".into()),
                album: Some("Dummy".into()),
                ..TrackTags::default()
            },
        }),
        device_name: Some("Topping E30 II".into()),
        output: Some(OutputInfo {
            sample_rate: 96_000,
            channels: 2,
            sample_format: "f32".into(),
            bit_perfect: true,
        }),
        ..PlayerState::default()
    }
}

#[test]
fn v_toggles_stage() {
    let mut app = app();
    press(&mut app, 'v');
    assert_eq!(app.view, View::Stage);
    press(&mut app, 'v');
    assert_eq!(app.view, View::Browse);
}

#[test]
fn esc_closes_the_drawer_first_then_leaves_stage() {
    let mut app = app();
    press(&mut app, 'v');
    press(&mut app, ']');
    assert!(app.queue_open);
    app.on_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert!(!app.queue_open);
    assert_eq!(app.view, View::Stage);
    app.on_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert_eq!(app.view, View::Browse);
}

#[test]
fn library_keys_do_nothing_on_stage() {
    let mut app = app();
    press(&mut app, 'v');
    press(&mut app, '/');
    assert!(!app.library.is_typing());
    press(&mut app, 'F');
    assert!(app.folder_prompt.is_none());
}

#[test]
fn stage_draws_details_and_the_signal_panel() {
    let mut app = app();
    app.view = View::Stage;
    let screen = draw_with(&mut app, &playing(), 120, 40);
    for want in ["Sour Times", "Portishead", "Signal", "bit perfect", "Topping E30 II"] {
        assert!(screen.contains(want), "missing {want:?}:\n{screen}");
    }
    assert!(!screen.contains("Library"), "no library pane on stage:\n{screen}");
}

#[test]
fn stage_draws_at_every_size() {
    let mut app = app();
    app.view = View::Stage;
    for &(w, h) in &[(80u16, 24u16), (120, 40), (60, 18), (40, 12), (30, 8), (20, 5), (10, 3)] {
        let screen = draw_with(&mut app, &playing(), w, h);
        assert_eq!(screen.lines().count(), usize::from(h));
    }
}
```

- [ ] **Step 5:** `cargo test -p znicz-tui`, clippy, fmt, `cargo build -p znicz-tui --examples`; commit `Add the stage view on v`.

---

### Task 5: Fold the inspector into Stage

**Files:**
- Delete: `znicz-tui/src/views/inspector.rs`
- Modify: `znicz-tui/src/app.rs`, `znicz-tui/src/keys.rs`, `znicz-tui/src/views/mod.rs`, `znicz-tui/src/views/help.rs`, `znicz-tui/examples/preview.rs`
- Tests: `znicz-tui/tests/{keys,mouse,render,stage}.rs`, `keys.rs` tests

- [ ] **Step 1: Write the failing tests**

`tests/stage.rs`:

```rust
#[test]
fn i_toggles_stage_too() {
    let mut app = app();
    press(&mut app, 'i');
    assert_eq!(app.view, View::Stage);
    press(&mut app, 'i');
    assert_eq!(app.view, View::Browse);
}

#[test]
fn the_stage_footer_offers_browse() {
    let mut app = app();
    app.view = View::Stage;
    let screen = draw_with(&mut app, &playing(), 100, 30);
    let footer = screen.lines().last().unwrap();
    assert!(footer.contains("v browse"), "{footer}");
    assert!(footer.contains("Space pause"), "{footer}");
}
```

In `keys.rs` tests: replace `Context::Inspector` with `Context::Stage` in `ALL_CONTEXTS` and in `table_backed_hints_name_a_documented_key`; replace `INSPECTOR` with `STAGE` in the tables list; update the help-section title lists (`"Inspector"` → `"Stage"`). In `views/help.rs` tests replace `Inspector` with `Stage` in the context list.

Replace the inspector tests in `tests/keys.rs` (around lines 150–190 and 1309), `tests/mouse.rs` (266, 597), and `tests/render.rs` (200, 318, 730) as follows:
- keys tests that pressed `i` and asserted `Modal::Inspector` → assert `app.view == View::Stage` (and `i` again → Browse); tests asserting list movement is ignored "in the inspector" → the same keys leave the library selection unchanged with `app.view = View::Stage`.
- mouse tests that set `Modal::Inspector` → use `Modal::Devices` (they test overlay clicks generally).
- render: remove `Modal::Inspector` from size loops and the `no_modal_covers…` list; replace `the_signal_inspector_shows_the_device_sample_format` with a Stage render at 90×24 asserting `f32` and `Signal`.

- [ ] **Step 2: Run** — compile errors (`Context::Stage`, `STAGE`) and failing toggle test.

- [ ] **Step 3: Implement**

`keys.rs`:
- `Context`: remove `Inspector`, add `Stage`.
- Hints: `const NEXT: Hint = hint("n next", KeyCode::Char('n'));`, `const BROWSE: Hint = hint("v browse", KeyCode::Char('v'));`; `Context::Stage => &[SPACE_PAUSE, NEXT, BROWSE, QUEUE_OPEN],`.
- Remove `INSPECTOR`; add `pub const STAGE: &[Binding] = &[b("v / i / Esc", "back to browse")];`.
- `CONTEXT_SECTIONS`: `("Inspector", INSPECTOR)` → `("Stage", STAGE)`; `context_title`: `Context::Stage => Some("Stage")`.
- `GLOBAL`: replace `b("i", "signal inspector")` with `b("v / i", "now playing (stage)")`. Adjust any keys.rs test that asserts `GLOBAL` has `i` with "inspector" (e.g. `the_global_table_keeps_its_letters`) to look for `"v / i"` / "now playing".

`app.rs`:
- Remove `Modal::Inspector` and every arm/guard mentioning it (`blocks_list_focus`, `on_wheel`, the `on_key` early return now only `Modal::Messages`, `on_esc` list, `context`, `step`/`page`/`go_first`/`go_last` early returns). Delete `toggle_inspector_modal`.
- `on_global_key`: `KeyCode::Char('i') => self.toggle_stage(),`.
- `context()`: in `Modal::None`, before the focus match:

```rust
            Modal::None if self.view == View::Stage
                && !(self.queue_open && self.focus == Focus::Queue) =>
            {
                Context::Stage
            }
```

`views/mod.rs`: remove `mod inspector;` and the `Modal::Inspector` arm. Delete `views/inspector.rs`.

`examples/preview.rs`: replace the inspector frame (`app.modal = Modal::Inspector`) with a Stage frame (`app.view = View::Stage`, label it "Stage").

- [ ] **Step 4:** `cargo test -p znicz-tui`, clippy, fmt, examples build; commit `Fold the inspector into the stage view`.

---

### Task 6: Browse transport without the cover

**Files:**
- Modify: `znicz-tui/src/layout.rs`, `znicz-tui/src/views/mod.rs`, `znicz-tui/src/views/now_playing.rs`
- Test: `layout.rs` tests, `znicz-tui/tests/render.rs`

- [ ] **Step 1: Write the failing tests**

`layout.rs` tests: delete `cover_uses_eight_rows_on_a_normal_window`, `the_cover_needs_a_24_row_window`, `cover_starts_on_the_library_left_border`; replace `transport_rows_include_the_message_row` with

```rust
    #[test]
    fn the_transport_is_text_with_a_message_row() {
        assert_eq!(transport_height(30), 3);
        assert_eq!(transport_height(20), 3);
        assert_eq!(transport_height(19), 2);
        assert_eq!(transport_height(15), 2);
    }
```

`tests/render.rs`: replace `library_keeps_most_of_the_window_when_cover_is_on` with

```rust
#[test]
fn browse_has_no_cover_even_when_show_cover_is_on() {
    let mut app = App::with_library(player(), None);
    app.tui.show_cover = true;
    draw(&mut app, 80, 24);
    assert_eq!(app.list_height, 20, "24 - 3 transport - 1 footer");
}
```

and change `the_cover_slot_is_fully_painted_when_nothing_is_playing` to draw with `app.view = znicz_tui::View::Stage` and compute the cover rect with `znicz_tui::layout::stage_layout(Rect::new(0, 0, 80, 22), 10, 20, true)` (Stage area at 80×24 is 20 list rows + 3 transport − 1 message row = 22 rows), asserting every cell of that rect is painted.

- [ ] **Step 2: Run** — FAIL (`list_height` 15; `transport_height` takes two args).

- [ ] **Step 3: Implement**

- `layout.rs`: delete `COVER_MIN_WINDOW`, `COVER_ROWS`, `cover_enabled`, `cover_chrome_split`; `transport_height(window: u16) -> u16` returns 2 below `COMPACT_HEIGHT`, else 3. Keep `cover_width` (Stage uses it).
- `views/mod.rs`: `transport_height(area.height)`; drop `show_cover` and `with_cover`; call `now_playing::render_transport(frame, chunks[1], app, state, !compact)`.
- `now_playing.rs`: `render_transport(frame, area, app, state, show_signal)` keeps only the text-transport body (today's `if !with_cover { … }` block, unconditionally). Delete `render_stacked_chrome`, `title_row`, `artist_album_row`, `seek_row`, and the `render_cover` call and imports they needed.

- [ ] **Step 4:** `cargo test -p znicz-tui`, clippy, fmt, examples build; commit `Drop the cover from the browse transport`.

---

### Task 7: Marquee titles

**Files:**
- Modify: `znicz-tui/src/app.rs`, `znicz-tui/src/views/now_playing.rs`, `znicz-tui/src/views/stage.rs`

- [ ] **Step 1: Write the failing tests**

In `now_playing.rs` tests:

```rust
    fn long_state() -> PlayerState {
        let mut state = PlayerState::default();
        state.status = PlaybackStatus::Playing;
        state.current_track = Some(znicz_core::TrackInfo {
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
        });
        state
    }

    #[test]
    fn without_a_marquee_a_long_title_is_cut() {
        // At 100 columns the title gets 35 (the seek bar takes up to 40).
        let row = text(&chrome_line(&long_state(), 100, None));
        assert!(row.contains('…'), "{row:?}");
    }

    #[test]
    fn with_a_marquee_a_long_title_scrolls() {
        let state = long_state();
        let mut m = crate::marquee::Marquee::default();
        let t0 = std::time::Instant::now();
        let start = text(&chrome_line(&state, 100, Some((&mut m, t0))));
        assert!(start.contains("A Very"), "{start:?}");
        assert!(!start.contains('…'), "{start:?}");
        let later = text(&chrome_line(
            &state,
            100,
            Some((&mut m, t0 + Duration::from_millis(2_000 + 5 * 200))),
        ));
        assert!(!later.contains("A Very"), "moved 5 columns: {later:?}");
        assert!(later.contains("y Long Title"), "{later:?}");
    }
```

Update every existing `chrome_line(state, width)` call (tests and `render_transport`) to the new signature.

In `stage.rs` tests nothing changes (`detail_lines` already takes a fitted title).

- [ ] **Step 2: Run** — compile errors on the new `chrome_line` signature.

- [ ] **Step 3: Implement**

`app.rs`: fields `pub(crate) transport_marquee: crate::marquee::Marquee,` and `pub(crate) stage_marquee: crate::marquee::Marquee,` (both `Default::default()`).

`now_playing.rs`:
- `fn chrome_line(state: &PlayerState, width: usize, marquee: Option<(&mut crate::marquee::Marquee, std::time::Instant)>) -> Line<'static>`.
- After `seek_len` / `seek_prefix` are known and `title_w` computed, add the scrolling path:

```rust
    let slot = middle_budget.saturating_sub(seek_prefix + seek_len);
    let subtitle_text = if include_subtitle { subtitle.clone().unwrap_or_default() } else { String::new() };
    let segment = format!("{title}{subtitle_text}");
    let overflows = segment.chars().count() > slot;
    if let (true, Some((marquee, now))) = (overflows, marquee) {
        let offset = marquee.offset(&segment, slot, now);
        let shown = crate::marquee::window(&segment, offset, slot);
        // Title style for the part that is still title, subtitle style after it.
        let title_left = title.chars().count().saturating_sub(offset);
        let title_part: String = shown.chars().take(title_left).collect();
        let sub_part: String = shown.chars().skip(title_left).collect();
        let mut spans = vec![
            Span::styled(sym, status_style),
            Span::styled(title_part, theme::title()),
            Span::styled(sub_part, theme::subtitle()),
        ];
        let pad = slot.saturating_sub(shown.chars().count());
        spans.push(Span::raw(" ".repeat(pad)));
        // …then the same seek / times / volume / toggles spans as the normal path.
        push_tail(&mut spans, state, seek_len, &times, include_volume, &volume, include_toggles, &toggles);
        return Line::from(spans);
    }
```

Extract today's seek + times + volume + toggles span building at the end of `chrome_line` into `fn push_tail(...)` (`#[allow(clippy::too_many_arguments)]` if needed) and call it from both paths, so the normal path is unchanged.
- `render_transport`: build the line with `chrome_line(state, width, app.tui.marquee.then(|| (&mut app.transport_marquee, Instant::now())))` (split the borrow: compute `let enabled = app.tui.marquee;` first, and pass `if enabled { Some((&mut app.transport_marquee, Instant::now())) } else { None }`).

`stage.rs` `render`: replace `let title = format::truncate(&title_text(state), width);` with

```rust
    let full = title_text(state);
    let title = if app.tui.marquee && full.chars().count() > width {
        let offset = app.stage_marquee.offset(&full, width, std::time::Instant::now());
        crate::marquee::window(&full, offset, width)
    } else {
        format::truncate(&full, width)
    };
```

- [ ] **Step 4:** `cargo test -p znicz-tui`, clippy, fmt; commit `Scroll long now-playing titles`.

---

### Task 8: Version and docs

- [ ] **Step 1:** Root `Cargo.toml` version `0.8.0` → `0.9.0`; `cargo build -p znicz`; update the "currently" version in `wiki/Rust/Cargo-Workspace.md`.
- [ ] **Step 2: README.** Key table: `v` / `i` — "Stage: full-screen now playing (cover, details, signal path)"; remove the inspector wording. Config: `show_cover` now means the Stage cover; `cover_protocol` applies to Stage; new `marquee = true`. Mention that long titles scroll.
- [ ] **Step 3: Wiki.** `Architecture/TUI.md`: a "Views" section (Browse / Stage, keys, Esc order), the Stage layout (wide, stacked, no cover, streams, nothing playing), the marquee (timing, reset, config), Browse transport rows (3 / 2, no cover), signal path now on Stage (replace the inspector section); modules table: remove `views/inspector.rs`, add `views/stage.rs` and `marquee.rs`. `Plans/Roadmap.md`: part 4 → `**Done** in 0.9.0 ([spec](../../docs/superpowers/specs/2026-11-09-stage-mode-design.md))`; `wiki/Issues.md`: move #46 and #34 to Closed (fixed in 0.9.0). Grep README/wiki for stale "inspector", "transport cover", "8-row cover", "24 rows" statements and fix those describing current behaviour.
- [ ] **Step 4:** `cargo test --workspace`, fmt; commit `Bump to 0.9.0 and document stage mode`.

---

### Task 9: Verify and open the PR

- [ ] **Step 1:** fmt check, clippy `-D warnings`, `cargo test --workspace` — all clean.
- [ ] **Step 2:** Tick every checkbox here; commit `Mark stage mode plan complete`.
- [ ] **Step 3:** Push `feat/stage-mode`; open the PR "Stage mode (TUI rework part 4)" with `Closes #46` and `Closes #34`, ending with `🤖 Generated with [Claude Code](https://claude.com/claude-code)`.
