//! Small helpers that turn numbers into strings fit for a narrow terminal.

use std::time::Duration;

use crate::tui_config::IconSet;

/// `1 track`, `1,240 tracks`.
pub fn tracks_label(n: usize) -> String {
    let noun = if n == 1 { "track" } else { "tracks" };
    format!("{} {noun}", crate::scan_status::grouped(n))
}

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
    d.map(duration).unwrap_or_else(|| "—".to_string())
}

/// Columns `duration_opt(d)` takes, computed without building the string.
pub fn duration_width(d: Option<Duration>) -> usize {
    // Mirrors `duration`: `M:SS`, or `H:MM:SS` from an hour up. Unknown is "—".
    fn digits(n: u64) -> usize {
        if n == 0 { 1 } else { n.ilog10() as usize + 1 }
    }
    let Some(d) = d else {
        return 1;
    };
    let secs = d.as_secs();
    let (h, m) = (secs / 3600, (secs % 3600) / 60);
    if h > 0 { digits(h) + 6 } else { digits(m) + 3 }
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

/// A volume bar `cells` wide with eighth-cell resolution: full blocks, at
/// most one partial block, then low blocks for the rest.
pub fn volume_bar(volume: f32, cells: usize) -> String {
    const PARTIAL: [char; 7] = ['▏', '▎', '▍', '▌', '▋', '▊', '▉'];
    let eighths = (volume.clamp(0.0, 1.0) * (cells * 8) as f32).round() as usize;
    let full = eighths / 8;
    let rest = eighths % 8;
    let mut bar = String::with_capacity(cells * 3);
    for _ in 0..full {
        bar.push('█');
    }
    if full < cells {
        bar.push(if rest > 0 { PARTIAL[rest - 1] } else { '▁' });
        for _ in full + 1..cells {
            bar.push('▁');
        }
    }
    bar
}

/// The marker before the volume bar: level-dependent, or a mute sign.
pub fn volume_icon(icons: IconSet, volume: f32, muted: bool) -> &'static str {
    let percent = (volume.clamp(0.0, 1.0) * 100.0).round() as u32;
    let level = match percent {
        0..=33 => 0,
        34..=66 => 1,
        _ => 2,
    };
    match (icons, muted) {
        (IconSet::Unicode, true) => "×",
        (IconSet::Unicode, false) => "◢",
        (IconSet::Emoji, true) => "🔇",
        (IconSet::Emoji, false) => ["🔈", "🔉", "🔊"][level],
        (IconSet::Nerd, true) => "\u{f075f}",
        (IconSet::Nerd, false) => ["\u{f057f}", "\u{f0580}", "\u{f057e}"][level],
    }
}

/// Terminal cells a string takes (emoji are two).
pub fn display_width(text: &str) -> usize {
    unicode_width::UnicodeWidthStr::width(text)
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
    fn tracks_are_singular_for_one_and_grouped_otherwise() {
        assert_eq!(tracks_label(0), "0 tracks");
        assert_eq!(tracks_label(1), "1 track");
        assert_eq!(tracks_label(2), "2 tracks");
        assert_eq!(tracks_label(1240), "1,240 tracks");
    }

    #[test]
    fn the_volume_icon_follows_the_level_and_mute() {
        use crate::tui_config::IconSet::*;
        assert_eq!(volume_icon(Unicode, 0.1, false), "◢");
        assert_eq!(volume_icon(Unicode, 0.9, false), "◢");
        assert_eq!(volume_icon(Unicode, 0.9, true), "×");
        assert_eq!(volume_icon(Emoji, 0.33, false), "🔈");
        assert_eq!(volume_icon(Emoji, 0.34, false), "🔉");
        assert_eq!(volume_icon(Emoji, 0.66, false), "🔉");
        assert_eq!(volume_icon(Emoji, 0.67, false), "🔊");
        assert_eq!(volume_icon(Emoji, 0.5, true), "🔇");
        assert_eq!(volume_icon(Nerd, 0.0, false), "\u{f057f}");
        assert_eq!(volume_icon(Nerd, 0.5, false), "\u{f0580}");
        assert_eq!(volume_icon(Nerd, 1.0, false), "\u{f057e}");
        assert_eq!(volume_icon(Nerd, 1.0, true), "\u{f075f}");
    }

    #[test]
    fn display_width_counts_emoji_as_two_cells() {
        assert_eq!(display_width("◢ ███"), 5);
        assert_eq!(display_width("🔊 ███"), 6);
    }

    #[test]
    fn durations_grow_an_hour_field_only_when_needed() {
        assert_eq!(duration(Duration::from_secs(9)), "0:09");
        assert_eq!(duration(Duration::from_secs(75)), "1:15");
        assert_eq!(duration(Duration::from_secs(3661)), "1:01:01");
        assert_eq!(duration_opt(None), "—");
    }

    #[test]
    fn duration_width_matches_the_formatted_text() {
        let secs = |s| Some(Duration::from_secs(s));
        for d in [
            None,
            secs(0),
            secs(59),
            secs(60),
            secs(3599),
            secs(3600),
            secs(36_000),
            secs(360_000),
        ] {
            assert_eq!(duration_width(d), duration_opt(d).chars().count(), "{d:?}");
        }
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
    fn the_volume_bar_has_eighth_cell_resolution() {
        assert_eq!(volume_bar(0.0, 6), "▁▁▁▁▁▁");
        assert_eq!(volume_bar(1.0, 6), "██████");
        assert_eq!(volume_bar(0.5, 6), "███▁▁▁");
        assert_eq!(volume_bar(1.0 / 8.0, 1), "▏");
        assert_eq!(volume_bar(3.0 / 8.0, 1), "▍");
        assert_eq!(volume_bar(0.52, 6), "███▏▁▁", "25 of 48 eighths");
        for v in [0.0, 0.01, 0.33, 0.77, 0.99, 1.0] {
            assert_eq!(volume_bar(v, 10).chars().count(), 10, "{v}");
        }
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
