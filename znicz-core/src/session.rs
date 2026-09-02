//! Live session: queue and transport extras across restarts.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::error::{Result, ZniczError};
use crate::player::commands::Command;
use crate::player::engine::PlayerHandle;
use crate::player::state::{PlayerState, QueueItem, RepeatMode};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Session {
    #[serde(default)]
    pub queue: Vec<QueueItem>,
    #[serde(default)]
    pub queue_position: usize,
    #[serde(default = "default_volume")]
    pub volume: f32,
    #[serde(default)]
    pub muted: bool,
    #[serde(default)]
    pub repeat: RepeatMode,
    #[serde(default)]
    pub shuffle: bool,
}

fn default_volume() -> f32 {
    1.0
}

impl Default for Session {
    fn default() -> Self {
        Self {
            queue: Vec::new(),
            queue_position: 0,
            volume: 1.0,
            muted: false,
            repeat: RepeatMode::Off,
            shuffle: false,
        }
    }
}

impl Session {
    pub fn from_state(state: &PlayerState) -> Self {
        Self {
            queue: state.queue.clone(),
            queue_position: state.queue_position,
            volume: state.volume,
            muted: state.muted,
            repeat: state.repeat,
            shuffle: state.shuffle,
        }
    }
}

/// Drop local files that are not on disk. Stream rows stay. Clamps the index.
/// Returns how many file rows were skipped.
pub fn prune_missing(session: &mut Session) -> usize {
    let before = session.queue.len();
    let mut kept = Vec::with_capacity(session.queue.len());
    let mut new_position = 0usize;
    for (index, item) in session.queue.iter().cloned().enumerate() {
        let keep = match &item {
            QueueItem::File { path } => path.is_file(),
            QueueItem::Stream { .. } => true,
        };
        if keep {
            if index < session.queue_position {
                new_position += 1;
            } else if index == session.queue_position {
                new_position = kept.len();
            }
            kept.push(item);
        }
    }
    let skipped = before - kept.len();
    session.queue = kept;
    session.queue_position = if session.queue.is_empty() {
        0
    } else {
        new_position.min(session.queue.len() - 1)
    };
    skipped
}

pub fn load(path: &Path) -> Result<Session> {
    if !path.is_file() {
        return Ok(Session::default());
    }
    let text = fs::read_to_string(path)?;
    match toml::from_str(&text) {
        Ok(session) => Ok(session),
        Err(e) => {
            tracing::warn!(path = %path.display(), error = %e, "session.toml unreadable; starting empty");
            Ok(Session::default())
        }
    }
}

pub fn save(path: &Path, session: &Session) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let text = toml::to_string_pretty(session)
        .map_err(|e| ZniczError::Player(format!("session.toml: {e}")))?;
    fs::write(path, text)?;
    Ok(())
}

/// Apply transport extras always. Replace the queue when `restore_queue` is true.
pub fn apply(player: &PlayerHandle, session: &Session, restore_queue: bool) -> Result<usize> {
    let mut session = session.clone();
    let skipped = if restore_queue {
        prune_missing(&mut session)
    } else {
        0
    };

    player.send_blocking(Command::SetVolume(session.volume))?;
    player.send_blocking(Command::SetMuted(session.muted))?;
    player.send_blocking(Command::SetRepeat(session.repeat))?;
    player.send_blocking(Command::SetShuffle(session.shuffle))?;

    if restore_queue {
        player.send_blocking(Command::ReplaceQueue {
            items: session.queue,
            position: session.queue_position,
        })?;
    }

    Ok(skipped)
}

pub fn save_from_player(player: &PlayerHandle, path: &Path) -> Result<()> {
    save(path, &Session::from_state(&player.state()))
}

/// How long persistable state must stay still before `session.toml` is written
/// while the player process is running. Clean exit still flushes immediately.
pub const SESSION_SAVE_DEBOUNCE: Duration = Duration::from_millis(500);

/// Writes `session.toml` when queue/transport extras have been stable.
pub(crate) struct SessionPersister {
    path: PathBuf,
    debounce: Duration,
    last_written: Session,
    pending: Option<Session>,
    pending_since: Option<Instant>,
}

impl SessionPersister {
    pub(crate) fn new(path: impl Into<PathBuf>, debounce: Duration) -> Self {
        Self {
            path: path.into(),
            debounce,
            last_written: Session::default(),
            pending: None,
            pending_since: None,
        }
    }

    /// Treat the current engine snapshot as already on disk (after restore).
    pub(crate) fn sync_from_player(&mut self, player: &PlayerHandle) {
        self.last_written = Session::from_state(&player.state());
        self.pending = None;
        self.pending_since = None;
    }

    /// Returns true if a write happened.
    pub(crate) fn tick(&mut self, player: &PlayerHandle) -> Result<bool> {
        let current = Session::from_state(&player.state());
        if current == self.last_written {
            self.pending = None;
            self.pending_since = None;
            return Ok(false);
        }
        if self.pending.as_ref() != Some(&current) {
            self.pending = Some(current.clone());
            self.pending_since = Some(Instant::now());
            if self.debounce.is_zero() {
                return self.write(current);
            }
            return Ok(false);
        }
        let since = self
            .pending_since
            .expect("pending snapshot always has a start time");
        if since.elapsed() >= self.debounce {
            self.write(current)
        } else {
            Ok(false)
        }
    }

