//! Scrolling text that is wider than its slot: hold, scroll, hold, jump back.
//!
//! The offset is a pure function of elapsed time, so it does not drift with
//! how often frames are drawn, and tests can pick any moment.

use std::time::{Duration, Instant};

/// Pause at the start and at the end of each pass.
pub const HOLD: Duration = Duration::from_secs(2);
/// Time per column moved; matches the TUI's redraw tick.
pub const STEP: Duration = Duration::from_millis(200);

/// Characters to skip from the start of a `len`-character text shown in
/// `width` columns, `elapsed` after it first appeared.
pub fn marquee_offset(len: usize, width: usize, elapsed: Duration) -> usize {
    if width == 0 || len <= width {
        return 0;
    }
    let max = len - width;
    let travel = STEP * max as u32;
    let cycle = HOLD + travel + HOLD;
    let t = Duration::from_nanos((elapsed.as_nanos() % cycle.as_nanos()) as u64);
    if t < HOLD {
        0
    } else {
        let moved = ((t - HOLD).as_millis() / STEP.as_millis()) as usize;
        moved.min(max)
    }
}

/// `width` characters of `text` starting at `offset` (no ellipsis).
pub fn window(text: &str, offset: usize, width: usize) -> String {
    text.chars().skip(offset).take(width).collect()
}

/// When the current text first showed at its width.
#[derive(Debug, Default)]
pub struct Marquee {
    shown: Option<(String, usize)>,
    started: Option<Instant>,
}

impl Marquee {
    /// The scroll offset for `text` in `width` columns at `now`. A different
    /// text or width starts a new pass.
    pub fn offset(&mut self, text: &str, width: usize, now: Instant) -> usize {
        let same = self
            .shown
            .as_ref()
            .is_some_and(|(t, w)| t == text && *w == width);
        if !same {
            self.shown = Some((text.to_string(), width));
            self.started = Some(now);
        }
        let elapsed = now.saturating_duration_since(self.started.unwrap_or(now));
        marquee_offset(text.chars().count(), width, elapsed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    #[test]
    fn text_that_fits_never_moves() {
        assert_eq!(marquee_offset(10, 10, ms(9_000)), 0);
        assert_eq!(marquee_offset(3, 10, ms(9_000)), 0);
        assert_eq!(marquee_offset(30, 0, ms(9_000)), 0);
    }

    #[test]
    fn it_holds_then_moves_one_column_per_step() {
        // 15 characters in 10 columns: max offset 5.
        assert_eq!(marquee_offset(15, 10, ms(0)), 0);
        assert_eq!(marquee_offset(15, 10, ms(1_999)), 0);
        assert_eq!(marquee_offset(15, 10, ms(2_000)), 0);
        assert_eq!(marquee_offset(15, 10, ms(2_200)), 1);
        assert_eq!(marquee_offset(15, 10, ms(2_600)), 3);
    }

    #[test]
    fn it_holds_at_the_end_then_jumps_back() {
        // Travel 5 × 200 ms = 1 s; cycle = 2 s + 1 s + 2 s = 5 s.
        assert_eq!(marquee_offset(15, 10, ms(3_000)), 5);
        assert_eq!(marquee_offset(15, 10, ms(4_900)), 5);
        assert_eq!(marquee_offset(15, 10, ms(5_000)), 0);
        assert_eq!(marquee_offset(15, 10, ms(7_200)), 1);
    }

    #[test]
    fn window_takes_a_slice_without_ellipsis() {
        assert_eq!(window("abcdefgh", 2, 4), "cdef");
        assert_eq!(window("żółw", 1, 2), "ół");
    }

    #[test]
    fn a_new_text_or_width_starts_over() {
        let mut m = Marquee::default();
        let t0 = Instant::now();
        let long = "a".repeat(30);
        assert_eq!(m.offset(&long, 10, t0), 0);
        assert_eq!(m.offset(&long, 10, t0 + ms(2_600)), 3);
        assert_eq!(m.offset(&long, 12, t0 + ms(2_600)), 0, "width change");
        let other = "b".repeat(30);
        assert_eq!(m.offset(&other, 12, t0 + ms(9_000)), 0, "text change");
    }
}
