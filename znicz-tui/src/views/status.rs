//! The hint line at the bottom of the frame.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::app::App;
use crate::footer_hits::{self, FooterLabel, PlacedHint, SEPARATOR};
use crate::keys::{self, Hint};
use crate::theme;
use crate::update::Release;

/// Key hints for the current context. Toasts never replace this line.
pub fn render_footer(frame: &mut Frame, area: Rect, app: &mut App) {
    let context = app.context();
    let unread = app.toasts.unread_alerts().map(|(count, _)| count);
    let labels: Vec<FooterLabel> = keys::context_hints(context)
        .iter()
        .map(|hint| display_label(*hint, unread))
        .collect();
    let more = keys::more_hint(context).map(FooterLabel::from);
    let has_more = more.is_some();
    let update = app.update.as_ref();
    let placed = footer_placement(area, &labels, more, update);
    app.hits.footer_hints = footer_hits::footer_hits(&placed);
    let badge_style = if update.is_some() {
        theme::update_badge()
    } else {
        theme::dim()
    };
    frame.render_widget(
        Paragraph::new(footer_line(area, &placed, badge_style)),
        area,
    );
    if !has_more {
        if let Some(rect) = lone_version_rect(area, &placed, update) {
            let version = version_badges(update).pop().unwrap_or_default();
            frame.render_widget(Paragraph::new(Span::styled(version, badge_style)), rect);
        }
    }
}

/// Where the version goes in a footer without `? more` (Help, prompts): the
/// right end, when it fits after the hints with two spaces to spare.
fn lone_version_rect(area: Rect, placed: &[PlacedHint], update: Option<&Release>) -> Option<Rect> {
    let width = version_badges(update).last()?.chars().count() as u16;
    let used = placed
        .last()
        .map_or(area.x, |hint| hint.rect.x + hint.rect.width);
    let start = area.x + area.width.checked_sub(width)?;
    (start >= used + 2).then(|| Rect::new(start, area.y, width, 1))
}

/// The version badges to try after `? more`, longest first: the running
/// version, and the newer one while an update is known.
fn version_badges(update: Option<&Release>) -> Vec<String> {
    let current = env!("CARGO_PKG_VERSION");
    match update {
        Some(release) => vec![
            format!("v{current} → {} available", release.version),
            format!("v{current} → {}", release.version),
        ],
        None => vec![format!("v{current}")],
    }
}

/// Places the hints, with `? more` carrying the version badge
/// (`? more · v0.16.0`, or `? more · v0.16.0 → 0.17.0 available`).
///
/// The longest badge that leaves the same hints on the row as plain `? more`
/// is used. When none does, the shortest badge stays and hints give way to
/// it, so the version shows unless the row cannot hold `? more` and it.
fn footer_placement(
    area: Rect,
    labels: &[FooterLabel],
    more: Option<FooterLabel>,
    update: Option<&Release>,
) -> Vec<PlacedHint> {
    let plain = footer_hits::layout_footer(area, labels, more.clone());
    let Some(more) = more else {
        return plain;
    };
    let badged = |badge: &String| {
        let mut label = more.clone();
        label.badge = Some(badge.clone());
        footer_hits::layout_footer(area, labels, Some(label))
    };
    let badges = version_badges(update);
    for badge in &badges {
        let placed = badged(badge);
        let same_hints = placed
            .iter()
            .map(|p| &p.label.text)
            .eq(plain.iter().map(|p| &p.label.text));
        if same_hints {
            return placed;
        }
    }
    let shortest = badges.last().expect("at least one badge");
    let placed = badged(shortest);
    let shows_more = placed.iter().any(|p| p.label.badge.is_some());
    if shows_more { placed } else { plain }
}

/// The hint as drawn: `M messages` carries the unread-alerts count.
fn display_label(hint: Hint, unread: Option<usize>) -> FooterLabel {
    let mut label = FooterLabel::from(hint);
    if hint == keys::MESSAGES_OPEN {
        if let Some(count) = unread {
            label.text = format!("{} ({count})", hint.label);
        }
    }
    label
}

