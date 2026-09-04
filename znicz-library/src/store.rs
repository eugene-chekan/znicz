use std::path::{Path, PathBuf};

use rusqlite::{params, Connection, OptionalExtension};

use crate::error::Result;
use crate::track::{AlbumSummary, ArtistSummary, SearchHit, SearchLimits, Track};

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS tracks (
    id             INTEGER PRIMARY KEY,
    path           TEXT NOT NULL UNIQUE,
    title          TEXT NOT NULL,
    artist         TEXT,
    album          TEXT,
    album_artist   TEXT,
    genre          TEXT,
    year           INTEGER,
    track_number   INTEGER,
    disc_number    INTEGER,
    codec          TEXT,
    sample_rate    INTEGER,
    channels       INTEGER,
    bits_per_sample INTEGER,
    duration_secs  REAL,
    modified_secs  INTEGER,
    -- Unicode-lowercased copies for search. SQLite LIKE only folds ASCII.
    title_folded        TEXT,
    artist_folded       TEXT,
    album_folded        TEXT,
    album_artist_folded TEXT
);
CREATE INDEX IF NOT EXISTS tracks_album_idx  ON tracks(album);
CREATE INDEX IF NOT EXISTS tracks_artist_idx ON tracks(artist);
CREATE INDEX IF NOT EXISTS tracks_title_idx  ON tracks(title);
";

const FOLDED_COLUMNS: &[&str] = &[
    "title_folded",
    "artist_folded",
    "album_folded",
    "album_artist_folded",
];

/// Columns in the order `row_to_track` expects them.
const COLUMNS: &str = "id, path, title, artist, album, album_artist, genre, year, \
     track_number, disc_number, codec, sample_rate, channels, bits_per_sample, duration_secs";

pub struct Library {
    conn: Connection,
}

