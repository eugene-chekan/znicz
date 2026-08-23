//! State for browsing the music library.
//!
//! The library lives in SQLite. Album lists and searches are indexed lookups,
//! fast enough to run while handling a keypress, so this pane queries directly
//! and keeps the results until the next navigation step.

use znicz_library::{AlbumSummary, Library, Track};

use crate::cursor::Cursor;

/// How many search hits to keep. Enough to scroll, small enough to stay quick.
const SEARCH_LIMIT: usize = 500;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mode {
    /// Album list: the default way in.
    Albums,
    /// Tracks of one album.
    Album(String),
    /// Results for the last search.
    Search(String),
    /// Every track, for a library whose files have no album tags.
    AllTracks,
}

/// What the cursor is sitting on, so the caller knows what a key should do.
#[derive(Debug, Clone)]
pub enum Item<'a> {
    Album(&'a AlbumSummary),
    Track(&'a Track),
}

pub struct LibraryPane {
    library: Option<Library>,
    mode: Mode,
    albums: Vec<AlbumSummary>,
    tracks: Vec<Track>,
    cursor: Cursor,
    /// Text being typed while the search prompt is open.
    input: Option<String>,
    /// Why the pane is empty, when it is.
    notice: Option<String>,
    h_offset: usize,
}

impl LibraryPane {
    /// Build the pane. Without a library it still renders, explaining how to
    /// create one, rather than disappearing from the interface.
    pub fn new(library: Option<Library>) -> Self {
        let mut pane = Self {
            library,
            mode: Mode::Albums,
            albums: Vec::new(),
            tracks: Vec::new(),
            cursor: Cursor::new(),
            input: None,
            notice: None,
            h_offset: 0,
        };
        pane.reload_albums();
        pane
    }

    pub fn is_available(&self) -> bool {
        self.library.is_some()
    }

    pub fn mode(&self) -> &Mode {
        &self.mode
    }

    pub fn notice(&self) -> Option<&str> {
        self.notice.as_deref()
    }

    pub fn albums(&self) -> &[AlbumSummary] {
        &self.albums
    }

    pub fn tracks(&self) -> &[Track] {
        &self.tracks
    }

    pub fn cursor(&self) -> &Cursor {
        &self.cursor
    }

    /// Number of rows on screen for the current mode.
    pub fn len(&self) -> usize {
        match self.mode {
            Mode::Albums => self.albums.len(),
            _ => self.tracks.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn selected_index(&self) -> Option<usize> {
        self.cursor.selected(self.len())
    }

    pub fn selected(&self) -> Option<Item<'_>> {
        let index = self.selected_index()?;
        match self.mode {
            Mode::Albums => self.albums.get(index).map(Item::Album),
            _ => self.tracks.get(index).map(Item::Track),
        }
    }

    /// Every track the selection stands for: one track, or a whole album.
    pub fn selected_tracks(&self) -> Vec<Track> {
        match self.selected() {
            Some(Item::Track(track)) => vec![track.clone()],
            Some(Item::Album(album)) => self.album_tracks(&album.album),
            None => Vec::new(),
        }
    }

    /// All rows currently listed, for "add everything".
    pub fn listed_tracks(&self) -> Vec<Track> {
        match self.mode {
            Mode::Albums => self
                .albums
                .iter()
                .flat_map(|album| self.album_tracks(&album.album))
                .collect(),
            _ => self.tracks.clone(),
        }
    }

    pub fn step(&mut self, delta: isize) {
        self.cursor.step(delta, self.len());
    }

    pub fn page(&mut self, delta: isize) {
        self.cursor.page(delta, self.len());
    }

    pub fn first(&mut self) {
        self.cursor.first();
    }

    pub fn last(&mut self) {
        self.cursor.last(self.len());
    }

    /// Open the album under the cursor. Returns false when there is nothing to open.
    pub fn enter(&mut self) -> bool {
        let Some(Item::Album(album)) = self.selected() else {
            return false;
        };
        let name = album.album.clone();
        self.tracks = self.album_tracks(&name);
        self.mode = Mode::Album(name);
        self.cursor.first();
        true
    }

    /// Leave an album or a search and return to the top level.
    pub fn back(&mut self) -> bool {
        if matches!(self.mode, Mode::Albums | Mode::AllTracks) {
            return false;
        }
        self.tracks.clear();
        self.cursor.first();
        // Goes back to albums, or to the flat track list when nothing is tagged.
        self.reload_albums();
        true
    }

    pub fn h_offset(&self) -> usize {
        self.h_offset
    }

    pub fn clamp_pan(&mut self, slot: usize) {
        let max = self.longest_middle().saturating_sub(slot);
        self.h_offset = self.h_offset.min(max);
    }

    pub fn pan(&mut self, delta: isize, slot: usize) {
        let max = self.longest_middle().saturating_sub(slot) as isize;
        let next = self.h_offset as isize + delta;
        self.h_offset = next.clamp(0, max.max(0)) as usize;
    }

    pub fn longest_middle(&self) -> usize {
        match self.mode {
            Mode::Albums => self
                .albums
                .iter()
                .map(Self::album_middle)
                .map(|s| s.chars().count())
                .max()
                .unwrap_or(0),
            _ => self
                .tracks
                .iter()
                .map(Self::track_middle)
                .map(|s| s.chars().count())
                .max()
                .unwrap_or(0),
        }
    }

    pub fn album_middle(album: &AlbumSummary) -> String {
        let year = album.year.map(|y| format!(" ({y})")).unwrap_or_default();
        let artist = album.album_artist.as_deref().unwrap_or("Unknown artist");
        format!("{}{year} — {artist}", album.album)
    }

    pub fn track_middle(track: &Track) -> String {
        match track.artist.as_deref() {
            Some(artist) => format!("{} — {artist}", track.title),
            None => track.title.clone(),
        }
    }

    // --- search prompt ---

    pub fn begin_search(&mut self) {
        self.input = Some(String::new());
    }

    pub fn input(&self) -> Option<&str> {
        self.input.as_deref()
    }

    pub fn is_typing(&self) -> bool {
        self.input.is_some()
    }

    pub fn push_char(&mut self, c: char) {
        if let Some(input) = self.input.as_mut() {
            input.push(c);
        }
    }

    pub fn pop_char(&mut self) {
        if let Some(input) = self.input.as_mut() {
            input.pop();
        }
    }

    pub fn cancel_search(&mut self) {
        self.input = None;
    }

    /// Run the typed query. Returns a message describing the outcome.
    pub fn submit_search(&mut self) -> String {
        let Some(query) = self.input.take() else {
            return String::new();
        };
        let query = query.trim().to_string();
        if query.is_empty() {
            return "search cancelled".to_string();
        }

        let Some(library) = self.library.as_ref() else {
            return "no library: run `znicz scan <dir>` first".to_string();
        };

        match library.search(&query, SEARCH_LIMIT) {
            Ok(tracks) => {
                let count = tracks.len();
                self.tracks = tracks;
                self.mode = Mode::Search(query.clone());
                self.cursor.first();
                self.notice = if count == 0 {
                    Some(format!("nothing matched \"{query}\""))
                } else {
                    None
                };
                match count {
                    0 => format!("no match for \"{query}\""),
                    1 => format!("1 match for \"{query}\""),
                    n => format!("{n} matches for \"{query}\""),
                }
            }
            Err(e) => {
                self.notice = Some(format!("search failed: {e}"));
                format!("search failed: {e}")
            }
        }
    }

    /// Re-read the album list, for after a scan.
    pub fn reload_albums(&mut self) {
        let Some(library) = self.library.as_ref() else {
            self.notice = Some("no library yet — run `znicz scan <dir>`".to_string());
            return;
        };

        let albums = match library.albums() {
            Ok(albums) => albums,
            Err(e) => {
                self.notice = Some(format!("could not read library: {e}"));
                return;
            }
        };

        if !albums.is_empty() {
            self.albums = albums;
            self.mode = Mode::Albums;
            self.notice = None;
            self.cursor.clamp(self.albums.len());
            return;
        }

        // No albums. Either the library really is empty, or the files carry no
        // album tag; in the second case list the tracks so they are reachable.
        self.albums.clear();
        match library.all_tracks(SEARCH_LIMIT) {
            Ok(tracks) if !tracks.is_empty() => {
                self.notice = None;
                self.tracks = tracks;
                self.mode = Mode::AllTracks;
                self.cursor.clamp(self.tracks.len());
            }
            Ok(_) => {
                self.mode = Mode::Albums;
                self.notice = Some("library is empty — run `znicz scan <dir>`".to_string());
            }
            Err(e) => self.notice = Some(format!("could not read library: {e}")),
        }
    }

    fn album_tracks(&self, album: &str) -> Vec<Track> {
        self.library
            .as_ref()
            .and_then(|library| library.browse_album(album).ok())
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn without_a_library_the_pane_explains_itself() {
        let pane = LibraryPane::new(None);
        assert!(!pane.is_available());
        assert!(pane.is_empty());
        assert!(
            pane.notice().unwrap().contains("scan"),
            "the notice should tell the user how to build a library"
        );
    }

    #[test]
    fn navigation_on_an_empty_pane_selects_nothing() {
        let mut pane = LibraryPane::new(None);
        pane.step(1);
        pane.last();
        assert!(pane.selected().is_none());
        assert!(pane.selected_tracks().is_empty());
        assert!(!pane.enter(), "there is nothing to open");
        assert!(!pane.back(), "already at the top level");
    }

    #[test]
    fn searching_without_a_library_says_so() {
        let mut pane = LibraryPane::new(None);
        pane.begin_search();
        assert!(pane.is_typing());
        for c in "wish".chars() {
            pane.push_char(c);
        }
        assert_eq!(pane.input(), Some("wish"));

        let message = pane.submit_search();
        assert!(message.contains("no library"), "got: {message}");
        assert!(!pane.is_typing(), "the prompt closes after submitting");
    }

    #[test]
    fn an_empty_query_is_treated_as_a_cancel() {
        let mut pane = LibraryPane::new(None);
        pane.begin_search();
        pane.push_char(' ');
        assert_eq!(pane.submit_search(), "search cancelled");
    }

    #[test]
    fn empty_pane_pan_stays_at_zero() {
        let mut pane = LibraryPane::new(None);
        pane.pan(5, 4);
        assert_eq!(pane.h_offset(), 0);
    }

    #[test]
    fn backspace_edits_the_query() {
        let mut pane = LibraryPane::new(None);
        pane.begin_search();
        for c in "abc".chars() {
            pane.push_char(c);
        }
        pane.pop_char();
        assert_eq!(pane.input(), Some("ab"));

        pane.cancel_search();
        assert!(!pane.is_typing());
    }
}
