//! Small helpers that turn numbers into strings fit for a narrow terminal.

use std::time::Duration;

/// `M:SS`, or `H:MM:SS` for anything an hour or longer.
pub fn duration(d: Duration) -> String {
    let secs = d.as_secs();
    let (h, m, s) = (secs / 3600, (secs % 3600) / 60, secs % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

/// A duration that may be unknown, as it is for radio streams.
pub fn duration_opt(d: Option<Duration>) -> String {
    d.map(duration).unwrap_or_else(|| "--:--".to_string())
}

/// Sample rate in kHz, dropping a pointless trailing `.0`.
pub fn khz(rate: u32) -> String {
    let khz = rate as f64 / 1000.0;
    if (khz.fract() * 10.0).round() == 0.0 {
        format!("{khz:.0} kHz")
    } else {
        format!("{khz:.1} kHz")
    }
}

pub fn channel_name(channels: u16) -> String {
    match channels {
        1 => "mono".to_string(),
        2 => "stereo".to_string(),
        n => format!("{n}ch"),
    }
}

/// A volume meter built from block characters, e.g. `████▁▁▁▁`.
pub fn volume_bar(volume: f32, width: usize) -> String {
    let filled = (volume.clamp(0.0, 1.0) * width as f32).round() as usize;
    let mut bar = String::with_capacity(width * 3);
    for i in 0..width {
        bar.push(if i < filled { '█' } else { '▁' });
    }
    bar
}

/// A seek bar: filled track, a knob at the playhead, then the rest.
///
/// Returns the three pieces separately so each can be styled on its own.
pub fn progress_bar(ratio: f64, width: usize) -> (String, String) {
    if width == 0 {
        return (String::new(), String::new());
    }
    let ratio = ratio.clamp(0.0, 1.0);
    let filled = ((ratio * width as f64).round() as usize).min(width);
    let done = "━".repeat(filled);
    let todo = "─".repeat(width - filled);
    (done, todo)
}

/// Skip `offset` characters, then truncate to `width`.
pub fn pan(text: &str, offset: usize, width: usize) -> String {
    let sliced: String = text.chars().skip(offset).collect();
    truncate(&sliced, width)
}

/// Cut a string to `max` columns, ending with `…` when something was dropped.
///
/// Counts characters rather than bytes so non-ASCII titles are not cut mid-character.
pub fn truncate(text: &str, max: usize) -> String {
    if max == 0 {
        return String::new();
    }
    let count = text.chars().count();
    if count <= max {
        return text.to_string();
    }
    let keep = max.saturating_sub(1);
    let mut out: String = text.chars().take(keep).collect();
    out.push('…');
    out
}

/// `1/12` style position marker for lists.
pub fn position_of(index: usize, total: usize) -> String {
    if total == 0 {
        "0/0".to_string()
    } else {
        format!("{}/{}", index + 1, total)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations_grow_an_hour_field_only_when_needed() {
        assert_eq!(duration(Duration::from_secs(9)), "0:09");
        assert_eq!(duration(Duration::from_secs(75)), "1:15");
        assert_eq!(duration(Duration::from_secs(3661)), "1:01:01");
        assert_eq!(duration_opt(None), "--:--");
    }

    #[test]
    fn sample_rates_read_naturally() {
        assert_eq!(khz(44_100), "44.1 kHz");
        assert_eq!(khz(48_000), "48 kHz");
        assert_eq!(khz(192_000), "192 kHz");
    }

    #[test]
    fn volume_bar_fills_proportionally() {
        assert_eq!(volume_bar(0.0, 4), "▁▁▁▁");
        assert_eq!(volume_bar(1.0, 4), "████");
        assert_eq!(volume_bar(0.5, 4), "██▁▁");
    }

    #[test]
    fn the_seek_bar_always_spans_the_given_width() {
        for permille in 0..=1000 {
            let (done, todo) = progress_bar(permille as f64 / 1000.0, 20);
            assert_eq!(
                done.chars().count() + todo.chars().count(),
                20,
                "bar must not change width as playback advances"
            );
        }
        let (done, _) = progress_bar(1.0, 10);
        assert_eq!(done.chars().count(), 10, "a finished track fills the bar");
        let (done, _) = progress_bar(0.0, 10);
        assert!(done.is_empty(), "a fresh track has nothing filled");
    }

    #[test]
    fn an_out_of_range_ratio_does_not_overflow_the_bar() {
        let (done, todo) = progress_bar(4.2, 8);
        assert_eq!(done.chars().count(), 8);
        assert!(todo.is_empty());
    }

    #[test]
    fn pan_skips_then_truncates() {
        assert_eq!(pan("abcdefghij", 0, 5), "abcd…");
        assert_eq!(pan("abcdefghij", 2, 5), "cdef…");
        assert_eq!(pan("abcdefghij", 8, 5), "ij");
        assert_eq!(pan("short", 0, 10), "short");
    }

    #[test]
    fn truncation_marks_what_it_dropped() {
        assert_eq!(truncate("short", 10), "short");
        assert_eq!(truncate("truncate me", 5), "trun…");
        assert_eq!(truncate("anything", 0), "");
    }

    #[test]
    fn truncation_counts_characters_not_bytes() {
        // Would panic or produce mojibake if this sliced by byte index.
        assert_eq!(truncate("Łódź nocą", 5), "Łódź…");
    }
}
