//! Reading `Artist - Title` out of an ICY StreamTitle, and loose name matching.

use unicode_normalization::UnicodeNormalization;
use unicode_normalization::char::is_combining_mark;

use crate::cover::StreamQuery;

const SEPARATORS: [&str; 2] = [" - ", " – "];

/// Split an ICY title into `(artist, title)` on the first ` - ` or ` – `.
///
/// `None` when there is no separator, either side is empty, or the whole
/// title is just the station name.
pub fn split_icy_title<'a>(title: &'a str, station_name: &str) -> Option<(&'a str, &'a str)> {
    let title = title.trim();
    if fold(title) == fold(station_name) {
        return None;
    }
    let (at, len) = SEPARATORS
        .iter()
        .filter_map(|sep| title.find(sep).map(|at| (at, sep.len())))
        .min_by_key(|(at, _)| *at)?;
    let artist = title[..at].trim();
    let song = title[at + len..].trim();
    (!artist.is_empty() && !song.is_empty()).then_some((artist, song))
}

/// Lowercase, accents removed, apostrophe look-alikes made `'`, whitespace
/// collapsed: a key for loose matching. ICY titles are quoted with `'`, so
/// streams such as AudioAddict's send a backtick instead (`90`s Love`).
pub fn fold(text: &str) -> String {
    let plain: String = text
        .nfkd()
        .filter(|c| !is_combining_mark(*c))
        .map(|c| match c {
            '`' | '\u{2018}' | '\u{2019}' => '\'',
            other => other,
        })
        .flat_map(char::to_lowercase)
        .collect();
    plain.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Whether a search hit's artist is the artist we asked for: folded, and
/// either name contains the other (`Ben Webster` / `Ben Webster Quartet`).
pub fn artist_matches(wanted: &str, found: &str) -> bool {
    let (wanted, found) = (fold(wanted), fold(found));
    !wanted.is_empty() && !found.is_empty() && (wanted.contains(&found) || found.contains(&wanted))
}

/// `(artist, title)` when search is allowed and the ICY title splits.
pub(crate) fn searchable(query: &StreamQuery) -> Option<(&str, &str)> {
    if !query.search {
        return None;
    }
    split_icy_title(query.icy_title.as_deref()?, &query.station_name)
}

/// `artist|title`, folded: the cache key shared by the search sources.
pub(crate) fn search_key(query: &StreamQuery) -> Option<String> {
    let (artist, title) = searchable(query)?;
    Some(format!("{}|{}", fold(artist), fold(title)))
}

/// What joins several artists in one credit (`A & B`, `A feat. B`, …).
const CREDIT_JOINERS: [&str; 10] = [
    " & ",
    " and ",
    " with ",
    " feat. ",
    " feat ",
    " ft. ",
    " featuring ",
    ", ",
    " x ",
    " + ",
];

/// The first artist of a multi-artist credit: the text up to the earliest
/// joiner, case-insensitive. A credit without joiners is returned whole.
pub fn lead_artist(credit: &str) -> &str {
    // ASCII lowercasing keeps byte offsets, so positions map back to `credit`.
    let lower = credit.to_ascii_lowercase();
    CREDIT_JOINERS
        .iter()
        .filter_map(|joiner| lower.find(joiner))
        .min()
        .map_or(credit, |at| credit[..at].trim())
}

/// Artist texts to search with, in order: the full credit, then its lead
/// artist when that differs.
fn artist_attempts(credit: &str) -> Vec<&str> {
    let lead = lead_artist(credit);
    if lead.is_empty() || lead == credit {
        vec![credit]
    } else {
        vec![credit, lead]
    }
}

/// The title without trailing bracketed notes: `Lover Man (Ramirez)` becomes
/// `Lover Man`. Stations append composers, versions and remaster years that
/// the search services do not have in the track name.
pub fn bare_title(title: &str) -> &str {
    let mut bare = title.trim_end();
    loop {
        let open = match bare.chars().last() {
            Some(')') => '(',
            Some(']') => '[',
            _ => return bare,
        };
        let Some(at) = bare.rfind(open) else {
            return bare;
        };
        let rest = bare[..at].trim_end();
        if rest.is_empty() {
            return bare;
        }
        bare = rest;
    }
}

/// `(artist, title)` pairs to search with, in order: as sent, with the bare
/// title, then the same with the lead artist. Repeats are dropped.
pub(crate) fn search_attempts<'a>(credit: &'a str, title: &'a str) -> Vec<(&'a str, &'a str)> {
    let mut attempts = Vec::new();
    for artist in artist_attempts(credit) {
        for song in [title, bare_title(title)] {
            if !attempts.contains(&(artist, song)) {
                attempts.push((artist, song));
            }
        }
    }
    attempts
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_artist_and_title() {
        assert_eq!(
            split_icy_title("Ben Webster - Willow Weep For Me", "FluxFM"),
            Some(("Ben Webster", "Willow Weep For Me"))
        );
        assert_eq!(
            split_icy_title("Serotonal – Memory Leaves", "HEAVY-MUSIC.NET"),
            Some(("Serotonal", "Memory Leaves"))
        );
    }

    #[test]
    fn the_first_separator_wins() {
        assert_eq!(split_icy_title("A - B - C", "X"), Some(("A", "B - C")));
    }

    #[test]
    fn titles_that_are_not_artist_and_title_do_not_split() {
        assert_eq!(split_icy_title("Live from Berlin", "X"), None);
        assert_eq!(split_icy_title(" - Song", "X"), None);
        assert_eq!(split_icy_title("Artist - ", "X"), None);
        assert_eq!(split_icy_title("Radio X - Live", "radio x - live"), None);
    }

    #[test]
    fn fold_ignores_case_accents_and_spacing() {
        assert_eq!(fold("  Björk   Guðmundsdóttir "), "bjork guðmundsdottir");
        assert_eq!(fold("SIGUR RÓS"), "sigur ros");
    }

    #[test]
    fn fold_treats_backticks_and_curly_quotes_as_apostrophes() {
        // ICY titles are quoted with `'`, so AudioAddict streams send `90`s Love`.
        assert_eq!(fold("90`s Love"), fold("90's Love"));
        assert_eq!(fold("Don\u{2019}t Stop"), "don't stop");
        assert_eq!(fold("\u{2018}Til Tuesday"), "'til tuesday");
    }

    #[test]
    fn artists_match_loosely() {
        assert!(artist_matches("bjork", "Björk"));
        assert!(artist_matches("Ben Webster", "Ben Webster Quartet"));
        assert!(artist_matches("Ben Webster Quartet", "Ben Webster"));
        assert!(!artist_matches("Serotonal", "Stereo Total"));
        assert!(!artist_matches("", "Anyone"));
    }

    #[test]
    fn the_lead_artist_is_the_credit_up_to_the_first_joiner() {
        assert_eq!(
            lead_artist("Anna Maria Jopek & Friends with Pat Metheny"),
            "Anna Maria Jopek"
        );
        assert_eq!(
            lead_artist("Nils Landgren With Esbjörn Svensson"),
            "Nils Landgren"
        );
        assert_eq!(
            lead_artist("Lizz Wright FEAT. Gregory Porter"),
            "Lizz Wright"
        );
        assert_eq!(lead_artist("A ft. B"), "A");
        assert_eq!(lead_artist("A featuring B"), "A");
        assert_eq!(lead_artist("A feat B"), "A");
        assert_eq!(lead_artist("Chick Corea and Gary Burton"), "Chick Corea");
        assert_eq!(lead_artist("Trio X, Guest"), "Trio X");
        assert_eq!(lead_artist("A x B"), "A");
        assert_eq!(lead_artist("A + B"), "A");
        assert_eq!(lead_artist("Ben Webster"), "Ben Webster");
    }

    #[test]
    fn attempts_are_the_credit_then_the_lead_artist() {
        assert_eq!(
            artist_attempts("Anna Maria Jopek & Friends with Pat Metheny"),
            vec![
                "Anna Maria Jopek & Friends with Pat Metheny",
                "Anna Maria Jopek"
            ]
        );
        assert_eq!(artist_attempts("Ben Webster"), vec!["Ben Webster"]);
    }

    #[test]
    fn the_bare_title_drops_trailing_bracketed_notes() {
        assert_eq!(bare_title("Lover Man (Ramirez)"), "Lover Man");
        assert_eq!(bare_title("So What (Live) [2011 Remaster]"), "So What");
        assert_eq!(bare_title("Willow Weep For Me"), "Willow Weep For Me");
        // Brackets inside the title stay.
        assert_eq!(
            bare_title("(I Can't Get No) Satisfaction"),
            "(I Can't Get No) Satisfaction"
        );
        // A title that is only a note is kept whole.
        assert_eq!(bare_title("(Untitled)"), "(Untitled)");
    }

    #[test]
    fn attempts_try_the_bare_title_then_the_lead_artist() {
        assert_eq!(
            search_attempts("Hans Koller", "Lover Man (Ramirez)"),
            vec![
                ("Hans Koller", "Lover Man (Ramirez)"),
                ("Hans Koller", "Lover Man"),
            ]
        );
        assert_eq!(
            search_attempts("A & B", "Song (Live)"),
            vec![
                ("A & B", "Song (Live)"),
                ("A & B", "Song"),
                ("A", "Song (Live)"),
                ("A", "Song")
            ]
        );
        assert_eq!(
            search_attempts("Ben Webster", "Willow Weep For Me"),
            vec![("Ben Webster", "Willow Weep For Me")]
        );
    }
}
