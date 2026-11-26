//! The pause rule between Znicz and Spotify, run by the player process.
//!
//! - Spotify starts (an op through Znicz that starts playback): the service's
//!   `on_start_playback` pauses the engine, if it plays.
//! - Znicz starts: a watcher sees the engine start playing and asks Spotify to
//!   pause. A single worker thread makes the calls, since one can block for
//!   ~40 s.
//!
//! The two cannot loop. The first only moves the engine away from `Playing`,
//! and the watcher acts only on a start. Spotify's own pause never goes
//! through the first path, because `Pause` does not start playback.
//!
//! Pause requests go through a bounded(1) channel. While one request is queued
//! behind a slow call, further requests coalesce into it, so no start is lost
//! and a flapping engine cannot pile up work.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use crossbeam_channel::{Receiver, Sender};
use znicz_core::{Command, PlaybackStatus, PlayerHandle, SpotifyConfig};
use znicz_spotify::{ServiceOptions, SpotifyService, default_token_path};

/// How often the watcher samples the engine.
const SAMPLE: Duration = Duration::from_millis(250);

/// Whether the engine has just started playing. `before` and `prev` are the
/// two samples before `now`. A start is a move into `Playing` from a status
/// that was not `Playing` on either sample, so a one-sample `Stopped` gap
/// between two `Playing` samples (a track change) does not count, and neither
/// does a start that the previous sample already counted.
pub(crate) fn started_playing(
    before: PlaybackStatus,
    prev: PlaybackStatus,
    now: PlaybackStatus,
) -> bool {
    now == PlaybackStatus::Playing
        && prev != PlaybackStatus::Playing
        && before != PlaybackStatus::Playing
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

/// Ask the worker for one pause. If a request is already queued, it will run
/// after the current call and see the same Spotify state, so this one is
/// dropped without losing a pause.
fn request_pause(tx: &Sender<()>) {
    let _ = tx.try_send(());
}

/// Watch the engine until `stop` is set. Each start asks the worker to pause
/// Spotify, so this loop never blocks on Spotify. The worker exits when the
/// watcher drops its sender, which happens when the watcher's loop ends.
pub fn spawn_pause_watcher(
    player: PlayerHandle,
    spotify: Arc<SpotifyService>,
    stop: Arc<AtomicBool>,
) -> std::io::Result<JoinHandle<()>> {
    let (tx, rx) = crossbeam_channel::bounded::<()>(1);
    spawn_pause_worker(spotify, rx)?;
    thread::Builder::new()
        .name("znicz-spotify-pause".into())
        .spawn(move || {
            let mut before = player
                .state_arc()
                .read()
                .map(|s| s.status)
                .unwrap_or(PlaybackStatus::Stopped);
            let mut prev = before;
            while !stop.load(Ordering::SeqCst) {
                thread::sleep(SAMPLE);
                let now = player
                    .state_arc()
                    .read()
                    .map(|s| s.status)
                    .unwrap_or(PlaybackStatus::Stopped);
                if started_playing(before, prev, now) {
                    request_pause(&tx);
                }
                before = prev;
                prev = now;
            }
        })
}

/// The one thread that makes Spotify pause calls, one at a time.
fn spawn_pause_worker(
    spotify: Arc<SpotifyService>,
    rx: Receiver<()>,
) -> std::io::Result<JoinHandle<()>> {
    thread::Builder::new()
        .name("spotify-pause".into())
        .spawn(move || {
            for () in rx.iter() {
                spotify.pause_if_playing();
            }
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use znicz_core::PlaybackStatus::{Paused, Playing, Stopped};

    #[test]
    fn a_start_needs_two_samples_that_were_not_playing() {
        assert!(started_playing(Stopped, Stopped, Playing));
        assert!(started_playing(Paused, Paused, Playing));
        // A one-sample Stopped gap between two Playing samples is a track change.
        assert!(!started_playing(Playing, Stopped, Playing));
        assert!(!started_playing(Playing, Playing, Playing));
        // The start was already counted on the previous sample.
        assert!(!started_playing(Stopped, Playing, Playing));
        assert!(!started_playing(Playing, Playing, Paused));
    }

    #[test]
    fn requests_made_while_a_pause_is_queued_coalesce() {
        let (tx, rx) = crossbeam_channel::bounded::<()>(1);
        request_pause(&tx);
        request_pause(&tx);
        assert_eq!(rx.len(), 1, "the second request joins the queued one");
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
        assert!(!started_playing(Playing, Playing, Paused));
    }
}
