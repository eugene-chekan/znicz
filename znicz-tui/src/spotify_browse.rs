//! The Spotify browser as a pure drill-down state machine.
//!
//! No IO here. The App sends the requests this returns, tags each with the
//! generation it was sent at, and feeds the replies back. A reply from an
//! earlier generation is dropped, so a list the user has left never grows.

use znicz_core::{SpotifyItem, SpotifyKind, SpotifyOp, SpotifyPage};

/// Rows per list request.
pub const PAGE: u32 = 50;
/// Fetch the next page when the cursor is this close to the end.
pub const NEAR_END: usize = 5;
/// Rows per search kind (Spotify's Development Mode limit).
pub const SEARCH_LIMIT: u32 = 10;
/// The top level, in order. `Search` opens the prompt; the others push a list.
pub const TOP: [&str; 4] = ["Search", "Playlists", "Liked songs", "Albums"];
/// The search groups, in the order they are drawn.
pub const SEARCH_GROUPS: [(SpotifyKind, &str); 4] = [
    (SpotifyKind::Track, "Tracks"),
    (SpotifyKind::Album, "Albums"),
    (SpotifyKind::Artist, "Artists"),
    (SpotifyKind::Playlist, "Playlists"),
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BrowseList {
    Playlists,
    LikedSongs,
    Albums,
    /// An album's or playlist's tracks.
    Tracks {
        uri: String,
    },
    /// An artist's albums.
    ArtistAlbums {
        uri: String,
    },
    /// One kind of one search, at most `SEARCH_LIMIT` rows.
    Search {
        query: String,
        kind: SpotifyKind,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Row {
    /// A search group heading; the cursor skips it.
    Header(&'static str),
    /// Boxed: a `Row` is one of many, and the header is far smaller than an item.
    Item(Box<SpotifyItem>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Level {
    /// The list this level shows. `None` for the top level and for a search.
    pub list: Option<BrowseList>,
    /// The breadcrumb part: `Playlists`, the album name, `Search: query`.
    pub title: String,
    /// Items of a list. Empty for a search, which keeps `groups` instead.
    pub items: Vec<SpotifyItem>,
    /// Search only: the four groups, in `SEARCH_GROUPS` order. Empty otherwise.
    pub groups: Vec<Vec<SpotifyItem>>,
    pub total: u32,
    /// Requests still on the way for this level.
    pub pending: u32,
    /// An index into `rows()`. Always on an item after a change (see `settle`).
    pub cursor: usize,
    /// Shown under the list (a playlist the user may not list, an error).
    pub notice: Option<String>,
}

/// A selectable row: its search group (`None` for a list), its index in that
/// list, and the item.
type Located = (Option<usize>, usize, SpotifyItem);

impl Level {
    fn new(list: Option<BrowseList>, title: String) -> Self {
        Self {
            list,
            title,
            items: Vec::new(),
            groups: Vec::new(),
            total: 0,
            pending: 0,
            cursor: 0,
            notice: None,
        }
    }

    pub fn is_search(&self) -> bool {
        !self.groups.is_empty()
    }

    /// The rows as drawn: headers and items, in order. Search groups with no
    /// results show no header.
    pub fn rows(&self) -> Vec<Row> {
        if !self.is_search() {
            return self
                .items
                .iter()
                .cloned()
                .map(|item| Row::Item(Box::new(item)))
                .collect();
        }
        let mut rows = Vec::new();
        for ((_, label), items) in SEARCH_GROUPS.iter().zip(&self.groups) {
            if items.is_empty() {
                continue;
            }
            rows.push(Row::Header(label));
            rows.extend(items.iter().cloned().map(|item| Row::Item(Box::new(item))));
        }
        rows
    }

    /// The cursor can land here (an item, not a header).
    pub fn selectable(&self, index: usize) -> bool {
        self.locate(index).is_some()
    }

    fn selectable_rows(&self) -> Vec<usize> {
        (0..self.rows().len())
            .filter(|&index| self.selectable(index))
            .collect()
    }

    /// The item at a row index, or `None` for a header or past the end.
    fn locate(&self, row: usize) -> Option<Located> {
        if !self.is_search() {
            return self.items.get(row).map(|item| (None, row, item.clone()));
        }
        let mut position = 0;
        for (group, items) in self.groups.iter().enumerate() {
            if items.is_empty() {
                continue;
            }
            position += 1; // the group's header
            for (index, item) in items.iter().enumerate() {
                if position == row {
                    return Some((Some(group), index, item.clone()));
                }
                position += 1;
            }
        }
        None
    }
}

/// What Enter or `l` does at the cursor, decided without IO.
#[derive(Debug, Clone, PartialEq)]
pub enum BrowseAction {
    /// Start playback.
    Play(SpotifyOp),
    /// Push a level; the request is `op` at generation `generation`.
    Load {
        generation: u64,
        list: BrowseList,
        title: String,
        op: SpotifyOp,
    },
    /// Open the search prompt (the Search row of the top level).
    OpenSearch,
    /// Nothing to do (a header, an empty list).
    Nothing,
}

#[derive(Debug, Clone)]
pub struct SpotifyBrowser {
    levels: Vec<Level>,
    generation: u64,
}

impl Default for SpotifyBrowser {
    fn default() -> Self {
        Self::new()
    }
}

impl SpotifyBrowser {
    /// A browser at the top level.
    pub fn new() -> Self {
        Self {
            levels: vec![Level::new(None, String::new())],
            generation: 0,
        }
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Back to the top level; bumps the generation so replies in flight are dropped.
    pub fn reset(&mut self) {
        self.generation += 1;
        self.levels = vec![Level::new(None, String::new())];
    }

    pub fn depth(&self) -> usize {
        self.levels.len()
    }

    pub fn level(&self) -> &Level {
        self.levels
            .last()
            .expect("a browser always has its top level")
    }

    fn current(&mut self) -> &mut Level {
        self.levels
            .last_mut()
            .expect("a browser always has its top level")
    }

    fn is_top(&self) -> bool {
        self.levels.len() == 1
    }

    /// Breadcrumb parts, top first. The top level has no part.
    pub fn crumbs(&self) -> Vec<&str> {
        self.levels
            .iter()
            .skip(1)
            .map(|level| level.title.as_str())
            .collect()
    }

    /// Move the cursor by `delta` selectable rows, clamped to the list.
    pub fn step(&mut self, delta: isize) {
        if self.is_top() {
            let cursor = &mut self.current().cursor;
            *cursor = (*cursor as isize + delta).clamp(0, TOP.len() as isize - 1) as usize;
            return;
        }
        let level = self.current();
        let selectable = level.selectable_rows();
        if selectable.is_empty() {
            return;
        }
        let position = selectable
            .iter()
            .position(|&index| index == level.cursor)
            .unwrap_or(0);
        let target = (position as isize + delta).clamp(0, selectable.len() as isize - 1) as usize;
        level.cursor = selectable[target];
    }

    pub fn first(&mut self) {
        self.jump(false);
    }

    pub fn last(&mut self) {
        self.jump(true);
    }

    fn jump(&mut self, to_end: bool) {
        if self.is_top() {
            self.current().cursor = if to_end { TOP.len() - 1 } else { 0 };
            return;
        }
        let level = self.current();
        let selectable = level.selectable_rows();
        let picked = if to_end {
            selectable.last()
        } else {
            selectable.first()
        };
        level.cursor = picked.copied().unwrap_or(0);
    }

    /// Enter at the cursor: play an album or playlist from the top, open an
    /// artist, play a track in its context.
    pub fn enter(&mut self) -> BrowseAction {
        self.activate(false)
    }

    /// `l` at the cursor: as `enter`, except an album or playlist opens its tracks.
    pub fn open(&mut self) -> BrowseAction {
        self.activate(true)
    }

    fn activate(&mut self, open: bool) -> BrowseAction {
        if self.is_top() {
            return match self.level().cursor {
                0 => BrowseAction::OpenSearch,
                1 => self.push(BrowseList::Playlists, TOP[1].to_string()),
                2 => self.push(BrowseList::LikedSongs, TOP[2].to_string()),
                _ => self.push(BrowseList::Albums, TOP[3].to_string()),
            };
        }
        let level = self.level();
        let Some((group, index, item)) = level.locate(level.cursor) else {
            return BrowseAction::Nothing;
        };
        let context = match &level.list {
            Some(BrowseList::Tracks { uri }) => Some(uri.clone()),
            _ => None,
        };
        let siblings = match group {
            Some(group) => level.groups[group].clone(),
            None => level.items.clone(),
        };
        match item.kind {
            SpotifyKind::Playlist | SpotifyKind::Album if open => self.push(
                BrowseList::Tracks {
                    uri: item.uri.clone(),
                },
                item.name,
            ),
            SpotifyKind::Playlist | SpotifyKind::Album => BrowseAction::Play(SpotifyOp::Play {
                context_uri: Some(item.uri),
                uris: Vec::new(),
                offset: None,
            }),
            SpotifyKind::Artist => self.push(
                BrowseList::ArtistAlbums {
                    uri: item.uri.clone(),
                },
                item.name,
            ),
            SpotifyKind::Track => {
                let offset = Some(index as u32);
                BrowseAction::Play(match context {
                    Some(uri) => SpotifyOp::Play {
                        context_uri: Some(uri),
                        uris: Vec::new(),
                        offset,
                    },
                    None => SpotifyOp::Play {
                        context_uri: None,
                        uris: siblings.into_iter().map(|track| track.uri).collect(),
                        offset,
                    },
                })
            }
        }
    }

    fn push(&mut self, list: BrowseList, title: String) -> BrowseAction {
        self.generation += 1;
        let op = op_for(&list, 0);
        let mut level = Level::new(Some(list.clone()), title.clone());
        level.pending = 1;
        self.levels.push(level);
        BrowseAction::Load {
            generation: self.generation,
            list,
            title,
            op,
        }
    }

    /// `h`: pop one level. False at the top.
    pub fn back(&mut self) -> bool {
        if self.is_top() {
            return false;
        }
        self.levels.pop();
        self.generation += 1;
        // The revealed level's requests were dropped with their generation.
        self.current().pending = 0;
        self.settle();
        true
    }

    /// Start a search for `query` (trimmed). Pushes the Search level and returns
    /// one `(generation, list, op)` per group. Empty query: returns nothing and changes nothing.
    pub fn start_search(&mut self, query: &str) -> Vec<(u64, BrowseList, SpotifyOp)> {
        let query = query.trim();
        if query.is_empty() {
            return Vec::new();
        }
        self.generation += 1;
        let mut level = Level::new(None, format!("Search: {query}"));
        level.groups = vec![Vec::new(); SEARCH_GROUPS.len()];
        level.pending = SEARCH_GROUPS.len() as u32;
        self.levels.push(level);
        let generation = self.generation;
        SEARCH_GROUPS
            .iter()
            .map(|(kind, _)| {
                let list = BrowseList::Search {
                    query: query.to_string(),
                    kind: *kind,
                };
                let op = op_for(&list, 0);
                (generation, list, op)
            })
            .collect()
    }

    /// Whether a reply for `list` fits the level on screen.
    fn accepts(&self, list: &BrowseList) -> bool {
        !self.is_top() && self.level().is_search() == matches!(list, BrowseList::Search { .. })
    }

    /// A page reply. False (and nothing changes) when the generation is not the current one.
    pub fn apply_page(
        &mut self,
        generation: u64,
        list: &BrowseList,
        _offset: u32,
        page: &SpotifyPage,
    ) -> bool {
        if generation != self.generation || !self.accepts(list) {
            return false;
        }
        let level = self.current();
        match list {
            BrowseList::Search { kind, .. } => {
                if let Some(group) = search_group(*kind) {
                    level.groups[group].extend(page.items.iter().cloned());
                }
            }
            _ => level.items.extend(page.items.iter().cloned()),
        }
        level.total = page.total;
        level.pending = level.pending.saturating_sub(1);
        self.settle();
        true
    }

    /// An error reply for a list, shown as the level's notice. Same generation rule.
    pub fn apply_error(&mut self, generation: u64, list: &BrowseList, message: String) -> bool {
        if !self.apply_failure(generation, list) {
            return false;
        }
        self.current().notice = Some(message);
        true
    }

    /// A failed reply that the caller reports itself (a toast). Same generation
    /// rule; the level's request is no longer pending.
    pub fn apply_failure(&mut self, generation: u64, list: &BrowseList) -> bool {
        if generation != self.generation || !self.accepts(list) {
            return false;
        }
        let level = self.current();
        level.pending = level.pending.saturating_sub(1);
        self.settle();
        true
    }

    /// `r`: request the current list again from its first page, clearing its
    /// rows. None at the top and for a search (which is not reloaded).
    pub fn reload(&mut self) -> Option<(u64, BrowseList, SpotifyOp)> {
        if self.is_top() || self.level().is_search() {
            return None;
        }
        let list = self.level().list.clone()?;
        self.generation += 1;
        let generation = self.generation;
        let level = self.current();
        level.items.clear();
        level.total = 0;
        level.notice = None;
        level.cursor = 0;
        level.pending = 1;
        Some((generation, list.clone(), op_for(&list, 0)))
    }

    /// The next page to request when the cursor is near the end, the level is
    /// not loading and more rows exist. Search never pages.
    pub fn next_page(&mut self) -> Option<(u64, BrowseList, SpotifyOp)> {
        if self.is_top() {
            return None;
        }
        let generation = self.generation;
        let level = self.current();
        if level.is_search() || level.pending > 0 {
            return None;
        }
        let list = level.list.clone()?;
        let loaded = level.items.len();
        if loaded >= level.total as usize || level.cursor + NEAR_END < level.rows().len() {
            return None;
        }
        level.pending = 1;
        Some((generation, list.clone(), op_for(&list, loaded as u32)))
    }

    /// Put the cursor on `row` (an index into `rows()`, or into `TOP` at the top)
    /// when that row can take it. A header or a row past the end does not.
    pub fn select_row(&mut self, row: usize) -> bool {
        if self.is_top() {
            if row >= TOP.len() {
                return false;
            }
            self.current().cursor = row;
            return true;
        }
        let level = self.current();
        if !level.selectable(row) {
            return false;
        }
        level.cursor = row;
        true
    }

    /// Keep the cursor on a selectable row after the rows changed.
    fn settle(&mut self) {
        let top = self.is_top();
        let level = self.current();
        if top {
            level.cursor = level.cursor.min(TOP.len() - 1);
            return;
        }
        let cursor = level.cursor;
        let selectable = level.selectable_rows();
        level.cursor = selectable
            .iter()
            .copied()
            .find(|&index| index >= cursor)
            .or_else(|| selectable.last().copied())
            .unwrap_or(0);
    }
}

/// The offset a list op asks for; 0 for any other op.
pub fn offset_of(op: &SpotifyOp) -> u32 {
    match op {
        SpotifyOp::Playlists { offset, .. }
        | SpotifyOp::LikedSongs { offset, .. }
        | SpotifyOp::Albums { offset, .. }
        | SpotifyOp::Tracks { offset, .. }
        | SpotifyOp::ArtistAlbums { offset, .. }
        | SpotifyOp::Search { offset, .. } => *offset,
        _ => 0,
    }
}

fn search_group(kind: SpotifyKind) -> Option<usize> {
    SEARCH_GROUPS.iter().position(|(group, _)| *group == kind)
}

/// The op for one page of a list.
pub fn op_for(list: &BrowseList, offset: u32) -> SpotifyOp {
    match list {
        BrowseList::Playlists => SpotifyOp::Playlists {
            offset,
            limit: PAGE,
        },
        BrowseList::LikedSongs => SpotifyOp::LikedSongs {
            offset,
            limit: PAGE,
        },
        BrowseList::Albums => SpotifyOp::Albums {
            offset,
            limit: PAGE,
        },
        BrowseList::Tracks { uri } => SpotifyOp::Tracks {
            uri: uri.clone(),
            offset,
            limit: PAGE,
        },
        BrowseList::ArtistAlbums { uri } => SpotifyOp::ArtistAlbums {
            uri: uri.clone(),
            offset,
            limit: PAGE,
        },
        BrowseList::Search { query, kind } => SpotifyOp::Search {
            query: query.clone(),
            kinds: vec![*kind],
            offset,
            limit: SEARCH_LIMIT,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(kind: SpotifyKind, n: u32) -> SpotifyItem {
        SpotifyItem {
            kind,
            uri: format!("spotify:{}:{n}", kind.as_api()),
            name: format!("{} {n}", kind.as_api()),
            subtitle: "Artist".into(),
            image_url: None,
            track: None,
        }
    }

    fn row(item: SpotifyItem) -> Row {
        Row::Item(Box::new(item))
    }

    fn track(n: u32) -> SpotifyItem {
        item(SpotifyKind::Track, n)
    }

    fn album_a() -> SpotifyItem {
        SpotifyItem {
            kind: SpotifyKind::Album,
            uri: "spotify:album:a".into(),
            name: "A".into(),
            subtitle: "Artist".into(),
            image_url: None,
            track: None,
        }
    }

    fn page_of(items: Vec<SpotifyItem>, offset: u32, total: u32) -> SpotifyPage {
        SpotifyPage {
            items,
            offset,
            total,
        }
    }

    fn search(kind: SpotifyKind) -> BrowseList {
        BrowseList::Search {
            query: "q".into(),
            kind,
        }
    }

    /// Moves to top row `row` (1 Playlists, 2 Liked songs, 3 Albums) and enters it.
    fn enter_top(b: &mut SpotifyBrowser, row: isize) -> (u64, BrowseList) {
        b.first();
        b.step(row);
        match b.enter() {
            BrowseAction::Load {
                generation, list, ..
            } => (generation, list),
            other => panic!("expected a load, got {other:?}"),
        }
    }

    #[test]
    fn the_top_level_lists_four_entries_and_search_opens_the_prompt() {
        let mut b = SpotifyBrowser::new();
        assert_eq!(b.level().rows().len(), 0); // top rows are drawn from TOP, not Row::Item
        assert_eq!(b.level().cursor, 0);
        assert_eq!(b.enter(), BrowseAction::OpenSearch);
        b.step(1);
        assert!(matches!(
            b.enter(),
            BrowseAction::Load {
                list: BrowseList::Playlists,
                ..
            }
        ));
    }

    #[test]
    fn entering_a_list_asks_for_page_zero_at_the_new_generation() {
        let mut b = SpotifyBrowser::new();
        b.step(1); // Playlists
        let before = b.generation();
        match b.enter() {
            BrowseAction::Load { generation, op, .. } => {
                assert!(generation > before);
                assert_eq!(
                    op,
                    SpotifyOp::Playlists {
                        offset: 0,
                        limit: PAGE
                    }
                );
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_page_from_an_old_generation_is_dropped() {
        let mut b = SpotifyBrowser::new();
        let (old, list) = enter_top(&mut b, 1);
        b.reset();
        assert!(!b.apply_page(old, &list, 0, &page_of(vec![track(1)], 0, 1)));
        assert!(!b.apply_error(old, &list, "late".into()));
        assert_eq!(b.depth(), 1);
    }

    #[test]
    fn paging_is_asked_for_only_near_the_end() {
        let mut b = SpotifyBrowser::new();
        let (generation, list) = enter_top(&mut b, 1);
        let items: Vec<_> = (0..50).map(|n| item(SpotifyKind::Playlist, n)).collect();
        assert!(b.apply_page(generation, &list, 0, &page_of(items, 0, 120)));
        b.first();
        assert!(b.next_page().is_none(), "far from the end");
        b.last(); // cursor 49, rows 50: within NEAR_END
        let (_, _, op) = b.next_page().expect("near the end");
        assert_eq!(
            op,
            SpotifyOp::Playlists {
                offset: 50,
                limit: PAGE
            }
        );
        assert!(b.next_page().is_none(), "one request at a time");
    }

    #[test]
    fn paging_stops_when_the_total_is_loaded() {
        let mut b = SpotifyBrowser::new();
        let (generation, list) = enter_top(&mut b, 1);
        let items: Vec<_> = (0..3).map(|n| item(SpotifyKind::Playlist, n)).collect();
        assert!(b.apply_page(generation, &list, 0, &page_of(items, 0, 3)));
        b.last();
        assert!(b.next_page().is_none(), "total 3, loaded 3");
    }

    #[test]
    fn a_failed_page_frees_the_level_to_page_again() {
        let mut b = SpotifyBrowser::new();
        let (generation, list) = enter_top(&mut b, 1);
        let items: Vec<_> = (0..50).map(|n| item(SpotifyKind::Playlist, n)).collect();
        assert!(b.apply_page(generation, &list, 0, &page_of(items, 0, 120)));
        b.last();
        assert!(b.next_page().is_some());
        assert!(b.apply_error(generation, &list, "network".into()));
        assert!(
            b.next_page().is_some(),
            "the failed page can be asked again"
        );
    }

    #[test]
    fn a_track_in_an_album_plays_with_the_album_as_context_and_its_index() {
        let mut b = SpotifyBrowser::new();
        let (generation, list) = enter_top(&mut b, 3); // Albums
        assert!(b.apply_page(generation, &list, 0, &page_of(vec![album_a()], 0, 1)));
        b.first();
        let BrowseAction::Load {
            generation: tracks_gen,
            list: tracks,
            ..
        } = b.open()
        else {
            panic!("open an album")
        };
        assert!(b.apply_page(
            tracks_gen,
            &tracks,
            0,
            &page_of(vec![track(1), track(2), track(3)], 0, 3)
        ));
        b.first();
        b.step(1);
        assert_eq!(
            b.enter(),
            BrowseAction::Play(SpotifyOp::Play {
                context_uri: Some("spotify:album:a".into()),
                uris: vec![],
                offset: Some(1),
            })
        );
    }

    #[test]
    fn a_search_track_plays_with_its_group_as_uris() {
        let mut b = SpotifyBrowser::new();
        let (generation, list, _) = b.start_search("x").remove(0); // the Tracks group
        assert!(b.apply_page(
            generation,
            &list,
            0,
            &page_of(vec![track(1), track(2)], 0, 2)
        ));
        b.step(1); // the first track is the cursor after settle; step to the second
        assert_eq!(
            b.enter(),
            BrowseAction::Play(SpotifyOp::Play {
                context_uri: None,
                uris: vec!["spotify:track:1".into(), "spotify:track:2".into()],
                offset: Some(1),
            })
        );
    }

    #[test]
    fn liked_songs_play_the_loaded_tracks_from_the_cursor() {
        let mut b = SpotifyBrowser::new();
        let (generation, list) = enter_top(&mut b, 2);
        assert_eq!(list, BrowseList::LikedSongs);
        assert!(b.apply_page(
            generation,
            &list,
            0,
            &page_of(vec![track(1), track(2), track(3)], 0, 3)
        ));
        b.last(); // row 2
        assert_eq!(
            b.enter(),
            BrowseAction::Play(SpotifyOp::Play {
                context_uri: None,
                uris: vec![
                    "spotify:track:1".into(),
                    "spotify:track:2".into(),
                    "spotify:track:3".into()
                ],
                offset: Some(2),
            })
        );
    }

    #[test]
    fn an_album_from_the_list_plays_from_the_top_but_open_shows_its_tracks() {
        let mut b = SpotifyBrowser::new();
        let (generation, list) = enter_top(&mut b, 3); // Albums
        assert!(b.apply_page(generation, &list, 0, &page_of(vec![album_a()], 0, 1)));
        b.first();
        assert_eq!(
            b.enter(),
            BrowseAction::Play(SpotifyOp::Play {
                context_uri: Some("spotify:album:a".into()),
                uris: vec![],
                offset: None,
            })
        );
        assert_eq!(b.depth(), 2, "enter does not push a level");
        match b.open() {
            BrowseAction::Load { list, op, .. } => {
                assert_eq!(
                    list,
                    BrowseList::Tracks {
                        uri: "spotify:album:a".into()
                    }
                );
                assert_eq!(
                    op,
                    SpotifyOp::Tracks {
                        uri: "spotify:album:a".into(),
                        offset: 0,
                        limit: PAGE,
                    }
                );
            }
            other => panic!("expected a load, got {other:?}"),
        }
    }

    #[test]
    fn an_unavailable_playlist_shows_a_notice_and_the_level_keeps_its_rows_empty() {
        let mut b = SpotifyBrowser::new();
        let (generation, list) = enter_top(&mut b, 1);
        assert!(b.apply_page(
            generation,
            &list,
            0,
            &page_of(vec![item(SpotifyKind::Playlist, 1)], 0, 1)
        ));
        b.first();
        let BrowseAction::Load {
            generation: tracks_gen,
            list: tracks,
            ..
        } = b.open()
        else {
            panic!("open a playlist")
        };
        let message = "Spotify only lists tracks for playlists you own or collaborate on.";
        assert!(b.apply_error(tracks_gen, &tracks, message.into()));
        assert_eq!(b.level().notice.as_deref(), Some(message));
        assert!(b.level().items.is_empty());
    }

    #[test]
    fn search_results_are_grouped_in_order_with_headers() {
        let mut b = SpotifyBrowser::new();
        let (generation, _, _) = b.start_search("q").remove(0);
        // Replies arrive out of order; the Artists group stays empty.
        assert!(b.apply_page(
            generation,
            &search(SpotifyKind::Playlist),
            0,
            &page_of(vec![item(SpotifyKind::Playlist, 1)], 0, 1)
        ));
        assert!(b.apply_page(
            generation,
            &search(SpotifyKind::Album),
            0,
            &page_of(vec![item(SpotifyKind::Album, 1)], 0, 1)
        ));
        assert!(b.apply_page(
            generation,
            &search(SpotifyKind::Track),
            0,
            &page_of(vec![track(1), track(2)], 0, 2)
        ));
        assert!(b.apply_page(
            generation,
            &search(SpotifyKind::Artist),
            0,
            &page_of(vec![], 0, 0)
        ));
        assert_eq!(
            b.level().rows(),
            vec![
                Row::Header("Tracks"),
                row(track(1)),
                row(track(2)),
                Row::Header("Albums"),
                row(item(SpotifyKind::Album, 1)),
                Row::Header("Playlists"),
                row(item(SpotifyKind::Playlist, 1)),
            ]
        );
    }

    #[test]
    fn the_cursor_skips_headers() {
        let mut b = SpotifyBrowser::new();
        let (generation, _, _) = b.start_search("q").remove(0);
        assert!(b.apply_page(
            generation,
            &search(SpotifyKind::Track),
            0,
            &page_of(vec![track(1), track(2)], 0, 2)
        ));
        assert!(b.apply_page(
            generation,
            &search(SpotifyKind::Album),
            0,
            &page_of(vec![item(SpotifyKind::Album, 1)], 0, 1)
        ));
        // Rows: 0 header, 1 track 1, 2 track 2, 3 header, 4 album 1.
        assert_eq!(b.level().cursor, 1, "settles on the first item");
        b.step(1);
        assert_eq!(b.level().cursor, 2);
        b.step(1);
        assert_eq!(b.level().cursor, 4, "skips the Albums header");
        b.step(-1);
        assert_eq!(b.level().cursor, 2);
    }

    #[test]
    fn an_empty_search_sends_nothing() {
        let mut b = SpotifyBrowser::new();
        assert!(b.start_search("   ").is_empty());
        assert_eq!(b.depth(), 1);
    }

    #[test]
    fn back_pops_one_level_and_bumps_the_generation() {
        let mut b = SpotifyBrowser::new();
        let (generation, _) = enter_top(&mut b, 1);
        assert_eq!(b.depth(), 2);
        assert!(b.back());
        assert_eq!(b.depth(), 1);
        assert!(b.generation() > generation);
        assert!(!b.back(), "nothing above the top");
    }

    #[test]
    fn back_reveals_a_level_whose_page_was_dropped_and_can_load_again() {
        let mut b = SpotifyBrowser::new();
        let (generation, list) = enter_top(&mut b, 1);
        let items: Vec<_> = (0..50).map(|n| item(SpotifyKind::Playlist, n)).collect();
        assert!(b.apply_page(generation, &list, 0, &page_of(items, 0, 120)));
        b.last();
        assert!(b.next_page().is_some(), "page two is on its way");
        assert!(
            matches!(b.open(), BrowseAction::Load { .. }),
            "open a playlist"
        );
        assert!(b.back());
        assert!(
            b.next_page().is_some(),
            "page two was dropped, so ask again"
        );
    }

    #[test]
    fn select_row_moves_the_cursor_to_an_item_and_refuses_a_header() {
        let mut b = SpotifyBrowser::new();
        let requests = b.start_search("so what");
        let (generation, list, _) = requests[0].clone();
        b.apply_page(
            generation,
            &list,
            0,
            &page_of(vec![track(1), track(2)], 0, 2),
        );
        // Rows: header Tracks, then the two tracks.
        assert!(!b.select_row(0), "a header cannot take the cursor");
        assert_eq!(b.level().cursor, 1);
        assert!(b.select_row(2));
        assert_eq!(b.level().cursor, 2);
    }

    #[test]
    fn seek_and_step_clamp_to_the_list() {
        let mut b = SpotifyBrowser::new();
        b.step(-5);
        assert_eq!(b.level().cursor, 0);
        let (generation, list) = enter_top(&mut b, 1);
        let items: Vec<_> = (0..3).map(|n| item(SpotifyKind::Playlist, n)).collect();
        assert!(b.apply_page(generation, &list, 0, &page_of(items, 0, 3)));
        b.last();
        assert_eq!(b.level().cursor, 2);
        b.step(-5);
        assert_eq!(b.level().cursor, 0);
        b.step(9);
        assert_eq!(b.level().cursor, 2);
    }
}
