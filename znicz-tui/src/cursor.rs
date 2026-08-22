//! Cursor position for the list panes.
//!
//! Kept apart from drawing so the movement rules can be tested without a
//! terminal. Lists change under the cursor (a scan finishes, a queue entry is
//! removed), so the length is passed in on every call rather than stored.

#[derive(Debug, Default, Clone, Copy)]
pub struct Cursor {
    index: usize,
}

impl Cursor {
    pub fn new() -> Self {
        Self::default()
    }

    /// Where the cursor sits, or `None` when the list is empty.
    pub fn selected(&self, len: usize) -> Option<usize> {
        if len == 0 {
            None
        } else {
            Some(self.index.min(len - 1))
        }
    }

    /// Raw index, for handing to ratatui's list state.
    pub fn index(&self) -> usize {
        self.index
    }

    /// Step by one or more rows, wrapping at both ends.
    ///
    /// Wrapping means holding `j` on a short list keeps cycling instead of
    /// silently stopping, which reads as a frozen UI.
    pub fn step(&mut self, delta: isize, len: usize) {
        if len == 0 {
            self.index = 0;
            return;
        }
        let len_i = len as isize;
        let current = self.index.min(len - 1) as isize;
        self.index = (current + delta).rem_euclid(len_i) as usize;
    }

    /// Jump a screenful, stopping at the ends instead of wrapping.
    pub fn page(&mut self, delta: isize, len: usize) {
        if len == 0 {
            self.index = 0;
            return;
        }
        let current = self.index.min(len - 1) as isize;
        self.index = (current + delta).clamp(0, len as isize - 1) as usize;
    }

    pub fn first(&mut self) {
        self.index = 0;
    }

    pub fn last(&mut self, len: usize) {
        self.index = len.saturating_sub(1);
    }

    /// Put the cursor on a specific row, for example the playing track.
    pub fn set(&mut self, index: usize, len: usize) {
        self.index = if len == 0 { 0 } else { index.min(len - 1) };
    }

    /// Keep the cursor inside a list that just got shorter.
    pub fn clamp(&mut self, len: usize) {
        self.index = if len == 0 { 0 } else { self.index.min(len - 1) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_list_has_nothing_selected() {
        let cursor = Cursor::new();
        assert_eq!(cursor.selected(0), None);
        assert_eq!(cursor.selected(3), Some(0));
    }

    #[test]
    fn stepping_wraps_at_both_ends() {
        let mut cursor = Cursor::new();
        cursor.step(-1, 3);
        assert_eq!(
            cursor.selected(3),
            Some(2),
            "up from the top goes to the end"
        );

        cursor.step(1, 3);
        assert_eq!(
            cursor.selected(3),
            Some(0),
            "down from the end goes to the top"
        );
    }

    #[test]
    fn paging_stops_at_the_ends() {
        let mut cursor = Cursor::new();
        cursor.page(-10, 5);
        assert_eq!(cursor.selected(5), Some(0));

        cursor.page(10, 5);
        assert_eq!(cursor.selected(5), Some(4));
    }

    #[test]
    fn a_shrinking_list_pulls_the_cursor_back_in() {
        let mut cursor = Cursor::new();
        cursor.last(10);
        assert_eq!(cursor.selected(10), Some(9));

        cursor.clamp(4);
        assert_eq!(cursor.selected(4), Some(3));

        cursor.clamp(0);
        assert_eq!(cursor.selected(0), None);
    }

    #[test]
    fn stepping_on_an_empty_list_is_harmless() {
        let mut cursor = Cursor::new();
        cursor.step(1, 0);
        cursor.page(5, 0);
        cursor.last(0);
        assert_eq!(cursor.selected(0), None);
    }

    #[test]
    fn setting_beyond_the_end_lands_on_the_last_row() {
        let mut cursor = Cursor::new();
        cursor.set(99, 3);
        assert_eq!(cursor.selected(3), Some(2));
    }
}