/// Hints at their placed columns: ` · ` between neighbours, spaces before `? more`,
/// and the badge (if any) after the hint's action, in `badge_style`.
fn footer_line(area: Rect, placed: &[PlacedHint], badge_style: Style) -> Line<'static> {
    let separator_width = SEPARATOR.chars().count() as u16;
    let mut spans = Vec::new();
    let mut x = area.x;
    for item in placed {
        let gap = item.rect.x.saturating_sub(x);
        if x > area.x && gap == separator_width {
            spans.push(Span::styled(SEPARATOR, theme::dim()));
        } else if gap > 0 {
            spans.push(Span::raw(" ".repeat(usize::from(gap))));
        }
        let (key, action) = item
            .label
            .text
            .split_once(' ')
            .unwrap_or((item.label.text.as_str(), ""));
        spans.push(Span::styled(key.to_string(), theme::key()));
        if !action.is_empty() {
            spans.push(Span::styled(format!(" {action}"), theme::dim()));
        }
        if let Some(badge) = &item.label.badge {
            spans.push(Span::styled(SEPARATOR, theme::dim()));
            spans.push(Span::styled(badge.clone(), badge_style));
        }
        x = item.rect.x + item.rect.width;
    }
    Line::from(spans)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::update::Release;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;
    use ratatui::style::Modifier;

    fn label(text: &str, c: char) -> FooterLabel {
        FooterLabel {
            text: text.to_string(),
            key: KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE),
            badge: None,
        }
    }

    /// The badge for an update to 9.9.9.
    fn update_text() -> String {
        format!("v{} → 9.9.9 available", env!("CARGO_PKG_VERSION"))
    }

    fn release(version: &str) -> Release {
        Release {
            version: version.into(),
            notes: String::new(),
            url: String::new(),
        }
    }

    /// The footer row as text, with the spans' content joined.
    fn drawn(area: Rect, labels: &[FooterLabel], update: Option<&Release>) -> String {
        let more = Some(label("? more", '?'));
        let placed = footer_placement(area, labels, more, update);
        footer_line(area, &placed, theme::dim())
            .spans
            .iter()
            .map(|span| span.content.to_string())
            .collect()
    }

    #[test]
    fn a_wide_footer_says_the_update_after_more() {
        let line = drawn(
            Rect::new(0, 0, 120, 1),
            &[label("/ search", '/')],
            Some(&release("9.9.9")),
        );
        assert!(
            line.ends_with(&format!("? more · {}", update_text())),
            "{line:?}"
        );
    }

    #[test]
    fn without_an_update_the_footer_shows_the_version() {
        let line = drawn(Rect::new(0, 0, 120, 1), &[label("/ search", '/')], None);
        let version = format!("? more · v{}", env!("CARGO_PKG_VERSION"));
        assert!(line.ends_with(&version), "{line:?}");
        assert!(!line.contains("available"), "{line:?}");
    }

    #[test]
    fn a_long_update_badge_shortens_before_hints_give_way() {
        // "/ search · ? more · v… → 9.9.9 available" does not fit 60 columns,
        // the short form does.
        let short = format!("v{} → 9.9.9", env!("CARGO_PKG_VERSION"));
        let labels = [label("/ search", '/')];
        let width = ("/ search · ? more · ".chars().count() + short.chars().count()) as u16;
        let line = drawn(Rect::new(0, 0, width, 1), &labels, Some(&release("9.9.9")));
        assert!(line.starts_with("/ search"), "{line:?}");
        assert!(line.ends_with(&format!("? more · {short}")), "{line:?}");
    }

    #[test]
    fn hints_give_way_to_the_version() {
        let version = format!("v{}", env!("CARGO_PKG_VERSION"));
        let labels = [label("/ search", '/'), label("a add", 'a')];
        // Room for "? more · v…" and one hint, not two.
        let width = ("a add · ? more · ".chars().count() + version.chars().count()) as u16;
        let line = drawn(Rect::new(0, 0, width, 1), &labels, None);
        assert!(line.ends_with(&format!("? more · {version}")), "{line:?}");
        assert!(!line.contains("/ search · a add"), "{line:?}");
    }

    #[test]
    fn a_row_too_narrow_for_the_version_keeps_plain_more() {
        let line = drawn(Rect::new(0, 0, 8, 1), &[], None);
        assert_eq!(line.trim(), "? more");
    }

    #[test]
    fn the_badge_uses_the_update_badge_style() {
        let placed = footer_placement(
            Rect::new(0, 0, 120, 1),
            &[label("/ search", '/')],
            Some(label("? more", '?')),
            Some(&release("9.9.9")),
        );
        let line = footer_line(Rect::new(0, 0, 120, 1), &placed, theme::update_badge());
        let badge = line
            .spans
            .iter()
            .find(|span| span.content == update_text())
            .expect("badge span");
        assert_eq!(badge.style, theme::update_badge());
    }

    #[test]
    fn the_rendered_footer_of_an_app_with_an_update_shows_the_badge() {
        let (player, _thread) = znicz_core::spawn_player(znicz_core::AudioConfig::default());
        let mut app = crate::app::App::with_library(player, None);
        app.update = Some(release("9.9.9"));
        let area = Rect::new(0, 0, 160, 1);
        let mut terminal = Terminal::new(TestBackend::new(160, 1)).unwrap();
        terminal
            .draw(|frame| render_footer(frame, area, &mut app))
            .unwrap();
        let buffer: &Buffer = terminal.backend().buffer();
        let row: String = (0..160).map(|x| buffer[(x, 0)].symbol()).collect();
        let badge = update_text();
        assert!(row.ends_with(&format!("? more · {badge}")), "{row:?}");
        // Columns, not bytes: the badge follows a `·`.
        let start = row[..row.find(&badge).unwrap()].chars().count() as u16;
        let cell = &buffer[(start, 0)];
        assert_eq!(cell.fg, theme::GOOD);
        assert!(cell.modifier.contains(Modifier::BOLD));
    }
}
