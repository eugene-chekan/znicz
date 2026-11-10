//! Reading `Artist - Title` out of an ICY StreamTitle, and loose name matching.

use unicode_normalization::char::is_combining_mark;
use unicode_normalization::UnicodeNormalization;

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

/// Lowercase, accents removed, whitespace collapsed: a key for loose matching.
pub fn fold(text: &str) -> String {
    let plain: String = text
        .nfkd()
        .filter(|c| !is_combining_mark(*c))
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
    fn artists_match_loosely() {
        assert!(artist_matches("bjork", "Björk"));
        assert!(artist_matches("Ben Webster", "Ben Webster Quartet"));
        assert!(artist_matches("Ben Webster Quartet", "Ben Webster"));
        assert!(!artist_matches("Serotonal", "Stereo Total"));
        assert!(!artist_matches("", "Anyone"));
    }
}