impl Library {
    /// Open (and create if needed) a library database.
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(path)?;
        Self::prepare(conn)
    }

    /// A throwaway library, used by tests.
    pub fn open_in_memory() -> Result<Self> {
        Self::prepare(Connection::open_in_memory()?)
    }

    fn prepare(conn: Connection) -> Result<Self> {
        // WAL keeps reads working while a scan writes.
        conn.pragma_update(None, "journal_mode", "WAL").ok();
        conn.pragma_update(None, "synchronous", "NORMAL").ok();
        conn.execute_batch(SCHEMA)?;
        ensure_folded_columns(&conn)?;
        let library = Self { conn };
        library.backfill_folded_columns()?;
        Ok(library)
    }

    /// Insert or refresh one indexed file, including Unicode-folded search text.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn upsert_track(
        &mut self,
        path: &Path,
        title: &str,
        artist: Option<String>,
        album: Option<String>,
        album_artist: Option<String>,
        genre: Option<String>,
        year: Option<u32>,
        track_number: Option<u32>,
        disc_number: Option<u32>,
        codec: Option<String>,
        sample_rate: Option<u32>,
        channels: Option<u16>,
        bits_per_sample: Option<u32>,
        duration_secs: Option<f64>,
        modified_secs: Option<i64>,
    ) -> Result<()> {
        let title_folded = fold_text(title);
        let artist_folded = artist.as_deref().map(fold_text);
        let album_folded = album.as_deref().map(fold_text);
        let album_artist_folded = album_artist.as_deref().map(fold_text);

        self.conn.execute(
            "INSERT INTO tracks (
                path, title, artist, album, album_artist, genre, year,
                track_number, disc_number, codec, sample_rate, channels,
                bits_per_sample, duration_secs, modified_secs,
                title_folded, artist_folded, album_folded, album_artist_folded
             ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15,
                ?16, ?17, ?18, ?19
             )
             ON CONFLICT(path) DO UPDATE SET
                title = excluded.title,
                artist = excluded.artist,
                album = excluded.album,
                album_artist = excluded.album_artist,
                genre = excluded.genre,
                year = excluded.year,
                track_number = excluded.track_number,
                disc_number = excluded.disc_number,
                codec = excluded.codec,
                sample_rate = excluded.sample_rate,
                channels = excluded.channels,
                bits_per_sample = excluded.bits_per_sample,
                duration_secs = excluded.duration_secs,
                modified_secs = excluded.modified_secs,
                title_folded = excluded.title_folded,
                artist_folded = excluded.artist_folded,
                album_folded = excluded.album_folded,
                album_artist_folded = excluded.album_artist_folded",
            params![
                path_str(path),
                title,
                artist,
                album,
                album_artist,
                genre,
                year,
                track_number,
                disc_number,
                codec,
                sample_rate,
                channels,
                bits_per_sample,
                duration_secs,
                modified_secs,
                title_folded,
                artist_folded,
                album_folded,
                album_artist_folded,
            ],
        )?;
        Ok(())
    }

    pub fn track_count(&self) -> Result<u64> {
        let count: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM tracks", [], |row| row.get(0))?;
        Ok(count as u64)
    }

    /// The stored modification time, used to skip unchanged files on a rescan.
    pub(crate) fn modified_secs(&self, path: &Path) -> Result<Option<i64>> {
        let value = self
            .conn
            .query_row(
                "SELECT modified_secs FROM tracks WHERE path = ?1",
                params![path_str(path)],
                |row| row.get::<_, Option<i64>>(0),
            )
            .optional()?;
        Ok(value.flatten())
    }

    /// Free text search across title, artist and album.
    ///
    /// Matching uses Unicode-lowercased copies of the tags. SQLite's own `LIKE`
    /// only folds ASCII, so a lowercase Cyrillic query would otherwise miss
    /// capitalized tags.
    pub fn search(&self, query: &str, limit: usize) -> Result<Vec<Track>> {
        let pattern = format!("%{}%", escape_like(&fold_text(query)));
        let sql = format!(
            "SELECT {COLUMNS} FROM tracks
             WHERE title_folded LIKE ?1 ESCAPE '\\'
                OR artist_folded LIKE ?1 ESCAPE '\\'
                OR album_folded LIKE ?1 ESCAPE '\\'
                OR album_artist_folded LIKE ?1 ESCAPE '\\'
             ORDER BY artist, album, disc_number, track_number, title
             LIMIT ?2"
        );

        let mut statement = self.conn.prepare(&sql)?;
        let rows = statement.query_map(params![pattern, limit as i64], row_to_track)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    /// Entity search for the TUI: distinct artists, distinct albums, then
    /// title-only track hits. Flat [`Self::search`] stays for MCP/CLI.
    pub fn search_entities(
        &self,
        query: &str,
        limits: SearchLimits,
    ) -> Result<Vec<SearchHit>> {
        let trimmed = query.trim();
        if trimmed.is_empty() {
            return Ok(Vec::new());
        }

        let pattern = format!("%{}%", escape_like(&fold_text(trimmed)));
        let mut hits = Vec::new();

        if limits.artists > 0 {
            let mut statement = self.conn.prepare(
                "SELECT MAX(name), COUNT(*)
                 FROM (
                     SELECT path, artist AS name FROM tracks
                     WHERE artist IS NOT NULL AND artist <> ''
                       AND artist_folded LIKE ?1 ESCAPE '\\'
                     UNION
                     SELECT path, album_artist AS name FROM tracks
                     WHERE album_artist IS NOT NULL AND album_artist <> ''
                       AND album_artist_folded LIKE ?1 ESCAPE '\\'
                 )
                 GROUP BY name COLLATE NOCASE
                 ORDER BY MAX(name) COLLATE NOCASE
                 LIMIT ?2",
            )?;
            let rows = statement.query_map(params![pattern, limits.artists as i64], |row| {
                Ok(SearchHit::Artist(ArtistSummary {
                    name: row.get(0)?,
                    track_count: row.get::<_, i64>(1)? as u32,
                }))
            })?;
            for row in rows {
                hits.push(row?);
            }
        }

        if limits.albums > 0 {
            let mut statement = self.conn.prepare(
                "SELECT album,
                        MAX(COALESCE(album_artist, artist)),
                        MAX(year),
                        COUNT(*),
                        SUM(duration_secs)
                 FROM tracks
                 WHERE album IS NOT NULL AND album <> ''
                   AND album_folded LIKE ?1 ESCAPE '\\'
                 GROUP BY album COLLATE NOCASE
                 ORDER BY album COLLATE NOCASE
                 LIMIT ?2",
            )?;
            let rows = statement.query_map(params![pattern, limits.albums as i64], |row| {
                Ok(SearchHit::Album(AlbumSummary {
                    album: row.get(0)?,
                    album_artist: row.get(1)?,
                    year: row.get::<_, Option<i64>>(2)?.map(|y| y as u32),
                    track_count: row.get::<_, i64>(3)? as u32,
                    total_secs: row.get(4)?,
                }))
            })?;
            for row in rows {
                hits.push(row?);
            }
        }

        if limits.tracks > 0 {
            let sql = format!(
                "SELECT {COLUMNS} FROM tracks
                 WHERE title_folded LIKE ?1 ESCAPE '\\'
                 ORDER BY artist, album, disc_number, track_number, title
                 LIMIT ?2"
            );
            let mut statement = self.conn.prepare(&sql)?;
            let rows =
                statement.query_map(params![pattern, limits.tracks as i64], row_to_track)?;
            for row in rows {
                hits.push(SearchHit::Track(row?));
            }
        }

        Ok(hits)
    }

    /// Tracks attributed to an artist name via `artist` or `album_artist`.
    pub fn tracks_for_artist(&self, name: &str) -> Result<Vec<Track>> {
        let sql = format!(
            "SELECT {COLUMNS} FROM tracks
             WHERE artist = ?1 COLLATE NOCASE
                OR album_artist = ?1 COLLATE NOCASE
             ORDER BY artist, album, disc_number, track_number, title"
        );
        let mut statement = self.conn.prepare(&sql)?;
        let rows = statement.query_map(params![name], row_to_track)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    /// Tracks in listening order, for libraries whose files carry no album tag.
    ///
    /// Without this, a folder of untagged files is indexed but invisible: album
    /// grouping has nothing to group by.
    pub fn all_tracks(&self, limit: usize) -> Result<Vec<Track>> {
        let sql = format!(
            "SELECT {COLUMNS} FROM tracks
             ORDER BY artist, album, disc_number, track_number, title
             LIMIT ?1"
        );

        let mut statement = self.conn.prepare(&sql)?;
        let rows = statement.query_map(params![limit as i64], row_to_track)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    /// Look up one track by its file path.
    pub fn get_by_path(&self, path: &Path) -> Result<Option<Track>> {
        let sql = format!("SELECT {COLUMNS} FROM tracks WHERE path = ?1");
        let track = self
            .conn
            .query_row(&sql, params![path_str(path)], row_to_track)
            .optional()?;
        Ok(track)
    }

    pub fn get_by_id(&self, id: i64) -> Result<Option<Track>> {
        let sql = format!("SELECT {COLUMNS} FROM tracks WHERE id = ?1");
        let track = self
            .conn
            .query_row(&sql, params![id], row_to_track)
            .optional()?;
        Ok(track)
    }

    /// Tracks of an album, in disc and track order.
    pub fn browse_album(&self, album: &str) -> Result<Vec<Track>> {
        let sql = format!(
            "SELECT {COLUMNS} FROM tracks
             WHERE album = ?1 COLLATE NOCASE
             ORDER BY disc_number, track_number, title"
        );
        let mut statement = self.conn.prepare(&sql)?;
        let rows = statement.query_map(params![album], row_to_track)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    /// All albums with a track count.
    pub fn albums(&self) -> Result<Vec<AlbumSummary>> {
        let mut statement = self.conn.prepare(
            "SELECT album,
                    MAX(COALESCE(album_artist, artist)),
                    MAX(year),
                    COUNT(*),
                    SUM(duration_secs)
             FROM tracks
             WHERE album IS NOT NULL AND album <> ''
             GROUP BY album COLLATE NOCASE
             ORDER BY album COLLATE NOCASE",
        )?;

        let rows = statement.query_map([], |row| {
            Ok(AlbumSummary {
                album: row.get(0)?,
                album_artist: row.get(1)?,
                year: row.get::<_, Option<i64>>(2)?.map(|y| y as u32),
                track_count: row.get::<_, i64>(3)? as u32,
                total_secs: row.get(4)?,
            })
        })?;

        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    /// Remove rows whose file no longer exists. Returns how many were dropped.
    pub fn remove_missing(&mut self) -> Result<usize> {
        let paths: Vec<String> = {
            let mut statement = self.conn.prepare("SELECT path FROM tracks")?;
            let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
        };

        let mut removed = 0;
        let transaction = self.conn.transaction()?;
        for path in paths {
            if !Path::new(&path).exists() {
                transaction.execute("DELETE FROM tracks WHERE path = ?1", params![path])?;
                removed += 1;
            }
        }
        transaction.commit()?;
        Ok(removed)
    }

    pub fn clear(&mut self) -> Result<()> {
        self.conn.execute("DELETE FROM tracks", [])?;
        Ok(())
    }

    /// Fill folded search columns for rows written before Unicode search existed.
    fn backfill_folded_columns(&self) -> Result<()> {
        let mut statement = self.conn.prepare(
            "SELECT id, title, artist, album, album_artist FROM tracks
             WHERE title_folded IS NULL",
        )?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, Option<String>>(4)?,
            ))
        })?;
        let pending: Vec<_> = rows.collect::<rusqlite::Result<Vec<_>>>()?;

        for (id, title, artist, album, album_artist) in pending {
            self.conn.execute(
                "UPDATE tracks SET
                    title_folded = ?1,
                    artist_folded = ?2,
                    album_folded = ?3,
                    album_artist_folded = ?4
                 WHERE id = ?5",
                params![
                    fold_text(&title),
                    artist.as_deref().map(fold_text),
                    album.as_deref().map(fold_text),
                    album_artist.as_deref().map(fold_text),
                    id,
                ],
            )?;
        }
        Ok(())
    }
}

