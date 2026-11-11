//! The hint line at the bottom of the frame.

use ratatui::Frame;
use ratatui::layout::Rect;
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
    let placed = footer_placement(area, &labels, more, app.update.as_ref());
    app.hits.footer_hints = footer_hits::footer_hits(&placed);
    frame.render_widget(Paragraph::new(footer_line(area, &placed)), area);
}

/// Places the hints, with `? more` carrying `N available` while an update is known.
///
/// The badge is kept only if it leaves the same hints on the row as plain
/// `? more` would. On a narrow terminal the plain hint is used, so no other
/// hint is dropped for the badge.
fn footer_placement(
    area: Rect,
    labels: &[FooterLabel],
    more: Option<FooterLabel>,
    update: Option<&Release>,
) -> Vec<PlacedHint> {
    let plain = footer_hits::layout_footer(area, labels, more.clone());
    let (Some(mut badged), Some(release)) = (more, update) else {
        return plain;
    };
    badged.badge = Some(format!("{} available", release.version));
    let with_badge = footer_hits::layout_footer(area, labels, Some(badged));
    let same_hints = with_badge
        .iter()
        .map(|p| &p.label.text)
        .eq(plain.iter().map(|p| &p.label.text));
    if same_hints { with_badge } else { plain }
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
/// and the badge (if any) after the hint's action.
fn footer_line(area: Rect, placed: &[PlacedHint]) -> Line<'static> {
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
            spans.push(Span::styled(badge.clone(), theme::update_badge()));
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
        footer_line(area, &placed)
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
        assert!(line.ends_with("? more · 9.9.9 available"), "{line:?}");
    }

    #[test]
    fn without_an_update_the_footer_ends_with_plain_more() {
        let line = drawn(Rect::new(0, 0, 120, 1), &[label("/ search", '/')], None);
        assert!(line.ends_with("? more"), "{line:?}");
        assert!(!line.contains("available"), "{line:?}");
    }

    #[test]
    fn a_badge_that_would_push_out_a_hint_falls_back_to_plain_more() {
        // Plain: "/ search · a add · ? more" fits 30 columns.
        // With the badge (24 columns), only "? more" would fit.
        let labels = [label("/ search", '/'), label("a add", 'a')];
        let line = drawn(Rect::new(0, 0, 30, 1), &labels, Some(&release("9.9.9")));
        assert!(line.starts_with("/ search · a add"), "{line:?}");
        assert!(line.ends_with("? more"), "{line:?}");
        assert!(!line.contains("available"), "{line:?}");
    }

    #[test]
    fn the_badge_uses_the_update_badge_style() {
        let placed = footer_placement(
            Rect::new(0, 0, 120, 1),
            &[label("/ search", '/')],
            Some(label("? more", '?')),
            Some(&release("9.9.9")),
        );
        let line = footer_line(Rect::new(0, 0, 120, 1), &placed);
        let badge = line
            .spans
            .iter()
            .find(|span| span.content == "9.9.9 available")
            .expect("badge span");
        assert_eq!(badge.style, theme::update_badge());
    }

    #[test]
    fn the_rendered_footer_of_an_app_with_an_update_shows_the_badge() {
        let (player, _thread) = znicz_core::spawn_player(znicz_core::AudioConfig::default());
        let mut app = crate::app::App::with_library(player, None);
        app.update = Some(release("9.9.9"));
        let area = Rect::new(0, 0, 120, 1);
        let mut terminal = Terminal::new(TestBackend::new(120, 1)).unwrap();
        terminal
            .draw(|frame| render_footer(frame, area, &mut app))
            .unwrap();
        let buffer: &Buffer = terminal.backend().buffer();
        let row: String = (0..120).map(|x| buffer[(x, 0)].symbol()).collect();
        assert!(row.ends_with("? more · 9.9.9 available"), "{row:?}");
        let start = row.find("9.9.9 available").unwrap() as u16;
        let cell = &buffer[(start, 0)];
        assert_eq!(cell.fg, theme::GOOD);
        assert!(cell.modifier.contains(Modifier::BOLD));
    }
}
