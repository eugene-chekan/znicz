//! Short-lived messages shown in the status area.
//!
//! Without these, a failed file or a finished action is invisible: the TUI owns
//! the screen, so anything written to the log or to stdout never reaches the
//! user. Every action and every player error becomes a toast instead.

use std::time::{Duration, Instant};

/// How long a message stays on screen.
const LIFETIME: Duration = Duration::from_secs(4);
/// Errors stay longer, since they usually need reading.
const ERROR_LIFETIME: Duration = Duration::from_secs(8);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Info,
    Warn,
    Error,
}

#[derive(Debug, Clone)]
pub struct Toast {
    pub text: String,
    pub level: Level,
    shown_at: Instant,
    lifetime: Duration,
}

impl Toast {
    fn new(text: String, level: Level, now: Instant) -> Self {
        let lifetime = match level {
            Level::Error => ERROR_LIFETIME,
            _ => LIFETIME,
        };
        Self {
            text,
            level,
            shown_at: now,
            lifetime,
        }
    }

    fn expired(&self, now: Instant) -> bool {
        now.duration_since(self.shown_at) >= self.lifetime
    }
}

/// Holds the most recent messages, newest first.
#[derive(Debug, Default)]
pub struct Toasts {
    items: Vec<Toast>,
}

/// Beyond this, older messages are dropped rather than queueing up unseen.
const MAX_KEPT: usize = 8;

impl Toasts {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn info(&mut self, text: impl Into<String>) {
        self.push(text.into(), Level::Info, Instant::now());
    }

    pub fn warn(&mut self, text: impl Into<String>) {
        self.push(text.into(), Level::Warn, Instant::now());
    }

    pub fn error(&mut self, text: impl Into<String>) {
        self.push(text.into(), Level::Error, Instant::now());
    }

    fn push(&mut self, text: String, level: Level, now: Instant) {
        self.items.insert(0, Toast::new(text, level, now));
        self.items.truncate(MAX_KEPT);
    }

    /// Drop messages that have had their time. Call once per frame.
    pub fn tick(&mut self) {
        self.tick_at(Instant::now());
    }

    fn tick_at(&mut self, now: Instant) {
        self.items.retain(|toast| !toast.expired(now));
    }

    /// The message to display, if any.
    pub fn current(&self) -> Option<&Toast> {
        self.items.first()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Recent messages, newest first, for the notification overlay.
    pub fn recent(&self) -> &[Toast] {
        &self.items
    }

    /// Up to three newest messages for the toast overlay.
    pub fn visible(&self) -> &[Toast] {
        let n = self.recent().len().min(3);
        &self.recent()[..n]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_newest_message_is_the_one_shown() {
        let mut toasts = Toasts::new();
        toasts.info("first");
        toasts.info("second");
        assert_eq!(toasts.current().unwrap().text, "second");
    }

    #[test]
    fn messages_disappear_once_their_time_is_up() {
        let mut toasts = Toasts::new();
        let now = Instant::now();
        toasts.push("hello".into(), Level::Info, now);

        toasts.tick_at(now + Duration::from_secs(1));
        assert!(!toasts.is_empty(), "should still be visible after 1s");

        toasts.tick_at(now + LIFETIME);
        assert!(toasts.is_empty(), "should be gone once the lifetime passes");
    }

    #[test]
    fn errors_outlive_ordinary_messages() {
        let mut toasts = Toasts::new();
        let now = Instant::now();
        toasts.push("broke".into(), Level::Error, now);

        toasts.tick_at(now + LIFETIME + Duration::from_millis(1));
        assert!(!toasts.is_empty(), "errors need longer to read");

        toasts.tick_at(now + ERROR_LIFETIME);
        assert!(toasts.is_empty());
    }

    #[test]
    fn a_flood_of_messages_does_not_grow_without_bound() {
        let mut toasts = Toasts::new();
        for i in 0..50 {
            toasts.info(format!("message {i}"));
        }
        assert_eq!(toasts.recent().len(), MAX_KEPT);
        assert_eq!(toasts.current().unwrap().text, "message 49");
    }
}
