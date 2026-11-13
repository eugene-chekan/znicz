//! The Settings modal: rows in groups, and a strip that explains the highlighted row.

use std::path::Path;

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};

use crate::app::App;
use crate::folder_complete;
use crate::format;
use crate::hit::ListHit;
use crate::settings::{Row, Values, hint_text, value_text};
use crate::theme;
use crate::views::{self, window::visible_window};

/// Below this inner height the explanation takes one line instead of two.
const TWO_LINE_STRIP_MIN_HEIGHT: u16 = 8;

/// Columns the row's label is indented by.
const INDENT: &str = "    ";

pub fn render(frame: &mut Frame, list: Rect, app: &mut App) {
    let popup = crate::layout::list_modal_rect(list);
    app.hits.overlay = Some(popup);
    app.hits.close = views::close_button_rect(popup);
    frame.render_widget(Clear, popup);

    let mut block = views::pane_block("Settings", true, None).title(views::close_title());
    if let Some(path) = config_path_label(app, popup.width) {
        block = block.title_bottom(Line::from(Span::styled(path, theme::dim())));
    }
    let inner = block.inner(popup);
    frame.render_widget(block, popup);

    let strip_lines: u16 = if inner.height < TWO_LINE_STRIP_MIN_HEIGHT {
        1
    } else {
        2
    };
    // The list takes what the rule and the strip leave.
    let list_height = inner.height.saturating_sub(1 + strip_lines);
    let list_area = Rect {
        height: list_height,
        ..inner
    };
    let rule = Rect {
        y: inner.y + list_height,
        height: u16::from(inner.height > list_height),
        ..inner
    };
    let strip = Rect {
        y: rule.y + rule.height,
        height: inner
            .height
            .saturating_sub(list_height + rule.height)
            .min(strip_lines),
        ..inner
    };

    let cursor = app.settings_cursor;
    let len = app.settings_rows.len();
    let window = visible_window(
        len,
        Some(cursor),
        app.settings_scroll,
        usize::from(list_height),
    );
    app.settings_scroll = window.start;
    app.hits.overlay_list = Some(ListHit {
        inner: list_area,
        offset: window.start,
        len,
    });
    let app: &App = app;

    let values = values(app);
    let width = usize::from(list_area.width);
    let lines: Vec<Line> = app.settings_rows[window.clone()]
        .iter()
        .enumerate()
        .map(|(i, row)| row_line(row, &values, window.start + i == cursor, width))
        .collect();
    frame.render_widget(Paragraph::new(lines), list_area);

    if rule.height > 0 {
        let line = "─".repeat(usize::from(rule.width));
        frame.render_widget(Paragraph::new(Span::styled(line, theme::dim())), rule);
    }

    let explanation = match &app.settings_confirm {
        Some((path, tracks)) => removal_question(path, *tracks),
        None => app
            .settings_rows
            .get(cursor)
            .map_or("", Row::explanation)
            .to_string(),
    };
    let strip_width = usize::from(strip.width);
    let text: Vec<Line> = wrap(&explanation, strip_width, usize::from(strip_lines))
        .into_iter()
        .map(|line| Line::from(Span::styled(line, theme::dim())))
        .collect();
    frame.render_widget(Paragraph::new(text), strip);
}

/// What the rows display, read from the app.
fn values(app: &App) -> Values<'_> {
    Values {
        tui: &app.tui,
        default_device: app.default_device.as_deref().map(|id| {
            app.devices
                .iter()
                .find(|device| device.id == id)
                .map_or(id, |device| device.name.as_str())
        }),
        bit_perfect: app.bit_perfect,
        update_check: app.check_for_updates,
        protocol_needs_restart: app.protocol_needs_restart(),
        protocol_unsupported: !crate::settings::protocol_supported(
            app.tui.cover_protocol,
            app.protocol_support,
        ),
        stage_shows: app.stage_frozen.as_ref(),
        bit_perfect_changed: app.bit_perfect_changed(),
    }
}

