//! Footer hint segments → key events and hit rects.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::Rect;

use crate::hit::FooterHit;

pub fn key_for_hint_segment(segment: &str) -> Option<KeyEvent> {
    let segment = segment.trim();
    if segment.is_empty() {
        return None;
    }
    // Close-preferring compound labels first.
    if segment.contains("Esc") && segment.contains("close") {
        return Some(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    }
    if segment.starts_with("Alt-") && segment.contains("pan") {
        return Some(KeyEvent::new(KeyCode::Right, KeyModifiers::ALT));
    }
    let key_part = segment.split_whitespace().next()?;
    match key_part {
        "Enter" => Some(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        "Esc" => Some(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
        "Space" => Some(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE)),
        "type" | "←" | "→" => None,
        s if s.chars().count() == 1 => {
            let c = s.chars().next()?;
            Some(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE))
        }
        _ => None,
    }
}

pub fn layout_footer_hits(area: Rect, line: &str) -> Vec<FooterHit> {
    if area.width == 0 || area.height == 0 {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut x = 0u16;
    let parts: Vec<&str> = line.split(" · ").collect();
    for (i, part) in parts.iter().enumerate() {
        let sep = if i + 1 < parts.len() { " · " } else { "" };
        let part_width = part.chars().count() as u16;
        if x.saturating_add(part_width) > area.width {
            break;
        }
        if let Some(key) = key_for_hint_segment(part) {
            out.push(FooterHit {
                rect: Rect {
                    x: area.x + x,
                    y: area.y,
                    width: part_width,
                    height: 1,
                },
                key,
            });
        }
        x = x.saturating_add(part_width);
        let sep_width = sep.chars().count() as u16;
        if x.saturating_add(sep_width) > area.width {
            break;
        }
        x = x.saturating_add(sep_width);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_common_footer_segments() {
        assert_eq!(
            key_for_hint_segment("? help").map(|k| k.code),
            Some(KeyCode::Char('?'))
        );
        assert_eq!(
            key_for_hint_segment("Esc close").map(|k| k.code),
            Some(KeyCode::Esc)
        );
        assert_eq!(
            key_for_hint_segment("i / Esc close").map(|k| k.code),
            Some(KeyCode::Esc)
        );
        assert_eq!(
            key_for_hint_segment("Enter play").map(|k| k.code),
            Some(KeyCode::Enter)
        );
        assert_eq!(
            key_for_hint_segment("Space pause").map(|k| k.code),
            Some(KeyCode::Char(' '))
        );
        assert_eq!(
            key_for_hint_segment("a add").map(|k| k.code),
            Some(KeyCode::Char('a'))
        );
        assert_eq!(
            key_for_hint_segment("C clear").map(|k| k.code),
            Some(KeyCode::Char('C'))
        );
        assert_eq!(
            key_for_hint_segment("/ search").map(|k| k.code),
            Some(KeyCode::Char('/'))
        );
        assert_eq!(
            key_for_hint_segment("] queue").map(|k| k.code),
            Some(KeyCode::Char(']'))
        );
        assert_eq!(
            key_for_hint_segment(", devices").map(|k| k.code),
            Some(KeyCode::Char(','))
        );
        assert_eq!(
            key_for_hint_segment("P").map(|k| k.code),
            Some(KeyCode::Char('P'))
        );
        assert_eq!(
            key_for_hint_segment("R").map(|k| k.code),
            Some(KeyCode::Char('R'))
        );
        let pan = key_for_hint_segment("Alt-← / Alt-→ pan").expect("pan");
        assert_eq!(pan.code, KeyCode::Right);
        assert!(pan.modifiers.contains(KeyModifiers::ALT));
        assert!(key_for_hint_segment("type").is_none());
        assert!(key_for_hint_segment("← →").is_none());
    }

    #[test]
    fn layout_skips_unmapped_and_clips_to_width() {
        let area = Rect::new(0, 23, 20, 1);
        let hits = layout_footer_hits(area, "/ search · a add · ? help");
        assert_eq!(hits.len(), 2); // "/ search" (8) + " · " (3) + "a add" (5) = 16; "? help" needs 9 more → clip
        assert_eq!(hits[0].key.code, KeyCode::Char('/'));
        assert_eq!(hits[0].rect, Rect::new(0, 23, 8, 1));
        assert_eq!(hits[1].key.code, KeyCode::Char('a'));
        assert_eq!(hits[1].rect, Rect::new(11, 23, 5, 1));
    }
}
