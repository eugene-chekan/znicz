//! One-line text with a caret.
//!
//! Every TUI prompt (library search, playlist save/rename/copy, radio name+URL)
//! shares this buffer so Left/Right/Home/End/Backspace/Delete behave the same.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineEdit {
    text: String,
    /// Caret as a count of Unicode scalar values from the start.
    cursor: usize,
}

impl LineEdit {
    pub fn new() -> Self {
        Self {
            text: String::new(),
            cursor: 0,
        }
    }

    pub fn from_text(text: impl Into<String>) -> Self {
        let text = text.into();
        let cursor = text.chars().count();
        Self { text, cursor }
    }

    pub fn as_str(&self) -> &str {
        &self.text
    }

    pub fn insert(&mut self, c: char) {
        let i = self.byte_index();
        self.text.insert(i, c);
        self.cursor += 1;
    }

    pub fn backspace(&mut self) {
        if self.cursor == 0 {
            return;
        }
        self.cursor -= 1;
        self.remove_char_at_cursor();
    }

    pub fn delete(&mut self) {
        if self.cursor >= self.text.chars().count() {
            return;
        }
        self.remove_char_at_cursor();
    }

    pub fn left(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
    }

    pub fn right(&mut self) {
        let len = self.text.chars().count();
        if self.cursor < len {
            self.cursor += 1;
        }
    }

    pub fn home(&mut self) {
        self.cursor = 0;
    }

    pub fn end(&mut self) {
        self.cursor = self.text.chars().count();
    }

    /// Apply a key that edits the line. Esc and Enter are left to the caller.
    ///
    /// Returns whether the key changed the buffer or the caret.
    pub fn on_key(&mut self, key: KeyEvent) -> bool {
        if key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
        {
            return false;
        }
        match key.code {
            KeyCode::Left => self.left(),
            KeyCode::Right => self.right(),
            KeyCode::Home => self.home(),
            KeyCode::End => self.end(),
            KeyCode::Backspace => self.backspace(),
            KeyCode::Delete => self.delete(),
            KeyCode::Char(c) => self.insert(c),
            _ => return false,
        }
        true
    }

    /// Text before and after the caret, for drawing `█` in the middle.
    pub fn split_at_cursor(&self) -> (&str, &str) {
        self.text.split_at(self.byte_index())
    }

    fn byte_index(&self) -> usize {
        self.text
            .chars()
            .take(self.cursor)
            .map(|c| c.len_utf8())
            .sum()
    }

    fn remove_char_at_cursor(&mut self) {
        let i = self.byte_index();
        let n = self.text[i..]
            .chars()
            .next()
            .map(|c| c.len_utf8())
            .unwrap_or(0);
        self.text.drain(i..i + n);
    }
}

impl Default for LineEdit {
    fn default() -> Self {
        Self::new()
    }
}

impl AsRef<str> for LineEdit {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl std::ops::Deref for LineEdit {
    type Target = str;

    fn deref(&self) -> &str {
        self.as_str()
    }
}

impl PartialEq<str> for LineEdit {
    fn eq(&self, other: &str) -> bool {
        self.text == other
    }
}

impl PartialEq<&str> for LineEdit {
    fn eq(&self, other: &&str) -> bool {
        self.text == *other
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_in_the_middle_fixes_a_typo() {
        let mut edit = LineEdit::from_text("Exmple");
        edit.left();
        edit.left();
        edit.left();
        edit.left();
        edit.insert('a');
        assert_eq!(edit.as_str(), "Example");
    }

    #[test]
    fn backspace_deletes_before_the_caret() {
        let mut edit = LineEdit::from_text("Example");
        edit.home();
        edit.right();
        edit.right();
        edit.right();
        edit.backspace();
        assert_eq!(edit.as_str(), "Exmple");
    }

    #[test]
    fn delete_removes_the_char_under_the_caret() {
        let mut edit = LineEdit::from_text("Exxample");
        edit.home();
        edit.right();
        edit.right();
        edit.delete();
        assert_eq!(edit.as_str(), "Example");
    }

    #[test]
    fn left_stops_at_the_start() {
        let mut edit = LineEdit::from_text("ab");
        edit.home();
        edit.left();
        edit.insert('x');
        assert_eq!(edit.as_str(), "xab");
    }

    #[test]
    fn multibyte_chars_do_not_panic() {
        let mut edit = LineEdit::from_text("żubr");
        edit.home();
        edit.right();
        edit.backspace();
        edit.insert('Z');
        assert_eq!(edit.as_str(), "Zubr");
    }

    #[test]
    fn on_key_inserts_after_moving_left() {
        let mut edit = LineEdit::from_text("sogs");
        edit.on_key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
        edit.on_key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
        edit.on_key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE));
        assert_eq!(edit.as_str(), "songs");
    }
}
