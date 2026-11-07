//! What the help overlay remembers while it is open.

use crate::keys::Context;
use crate::line_edit::LineEdit;

/// Lines moved by Ctrl-d / Ctrl-u and the page keys inside help.
pub const HELP_PAGE: isize = 10;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HelpState {
    /// Where help was opened from; its keys are listed first.
    pub context: Context,
    /// First visible line. Clamped to the content when drawn.
    pub scroll: usize,
    pub query: LineEdit,
    /// Typing goes into `query` instead of scrolling.
    pub editing: bool,
}

impl HelpState {
    pub fn open(context: Context) -> Self {
        Self {
            context,
            scroll: 0,
            query: LineEdit::new(),
            editing: false,
        }
    }

    pub fn scroll_by(&mut self, delta: isize) {
        self.scroll = if delta < 0 {
            self.scroll.saturating_sub(delta.unsigned_abs())
        } else {
            self.scroll.saturating_add(delta.unsigned_abs())
        };
    }

    /// Keep the last page full: never scroll past `content - visible`.
    pub fn clamp_scroll(&mut self, content: usize, visible: usize) {
        self.scroll = self.scroll.min(content.saturating_sub(visible));
    }

    /// The filter row shows while typing, and while a kept filter applies.
    pub fn shows_prompt(&self) -> bool {
        self.editing || !self.query.is_empty()
    }
}

impl Default for HelpState {
    fn default() -> Self {
        Self::open(Context::Library)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scrolling_up_stops_at_the_top() {
        let mut help = HelpState::default();
        help.scroll_by(-3);
        assert_eq!(help.scroll, 0);
        help.scroll_by(4);
        help.scroll_by(-1);
        assert_eq!(help.scroll, 3);
    }

    #[test]
    fn clamping_keeps_the_last_page_full() {
        let mut help = HelpState {
            scroll: usize::MAX,
            ..HelpState::default()
        };
        help.clamp_scroll(50, 20);
        assert_eq!(help.scroll, 30);
        help.clamp_scroll(10, 20);
        assert_eq!(help.scroll, 0);
    }

    #[test]
    fn the_prompt_shows_while_editing_or_filtered() {
        let mut help = HelpState::open(Context::Queue);
        assert!(!help.shows_prompt());
        help.editing = true;
        assert!(help.shows_prompt());
        help.editing = false;
        help.query = LineEdit::from_text("vol");
        assert!(help.shows_prompt());
    }
}
