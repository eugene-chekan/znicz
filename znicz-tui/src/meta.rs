//! Titles and durations for queue entries, resolved off the UI thread.
//!
//! The player's queue is a list of paths, but a music player has to show
//! "Artist — Title", not `04 - track.flac`. Reading tags takes a file open and
//! a seek each, so doing it while drawing would stutter on a large queue.
//! Instead the UI asks for a path, gets whatever is known now, and a worker
//! thread fills in the rest for the next frame.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crossbeam_channel::{Sender, unbounded};

/// What the UI needs to draw one queue row.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Entry {
    pub title: String,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub duration: Option<Duration>,
}

impl Entry {
    /// "Artist — Title" when the artist is known, otherwise just the title.
    pub fn label(&self) -> String {
        match self.artist.as_deref() {
            Some(artist) => format!("{artist} — {}", self.title),
            None => self.title.clone(),
        }
    }
}

type Cache = Arc<Mutex<HashMap<PathBuf, Option<Entry>>>>;

/// A lazily filled tag cache backed by one worker thread.
pub struct MetaCache {
    cache: Cache,
    requests: Sender<PathBuf>,
}

impl Default for MetaCache {
    fn default() -> Self {
        Self::new()
    }
}

impl MetaCache {
    pub fn new() -> Self {
        let cache: Cache = Arc::new(Mutex::new(HashMap::new()));
        let (requests, incoming) = unbounded::<PathBuf>();

        let worker_cache = cache.clone();
        // Detached on purpose: the channel closes when the app drops the
        // cache, which ends the loop and the thread with it.
        std::thread::Builder::new()
            .name("znicz-tags".into())
            .spawn(move || {
                while let Ok(path) = incoming.recv() {
                    let entry = read_entry(&path);
                    worker_cache.lock().unwrap().insert(path, Some(entry));
                }
            })
            .expect("failed to spawn tag reader thread");

        Self { cache, requests }
    }

    /// Tags for a path, or `None` while the worker is still reading them.
    ///
    /// A miss queues the work, so simply drawing the queue is enough to get it
    /// filled in.
    pub fn get(&self, path: &Path) -> Option<Entry> {
        let mut cache = self.cache.lock().unwrap();
        if let Some(entry) = cache.get(path) {
            return entry.clone();
        }
        // Mark as pending so a redraw does not queue the same file again.
        cache.insert(path.to_path_buf(), None);
        drop(cache);

        self.requests.send(path.to_path_buf()).ok();
        None
    }

    /// Store tags we already have, such as a row from the library database.
    /// Saves the worker from reading a file we have read once already.
    pub fn insert(&self, path: PathBuf, entry: Entry) {
        self.cache.lock().unwrap().insert(path, Some(entry));
    }

    /// Rows resolved so far, used by the tests and for diagnostics.
    pub fn resolved_count(&self) -> usize {
        self.cache
            .lock()
            .unwrap()
            .values()
            .filter(|v| v.is_some())
            .count()
    }
}

fn read_entry(path: &Path) -> Entry {
    let meta = znicz_core::read_metadata(path);
    Entry {
        title: meta
            .tags
            .title
            .clone()
            .unwrap_or_else(|| znicz_core::title_from_path(path)),
        artist: meta.tags.artist.clone(),
        album: meta.tags.album.clone(),
        duration: meta.properties.duration,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_first_look_misses_then_resolves_in_the_background() {
        let cache = MetaCache::new();
        let path = PathBuf::from("/definitely/not/a/real/song.flac");

        assert_eq!(cache.get(&path), None, "nothing is known yet");

        // The worker still produces an entry for an unreadable file, falling
        // back to the file name, so the row never stays blank.
        let mut entry = None;
        for _ in 0..100 {
            std::thread::sleep(Duration::from_millis(10));
            if let Some(found) = cache.get(&path) {
                entry = Some(found);
                break;
            }
        }

        let entry = entry.expect("worker should have answered");
        assert_eq!(entry.title, "song");
    }

    #[test]
    fn known_tags_can_be_supplied_directly() {
        let cache = MetaCache::new();
        let path = PathBuf::from("/music/a.flac");
        cache.insert(
            path.clone(),
            Entry {
                title: "Kashmir".into(),
                artist: Some("Led Zeppelin".into()),
                album: None,
                duration: None,
            },
        );

        let entry = cache
            .get(&path)
            .expect("inserted entries are available at once");
        assert_eq!(entry.label(), "Led Zeppelin — Kashmir");
    }

    #[test]
    fn a_row_without_an_artist_shows_just_the_title() {
        let entry = Entry {
            title: "Untitled".into(),
            ..Entry::default()
        };
        assert_eq!(entry.label(), "Untitled");
    }
}
