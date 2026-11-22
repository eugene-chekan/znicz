//! Gapless playback: tracks already decoded into the output buffer but not
//! yet heard.
//!
//! When a local track finishes decoding, the engine opens the next one and
//! keeps feeding the same stream. The buffer then holds the end of one track
//! and the start of the next. A `PendingTrack` remembers where the next one
//! begins, so the shown track switches when the speakers get there.

use std::collections::VecDeque;

use crate::player::state::{QueueItem, TrackInfo};

/// A chained track waiting for its first sample to be played.
#[derive(Debug, Clone)]
pub struct PendingTrack {
    /// Queue row it was picked from.
    pub index: usize,
    pub item: QueueItem,
    pub info: TrackInfo,
    /// Feeder sample count (all channels) at the track's first sample,
    /// counted from the start of the track before it.
    pub starts_at: u64,
}

/// Chained tracks in play order.
#[derive(Debug, Default)]
pub struct PendingTracks {
    tracks: VecDeque<PendingTrack>,
}

impl PendingTracks {
    pub fn push(&mut self, track: PendingTrack) {
        self.tracks.push_back(track);
    }

    pub fn clear(&mut self) {
        self.tracks.clear();
    }

    pub fn is_empty(&self) -> bool {
        self.tracks.is_empty()
    }

    /// The last chained track: the next pick follows it.
    pub fn last(&self) -> Option<&PendingTrack> {
        self.tracks.back()
    }

    /// The first pending track once `played` has reached its first sample.
    /// The remaining tracks are shifted to count from the returned one, as the
    /// caller does with the feeder.
    pub fn take_started(&mut self, played: u64) -> Option<PendingTrack> {
        if self.tracks.front()?.starts_at > played {
            return None;
        }
        let started = self.tracks.pop_front()?;
        for track in &mut self.tracks {
            track.starts_at -= started.starts_at;
        }
        Some(started)
    }
}

/// Whether the track after the one that just finished decoding may join it
/// in the same stream. Repeat-one loops seamlessly; radio keeps stop-and-reopen.
pub fn may_chain(current_is_stream: bool) -> bool {
    !current_is_stream
}

/// Where a chained track sits in the queue now. The queue may have been
/// edited since it was picked.
pub fn resolve_position(queue: &[QueueItem], index: usize, item: &QueueItem) -> Option<usize> {
    if queue.get(index) == Some(item) {
        return Some(index);
    }
    queue.iter().position(|row| row == item)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(name: &str) -> QueueItem {
        QueueItem::file(format!("/music/{name}.flac"))
    }

    fn pending(index: usize, name: &str, starts_at: u64) -> PendingTrack {
        PendingTrack {
            index,
            item: file(name),
            info: TrackInfo {
                path: None,
                url: None,
                icy_stream_url: None,
                title: name.to_string(),
                codec: "FLAC".into(),
                sample_rate: 44_100,
                channels: 2,
                bits_per_sample: Some(16),
                bitrate_kbps: None,
                duration: None,
                tags: Default::default(),
            },
            starts_at,
        }
    }

    /// The parts of a taken track the tests compare.
    fn key(track: Option<PendingTrack>) -> Option<(usize, u64)> {
        track.map(|track| (track.index, track.starts_at))
    }

    #[test]
    fn waits_until_the_first_sample_is_played() {
        let mut tracks = PendingTracks::default();
        tracks.push(pending(1, "b", 1000));

        assert_eq!(key(tracks.take_started(999)), None);
        assert_eq!(key(tracks.take_started(1000)), Some((1, 1000)));
        assert!(tracks.is_empty());
    }

    #[test]
    fn later_tracks_count_from_the_one_taken() {
        let mut tracks = PendingTracks::default();
        tracks.push(pending(1, "b", 1000));
        tracks.push(pending(2, "c", 1500));

        assert_eq!(key(tracks.take_started(1200)), Some((1, 1000)));
        // Played is now counted from b: 200. c starts 500 into b.
        assert_eq!(key(tracks.take_started(200)), None);
        assert_eq!(key(tracks.take_started(500)), Some((2, 500)));
    }

    #[test]
    fn last_follows_the_latest_chain() {
        let mut tracks = PendingTracks::default();
        assert!(tracks.last().is_none());
        tracks.push(pending(3, "d", 10));
        tracks.push(pending(4, "e", 20));
        assert_eq!(tracks.last().map(|track| track.index), Some(4));
        tracks.clear();
        assert!(tracks.is_empty());
        assert!(tracks.last().is_none());
    }

    #[test]
    fn radio_does_not_chain() {
        assert!(may_chain(false));
        assert!(!may_chain(true));
    }

    #[test]
    fn an_unchanged_row_keeps_its_index() {
        let queue = vec![file("a"), file("b")];
        assert_eq!(resolve_position(&queue, 1, &file("b")), Some(1));
    }

    #[test]
    fn a_moved_row_is_found_again() {
        let queue = vec![file("b"), file("a")];
        assert_eq!(resolve_position(&queue, 1, &file("b")), Some(0));
    }

    #[test]
    fn a_removed_row_has_no_position() {
        let queue = vec![file("a")];
        assert_eq!(resolve_position(&queue, 1, &file("b")), None);
    }
}