    pub(crate) fn flush(&mut self, player: &PlayerHandle) -> Result<()> {
        let current = Session::from_state(&player.state());
        if current != self.last_written {
            save(&self.path, &current)?;
            self.last_written = current;
        }
        self.pending = None;
        self.pending_since = None;
        Ok(())
    }

    fn write(&mut self, current: Session) -> Result<bool> {
        save(&self.path, &current)?;
        self.last_written = current;
        self.pending = None;
        self.pending_since = None;
        Ok(true)
    }
}

pub fn restore(player: &PlayerHandle, path: &Path, restore_queue: bool) -> Result<usize> {
    apply(player, &load(path)?, restore_queue)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prune_missing_drops_gone_files_and_keeps_streams() {
        let gone = std::env::temp_dir().join("znicz-session-missing-file.flac");
        let _ = std::fs::remove_file(&gone);
        let mut session = Session {
            queue: vec![
                QueueItem::file(&gone),
                QueueItem::stream("Live", "https://example.com/s"),
                QueueItem::file("/no/such/znicz/track.flac"),
            ],
            queue_position: 2,
            ..Session::default()
        };
        let skipped = prune_missing(&mut session);
        assert_eq!(skipped, 2);
        assert_eq!(session.queue.len(), 1);
        assert!(session.queue[0].is_stream());
        assert_eq!(session.queue_position, 0);
    }

    #[test]
    fn save_and_load_round_trip_files_and_streams() {
        let dir = std::env::temp_dir().join(format!("znicz-session-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("session.toml");
        let session = Session {
            queue: vec![
                QueueItem::file("/music/a.flac"),
                QueueItem::stream("Live", "https://example.com/s"),
            ],
            queue_position: 1,
            volume: 0.4,
            muted: true,
            repeat: RepeatMode::All,
            shuffle: true,
        };
        save(&path, &session).expect("save");
        let loaded = load(&path).expect("load");
        assert_eq!(loaded, session);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_session_file_is_empty_defaults() {
        let path = std::env::temp_dir().join("znicz-session-does-not-exist.toml");
        let _ = std::fs::remove_file(&path);
        let session = load(&path).expect("load");
        assert_eq!(session, Session::default());
    }

    #[test]
    fn apply_restores_volume_and_a_stream_row_without_playing() {
        let (player, _thread) = crate::spawn_player(crate::AudioConfig::default());
        let session = Session {
            queue: vec![QueueItem::stream("Live", "https://example.com/s")],
            queue_position: 0,
            volume: 0.3,
            muted: true,
            repeat: RepeatMode::One,
            shuffle: true,
        };
        let skipped = apply(&player, &session, true).expect("apply");
        assert_eq!(skipped, 0);
        let state = player.state();
        assert!((state.volume - 0.3).abs() < 1e-6);
        assert!(state.muted);
        assert_eq!(state.repeat, RepeatMode::One);
        assert!(state.shuffle);
        assert_eq!(state.queue.len(), 1);
        assert!(state.queue[0].is_stream());
        assert_eq!(state.status, crate::PlaybackStatus::Stopped);
        assert!(state.current_track.is_none());
    }

    fn temp_session_path() -> std::path::PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "znicz-session-persist-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("session.toml")
    }

    #[test]
    fn mute_is_not_written_before_debounce() {
        let path = temp_session_path();
        let (player, _thread) = crate::spawn_player(crate::AudioConfig::default());
        let mut persister = SessionPersister::new(&path, std::time::Duration::from_millis(200));
        persister.sync_from_player(&player);
        player.send_blocking(Command::SetMuted(true)).expect("mute");
        persister.tick(&player).expect("tick");
        std::thread::sleep(std::time::Duration::from_millis(20));
        persister.tick(&player).expect("tick");
        assert!(
            !path.is_file(),
            "session.toml must wait until the snapshot is stable"
        );
    }

    #[test]
    fn mute_is_written_after_the_snapshot_is_stable() {
        let path = temp_session_path();
        let (player, _thread) = crate::spawn_player(crate::AudioConfig::default());
        let mut persister = SessionPersister::new(&path, std::time::Duration::from_millis(30));
        persister.sync_from_player(&player);
        player.send_blocking(Command::SetMuted(true)).expect("mute");
        persister.tick(&player).expect("tick");
        std::thread::sleep(std::time::Duration::from_millis(50));
        persister.tick(&player).expect("tick");
        let loaded = load(&path).expect("load");
        assert!(loaded.muted);
    }

    #[test]
    fn flush_writes_a_pending_mute_without_waiting() {
        let path = temp_session_path();
        let (player, _thread) = crate::spawn_player(crate::AudioConfig::default());
        let mut persister = SessionPersister::new(&path, std::time::Duration::from_millis(5_000));
        persister.sync_from_player(&player);
        player.send_blocking(Command::SetMuted(true)).expect("mute");
        persister.tick(&player).expect("tick");
        persister.flush(&player).expect("flush");
        let loaded = load(&path).expect("load");
        assert!(loaded.muted);
    }
}
