use std::path::{Path, PathBuf};

use rusqlite::{Connection, OptionalExtension, params};

use crate::error::Result;
use crate::track::{AlbumSummary, Track};

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
    modified_secs  INTEGER
);
CREATE INDEX IF NOT EXISTS tracks_album_idx  ON tracks(album);
CREATE INDEX IF NOT EXISTS tracks_artist_idx ON tracks(artist);
CREATE INDEX IF NOT EXISTS tracks_title_idx  ON tracks(title);
";

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
        Ok(Self { conn })
    }

    pub(crate) fn connection(&mut self) -> &mut Connection {
        &mut self.conn
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
    pub fn search(&self, query: &str, limit: usize) -> Result<Vec<Track>> {
        let pattern = format!("%{}%", escape_like(query));
        let sql = format!(
            "SELECT {COLUMNS} FROM tracks
             WHERE title LIKE ?1 ESCAPE '\\'
                OR artist LIKE ?1 ESCAPE '\\'
                OR album LIKE ?1 ESCAPE '\\'
                OR album_artist LIKE ?1 ESCAPE '\\'
             ORDER BY artist, album, disc_number, track_number, title
             LIMIT ?2"
        );

        let mut statement = self.conn.prepare(&sql)?;
        let rows = statement.query_map(params![pattern, limit as i64], row_to_track)?;
        rows.collect::<rusqlite::Result<Vec<_>>>().map_err(Into::into)
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
        rows.collect::<rusqlite::Result<Vec<_>>>().map_err(Into::into)
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

        rows.collect::<rusqlite::Result<Vec<_>>>().map_err(Into::into)
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
}