/// Path as text. Lossy only for non-UTF-8 paths, which we cannot store anyway.
pub(crate) fn path_str(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

/// `%` and `_` are wildcards in LIKE, so a literal search must escape them.
fn escape_like(query: &str) -> String {
    query
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

/// Unicode-aware lowercase for search. Unlike SQLite's `lower()`, this folds
/// Cyrillic and other non-ASCII letters.
fn fold_text(value: &str) -> String {
    value.to_lowercase()
}

fn ensure_folded_columns(conn: &Connection) -> Result<()> {
    let existing = table_columns(conn)?;
    for column in FOLDED_COLUMNS {
        if !existing.iter().any(|name| name == *column) {
            conn.execute(&format!("ALTER TABLE tracks ADD COLUMN {column} TEXT"), [])?;
        }
    }
    Ok(())
}

fn table_columns(conn: &Connection) -> Result<Vec<String>> {
    let mut statement = conn.prepare("PRAGMA table_info(tracks)")?;
    let rows = statement.query_map([], |row| row.get::<_, String>(1))?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(Into::into)
}

fn row_to_track(row: &rusqlite::Row<'_>) -> rusqlite::Result<Track> {
    Ok(Track {
        id: row.get(0)?,
        path: PathBuf::from(row.get::<_, String>(1)?),
        title: row.get(2)?,
        artist: row.get(3)?,
        album: row.get(4)?,
        album_artist: row.get(5)?,
        genre: row.get(6)?,
        year: row.get::<_, Option<i64>>(7)?.map(|v| v as u32),
        track_number: row.get::<_, Option<i64>>(8)?.map(|v| v as u32),
        disc_number: row.get::<_, Option<i64>>(9)?.map(|v| v as u32),
        codec: row.get(10)?,
        sample_rate: row.get::<_, Option<i64>>(11)?.map(|v| v as u32),
        channels: row.get::<_, Option<i64>>(12)?.map(|v| v as u16),
        bits_per_sample: row.get::<_, Option<i64>>(13)?.map(|v| v as u32),
        duration_secs: row.get(14)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn like_wildcards_are_escaped() {
        assert_eq!(escape_like("100%"), "100\\%");
        assert_eq!(escape_like("a_b"), "a\\_b");
        assert_eq!(escape_like("plain"), "plain");
    }

    #[test]
    fn empty_library_has_no_tracks() {
        let library = Library::open_in_memory().expect("open");
        assert_eq!(library.track_count().unwrap(), 0);
        assert!(library.search("anything", 10).unwrap().is_empty());
        assert!(library.albums().unwrap().is_empty());
    }

    /// SQLite LIKE only folds ASCII. Cyrillic must still match case-insensitively
    /// (see GitHub #44).
    #[test]
    fn search_matches_cyrillic_case_insensitively() {
        let mut library = Library::open_in_memory().expect("open");
        library
            .upsert_track(
                Path::new("/music/lyapis.flac"),
                "Ау",
                Some("Ляпис Трубецкой".into()),
                Some("Веселые Картинки".into()),
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                Some(0),
            )
            .expect("upsert");

        let lower = library.search("веселые", 10).expect("lower");
        assert_eq!(
            lower.len(),
            1,
            "lowercase Cyrillic query should match capitalized album, got {lower:?}"
        );
        assert_eq!(lower[0].album.as_deref(), Some("Веселые Картинки"));

        let upper = library.search("ВЕСЕЛЫЕ", 10).expect("upper");
        assert_eq!(upper.len(), 1);

        let artist = library.search("ляпис", 10).expect("artist");
        assert_eq!(artist.len(), 1);

        library
            .upsert_track(
                Path::new("/music/dummy.flac"),
                "Mysterons",
                Some("Portishead".into()),
                Some("Dummy".into()),
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                Some(0),
            )
            .expect("upsert ascii");
        assert_eq!(library.search("portishead", 10).expect("ascii").len(), 1);
    }

    #[test]
    fn opening_backfills_folded_columns_for_legacy_rows() {
        let dir = std::env::temp_dir().join("znicz-library-fold-migrate");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("dir");
        let path = dir.join("library.db");

        {
            let conn = Connection::open(&path).expect("create legacy db");
            conn.execute_batch(
                "CREATE TABLE tracks (
                    id INTEGER PRIMARY KEY,
                    path TEXT NOT NULL UNIQUE,
                    title TEXT NOT NULL,
                    artist TEXT,
                    album TEXT,
                    album_artist TEXT,
                    genre TEXT,
                    year INTEGER,
                    track_number INTEGER,
                    disc_number INTEGER,
                    codec TEXT,
                    sample_rate INTEGER,
                    channels INTEGER,
                    bits_per_sample INTEGER,
                    duration_secs REAL,
                    modified_secs INTEGER
                );",
            )
            .expect("legacy schema");
            conn.execute(
                "INSERT INTO tracks (path, title, artist, album, modified_secs)
                 VALUES (?1, ?2, ?3, ?4, 0)",
                params![
                    "/music/lyapis.flac",
                    "Ау",
                    "Ляпис Трубецкой",
                    "Веселые Картинки",
                ],
            )
            .expect("legacy insert");
        }

        let library = Library::open(&path).expect("open migrates");
        let found = library.search("веселые", 10).expect("search");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].album.as_deref(), Some("Веселые Картинки"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    fn seed_entity_fixture(library: &mut Library) {
        library
            .upsert_track(
                Path::new("/music/love1.flac"),
                "Alone",
                Some("Love".into()),
                Some("Forever".into()),
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                Some(0),
            )
            .unwrap();
        library
            .upsert_track(
                Path::new("/music/love2.flac"),
                "Together",
                Some("Love".into()),
                Some("Forever".into()),
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                Some(0),
            )
            .unwrap();
        library
            .upsert_track(
                Path::new("/music/album-love.flac"),
                "Opening",
                Some("Other".into()),
                Some("Love".into()),
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                Some(0),
            )
            .unwrap();
        library
            .upsert_track(
                Path::new("/music/title-love.flac"),
                "Love Song",
                Some("Singer".into()),
                Some("Hits".into()),
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                Some(0),
            )
            .unwrap();
    }

    #[test]
    fn search_entities_artist_query_returns_artist_not_every_track() {
        let mut library = Library::open_in_memory().unwrap();
        seed_entity_fixture(&mut library);
        let hits = library
            .search_entities("Love", SearchLimits::default())
            .unwrap();

        let artists: Vec<_> = hits
            .iter()
            .filter_map(|h| match h {
                SearchHit::Artist(a) => Some(a.name.as_str()),
                _ => None,
            })
            .collect();
        assert!(artists.iter().any(|n| n.eq_ignore_ascii_case("Love")));

        let albums: Vec<_> = hits
            .iter()
            .filter_map(|h| match h {
                SearchHit::Album(a) => Some(a.album.as_str()),
                _ => None,
            })
            .collect();
        assert!(albums.iter().any(|n| n.eq_ignore_ascii_case("Love")));

        let track_titles: Vec<_> = hits
            .iter()
            .filter_map(|h| match h {
                SearchHit::Track(t) => Some(t.title.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(track_titles, vec!["Love Song"]);
        assert!(!track_titles.iter().any(|t| *t == "Alone" || *t == "Together"));
    }

    #[test]
    fn search_entities_orders_artists_then_albums_then_tracks() {
        let mut library = Library::open_in_memory().unwrap();
        seed_entity_fixture(&mut library);
        let hits = library
            .search_entities("Love", SearchLimits::default())
            .unwrap();
        let mut seen_album = false;
        let mut seen_track = false;
        for hit in &hits {
            match hit {
                SearchHit::Artist(_) => assert!(!seen_album && !seen_track),
                SearchHit::Album(_) => {
                    seen_album = true;
                    assert!(!seen_track);
                }
                SearchHit::Track(_) => seen_track = true,
            }
        }
        assert!(seen_album && seen_track);
    }

    #[test]
    fn search_entities_cyrillic_fold_matches_artist() {
        let mut library = Library::open_in_memory().unwrap();
        library
            .upsert_track(
                Path::new("/music/lyapis.flac"),
                "Ау",
                Some("Ляпис Трубецкой".into()),
                Some("Веселые Картинки".into()),
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                Some(0),
            )
            .unwrap();
        let hits = library
            .search_entities("ляпис", SearchLimits::default())
            .unwrap();
        assert!(
            matches!(&hits[..], [SearchHit::Artist(a), ..] if a.name.contains("Ляпис")),
            "got {hits:?}"
        );
        assert!(hits.iter().all(|h| !matches!(h, SearchHit::Track(_))));
    }

    #[test]
    fn tracks_for_artist_matches_artist_or_album_artist() {
        let mut library = Library::open_in_memory().unwrap();
        library
            .upsert_track(
                Path::new("/music/a.flac"),
                "One",
                Some("Band".into()),
                Some("LP".into()),
                Some("Various".into()),
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                Some(0),
            )
            .unwrap();
        library
            .upsert_track(
                Path::new("/music/b.flac"),
                "Two",
                Some("Other".into()),
                Some("Comp".into()),
                Some("Band".into()),
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                Some(0),
            )
            .unwrap();
        let tracks = library.tracks_for_artist("band").unwrap();
        assert_eq!(tracks.len(), 2);
    }

    #[test]
    fn empty_entity_query_returns_no_hits() {
        let library = Library::open_in_memory().unwrap();
        assert!(library
            .search_entities("  ", SearchLimits::default())
            .unwrap()
            .is_empty());
    }
}
