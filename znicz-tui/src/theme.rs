//! One place for every colour and style in the interface.
//!
//! Views ask for a named style instead of picking colours themselves, so the
//! whole player stays consistent and a different palette is a one-file change.

use ratatui::style::{Color, Modifier, Style};

/// Terminals reuse the user's own palette for the 16 base colours, so naming
/// them (instead of using RGB) keeps Znicz looking at home in any theme.
pub const ACCENT: Color = Color::Cyan;
pub const ACCENT_ALT: Color = Color::Magenta;
pub const GOOD: Color = Color::Green;
pub const WARN: Color = Color::Yellow;
pub const BAD: Color = Color::Red;
pub const MUTED: Color = Color::DarkGray;
pub const TEXT: Color = Color::Gray;
pub const STRONG: Color = Color::White;

pub fn title() -> Style {
    Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)
}

pub fn subtitle() -> Style {
    Style::default().fg(TEXT)
}

pub fn dim() -> Style {
    Style::default().fg(MUTED)
}

pub fn text() -> Style {
    Style::default().fg(TEXT)
}

pub fn strong() -> Style {
    Style::default().fg(STRONG).add_modifier(Modifier::BOLD)
}

/// Border of the pane that has keyboard focus.
pub fn border_active() -> Style {
    Style::default().fg(ACCENT)
}

pub fn border_idle() -> Style {
    Style::default().fg(MUTED)
}

/// The row under the cursor in a list.
pub fn selected() -> Style {
    Style::default()
        .bg(Color::DarkGray)
        .fg(STRONG)
        .add_modifier(Modifier::BOLD)
}

/// The queue entry that is currently playing.
pub fn playing() -> Style {
    Style::default().fg(GOOD).add_modifier(Modifier::BOLD)
}

pub fn progress() -> Style {
    Style::default().fg(ACCENT)
}

pub fn progress_track() -> Style {
    Style::default().fg(MUTED)
}

pub fn good() -> Style {
    Style::default().fg(GOOD).add_modifier(Modifier::BOLD)
}

pub fn warn() -> Style {
    Style::default().fg(WARN)
}

pub fn info() -> Style {
    Style::default()
        .fg(Color::Blue)
        .add_modifier(Modifier::BOLD)
}

pub fn bad() -> Style {
    Style::default().fg(BAD).add_modifier(Modifier::BOLD)
}

/// Highlight for an enabled toggle such as shuffle or repeat.
pub fn toggle_on() -> Style {
    Style::default().fg(ACCENT_ALT).add_modifier(Modifier::BOLD)
}

pub fn toggle_off() -> Style {
    Style::default().fg(MUTED)
}

/// Key names inside hint lines and the help overlay.
pub fn key() -> Style {
    Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)
}
