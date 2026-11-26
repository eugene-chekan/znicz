//! The pause rule between Znicz and Spotify, run by the player process.
//!
//! - Spotify starts (an op through Znicz that starts playback): the service's
//!   `on_start_playback` pauses the engine, if it plays.
//! - Znicz starts: a watcher sees the engine move into `Playing` and asks
//!   Spotify to pause, on a short-lived thread (the call can block for ~40 s).
//!
//! The two cannot loop. The first only moves the engine away from `Playing`,
//! and the watcher acts only on a move into `Playing`. Spotify's own pause
//! never goes through the first path, because `Pause` does not start playback.
//!
//! The watcher allows one pause call at a time, so an engine that flaps in and
//! out of `Playing` while a call is slow cannot pile up threads.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use znicz_core::{Command, PlaybackStatus, PlayerHandle, SpotifyConfig};
use znicz_spotify::{ServiceOptions, SpotifyService, default_token_path};

/// How often the watcher samples the engine.
const SAMPLE: Duration = Duration::from_millis(250);

/// Whether the engine has just started playing.
pub fn started_playing(prev: PlaybackStatus, now: PlaybackStatus) -> bool {
    prev != PlaybackStatus::Playing && now == PlaybackStatus::Playing
}

/// Pause the engine, but only while it plays. `Pause` on a Stopped engine
/// sets its status to `Paused`, which would misreport a stopped player.
///
/// Called from inside `SpotifyHost::apply` on the IPC thread. It takes no lock
/// the IPC server holds, and it only queues the command: it does not wait for
/// the engine. The Spotify Web call that follows is what takes the time.
pub fn pause_engine_if_playing(player: &PlayerHandle) {
    if player.state().status != PlaybackStatus::Playing {
        return;
    }
    if let Err(e) = player.send(Command::Pause) {
        tracing::warn!(error = %e, "could not pause Znicz for Spotify");
    }
}

/// Build the player's Spotify service. `None` only when there is no data
/// folder for the token; with no client ID the service still runs
/// (`NotConfigured`).
pub fn start_service(config: SpotifyConfig, player: &PlayerHandle) -> Option<Arc<SpotifyService>> {
    let Some(token_path) = default_token_path() else {
        tracing::warn!("no data folder for the Spotify token: Spotify is unavailable");
        return None;
    };
    // A clone of the player handle is cheap and owns nothing of the service,
    // so the service and the player do not keep each other alive.
    let engine = player.clone();
    let options = ServiceOptions {
        token_path,
        on_start_playback: Box::new(move || pause_engine_if_playing(&engine)),
    };
    Some(Arc::new(SpotifyService::start(config, options)))
}

/// The right to run one Spotify pause call. Held by the call's thread and
/// released when it ends, however it ends (including a failed thread spawn,
/// which drops the closure and with it the slot).
struct PauseSlot(Arc<AtomicBool>);

impl PauseSlot {
    /// `None` while another pause call holds the slot.
    fn take(flag: &Arc<AtomicBool>) -> Option<Self> {
        flag.compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .ok()
            .map(|_| Self(flag.clone()))
    }
}

impl Drop for PauseSlot {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

/// Watch the engine until `stop` is set. Each move into `Playing` asks
/// Spotify to pause, on its own thread, so this loop never blocks on Spotify.
/// A start that comes while an earlier call still runs is skipped: that call
/// reads Spotify's state itself, and flapping must not spawn a thread per flap.
pub fn spawn_pause_watcher(
    player: PlayerHandle,
    spotify: Arc<SpotifyService>,
    stop: Arc<AtomicBool>,
) -> std::io::Result<JoinHandle<()>> {
    thread::Builder::new()
        .name("znicz-spotify-pause".into())
        .spawn(move || {
            let in_flight = Arc::new(AtomicBool::new(false));
            let mut prev = player.state().status;
            while !stop.load(Ordering::SeqCst) {
                thread::sleep(SAMPLE);
                let now = player.state().status;
                if started_playing(prev, now) {
                    if let Some(slot) = PauseSlot::take(&in_flight) {
                        let spotify = spotify.clone();
                        let _ = thread::Builder::new()
                            .name("znicz-spotify-pause-call".into())
                            .spawn(move || {
                                let _slot = slot;
                                spotify.pause_if_playing();
                            });
                    }
                }
                prev = now;
            }
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use znicz_core::PlaybackStatus::{Paused, Playing, Stopped};

    #[test]
    fn only_a_move_into_playing_counts_as_starting() {
        assert!(started_playing(Stopped, Playing));
        assert!(started_playing(Paused, Playing));
        assert!(!started_playing(Playing, Playing));
        assert!(!started_playing(Playing, Paused));
        assert!(!started_playing(Stopped, Paused));
        assert!(!started_playing(Paused, Stopped));
    }

    #[test]
    fn only_one_pause_call_runs_at_a_time() {
        let flag = Arc::new(AtomicBool::new(false));
        let first = PauseSlot::take(&flag).expect("free");
        assert!(
            PauseSlot::take(&flag).is_none(),
            "busy while the first is held"
        );
        drop(first);
        assert!(
            PauseSlot::take(&flag).is_some(),
            "free again once it is dropped"
        );
    }

    #[test]
    fn pausing_for_spotify_leaves_a_stopped_engine_stopped() {
        let (player, _thread) = znicz_core::spawn_player(znicz_core::AudioConfig::default());
        pause_engine_if_playing(&player);
        // A state-neutral command, sent after the pause would have been: its
        // answer means the engine has processed everything sent before it.
        let _ = player.send_blocking(znicz_core::Command::SetMuted(false));
        assert_eq!(player.state().status, Stopped);
    }

    #[test]
    fn the_engine_pausing_for_spotify_is_not_a_start() {
        // (a) moves the engine from Playing to Paused; the watcher only reacts
        // to a move into Playing, so (a) cannot trigger (b).
        assert!(!started_playing(Playing, Paused));
    }
}
