//! Where each footer hint lands, so a click can run that hint's key.

use crossterm::event::KeyEvent;
use ratatui::layout::Rect;

use crate::hit::FooterHit;
use crate::keys::Hint;

/// Drawn between neighbouring hints.
pub const SEPARATOR: &str = " · ";
const SEPARATOR_WIDTH: u16 = 3;

/// What a footer hint says on screen, and the key a click sends.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FooterLabel {
    pub text: String,
    pub key: KeyEvent,
}

impl From<Hint> for FooterLabel {
    fn from(hint: Hint) -> Self {
        Self {
            text: hint.label.to_string(),
            key: hint.key,
        }
    }
}

/// A hint and the cells it occupies on the footer row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlacedHint {
    pub rect: Rect,
    pub label: FooterLabel,
}

fn label_width(label: &FooterLabel) -> u16 {
    label.text.chars().count() as u16
}

/// Lay hints out left to right, keeping `more` right-aligned.
///
/// The space for `more` (plus one separator) is reserved first, so it is never
/// pushed off by a long hint list. Hints that do not fit whole are dropped
/// rather than cut.
pub fn layout_footer(
    area: Rect,
    labels: &[FooterLabel],
    more: Option<FooterLabel>,
) -> Vec<PlacedHint> {
    if area.width == 0 || area.height == 0 {
        return Vec::new();
    }
    let more = more.filter(|m| label_width(m) <= area.width);
    let budget = match &more {
        Some(m) => area
            .width
            .saturating_sub(label_width(m))
            .saturating_sub(SEPARATOR_WIDTH),
        None => area.width,
    };

    let mut placed = Vec::new();
    let mut x: u16 = 0;
    for label in labels {
        let gap = if placed.is_empty() {
            0
        } else {
            SEPARATOR_WIDTH
        };
        let width = label_width(label);
        if u32::from(x) + u32::from(gap) + u32::from(width) > u32::from(budget) {
            break;
        }
        x += gap;
        placed.push(PlacedHint {
            rect: Rect::new(area.x + x, area.y, width, 1),
            label: label.clone(),
        });
        x += width;
    }

    if let Some(more) = more {
        let width = label_width(&more);
        placed.push(PlacedHint {
            rect: Rect::new(area.x + area.width - width, area.y, width, 1),
            label: more,
        });
    }
    placed
}

/// Click targets for placed hints.
pub fn footer_hits(placed: &[PlacedHint]) -> Vec<FooterHit> {
    placed
        .iter()
        .map(|p| FooterHit {
            rect: p.rect,
            key: p.label.key,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn h(label: &str, c: char) -> FooterLabel {
        FooterLabel {
            text: label.to_string(),
            key: KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE),
        }
    }

    fn labels(placed: &[PlacedHint]) -> Vec<String> {
        placed.iter().map(|p| p.label.text.clone()).collect()
    }

    #[test]
    fn hints_run_left_to_right_and_more_sits_on_the_right() {
        let area = Rect::new(0, 23, 40, 1);
        let placed = layout_footer(
            area,
            &[h("/ search", '/'), h("a add", 'a')],
            Some(h("? more", '?')),
        );
        assert_eq!(labels(&placed), vec!["/ search", "a add", "? more"]);
        assert_eq!(placed[0].rect, Rect::new(0, 23, 8, 1));
        assert_eq!(placed[1].rect, Rect::new(11, 23, 5, 1));
        assert_eq!(placed[2].rect, Rect::new(34, 23, 6, 1));
    }

    #[test]
    fn hints_that_would_crowd_more_are_dropped() {
        // Budget is 20 - 6 ("? more") - 3 (separator) = 11: only "/ search" fits.
        let placed = layout_footer(
            Rect::new(0, 0, 20, 1),
            &[h("/ search", '/'), h("a add", 'a')],
            Some(h("? more", '?')),
        );
        assert_eq!(labels(&placed), vec!["/ search", "? more"]);
    }

    #[test]
    fn without_more_hints_use_the_full_width() {
        let placed = layout_footer(
            Rect::new(0, 0, 16, 1),
            &[h("/ search", '/'), h("a add", 'a')],
            None,
        );
        assert_eq!(labels(&placed), vec!["/ search", "a add"]);
    }

    #[test]
    fn a_footer_too_narrow_for_any_label_places_nothing() {
        let placed = layout_footer(
            Rect::new(0, 0, 4, 1),
            &[h("/ search", '/')],
            Some(h("? more", '?')),
        );
        assert!(placed.is_empty());
        assert!(layout_footer(Rect::new(0, 0, 0, 1), &[h("a add", 'a')], None).is_empty());
    }

    #[test]
    fn hits_carry_each_hints_key() {
        let placed = layout_footer(
            Rect::new(0, 23, 40, 1),
            &[h("a add", 'a')],
            Some(h("? more", '?')),
        );
        let keys: Vec<KeyCode> = footer_hits(&placed).iter().map(|h| h.key.code).collect();
        assert_eq!(keys, [KeyCode::Char('a'), KeyCode::Char('?')]);
    }
}
