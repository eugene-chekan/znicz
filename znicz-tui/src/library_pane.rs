//! State for browsing the music library.
//!
//! Artist-first hierarchy (artist → album → track) with three presentations:
//! columns when wide enough, expandable tree, or single-column paging.

use std::collections::{HashMap, HashSet};

use znicz_library::{AlbumSummary, ArtistSummary, Library, SearchHit, SearchLimits, Track};

use crate::cursor::Cursor;
use crate::layout;
use crate::line_edit::LineEdit;
use crate::playing::PlayingRef;
use crate::tui_config::LibraryLayout;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColumnFocus {
    Artists,
    Albums,
    Tracks,
}

impl ColumnFocus {
    pub fn next(self) -> Self {
        match self {
            Self::Artists => Self::Albums,
            Self::Albums => Self::Tracks,
            Self::Tracks => Self::Artists,
        }
    }

    pub fn prev(self) -> Self {
        match self {
            Self::Artists => Self::Tracks,
            Self::Albums => Self::Artists,
            Self::Tracks => Self::Albums,
        }
    }
}

/// How browse is painted for the current width and preference.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EffectiveLayout {
    Columns,
    Tree,
    Paging,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mode {
    /// Artist → album → track browse.
    Browse,
    /// Results for the last search.
    Search(String),
    /// Every track, for a library whose files have no album tags.
    AllTracks,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum TreeNodeId {
    Artist(String),
    Album { artist: String, album: String },
}

/// One visible row in the flattened tree.
#[derive(Debug, Clone, PartialEq)]
pub enum TreeRow {
    Artist {
        artist: ArtistSummary,
        expanded: bool,
    },
    Album {
        artist: String,
        album: AlbumSummary,
        expanded: bool,
        depth: usize,
    },
    Track {
        album_key: String,
        index: usize,
        depth: usize,
    },
}

impl TreeRow {
    /// Nesting level: artists 0, albums 1, tracks 2.
    fn depth(&self) -> usize {
        match self {
            Self::Artist { .. } => 0,
            Self::Album { depth, .. } | Self::Track { depth, .. } => *depth,
        }
    }
}

/// What the cursor is sitting on, so the caller knows what a key should do.
#[derive(Debug, Clone)]
pub enum Item<'a> {
    Artist(&'a ArtistSummary),
    Album(&'a AlbumSummary),
    Track(&'a Track),
}

pub struct LibraryPane {
    library: Option<Library>,
    mode: Mode,
    artists: Vec<ArtistSummary>,
    albums: Vec<AlbumSummary>,
    tracks: Vec<Track>,
    search_hits: Vec<SearchHit>,
    artist_cursor: Cursor,
    album_cursor: Cursor,
    track_cursor: Cursor,
    /// Single-list cursor for Search / AllTracks / legacy inject helpers.
    list_cursor: Cursor,
    column_focus: ColumnFocus,
    paging_level: ColumnFocus,
    expanded: HashSet<TreeNodeId>,
    /// Albums of each expanded artist, keyed by case-folded artist name.
    /// Tree rows read these so a redraw does not re-scan the library.
    tree_albums_by_artist: HashMap<String, Vec<AlbumSummary>>,
    tree_cursor: Cursor,
    /// Tracks loaded for expanded albums (keyed by album name, case-folded).
    tree_track_cache: std::collections::HashMap<String, Vec<Track>>,
    /// Album summaries for expanded artists (keyed by album name, case-folded).
    tree_album_cache: std::collections::HashMap<String, AlbumSummary>,
    preferred_layout: LibraryLayout,
    /// Text being typed while the search prompt is open.
    input: Option<LineEdit>,
    /// Why the pane is empty, when it is.
    notice: Option<String>,
    h_offset: usize,
}

impl LibraryPane {
    pub fn new(library: Option<Library>) -> Self {
        let mut pane = Self {
            library,
            mode: Mode::Browse,
            artists: Vec::new(),
            albums: Vec::new(),
            tracks: Vec::new(),
            search_hits: Vec::new(),
            artist_cursor: Cursor::new(),
            album_cursor: Cursor::new(),
            track_cursor: Cursor::new(),
            list_cursor: Cursor::new(),
            column_focus: ColumnFocus::Artists,
            paging_level: ColumnFocus::Artists,
            expanded: HashSet::new(),
            tree_albums_by_artist: HashMap::new(),
            tree_cursor: Cursor::new(),
            tree_track_cache: std::collections::HashMap::new(),
            tree_album_cache: std::collections::HashMap::new(),
            preferred_layout: LibraryLayout::Columns,
            input: None,
            notice: None,
            h_offset: 0,
        };
        pane.reload_browse();
        pane
    }

    pub fn set_preferred_layout(&mut self, layout: LibraryLayout) {
        self.preferred_layout = layout;
    }

    pub fn preferred_layout(&self) -> LibraryLayout {
        self.preferred_layout
    }

    pub fn effective_layout(&self, strip_inner: usize) -> EffectiveLayout {
        if !matches!(self.mode, Mode::Browse) {
            return EffectiveLayout::Paging;
        }
        match self.preferred_layout {
            LibraryLayout::Tree => EffectiveLayout::Tree,
            LibraryLayout::Columns if layout::columns_usable(strip_inner) => {
                EffectiveLayout::Columns
            }
            LibraryLayout::Columns => EffectiveLayout::Paging,
        }
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

    pub fn artists(&self) -> &[ArtistSummary] {
        &self.artists
    }

    pub fn albums(&self) -> &[AlbumSummary] {
        &self.albums
    }

    pub fn tracks(&self) -> &[Track] {
        &self.tracks
    }

    pub fn search_hits(&self) -> &[SearchHit] {
        &self.search_hits
    }

    pub fn column_focus(&self) -> ColumnFocus {
        self.column_focus
    }

    pub fn paging_level(&self) -> ColumnFocus {
        self.paging_level
    }

    pub fn selected_artist_index(&self) -> Option<usize> {
        self.artist_cursor.selected(self.artists.len())
    }

    pub fn selected_album_index(&self) -> Option<usize> {
        self.album_cursor.selected(self.albums.len())
    }

    pub fn selected_track_index(&self) -> Option<usize> {
        self.track_cursor.selected(self.tracks.len())
    }

    pub fn focus_column(&mut self, focus: ColumnFocus) {
        self.column_focus = focus;
        self.h_offset = 0;
    }

    pub fn cycle_column(&mut self, forward: bool) {
        self.column_focus = if forward {
            self.column_focus.next()
        } else {
            self.column_focus.prev()
        };
        self.h_offset = 0;
    }

    /// Number of rows on the focused list for the active layout.
    pub fn len(&self, strip_inner: usize) -> usize {
        match self.mode {
            Mode::Search(_) => self.search_hits.len(),
            Mode::AllTracks => self.tracks.len(),
            Mode::Browse => match self.effective_layout(strip_inner) {
                EffectiveLayout::Columns => match self.column_focus {
                    ColumnFocus::Artists => self.artists.len(),
                    ColumnFocus::Albums => self.albums.len(),
                    ColumnFocus::Tracks => self.tracks.len(),
                },
                EffectiveLayout::Paging => match self.paging_level {
                    ColumnFocus::Artists => self.artists.len(),
                    ColumnFocus::Albums => self.albums.len(),
                    ColumnFocus::Tracks => self.tracks.len(),
                },
                EffectiveLayout::Tree => self.tree_rows().len(),
            },
        }
    }

    pub fn is_empty(&self, strip_inner: usize) -> bool {
        self.len(strip_inner) == 0
    }

    pub fn selected_index(&self, strip_inner: usize) -> Option<usize> {
        match self.mode {
            Mode::Search(_) | Mode::AllTracks => self.list_cursor.selected(self.len(strip_inner)),
            Mode::Browse => match self.effective_layout(strip_inner) {
                EffectiveLayout::Columns => match self.column_focus {
                    ColumnFocus::Artists => self.selected_artist_index(),
                    ColumnFocus::Albums => self.selected_album_index(),
                    ColumnFocus::Tracks => self.selected_track_index(),
                },
                EffectiveLayout::Paging => match self.paging_level {
                    ColumnFocus::Artists => self.selected_artist_index(),
                    ColumnFocus::Albums => self.selected_album_index(),
                    ColumnFocus::Tracks => self.selected_track_index(),
                },
                EffectiveLayout::Tree => self.tree_cursor.selected(self.tree_rows().len()),
            },
        }
    }

    pub fn selected(&self, strip_inner: usize) -> Option<Item<'_>> {
        match self.mode {
            Mode::Search(_) => {
                let index = self.list_cursor.selected(self.search_hits.len())?;
                self.search_hits.get(index).map(|hit| match hit {
                    SearchHit::Artist(artist) => Item::Artist(artist),
                    SearchHit::Album(album) => Item::Album(album),
                    SearchHit::Track(track) => Item::Track(track),
                })
            }
            Mode::AllTracks => {
                let index = self.list_cursor.selected(self.tracks.len())?;
                self.tracks.get(index).map(Item::Track)
            }
            Mode::Browse => match self.effective_layout(strip_inner) {
                EffectiveLayout::Tree => {
                    let rows = self.tree_rows();
                    let index = self.tree_cursor.selected(rows.len())?;
                    match rows.get(index)? {
                        TreeRow::Artist { artist, .. } => {
                            let name = artist.name.clone();
                            self.artists
                                .iter()
                                .find(|a| a.name == name)
                                .map(Item::Artist)
                        }
                        TreeRow::Album { album, .. } => {
                            let key = album.album.to_lowercase();
                            self.tree_album_cache
                                .get(&key)
                                .or_else(|| {
                                    self.albums
                                        .iter()
                                        .find(|a| a.album.eq_ignore_ascii_case(&album.album))
                                })
                                .map(Item::Album)
                        }
                        TreeRow::Track {
                            album_key, index, ..
                        } => self
                            .tree_track_cache
                            .get(album_key)
                            .and_then(|tracks| tracks.get(*index))
                            .map(Item::Track),
                    }
                }
                EffectiveLayout::Columns => match self.column_focus {
                    ColumnFocus::Artists => self
                        .selected_artist_index()
                        .and_then(|i| self.artists.get(i).map(Item::Artist)),
                    ColumnFocus::Albums => self
                        .selected_album_index()
                        .and_then(|i| self.albums.get(i).map(Item::Album)),
                    ColumnFocus::Tracks => self
                        .selected_track_index()
                        .and_then(|i| self.tracks.get(i).map(Item::Track)),
                },
                EffectiveLayout::Paging => match self.paging_level {
                    ColumnFocus::Artists => self
                        .selected_artist_index()
                        .and_then(|i| self.artists.get(i).map(Item::Artist)),
                    ColumnFocus::Albums => self
                        .selected_album_index()
                        .and_then(|i| self.albums.get(i).map(Item::Album)),
                    ColumnFocus::Tracks => self
                        .selected_track_index()
                        .and_then(|i| self.tracks.get(i).map(Item::Track)),
                },
            },
        }
    }

    /// Every track the selection stands for.
    pub fn selected_tracks(&self, strip_inner: usize) -> Vec<Track> {
        match self.selected(strip_inner) {
            Some(Item::Track(track)) => vec![track.clone()],
            Some(Item::Album(album)) => self.album_tracks(&album.album),
            Some(Item::Artist(artist)) => self.artist_tracks(&artist.name),
            None => Vec::new(),
        }
    }

    /// All rows currently listed, for "add everything".
    pub fn listed_tracks(&self, strip_inner: usize) -> Vec<Track> {
        match self.mode {
            Mode::Search(_) => self
                .search_hits
                .iter()
                .filter_map(|hit| match hit {
                    SearchHit::Track(track) => Some(track.clone()),
                    _ => None,
                })
                .collect(),
            Mode::AllTracks => self.tracks.clone(),
            Mode::Browse => match self.effective_layout(strip_inner) {
                EffectiveLayout::Columns => match self.column_focus {
                    ColumnFocus::Artists => self
                        .artists
                        .iter()
                        .flat_map(|a| self.artist_tracks(&a.name))
                        .collect(),
                    ColumnFocus::Albums => self
                        .albums
                        .iter()
                        .flat_map(|a| self.album_tracks(&a.album))
                        .collect(),
                    ColumnFocus::Tracks => self.tracks.clone(),
                },
                EffectiveLayout::Paging => match self.paging_level {
                    ColumnFocus::Artists => self
                        .artists
                        .iter()
                        .flat_map(|a| self.artist_tracks(&a.name))
                        .collect(),
                    ColumnFocus::Albums => self
                        .albums
                        .iter()
                        .flat_map(|a| self.album_tracks(&a.album))
                        .collect(),
                    ColumnFocus::Tracks => self.tracks.clone(),
                },
                EffectiveLayout::Tree => {
                    // Everything under currently expanded visible scope: all artists' tracks.
                    self.artists
                        .iter()
                        .flat_map(|a| self.artist_tracks(&a.name))
                        .collect()
                }
            },
        }
    }

    pub fn step(&mut self, delta: isize, strip_inner: usize) {
        match self.mode {
            Mode::Search(_) | Mode::AllTracks => {
                self.list_cursor.step(delta, self.len(strip_inner));
            }
            Mode::Browse => match self.effective_layout(strip_inner) {
                EffectiveLayout::Columns => match self.column_focus {
                    ColumnFocus::Artists => {
                        self.artist_cursor.step(delta, self.artists.len());
                        self.refresh_albums_for_selection();
                    }
                    ColumnFocus::Albums => {
                        self.album_cursor.step(delta, self.albums.len());
                        self.refresh_tracks_for_selection();
                    }
                    ColumnFocus::Tracks => {
                        self.track_cursor.step(delta, self.tracks.len());
                    }
                },
                EffectiveLayout::Paging => match self.paging_level {
                    ColumnFocus::Artists => self.artist_cursor.step(delta, self.artists.len()),
                    ColumnFocus::Albums => self.album_cursor.step(delta, self.albums.len()),
                    ColumnFocus::Tracks => self.track_cursor.step(delta, self.tracks.len()),
                },
                EffectiveLayout::Tree => {
                    self.tree_cursor.step(delta, self.tree_rows().len());
                }
            },
        }
        self.h_offset = 0;
    }

    pub fn set_index(&mut self, index: usize, strip_inner: usize) {
        match self.mode {
            Mode::Search(_) | Mode::AllTracks => {
                self.list_cursor.set(index, self.len(strip_inner));
            }
            Mode::Browse => match self.effective_layout(strip_inner) {
                EffectiveLayout::Columns => match self.column_focus {
                    ColumnFocus::Artists => {
                        self.artist_cursor.set(index, self.artists.len());
                        self.refresh_albums_for_selection();
                    }
                    ColumnFocus::Albums => {
                        self.album_cursor.set(index, self.albums.len());
                        self.refresh_tracks_for_selection();
                    }
                    ColumnFocus::Tracks => {
                        self.track_cursor.set(index, self.tracks.len());
                    }
                },
                EffectiveLayout::Paging => match self.paging_level {
                    ColumnFocus::Artists => self.artist_cursor.set(index, self.artists.len()),
                    ColumnFocus::Albums => self.album_cursor.set(index, self.albums.len()),
                    ColumnFocus::Tracks => self.track_cursor.set(index, self.tracks.len()),
                },
                EffectiveLayout::Tree => {
                    self.tree_cursor.set(index, self.tree_rows().len());
                }
            },
        }
        self.h_offset = 0;
    }

    /// Select a row in a specific column (mouse click).
    pub fn set_column_index(&mut self, column: ColumnFocus, index: usize) {
        match column {
            ColumnFocus::Artists => {
                self.artist_cursor.set(index, self.artists.len());
                self.column_focus = ColumnFocus::Artists;
                self.refresh_albums_for_selection();
            }
            ColumnFocus::Albums => {
                self.album_cursor.set(index, self.albums.len());
                self.column_focus = ColumnFocus::Albums;
                self.refresh_tracks_for_selection();
            }
            ColumnFocus::Tracks => {
                self.track_cursor.set(index, self.tracks.len());
                self.column_focus = ColumnFocus::Tracks;
            }
        }
        self.h_offset = 0;
    }

    pub fn page(&mut self, delta: isize, strip_inner: usize) {
        match self.mode {
            Mode::Search(_) | Mode::AllTracks => {
                self.list_cursor.page(delta, self.len(strip_inner));
            }
            Mode::Browse => match self.effective_layout(strip_inner) {
                EffectiveLayout::Columns => match self.column_focus {
                    ColumnFocus::Artists => {
                        self.artist_cursor.page(delta, self.artists.len());
                        self.refresh_albums_for_selection();
                    }
                    ColumnFocus::Albums => {
                        self.album_cursor.page(delta, self.albums.len());
                        self.refresh_tracks_for_selection();
                    }
                    ColumnFocus::Tracks => self.track_cursor.page(delta, self.tracks.len()),
                },
                EffectiveLayout::Paging => match self.paging_level {
                    ColumnFocus::Artists => self.artist_cursor.page(delta, self.artists.len()),
                    ColumnFocus::Albums => self.album_cursor.page(delta, self.albums.len()),
                    ColumnFocus::Tracks => self.track_cursor.page(delta, self.tracks.len()),
                },
                EffectiveLayout::Tree => self.tree_cursor.page(delta, self.tree_rows().len()),
            },
        }
        self.h_offset = 0;
    }

    pub fn first(&mut self, strip_inner: usize) {
        self.set_index(0, strip_inner);
    }

    pub fn last(&mut self, strip_inner: usize) {
        let len = self.len(strip_inner);
        if len > 0 {
            self.set_index(len - 1, strip_inner);
        }
    }

    /// Enter / open the current selection. Returns what happened.
    pub fn enter(&mut self, strip_inner: usize) -> EnterResult {
        match self.mode {
            Mode::Search(_) => EnterResult::None,
            Mode::AllTracks => EnterResult::None,
            Mode::Browse => match self.effective_layout(strip_inner) {
                EffectiveLayout::Columns => match self.column_focus {
                    ColumnFocus::Artists => {
                        if self.selected_artist_index().is_none() {
                            return EnterResult::None;
                        }
                        self.refresh_albums_for_selection();
                        self.column_focus = ColumnFocus::Albums;
                        self.h_offset = 0;
                        EnterResult::Moved
                    }
                    ColumnFocus::Albums => {
                        if self.selected_album_index().is_none() {
                            return EnterResult::None;
                        }
                        self.refresh_tracks_for_selection();
                        self.column_focus = ColumnFocus::Tracks;
                        self.h_offset = 0;
                        EnterResult::Moved
                    }
                    ColumnFocus::Tracks => EnterResult::None,
                },
                EffectiveLayout::Paging => match self.paging_level {
                    ColumnFocus::Artists => {
                        if self.selected_artist_index().is_none() {
                            return EnterResult::None;
                        }
                        self.refresh_albums_for_selection();
                        self.paging_level = ColumnFocus::Albums;
                        self.album_cursor.first();
                        self.h_offset = 0;
                        EnterResult::Moved
                    }
                    ColumnFocus::Albums => {
                        if self.selected_album_index().is_none() {
                            return EnterResult::None;
                        }
                        self.refresh_tracks_for_selection();
                        self.paging_level = ColumnFocus::Tracks;
                        self.track_cursor.first();
                        self.h_offset = 0;
                        EnterResult::Moved
                    }
                    ColumnFocus::Tracks => EnterResult::None,
                },
                EffectiveLayout::Tree => {
                    if self.toggle_expand_at_cursor() {
                        EnterResult::Moved
                    } else {
                        EnterResult::None
                    }
                }
            },
        }
    }

    /// Leave search or step back one paging level.
    pub fn back(&mut self) -> bool {
        match self.mode {
            Mode::Search(_) => {
                self.search_hits.clear();
                self.list_cursor.first();
                self.h_offset = 0;
                self.reload_browse();
                true
            }
            Mode::AllTracks => false,
            Mode::Browse => {
                if self.preferred_layout == LibraryLayout::Tree {
                    return false;
                }
                // Paging (or columns with Esc as mild no-op unless paging)
                match self.paging_level {
                    ColumnFocus::Tracks => {
                        self.paging_level = ColumnFocus::Albums;
                        self.tracks.clear();
                        self.h_offset = 0;
                        true
                    }
                    ColumnFocus::Albums => {
                        self.paging_level = ColumnFocus::Artists;
                        self.albums.clear();
                        self.tracks.clear();
                        self.h_offset = 0;
                        true
                    }
                    ColumnFocus::Artists => false,
                }
            }
        }
    }

    pub fn toggle_expand_at_cursor(&mut self) -> bool {
        let rows = self.tree_rows();
        let Some(index) = self.tree_cursor.selected(rows.len()) else {
            return false;
        };
        let Some(row) = rows.get(index).cloned() else {
            return false;
        };
        match row {
            TreeRow::Artist { artist, expanded } => {
                let id = TreeNodeId::Artist(artist.name.clone());
                if expanded {
                    self.expanded.remove(&id);
                    self.expanded.retain(|n| match n {
                        TreeNodeId::Album { artist: a, .. } => a != &artist.name,
                        TreeNodeId::Artist(_) => true,
                    });
                    // Drop cached albums/tracks for this artist.
                    let albums = match self
                        .tree_albums_by_artist
                        .remove(&artist.name.to_lowercase())
                    {
                        Some(albums) => albums,
                        None => self
                            .library
                            .as_ref()
                            .and_then(|lib| lib.albums_for_browse_artist(&artist.name).ok())
                            .unwrap_or_default(),
                    };
                    for album in albums {
                        let key = album.album.to_lowercase();
                        self.tree_album_cache.remove(&key);
                        self.tree_track_cache.remove(&key);
                    }
                } else {
                    self.expanded.insert(id);
                    let albums = self
                        .library
                        .as_ref()
                        .and_then(|lib| lib.albums_for_browse_artist(&artist.name).ok())
                        .unwrap_or_default();
                    for album in &albums {
                        self.tree_album_cache
                            .insert(album.album.to_lowercase(), album.clone());
                    }
                    self.tree_albums_by_artist
                        .insert(artist.name.to_lowercase(), albums.clone());
                    self.albums = albums;
                }
                true
            }
            TreeRow::Album {
                artist,
                album,
                expanded,
                ..
            } => {
                let id = TreeNodeId::Album {
                    artist: artist.clone(),
                    album: album.album.clone(),
                };
                let key = album.album.to_lowercase();
                if expanded {
                    self.expanded.remove(&id);
                    self.tree_track_cache.remove(&key);
                } else {
                    self.expanded.insert(id);
                    let tracks = self.album_tracks(&album.album);
                    self.tracks = tracks.clone();
                    self.tree_track_cache.insert(key, tracks);
                    self.tree_album_cache
                        .insert(album.album.to_lowercase(), album);
                }
                true
            }
            TreeRow::Track { .. } => false,
        }
    }

    /// `h` / `l`: move left or right without ever playing a track.
    ///
    /// Columns step between columns, paging drills in or backs out, the tree
    /// expands, collapses, or climbs to the parent. Search results ignore it.
    /// Returns whether anything changed.
    pub fn move_horizontal(&mut self, forward: bool, strip_inner: usize) -> bool {
        if !matches!(self.mode, Mode::Browse) {
            return false;
        }
        match self.effective_layout(strip_inner) {
            EffectiveLayout::Columns => {
                if forward {
                    self.column_focus != ColumnFocus::Tracks
                        && self.enter(strip_inner) == EnterResult::Moved
                } else if self.column_focus == ColumnFocus::Artists {
                    false
                } else {
                    self.focus_column(self.column_focus.prev());
                    true
                }
            }
            EffectiveLayout::Paging => {
                if forward {
                    self.paging_level != ColumnFocus::Tracks
                        && self.enter(strip_inner) == EnterResult::Moved
                } else {
                    self.back()
                }
            }
            EffectiveLayout::Tree => {
                if forward {
                    self.tree_expand()
                } else {
                    self.tree_collapse_or_parent()
                }
            }
        }
    }

    /// Expand the closed artist or album under the cursor.
    fn tree_expand(&mut self) -> bool {
        let rows = self.tree_rows();
        let Some(index) = self.tree_cursor.selected(rows.len()) else {
            return false;
        };
        match rows.get(index) {
            Some(TreeRow::Artist {
                expanded: false, ..
            })
            | Some(TreeRow::Album {
                expanded: false, ..
            }) => self.toggle_expand_at_cursor(),
            _ => false,
        }
    }

    /// Collapse the open node under the cursor, or move to its parent row.
    fn tree_collapse_or_parent(&mut self) -> bool {
        let rows = self.tree_rows();
        let Some(index) = self.tree_cursor.selected(rows.len()) else {
            return false;
        };
        match &rows[index] {
            TreeRow::Artist { expanded: true, .. } | TreeRow::Album { expanded: true, .. } => {
                self.toggle_expand_at_cursor()
            }
            row => {
                let depth = row.depth();
                match rows[..index].iter().rposition(|r| r.depth() < depth) {
                    Some(parent) => {
                        self.tree_cursor.set(parent, rows.len());
                        self.h_offset = 0;
                        true
                    }
                    None => false,
                }
            }
        }
    }

    pub fn tree_track_cache_get(&self, album_key: &str, index: usize) -> Option<&Track> {
        self.tree_track_cache
            .get(album_key)
            .and_then(|tracks| tracks.get(index))
    }

    pub fn tree_rows(&self) -> Vec<TreeRow> {
        let mut rows = Vec::new();
        for artist in &self.artists {
            let artist_id = TreeNodeId::Artist(artist.name.clone());
            let expanded = self.expanded.contains(&artist_id);
            rows.push(TreeRow::Artist {
                artist: artist.clone(),
                expanded,
            });
            if !expanded {
                continue;
            }
            let artist_key = artist.name.to_lowercase();
            let cached = self.tree_albums_by_artist.get(&artist_key);
            let fallback;
            let albums: &[AlbumSummary] = match cached {
                Some(albums) => albums,
                None => {
                    fallback = self
                        .library
                        .as_ref()
                        .and_then(|lib| lib.albums_for_browse_artist(&artist.name).ok())
                        .unwrap_or_default();
                    &fallback
                }
            };
            for album in albums {
                let album_id = TreeNodeId::Album {
                    artist: artist.name.clone(),
                    album: album.album.clone(),
                };
                let album_expanded = self.expanded.contains(&album_id);
                let album_key = album.album.to_lowercase();
                rows.push(TreeRow::Album {
                    artist: artist.name.clone(),
                    album: album.clone(),
                    expanded: album_expanded,
                    depth: 1,
                });
                if album_expanded {
                    if let Some(tracks) = self.tree_track_cache.get(&album_key) {
                        for index in 0..tracks.len() {
                            rows.push(TreeRow::Track {
                                album_key: album_key.clone(),
                                index,
                                depth: 2,
                            });
                        }
                    }
                }
            }
        }
        rows
    }

    pub fn h_offset(&self) -> usize {
        self.h_offset
    }

    pub fn offset_for(&self, index: usize, strip_inner: usize) -> usize {
        self.offset_for_row(index, self.selected_index(strip_inner))
    }

    /// Pan offset for one row when the selection has already been resolved.
    ///
    /// Tree rendering resolves the selected index once per frame; calling
    /// [`Self::offset_for`] per row rebuilds the tree for every row.
    pub fn offset_for_row(&self, index: usize, selected: Option<usize>) -> usize {
        if selected == Some(index) {
            self.h_offset
        } else {
            0
        }
    }

    pub fn clamp_pan(&mut self, slot: usize, strip_inner: usize, playing: Option<&PlayingRef>) {
        let max = self
            .selected_middle_len(strip_inner, playing)
            .saturating_sub(slot);
        self.h_offset = self.h_offset.min(max);
    }

    pub fn pan(
        &mut self,
        delta: isize,
        slot: usize,
        strip_inner: usize,
        playing: Option<&PlayingRef>,
    ) {
        let max = self
            .selected_middle_len(strip_inner, playing)
            .saturating_sub(slot) as isize;
        let next = self.h_offset as isize + delta;
        self.h_offset = next.clamp(0, max.max(0)) as usize;
    }

    /// Columns a row's playing marker adds to its middle text: ` ♪` after an
    /// artist or album name, `▶ ` before a tree track. Flat track rows mark
    /// the right-hand number instead, so they add none.
    fn marker_width(item: &Item, playing: Option<&PlayingRef>, tree: bool) -> usize {
        let Some(p) = playing else {
            return 0;
        };
        let hit = match item {
            Item::Artist(a) => p.is_artist(a),
            Item::Album(a) => p.is_album(a),
            Item::Track(t) => tree && p.is_track(t),
        };
        if hit {
            2
        } else {
            0
        }
    }

    fn middle_len(item: &Item, playing: Option<&PlayingRef>, tree: bool) -> usize {
        let text = match item {
            Item::Artist(a) => Self::artist_middle(a),
            Item::Album(a) => Self::album_middle(a),
            Item::Track(t) => Self::track_middle(t),
        };
        text.chars().count() + Self::marker_width(item, playing, tree)
    }

    fn selected_middle_len(&self, strip_inner: usize, playing: Option<&PlayingRef>) -> usize {
        let tree = self.is_tree(strip_inner);
        match self.selected(strip_inner) {
            Some(item) => Self::middle_len(&item, playing, tree),
            None => 0,
        }
    }

    fn is_tree(&self, strip_inner: usize) -> bool {
        matches!(self.mode, Mode::Browse)
            && self.effective_layout(strip_inner) == EffectiveLayout::Tree
    }

    pub fn longest_middle(&self, strip_inner: usize, playing: Option<&PlayingRef>) -> usize {
        let len = |item: Item| Self::middle_len(&item, playing, false);
        match self.mode {
            Mode::Search(_) => self
                .search_hits
                .iter()
                .map(|hit| match hit {
                    SearchHit::Artist(artist) => len(Item::Artist(artist)),
                    SearchHit::Album(album) => len(Item::Album(album)),
                    SearchHit::Track(track) => len(Item::Track(track)),
                })
                .max()
                .unwrap_or(0),
            Mode::AllTracks => self
                .tracks
                .iter()
                .map(|t| len(Item::Track(t)))
                .max()
                .unwrap_or(0),
            Mode::Browse => match self.effective_layout(strip_inner) {
                EffectiveLayout::Tree => self
                    .tree_rows()
                    .iter()
                    .map(|row| match row {
                        TreeRow::Artist { artist, .. } => len(Item::Artist(artist)),
                        TreeRow::Album { album, .. } => len(Item::Album(album)),
                        TreeRow::Track {
                            album_key, index, ..
                        } => self
                            .tree_track_cache
                            .get(album_key)
                            .and_then(|tracks| tracks.get(*index))
                            .map(|t| Self::middle_len(&Item::Track(t), playing, true))
                            .unwrap_or(0),
                    })
                    .max()
                    .unwrap_or(0),
                _ => match self.column_focus {
                    ColumnFocus::Artists => self
                        .artists
                        .iter()
                        .map(|a| len(Item::Artist(a)))
                        .max()
                        .unwrap_or(0),
                    ColumnFocus::Albums => self
                        .albums
                        .iter()
                        .map(|a| len(Item::Album(a)))
                        .max()
                        .unwrap_or(0),
                    ColumnFocus::Tracks => self
                        .tracks
                        .iter()
                        .map(|t| len(Item::Track(t)))
                        .max()
                        .unwrap_or(0),
                },
            },
        }
    }

    pub fn artist_middle(artist: &ArtistSummary) -> String {
        artist.name.clone()
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
        self.input = Some(LineEdit::new());
    }

    pub fn input(&self) -> Option<&str> {
        self.input.as_deref()
    }

    pub fn prompt(&self) -> Option<&LineEdit> {
        self.input.as_ref()
    }

    pub fn prompt_mut(&mut self) -> Option<&mut LineEdit> {
        self.input.as_mut()
    }

    pub fn is_typing(&self) -> bool {
        self.input.is_some()
    }

    pub fn push_char(&mut self, c: char) {
        if let Some(input) = self.input.as_mut() {
            input.insert(c);
        }
    }

    pub fn pop_char(&mut self) {
        if let Some(input) = self.input.as_mut() {
            input.backspace();
        }
    }

    pub fn cancel_search(&mut self) {
        self.input = None;
    }

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

        match library.search_entities(&query, SearchLimits::default()) {
            Ok(hits) => {
                let count = hits.len();
                self.search_hits = hits;
                self.mode = Mode::Search(query.clone());
                self.list_cursor.first();
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

    /// Leave search and focus a browse artist.
    pub fn focus_artist(&mut self, name: &str) -> bool {
        self.search_hits.clear();
        self.reload_browse();
        let Some(index) = self
            .artists
            .iter()
            .position(|a| a.name.eq_ignore_ascii_case(name))
        else {
            return false;
        };
        self.artist_cursor.set(index, self.artists.len());
        self.refresh_albums_for_selection();
        self.column_focus = ColumnFocus::Albums;
        self.paging_level = ColumnFocus::Albums;
        self.expanded
            .insert(TreeNodeId::Artist(self.artists[index].name.clone()));
        self.sync_tree_cursor_to_artist(&self.artists[index].name.clone());
        true
    }

    /// Leave search and focus a browse album (and its artist when known).
    pub fn focus_album(&mut self, album: &str, album_artist: Option<&str>) -> bool {
        self.search_hits.clear();
        self.reload_browse();

        let browse_artist = if let Some(aa) = album_artist.filter(|s| !s.is_empty()) {
            // Prefer matching browse artist by album_artist name.
            self.artists
                .iter()
                .find(|a| a.name.eq_ignore_ascii_case(aa))
                .map(|a| a.name.clone())
                .or_else(|| {
                    // Fall back: find which browse artist owns this album.
                    self.find_browse_artist_for_album(album)
                })
        } else {
            self.find_browse_artist_for_album(album)
        };

        let Some(artist_name) = browse_artist else {
            return false;
        };

        let Some(artist_index) = self
            .artists
            .iter()
            .position(|a| a.name.eq_ignore_ascii_case(&artist_name))
        else {
            return false;
        };

        self.artist_cursor.set(artist_index, self.artists.len());
        self.refresh_albums_for_selection();
        let Some(album_index) = self
            .albums
            .iter()
            .position(|a| a.album.eq_ignore_ascii_case(album))
        else {
            return false;
        };
        self.album_cursor.set(album_index, self.albums.len());
        self.refresh_tracks_for_selection();
        self.column_focus = ColumnFocus::Tracks;
        self.paging_level = ColumnFocus::Tracks;
        self.expanded
            .insert(TreeNodeId::Artist(artist_name.clone()));
        self.expanded.insert(TreeNodeId::Album {
            artist: artist_name.clone(),
            album: self.albums[album_index].album.clone(),
        });
        self.sync_tree_cursor_to_album(&artist_name, &self.albums[album_index].album.clone());
        true
    }

    /// `J`: leave search and select `track` in browse, in every layout.
    pub fn reveal(&mut self, track: &Track, strip_inner: usize) -> bool {
        let Some(album) = track.album.as_deref() else {
            return false;
        };
        let album_artist = track.album_artist.as_deref().or(track.artist.as_deref());
        if !self.focus_album(album, album_artist) {
            return false;
        }
        let Some(index) = self.tracks.iter().position(|t| t.path == track.path) else {
            return false;
        };
        self.track_cursor.set(index, self.tracks.len());
        if self.effective_layout(strip_inner) == EffectiveLayout::Tree {
            self.reveal_in_tree(index);
        }
        self.h_offset = 0;
        true
    }

    /// Load the album's tracks into the tree and put the cursor on one of them.
    fn reveal_in_tree(&mut self, track_index: usize) {
        let Some(album) = self
            .selected_album_index()
            .and_then(|i| self.albums.get(i))
            .cloned()
        else {
            return;
        };
        let key = album.album.to_lowercase();
        self.tree_track_cache
            .insert(key.clone(), self.tracks.clone());
        self.tree_album_cache.insert(key.clone(), album);
        let rows = self.tree_rows();
        if let Some(row) = rows.iter().position(|r| {
            matches!(r, TreeRow::Track { album_key, index, .. }
                if *album_key == key && *index == track_index)
        }) {
            self.tree_cursor.set(row, rows.len());
        }
    }

    fn find_browse_artist_for_album(&self, album: &str) -> Option<String> {
        let library = self.library.as_ref()?;
        for artist in &self.artists {
            if let Ok(albums) = library.albums_for_browse_artist(&artist.name) {
                if albums.iter().any(|a| a.album.eq_ignore_ascii_case(album)) {
                    return Some(artist.name.clone());
                }
            }
        }
        None
    }

    fn sync_tree_cursor_to_artist(&mut self, name: &str) {
        let rows = self.tree_rows();
        if let Some(i) = rows
            .iter()
            .position(|r| matches!(r, TreeRow::Artist { artist, .. } if artist.name == name))
        {
            self.tree_cursor.set(i, rows.len());
        }
    }

    fn sync_tree_cursor_to_album(&mut self, artist: &str, album: &str) {
        let rows = self.tree_rows();
        if let Some(i) = rows.iter().position(|r| {
            matches!(
                r,
                TreeRow::Album {
                    artist: a,
                    album: al,
                    ..
                } if a == artist && al.album == album
            )
        }) {
            self.tree_cursor.set(i, rows.len());
        }
    }

    /// Re-read browse roots (after scan / Esc from search).
    pub fn reload_browse(&mut self) {
        let Some(library) = self.library.as_ref() else {
            self.notice = Some("no library yet — run `znicz scan <dir>`".to_string());
            return;
        };

        let artists = match library.browse_artists() {
            Ok(artists) => artists,
            Err(e) => {
                self.notice = Some(format!("could not read library: {e}"));
                return;
            }
        };

        if !artists.is_empty() {
            self.artists = artists;
            self.search_hits.clear();
            self.mode = Mode::Browse;
            self.notice = None;
            self.column_focus = ColumnFocus::Artists;
            self.paging_level = ColumnFocus::Artists;
            self.artist_cursor.clamp(self.artists.len());
            self.refresh_albums_for_selection();
            self.refresh_expanded_artists();
            return;
        }

        self.artists.clear();
        self.albums.clear();
        self.search_hits.clear();
        self.tree_albums_by_artist.clear();
        match library.all_tracks(500) {
            Ok(tracks) if !tracks.is_empty() => {
                self.notice = None;
                self.tracks = tracks;
                self.mode = Mode::AllTracks;
                self.list_cursor.clamp(self.tracks.len());
            }
            Ok(_) => {
                self.mode = Mode::Browse;
                self.notice = Some("library is empty — run `znicz scan <dir>`".to_string());
            }
            Err(e) => self.notice = Some(format!("could not read library: {e}")),
        }
    }

    /// Compatibility alias used by reload key.
    pub fn reload_albums(&mut self) {
        self.reload_browse();
    }

    fn refresh_albums_for_selection(&mut self) {
        let Some(index) = self.selected_artist_index() else {
            self.albums.clear();
            self.tracks.clear();
            return;
        };
        let name = self.artists[index].name.clone();
        self.load_albums_for_artist(&name);
        self.album_cursor.clamp(self.albums.len());
        self.refresh_tracks_for_selection();
    }

    fn load_albums_for_artist(&mut self, name: &str) {
        let albums = self
            .library
            .as_ref()
            .and_then(|lib| lib.albums_for_browse_artist(name).ok())
            .unwrap_or_default();
        self.tree_albums_by_artist
            .insert(name.to_lowercase(), albums.clone());
        self.albums = albums;
    }

    /// Refresh cached album lists for every expanded artist after a reload, so
    /// an external scan shows up without waiting for a collapse and re-expand.
    fn refresh_expanded_artists(&mut self) {
        let names: Vec<String> = self
            .expanded
            .iter()
            .filter_map(|id| match id {
                TreeNodeId::Artist(name) => Some(name.clone()),
                TreeNodeId::Album { .. } => None,
            })
            .collect();
        for name in names {
            let albums = match self.library.as_ref() {
                Some(library) => library.albums_for_browse_artist(&name).unwrap_or_default(),
                None => continue,
            };
            self.tree_albums_by_artist
                .insert(name.to_lowercase(), albums);
        }
    }

    fn refresh_tracks_for_selection(&mut self) {
        let Some(index) = self.selected_album_index() else {
            self.tracks.clear();
            return;
        };
        let name = self.albums[index].album.clone();
        self.tracks = self.album_tracks(&name);
        self.track_cursor.clamp(self.tracks.len());
    }

    fn album_tracks(&self, album: &str) -> Vec<Track> {
        self.library
            .as_ref()
            .and_then(|library| library.browse_album(album).ok())
            .unwrap_or_default()
    }

    fn artist_tracks(&self, artist: &str) -> Vec<Track> {
        self.library
            .as_ref()
            .and_then(|library| {
                // Prefer browse attribution: all albums under this browse artist.
                let albums = library.albums_for_browse_artist(artist).ok()?;
                let mut tracks = Vec::new();
                for album in albums {
                    tracks.extend(library.browse_album(&album.album).ok()?);
                }
                if tracks.is_empty() {
                    library.tracks_for_artist(artist).ok()
                } else {
                    Some(tracks)
                }
            })
            .unwrap_or_default()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnterResult {
    None,
    Moved,
}

impl LibraryPane {
    /// The library's row for a file, if it has one.
    pub fn locate(&self, path: &std::path::Path) -> Option<Track> {
        self.library.as_ref()?.get_by_path(path).ok().flatten()
    }

    /// Seed album rows for integration tests (paging albums level).
    pub fn inject_albums_for_test(&mut self, albums: Vec<AlbumSummary>) {
        self.artists = vec![ArtistSummary {
            name: "Test Artist".into(),
            track_count: albums.iter().map(|a| a.track_count).sum(),
        }];
        self.albums = albums;
        self.tracks.clear();
        self.search_hits.clear();
        self.mode = Mode::Browse;
        self.column_focus = ColumnFocus::Albums;
        self.paging_level = ColumnFocus::Albums;
        self.preferred_layout = LibraryLayout::Columns;
        self.notice = None;
        self.artist_cursor.first();
        self.album_cursor.clamp(self.albums.len());
    }

    /// Seed track rows for integration tests (paging tracks level).
    pub fn inject_tracks_for_test(&mut self, tracks: Vec<Track>) {
        self.artists = vec![ArtistSummary {
            name: "Test Artist".into(),
            track_count: tracks.len() as u32,
        }];
        self.albums = vec![AlbumSummary {
            album: "test".into(),
            album_artist: Some("Test Artist".into()),
            year: None,
            track_count: tracks.len() as u32,
            total_secs: None,
        }];
        self.tracks = tracks;
        self.search_hits.clear();
        self.mode = Mode::Browse;
        self.column_focus = ColumnFocus::Tracks;
        self.paging_level = ColumnFocus::Tracks;
        self.preferred_layout = LibraryLayout::Columns;
        self.notice = None;
        self.track_cursor.clamp(self.tracks.len());
    }

    /// Seed search hits for integration tests.
    pub fn inject_search_hits_for_test(&mut self, query: &str, hits: Vec<SearchHit>) {
        self.search_hits = hits;
        self.mode = Mode::Search(query.into());
        self.notice = None;
        self.list_cursor.clamp(self.search_hits.len());
    }

    /// Seed artists for browse tests.
    pub fn inject_artists_for_test(&mut self, artists: Vec<ArtistSummary>) {
        self.artists = artists;
        self.albums.clear();
        self.tracks.clear();
        self.search_hits.clear();
        self.mode = Mode::Browse;
        self.column_focus = ColumnFocus::Artists;
        self.paging_level = ColumnFocus::Artists;
        self.notice = None;
        self.artist_cursor.clamp(self.artists.len());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn strip() -> usize {
        // Narrow → paging for columns preference.
        40
    }

    fn wide() -> usize {
        80
    }

    #[test]
    fn without_a_library_the_pane_explains_itself() {
        let pane = LibraryPane::new(None);
        assert!(!pane.is_available());
        assert!(pane.is_empty(strip()));
        assert!(
            pane.notice().unwrap().contains("scan"),
            "the notice should tell the user how to build a library"
        );
    }

    #[test]
    fn navigation_on_an_empty_pane_selects_nothing() {
        let mut pane = LibraryPane::new(None);
        pane.step(1, strip());
        pane.last(strip());
        assert!(pane.selected(strip()).is_none());
        assert!(pane.selected_tracks(strip()).is_empty());
        assert_eq!(pane.enter(strip()), EnterResult::None);
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
        pane.pan(5, 4, strip(), None);
        assert_eq!(pane.h_offset(), 0);
    }

    #[test]
    fn pan_offset_applies_only_to_the_selected_row() {
        let mut pane = LibraryPane::new(None);
        pane.inject_albums_for_test(vec![long_album("a"), long_album("b")]);
        pane.pan(3, 10, strip(), None);
        assert_eq!(pane.offset_for(0, strip()), 3);
        assert_eq!(pane.offset_for(1, strip()), 0);
    }

    #[test]
    fn moving_the_cursor_resets_pan() {
        let mut pane = LibraryPane::new(None);
        pane.inject_albums_for_test(vec![long_album("a"), long_album("b")]);
        pane.pan(4, 10, strip(), None);
        pane.step(1, strip());
        assert_eq!(pane.offset_for(0, strip()), 0);
        assert_eq!(pane.h_offset(), 0);
    }

    #[test]
    fn pan_clamps_to_the_selected_row_not_the_longest() {
        let mut pane = LibraryPane::new(None);
        pane.inject_albums_for_test(vec![
            AlbumSummary {
                album: "Hi".into(),
                album_artist: None,
                year: None,
                track_count: 1,
                total_secs: None,
            },
            long_album("long"),
        ]);
        pane.pan(20, 40, strip(), None);
        assert_eq!(pane.h_offset(), 0);
    }

    #[test]
    fn a_playing_rows_marker_counts_towards_the_longest_middle() {
        let mut pane = LibraryPane::new(None);
        let album = long_album("x");
        pane.inject_albums_for_test(vec![album.clone()]);
        let playing = PlayingRef {
            path: std::path::PathBuf::from("/music/a.flac"),
            artist: None,
            album: Some(album.album.clone()),
        };
        let plain = pane.longest_middle(strip(), None);
        assert_eq!(pane.longest_middle(strip(), Some(&playing)), plain + 2);
        pane.pan(1000, 10, strip(), Some(&playing));
        assert_eq!(pane.h_offset(), plain + 2 - 10);
    }

    fn long_album(name: &str) -> AlbumSummary {
        AlbumSummary {
            album: name.repeat(50),
            album_artist: None,
            year: None,
            track_count: 1,
            total_secs: Some(125.0),
        }
    }

    #[test]
    fn pan_moves_offset_when_middle_is_longer_than_slot() {
        let mut pane = LibraryPane::new(None);
        pane.inject_albums_for_test(vec![AlbumSummary {
            album: "x".repeat(50),
            album_artist: None,
            year: None,
            track_count: 1,
            total_secs: Some(125.0),
        }]);
        let slot = 20;
        assert!(pane.longest_middle(strip(), None) > slot);
        pane.pan(1, slot, strip(), None);
        assert_eq!(pane.h_offset(), 1);
        pane.pan(2, slot, strip(), None);
        assert_eq!(pane.h_offset(), 3);
    }

    #[test]
    fn clamp_pan_shrinks_offset_when_slot_grows() {
        let mut pane = LibraryPane::new(None);
        pane.inject_albums_for_test(vec![AlbumSummary {
            album: "x".repeat(50),
            album_artist: None,
            year: None,
            track_count: 1,
            total_secs: Some(125.0),
        }]);
        let narrow = 10;
        let wide_slot = 30;
        pane.pan(100, narrow, strip(), None);
        let max_narrow = pane.longest_middle(strip(), None).saturating_sub(narrow);
        assert_eq!(pane.h_offset(), max_narrow);
        pane.clamp_pan(wide_slot, strip(), None);
        let max_wide = pane.longest_middle(strip(), None).saturating_sub(wide_slot);
        assert_eq!(pane.h_offset(), max_wide);
        assert!(max_wide < max_narrow);
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

    #[test]
    fn enter_does_not_open_album_from_search_hits() {
        let mut pane = LibraryPane::new(None);
        pane.inject_search_hits_for_test(
            "love",
            vec![SearchHit::Album(AlbumSummary {
                album: "Love".into(),
                album_artist: Some("Other".into()),
                year: None,
                track_count: 1,
                total_secs: None,
            })],
        );
        assert_eq!(pane.enter(strip()), EnterResult::None);
        assert!(matches!(pane.mode(), Mode::Search(_)));
    }

    #[test]
    fn search_listed_tracks_are_title_hits_only() {
        use std::path::PathBuf;

        let mut pane = LibraryPane::new(None);
        pane.inject_search_hits_for_test(
            "Love",
            vec![
                SearchHit::Artist(ArtistSummary {
                    name: "Love".into(),
                    track_count: 2,
                }),
                SearchHit::Album(AlbumSummary {
                    album: "Love".into(),
                    album_artist: Some("Other".into()),
                    year: None,
                    track_count: 1,
                    total_secs: None,
                }),
                SearchHit::Track(Track {
                    id: 1,
                    path: PathBuf::from("/music/title-love.flac"),
                    title: "Love Song".into(),
                    artist: Some("Singer".into()),
                    album: Some("Hits".into()),
                    album_artist: None,
                    genre: None,
                    year: None,
                    track_number: None,
                    disc_number: None,
                    codec: None,
                    sample_rate: None,
                    channels: None,
                    bits_per_sample: None,
                    duration_secs: None,
                }),
            ],
        );
        let listed = pane.listed_tracks(strip());
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].title, "Love Song");
    }

    #[test]
    fn search_selected_artist_queues_artist_tracks() {
        let mut library = Library::open_in_memory().expect("open");
        library
            .upsert_track_for_test(
                Path::new("/music/a.flac"),
                "Alone",
                Some("Love".into()),
                Some("Forever".into()),
                None,
            )
            .unwrap();
        library
            .upsert_track_for_test(
                Path::new("/music/b.flac"),
                "Together",
                Some("Love".into()),
                Some("Forever".into()),
                None,
            )
            .unwrap();

        let mut pane = LibraryPane::new(Some(library));
        pane.begin_search();
        for c in "Love".chars() {
            pane.push_char(c);
        }
        let message = pane.submit_search();
        assert!(message.contains("match"), "got: {message}");
        assert!(matches!(pane.selected(strip()), Some(Item::Artist(_))));
        assert_eq!(pane.selected_tracks(strip()).len(), 2);
        assert_eq!(
            pane.listed_tracks(strip()).len(),
            0,
            "no title hits for Love"
        );
    }

    #[test]
    fn paging_enter_drills_artist_to_albums_to_tracks() {
        let mut library = Library::open_in_memory().expect("open");
        library
            .upsert_track_for_test(
                Path::new("/music/a.flac"),
                "So What",
                Some("Miles".into()),
                Some("Kind of Blue".into()),
                None,
            )
            .unwrap();
        let mut pane = LibraryPane::new(Some(library));
        pane.set_preferred_layout(LibraryLayout::Columns);
        assert_eq!(pane.effective_layout(strip()), EffectiveLayout::Paging);
        assert_eq!(pane.paging_level(), ColumnFocus::Artists);
        assert_eq!(pane.enter(strip()), EnterResult::Moved);
        assert_eq!(pane.paging_level(), ColumnFocus::Albums);
        assert_eq!(pane.enter(strip()), EnterResult::Moved);
        assert_eq!(pane.paging_level(), ColumnFocus::Tracks);
        assert!(matches!(pane.selected(strip()), Some(Item::Track(_))));
        assert!(pane.back());
        assert_eq!(pane.paging_level(), ColumnFocus::Albums);
    }

    #[test]
    fn wide_strip_uses_columns() {
        let mut library = Library::open_in_memory().expect("open");
        library
            .upsert_track_for_test(
                Path::new("/music/a.flac"),
                "So What",
                Some("Miles".into()),
                Some("Kind of Blue".into()),
                None,
            )
            .unwrap();
        let pane = LibraryPane::new(Some(library));
        assert_eq!(pane.effective_layout(wide()), EffectiveLayout::Columns);
    }

    #[test]
    fn tree_layout_expands_and_remembers_session_state() {
        let mut library = Library::open_in_memory().expect("open");
        library
            .upsert_track_for_test(
                Path::new("/music/a.flac"),
                "So What",
                Some("Miles".into()),
                Some("Kind of Blue".into()),
                None,
            )
            .unwrap();
        let mut pane = LibraryPane::new(Some(library));
        pane.set_preferred_layout(LibraryLayout::Tree);
        assert_eq!(pane.effective_layout(strip()), EffectiveLayout::Tree);
        assert_eq!(pane.tree_rows().len(), 1);
        assert_eq!(pane.enter(strip()), EnterResult::Moved);
        assert!(pane.expanded.contains(&TreeNodeId::Artist("Miles".into())));
        assert!(pane.tree_rows().len() >= 2);
        assert_eq!(pane.enter(strip()), EnterResult::Moved); // still on artist — toggle collapse
                                                             // After toggle collapse, expand again and open album
        assert_eq!(pane.enter(strip()), EnterResult::Moved);
        pane.step(1, strip()); // album row
        assert_eq!(pane.enter(strip()), EnterResult::Moved);
        assert!(pane.expanded.contains(&TreeNodeId::Album {
            artist: "Miles".into(),
            album: "Kind of Blue".into()
        }));
    }

    #[test]
    fn only_the_selected_row_gets_the_pan_offset_regardless_of_layout() {
        let mut pane = LibraryPane::new(None);
        pane.inject_albums_for_test(vec![long_album("a"), long_album("b")]);
        pane.pan(4, 10, strip(), None);

        assert_eq!(pane.offset_for_row(0, Some(0)), 4);
        assert_eq!(pane.offset_for_row(1, Some(0)), 0);
        assert_eq!(pane.offset_for_row(0, None), 0);
    }

    #[test]
    fn expanded_tree_rows_use_cached_albums_after_the_library_goes_away() {
        let mut library = Library::open_in_memory().expect("open");
        library
            .upsert_track_for_test(
                Path::new("/music/a.flac"),
                "So What",
                Some("Miles".into()),
                Some("Kind of Blue".into()),
                None,
            )
            .unwrap();
        let mut pane = LibraryPane::new(Some(library));
        pane.set_preferred_layout(LibraryLayout::Tree);

        assert_eq!(pane.enter(strip()), EnterResult::Moved, "expand Miles");
        let with_library = pane.tree_rows().len();
        assert!(with_library >= 2, "artist plus its album");

        pane.library = None;
        assert_eq!(
            pane.tree_rows().len(),
            with_library,
            "expanded rows must come from the cache, not the connection"
        );
    }

    #[test]
    fn collapsing_an_artist_drops_its_cached_albums() {
        let mut library = Library::open_in_memory().expect("open");
        library
            .upsert_track_for_test(
                Path::new("/music/a.flac"),
                "So What",
                Some("Miles".into()),
                Some("Kind of Blue".into()),
                None,
            )
            .unwrap();
        let mut pane = LibraryPane::new(Some(library));
        pane.set_preferred_layout(LibraryLayout::Tree);

        assert_eq!(pane.enter(strip()), EnterResult::Moved, "expand Miles");
        assert!(pane.tree_albums_by_artist.contains_key("miles"));

        assert_eq!(pane.enter(strip()), EnterResult::Moved, "collapse Miles");
        assert!(!pane.tree_albums_by_artist.contains_key("miles"));
        assert_eq!(pane.tree_rows().len(), 1);
    }

    #[test]
    fn reload_refreshes_albums_for_every_expanded_artist() {
        let mut library = Library::open_in_memory().expect("open");
        for (path, title, artist, album) in [
            ("/music/1.flac", "Giant Steps", "Coltrane", "Giant Steps"),
            ("/music/2.flac", "So What", "Miles", "Kind of Blue"),
        ] {
            library
                .upsert_track_for_test(
                    Path::new(path),
                    title,
                    Some(artist.into()),
                    Some(album.into()),
                    None,
                )
                .unwrap();
        }
        let mut pane = LibraryPane::new(Some(library));
        pane.set_preferred_layout(LibraryLayout::Tree);

        assert_eq!(pane.enter(strip()), EnterResult::Moved, "expand Coltrane");
        pane.step(1, strip()); // Giant Steps
        pane.step(1, strip()); // Miles
        assert_eq!(pane.enter(strip()), EnterResult::Moved, "expand Miles");

        // A scan adds a second Miles album after his list was cached.
        pane.library
            .as_mut()
            .unwrap()
            .upsert_track_for_test(
                Path::new("/music/3.flac"),
                "Bitches Brew",
                Some("Miles".into()),
                Some("Bitches Brew".into()),
                None,
            )
            .unwrap();

        pane.reload_browse();

        assert!(
            pane.tree_rows().iter().any(
                |row| matches!(row, TreeRow::Album { album, .. } if album.album == "Bitches Brew")
            ),
            "reload must refresh every expanded artist, not only the selected one"
        );
    }

    #[test]
    fn focus_artist_leaves_search() {
        let mut library = Library::open_in_memory().expect("open");
        library
            .upsert_track_for_test(
                Path::new("/music/a.flac"),
                "So What",
                Some("Miles".into()),
                Some("Kind of Blue".into()),
                None,
            )
            .unwrap();
        let mut pane = LibraryPane::new(Some(library));
        pane.inject_search_hits_for_test(
            "miles",
            vec![SearchHit::Artist(ArtistSummary {
                name: "Miles".into(),
                track_count: 1,
            })],
        );
        assert!(pane.focus_artist("Miles"));
        assert_eq!(pane.mode(), &Mode::Browse);
        assert_eq!(pane.paging_level(), ColumnFocus::Albums);
        assert_eq!(pane.albums().len(), 1);
    }

    /// Miles (one album, two tracks) sorts before Portishead (one track).
    fn freddie(pane: &LibraryPane) -> Track {
        pane.locate(Path::new("/music/a2.flac"))
            .expect("in library")
    }

    fn selected_path(pane: &LibraryPane, strip: usize) -> Option<std::path::PathBuf> {
        match pane.selected(strip) {
            Some(Item::Track(track)) => Some(track.path.clone()),
            _ => None,
        }
    }

    #[test]
    fn reveal_selects_artist_album_and_track_in_columns() {
        let mut pane = LibraryPane::new(Some(two_artist_library()));
        let track = freddie(&pane);
        assert!(pane.reveal(&track, wide()));
        assert_eq!(pane.column_focus(), ColumnFocus::Tracks);
        assert_eq!(selected_path(&pane, wide()), Some("/music/a2.flac".into()));
    }

    #[test]
    fn reveal_drills_paging_to_the_track() {
        let mut pane = LibraryPane::new(Some(two_artist_library()));
        let track = freddie(&pane);
        assert!(pane.reveal(&track, strip()));
        assert_eq!(pane.paging_level(), ColumnFocus::Tracks);
        assert_eq!(selected_path(&pane, strip()), Some("/music/a2.flac".into()));
    }

    #[test]
    fn reveal_expands_the_tree_to_the_track() {
        let mut pane = LibraryPane::new(Some(two_artist_library()));
        pane.set_preferred_layout(LibraryLayout::Tree);
        let track = freddie(&pane);
        assert!(pane.reveal(&track, strip()));
        assert_eq!(selected_path(&pane, strip()), Some("/music/a2.flac".into()));
    }

    #[test]
    fn reveal_leaves_search_first() {
        let mut pane = LibraryPane::new(Some(two_artist_library()));
        pane.begin_search();
        for c in "Roads".chars() {
            pane.push_char(c);
        }
        pane.submit_search();
        assert!(matches!(pane.mode(), Mode::Search(_)));
        let track = freddie(&pane);
        assert!(pane.reveal(&track, wide()));
        assert!(matches!(pane.mode(), Mode::Browse));
    }

    #[test]
    fn reveal_without_an_album_fails() {
        let mut pane = LibraryPane::new(Some(two_artist_library()));
        let mut track = freddie(&pane);
        track.album = None;
        assert!(!pane.reveal(&track, wide()));
    }

    fn two_artist_library() -> Library {
        let mut library = Library::open_in_memory().expect("open");
        for (path, title, artist, album) in [
            ("/music/a1.flac", "So What", "Miles", "Kind of Blue"),
            (
                "/music/a2.flac",
                "Freddie Freeloader",
                "Miles",
                "Kind of Blue",
            ),
            ("/music/b1.flac", "Roads", "Portishead", "Dummy"),
        ] {
            library
                .upsert_track_for_test(
                    Path::new(path),
                    title,
                    Some(artist.into()),
                    Some(album.into()),
                    None,
                )
                .unwrap();
        }
        library
    }

    #[test]
    fn h_and_l_step_through_columns_without_wrapping() {
        let mut pane = LibraryPane::new(Some(two_artist_library()));
        assert_eq!(pane.effective_layout(wide()), EffectiveLayout::Columns);

        assert!(!pane.move_horizontal(false, wide()), "h stops at Artists");
        assert_eq!(pane.column_focus(), ColumnFocus::Artists);
        assert!(pane.move_horizontal(true, wide()));
        assert_eq!(pane.column_focus(), ColumnFocus::Albums);
        assert!(pane.move_horizontal(true, wide()));
        assert_eq!(pane.column_focus(), ColumnFocus::Tracks);
        assert!(!pane.move_horizontal(true, wide()), "l stops at Tracks");
        assert_eq!(pane.column_focus(), ColumnFocus::Tracks);
        assert!(pane.move_horizontal(false, wide()));
        assert_eq!(pane.column_focus(), ColumnFocus::Albums);
    }

    #[test]
    fn h_and_l_drill_in_and_back_out_in_paging() {
        let mut pane = LibraryPane::new(Some(two_artist_library()));
        assert_eq!(pane.effective_layout(strip()), EffectiveLayout::Paging);

        assert!(
            !pane.move_horizontal(false, strip()),
            "h at the top does nothing"
        );
        assert!(pane.move_horizontal(true, strip()));
        assert_eq!(pane.paging_level(), ColumnFocus::Albums);
        assert!(pane.move_horizontal(true, strip()));
        assert_eq!(pane.paging_level(), ColumnFocus::Tracks);
        assert!(
            !pane.move_horizontal(true, strip()),
            "l on a track does nothing"
        );
        assert_eq!(pane.paging_level(), ColumnFocus::Tracks);
        assert!(pane.move_horizontal(false, strip()));
        assert_eq!(pane.paging_level(), ColumnFocus::Albums);
        assert!(pane.move_horizontal(false, strip()));
        assert_eq!(pane.paging_level(), ColumnFocus::Artists);
    }

    #[test]
    fn h_and_l_expand_collapse_and_climb_the_tree() {
        let mut pane = LibraryPane::new(Some(two_artist_library()));
        pane.set_preferred_layout(LibraryLayout::Tree);
        let s = strip();
        assert_eq!(pane.tree_rows().len(), 2, "two collapsed artists");

        assert!(pane.move_horizontal(true, s), "l expands the artist");
        assert_eq!(pane.tree_rows().len(), 3);
        assert!(
            !pane.move_horizontal(true, s),
            "l on an open artist does nothing"
        );

        pane.step(1, s); // album row
        assert!(pane.move_horizontal(true, s), "l expands the album");
        assert_eq!(pane.tree_rows().len(), 5);

        pane.step(1, s); // first track
        assert_eq!(pane.selected_index(s), Some(2));
        assert!(!pane.move_horizontal(true, s), "l on a track does nothing");

        assert!(
            pane.move_horizontal(false, s),
            "h on a track climbs to its album"
        );
        assert_eq!(pane.selected_index(s), Some(1));
        assert!(
            pane.move_horizontal(false, s),
            "h on an open album collapses it"
        );
        assert_eq!(pane.tree_rows().len(), 3);
        assert_eq!(pane.selected_index(s), Some(1));
        assert!(
            pane.move_horizontal(false, s),
            "h on a closed album climbs to the artist"
        );
        assert_eq!(pane.selected_index(s), Some(0));
        assert!(
            pane.move_horizontal(false, s),
            "h on an open artist collapses it"
        );
        assert_eq!(pane.tree_rows().len(), 2);
        assert!(
            !pane.move_horizontal(false, s),
            "h on a closed top-level artist does nothing"
        );
    }

    #[test]
    fn h_and_l_do_nothing_in_search_results() {
        let mut pane = LibraryPane::new(None);
        pane.inject_search_hits_for_test(
            "Love",
            vec![SearchHit::Artist(ArtistSummary {
                name: "Love".into(),
                track_count: 1,
            })],
        );
        assert!(!pane.move_horizontal(true, wide()));
        assert!(!pane.move_horizontal(false, wide()));
        assert!(matches!(pane.mode(), Mode::Search(_)));
    }
}