/// One row: a heading, or the label with its value and hint flush right.
fn row_line(row: &Row, values: &Values, selected: bool, width: usize) -> Line<'static> {
    if let Row::Heading(title) = row {
        return Line::from(Span::styled(format!("  {title}"), theme::strong()));
    }
    let mut tail = vec![Span::styled(value_text(row, values), theme::text())];
    if let Some(hint) = hint_text(row, values) {
        tail.push(Span::styled(format!("  {hint}"), theme::dim()));
    }
    let tail_width: usize = tail.iter().map(|span| span.content.chars().count()).sum();

    let head = format::truncate(
        &format!("{INDENT}{}", row.label()),
        width.saturating_sub(tail_width + 1),
    );
    let gap = width.saturating_sub(head.chars().count() + tail_width);
    let mut spans = vec![Span::styled(
        format!("{head}{}", " ".repeat(gap)),
        theme::text(),
    )];
    spans.extend(tail);
    let line = Line::from(spans);
    if selected {
        line.style(views::highlight(true))
    } else {
        line
    }
}

/// The config file's path for the bottom border: `~` for home, cut from the left.
fn config_path_label(app: &App, popup_width: u16) -> Option<String> {
    let file = app.config_file.as_ref()?;
    let path = match dirs::home_dir() {
        Some(home) => folder_complete::tilde(file.path(), &home),
        None => file.path().display().to_string(),
    };
    // Two corners and a space on each side of the label.
    let room = usize::from(popup_width).saturating_sub(4);
    Some(format!(" {} ", cut_left(&path, room)))
}

/// The question a folder removal asks. The keys come first, so they stay
/// visible when a one-line strip cuts the end. The count is left out when the
/// library could not be read.
fn removal_question(path: &Path, tracks: Option<usize>) -> String {
    let shown = match dirs::home_dir() {
        Some(home) => folder_complete::tilde(path, &home),
        None => path.display().to_string(),
    };
    let tracks = match tracks {
        Some(count) => format::tracks_label(count),
        None => "tracks".to_string(),
    };
    format!("Enter remove · Esc keep — Remove {shown} and its {tracks} from the library?")
}

/// Keep the end of `text` that fits in `max` columns, marked with a leading `…`.
fn cut_left(text: &str, max: usize) -> String {
    let count = text.chars().count();
    if count <= max {
        return text.to_string();
    }
    if max == 0 {
        return String::new();
    }
    let kept: String = text.chars().skip(count - (max - 1)).collect();
    format!("…{kept}")
}

/// Word-wrap `text` to `width` columns and at most `max_lines` lines. Whatever
/// does not fit ends the last line with `…`.
fn wrap(text: &str, width: usize, max_lines: usize) -> Vec<String> {
    let max_lines = max_lines.max(1);
    let mut lines: Vec<String> = Vec::new();
    for word in text.split_whitespace() {
        match lines.last_mut() {
            Some(line) if line.chars().count() + 1 + word.chars().count() <= width => {
                line.push(' ');
                line.push_str(word);
            }
            _ => lines.push(word.to_string()),
        }
    }
    if lines.len() > max_lines {
        let rest = lines.split_off(max_lines - 1).join(" ");
        lines.push(rest);
    }
    lines
        .iter()
        .map(|line| format::truncate(line, width))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrap_breaks_at_words_and_keeps_the_text_whole_when_it_fits() {
        assert_eq!(wrap("one two three", 20, 2), ["one two three"]);
        assert_eq!(wrap("one two three", 7, 2), ["one two", "three"]);
    }

    #[test]
    fn wrap_stops_at_the_line_limit_with_an_ellipsis() {
        let lines = wrap("one two three four five six", 7, 2);
        assert_eq!(lines, ["one two", "three …"]);
        assert_eq!(wrap("one two three four", 9, 1), ["one two …"]);
    }

    #[test]
    fn wrap_of_nothing_is_empty() {
        assert!(wrap("", 10, 2).is_empty());
    }

    #[test]
    fn cut_left_keeps_the_end_of_a_long_path() {
        assert_eq!(cut_left("~/Music", 20), "~/Music");
        assert_eq!(cut_left("~/Music/Archive/config.toml", 12), "…config.toml");
        assert_eq!(cut_left("abc", 0), "");
    }
}
