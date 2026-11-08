//! Messages: live ones for the transport row, and a history for the Messages list.
//!
//! Without these, a failed file or a finished action is invisible: the TUI owns
//! the screen, so anything written to the log or to stdout never reaches the
//! user. Every action and every player error becomes a toast instead.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// How long a message stays on screen.
const LIFETIME: Duration = Duration::from_secs(4);
/// Errors stay longer, since they usually need reading.
const ERROR_LIFETIME: Duration = Duration::from_secs(8);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Info,
    Success,
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

/// Live messages (newest first) and the history behind them.
#[derive(Debug, Default)]
pub struct Toasts {
    items: Vec<Toast>,
    history: VecDeque<Entry>,
    /// The Messages list is open: new alerts are seen as they arrive.
    viewing: bool,
}

/// Beyond this, older messages are dropped rather than queueing up unseen.
const MAX_KEPT: usize = 8;
/// How many messages the Messages list remembers.
const HISTORY: usize = 50;

/// One remembered message.
#[derive(Debug, Clone)]
pub struct Entry {
    pub text: String,
    pub level: Level,
    pub at: Instant,
    /// A warning or error the user has not seen in the Messages list yet.
    pub unread: bool,
    /// Was unread when the Messages list opened; drawn bold until it closes.
    pub fresh: bool,
}

impl Toasts {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn info(&mut self, text: impl Into<String>) {
        self.push(text.into(), Level::Info, Instant::now());
    }

    pub fn success(&mut self, text: impl Into<String>) {
        self.push(text.into(), Level::Success, Instant::now());
    }

    pub fn warn(&mut self, text: impl Into<String>) {
        self.push(text.into(), Level::Warn, Instant::now());
    }

    pub fn error(&mut self, text: impl Into<String>) {
        self.push(text.into(), Level::Error, Instant::now());
    }

    fn push(&mut self, text: String, level: Level, now: Instant) {
        self.history.push_front(Entry {
            text: text.clone(),
            level,
            at: now,
            unread: matches!(level, Level::Warn | Level::Error) && !self.viewing,
            fresh: false,
        });
        self.history.truncate(HISTORY);
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

    /// Live messages behind the newest one, for the `+N` on the message row.
    pub fn live_extra(&self) -> usize {
        self.items.len().saturating_sub(1)
    }

    /// Remembered messages, newest first.
    pub fn history(&self) -> impl Iterator<Item = &Entry> {
        self.history.iter()
    }

    pub fn history_len(&self) -> usize {
        self.history.len()
    }

    /// Unread warnings and errors: how many, and the worst level.
    pub fn unread_alerts(&self) -> Option<(usize, Level)> {
        let mut count = 0;
        let mut worst = Level::Warn;
        for entry in self.history.iter().filter(|e| e.unread) {
            count += 1;
            if entry.level == Level::Error {
                worst = Level::Error;
            }
        }
        (count > 0).then_some((count, worst))
    }

    /// The Messages list opened: unread rows become fresh, nothing stays unread.
    pub fn mark_read(&mut self) {
        self.viewing = true;
        for entry in &mut self.history {
            entry.fresh = entry.unread;
            entry.unread = false;
        }
    }

    /// The Messages list closed.
    pub fn clear_fresh(&mut self) {
        self.viewing = false;
        for entry in &mut self.history {
            entry.fresh = false;
        }
    }

    pub fn clear_history(&mut self) {
        self.history.clear();
    }

    /// Expire every live message now, keeping the history.
    #[doc(hidden)]
    pub fn expire_live_for_test(&mut self) {
        self.items.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alerts_arriving_while_the_list_is_open_count_as_read() {
        let mut toasts = Toasts::new();
        toasts.mark_read();
        toasts.error("late");
        toasts.clear_fresh();
        assert_eq!(toasts.unread_alerts(), None);
    }

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

    #[test]
    fn success_is_its_own_level() {
        let mut toasts = Toasts::new();
        toasts.success("queue cleared");
        let toast = toasts.current().unwrap();
        assert_eq!(toast.level, Level::Success);
        assert_eq!(toast.text, "queue cleared");
    }

    #[test]
    fn history_keeps_fifty_messages_after_they_expire() {
        let mut toasts = Toasts::new();
        let now = Instant::now();
        for i in 0..60 {
            toasts.push(format!("m{i}"), Level::Info, now);
        }
        toasts.tick_at(now + ERROR_LIFETIME);
        assert!(toasts.is_empty(), "live messages expired");
        assert_eq!(toasts.history_len(), HISTORY);
        assert_eq!(toasts.history().next().unwrap().text, "m59", "newest first");
    }

    #[test]
    fn only_warnings_and_errors_are_unread() {
        let mut toasts = Toasts::new();
        toasts.info("added 4 tracks");
        toasts.success("queue cleared");
        assert_eq!(toasts.unread_alerts(), None);
        toasts.warn("stream dropped");
        assert_eq!(toasts.unread_alerts(), Some((1, Level::Warn)));
        toasts.error("device refused 96 kHz");
        assert_eq!(
            toasts.unread_alerts(),
            Some((2, Level::Error)),
            "error is worst"
        );
    }

    #[test]
    fn marking_read_keeps_the_rows_fresh_until_cleared() {
        let mut toasts = Toasts::new();
        toasts.warn("stream dropped");
        toasts.info("added");
        toasts.mark_read();
        assert_eq!(toasts.unread_alerts(), None);
        let fresh: Vec<bool> = toasts.history().map(|e| e.fresh).collect();
        assert_eq!(fresh, [false, true], "newest first: info, then the warning");
        toasts.clear_fresh();
        assert!(toasts.history().all(|e| !e.fresh));
    }

    #[test]
    fn clearing_history_leaves_live_messages() {
        let mut toasts = Toasts::new();
        toasts.error("broke");
        toasts.clear_history();
        assert_eq!(toasts.history_len(), 0);
        assert_eq!(toasts.unread_alerts(), None);
        assert!(!toasts.is_empty());
    }

    #[test]
    fn live_extra_counts_messages_behind_the_newest() {
        let mut toasts = Toasts::new();
        assert_eq!(toasts.live_extra(), 0);
        toasts.info("a");
        assert_eq!(toasts.live_extra(), 0);
        toasts.info("b");
        toasts.info("c");
        assert_eq!(toasts.live_extra(), 2);
    }
}
