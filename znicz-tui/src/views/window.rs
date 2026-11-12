//! Which rows of a list are on screen, so only those need building.

use std::ops::Range;

/// The rows a list shows, by ratatui's `List` scrolling rule (no scroll
/// padding): keep `offset` until the selection would leave the screen.
pub fn visible_window(
    len: usize,
    selected: Option<usize>,
    offset: usize,
    height: usize,
) -> Range<usize> {
    if len == 0 || height == 0 {
        let start = offset.min(len);
        return start..start;
    }
    let mut start = offset.min(len - 1);
    if let Some(selected) = selected.map(|s| s.min(len - 1)) {
        if selected < start {
            start = selected;
        } else if selected >= start + height {
            start = selected + 1 - height;
        }
    }
    start..(start + height).min(len)
}

#[cfg(test)]
mod tests {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::widgets::{List, ListItem, ListState};

    use super::*;

    #[test]
    fn an_empty_list_shows_nothing() {
        assert_eq!(visible_window(0, None, 0, 10), 0..0);
        assert_eq!(visible_window(0, Some(3), 5, 10), 0..0);
    }

    #[test]
    fn zero_height_shows_nothing() {
        assert_eq!(visible_window(50, Some(20), 10, 0), 10..10);
    }

    #[test]
    fn without_a_selection_the_offset_is_kept() {
        assert_eq!(visible_window(50, None, 10, 5), 10..15);
    }

    #[test]
    fn without_a_selection_an_offset_past_the_end_is_clamped() {
        assert_eq!(visible_window(50, None, 99, 5), 49..50);
    }

    #[test]
    fn a_selection_above_the_window_pulls_it_up() {
        assert_eq!(visible_window(50, Some(3), 10, 5), 3..8);
    }

    #[test]
    fn a_selection_below_the_window_puts_it_on_the_last_row() {
        assert_eq!(visible_window(50, Some(20), 0, 5), 16..21);
    }

    #[test]
    fn a_selection_inside_the_window_keeps_the_offset() {
        assert_eq!(visible_window(50, Some(12), 10, 5), 10..15);
    }

    #[test]
    fn an_offset_past_the_end_is_clamped_to_the_last_row() {
        assert_eq!(visible_window(10, Some(9), 40, 4), 9..10);
    }

    #[test]
    fn a_short_list_shows_all_of_itself() {
        assert_eq!(visible_window(3, Some(1), 0, 10), 0..3);
    }

    /// Start row ratatui settles on after rendering the same list.
    fn ratatui_offset(len: usize, selected: Option<usize>, offset: usize, height: usize) -> usize {
        let items: Vec<ListItem> = (0..len).map(|i| ListItem::new(i.to_string())).collect();
        let mut state = ListState::default();
        // `select(None)` resets the offset, so select first.
        state.select(selected);
        *state.offset_mut() = offset;
        let mut terminal = Terminal::new(TestBackend::new(8, height as u16)).unwrap();
        terminal
            .draw(|frame| {
                let area = frame.area();
                frame.render_stateful_widget(List::new(items.clone()), area, &mut state);
            })
            .unwrap();
        state.offset()
    }

    #[test]
    fn the_window_matches_what_ratatui_scrolls_to() {
        let mut checked = 0;
        for len in [1, 2, 3, 7, 20] {
            for height in [1, 2, 4, 7] {
                for selected in std::iter::once(None).chain((0..len).map(Some)) {
                    for offset in 0..=len + 1 {
                        let expected = ratatui_offset(len, selected, offset, height);
                        let got = visible_window(len, selected, offset, height).start;
                        assert_eq!(
                            got, expected,
                            "len {len}, selected {selected:?}, offset {offset}, height {height}"
                        );
                        checked += 1;
                    }
                }
            }
        }
        assert!(checked > 1000);
    }
}
