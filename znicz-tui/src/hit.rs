use crossterm::event::KeyEvent;
use ratatui::layout::{Position, Rect};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ListHit {
    pub inner: Rect,
    pub offset: usize,
    pub len: usize,
}

impl ListHit {
    pub fn row_at(self, column: u16, row: u16) -> Option<usize> {
        if !self.inner.contains(Position { x: column, y: row }) {
            return None;
        }
        let index = self.offset + usize::from(row.saturating_sub(self.inner.y));
        (index < self.len).then_some(index)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FooterHit {
    pub rect: Rect,
    pub key: KeyEvent,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct HitMap {
    pub library: Option<ListHit>,
    pub library_artists: Option<ListHit>,
    pub library_albums: Option<ListHit>,
    pub library_tracks: Option<ListHit>,
    pub queue: Option<ListHit>,
    pub overlay: Option<Rect>,
    pub overlay_list: Option<ListHit>,
    pub queue_toggle: Option<Rect>,
    pub library_pane: Option<Rect>,
    pub search_prompt: Option<Rect>,
    pub close: Option<Rect>,
    pub footer_hints: Vec<FooterHit>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::layout::Position;

    fn list() -> ListHit {
        ListHit {
            inner: Rect::new(1, 1, 20, 5),
            offset: 2,
            len: 10,
        }
    }

    #[test]
    fn a_click_on_the_first_visible_row_uses_the_offset() {
        assert_eq!(list().row_at(2, 1), Some(2));
        assert_eq!(list().row_at(2, 2), Some(3));
    }

    #[test]
    fn a_click_past_the_last_item_is_ignored() {
        let hit = ListHit {
            inner: Rect::new(1, 1, 20, 8),
            offset: 0,
            len: 2,
        };
        assert_eq!(hit.row_at(2, 3), None);
    }

    #[test]
    fn a_click_outside_the_inner_rect_is_ignored() {
        assert_eq!(list().row_at(0, 1), None);
        assert_eq!(list().row_at(2, 0), None);
        assert_eq!(list().row_at(2, 6), None);
    }

    #[test]
    fn contains_uses_ratatui_position() {
        let rect = Rect::new(10, 5, 4, 3);
        assert!(rect.contains(Position { x: 10, y: 5 }));
        assert!(!rect.contains(Position { x: 14, y: 5 }));
    }

    #[test]
    fn hit_map_default_has_no_close_or_footer_hints() {
        let hits = HitMap::default();
        assert!(hits.close.is_none());
        assert!(hits.footer_hints.is_empty());
    }

    #[test]
    fn footer_hit_stores_rect_and_key() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let hit = FooterHit {
            rect: Rect::new(0, 23, 7, 1),
            key: KeyEvent::new(KeyCode::Char('?'), KeyModifiers::NONE),
        };
        assert_eq!(hit.rect.width, 7);
        assert_eq!(hit.key.code, KeyCode::Char('?'));
    }
}
