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

/// The engine's status right now.
fn status(player: &PlayerHandle) -> PlaybackStatus {
    player
        .state_arc()
        .read()
        .map(|s| s.status)
        .unwrap_or_else(|e| e.into_inner().status)
}

/// The watcher's sample history. It reports whether the newest sample is a
/// start: a move into `Playing` from a status that was not `Playing` on either
/// of the two samples before it. So a one-sample `Stopped` gap between two
/// `Playing` samples (a track change) does not count, and a start that the
/// previous sample already counted is not counted again.
pub(crate) struct StartDetector {
    before: PlaybackStatus,
    prev: PlaybackStatus,
}

impl StartDetector {
    /// `initial` is the first sample, taken when the watcher starts.
    pub(crate) fn new(initial: PlaybackStatus) -> Self {
        Self {
            before: initial,
            prev: initial,
        }
    }

    /// Record the next sample and say whether it is a start.
    pub(crate) fn observe(&mut self, now: PlaybackStatus) -> bool {
        let start = now == PlaybackStatus::Playing
            && self.prev != PlaybackStatus::Playing
            && self.before != PlaybackStatus::Playing;
        self.before = self.prev;
        self.prev = now;
        start
    }
}

/// Pause the engine, but only while it plays. `Pause` on a Stopped engine
/// sets its status to `Paused`, which would misreport a stopped player.
///
/// Called from inside `SpotifyHost::apply` on the IPC thread. It takes no lock
/// the IPC server holds, and it only queues the command: it does not wait for
/// the engine. The Spotify Web call that follows is what takes the time.
pub fn pause_engine_if_playing(player: &PlayerHandle) {
    if status(player) != PlaybackStatus::Playing {
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
    spawn_pause_worker(player.clone(), spotify, rx)?;
    thread::Builder::new()
        .name("znicz-spotify-pause".into())
        .spawn(move || {
            let mut detector = StartDetector::new(status(&player));
            while !stop.load(Ordering::SeqCst) {
                thread::sleep(SAMPLE);
                if detector.observe(status(&player)) {
                    request_pause(&tx);
                }
            }
        })
}

/// The one thread that makes Spotify pause calls, one at a time.
fn spawn_pause_worker(
    player: PlayerHandle,
    spotify: Arc<SpotifyService>,
    rx: Receiver<()>,
) -> std::io::Result<JoinHandle<()>> {
    thread::Builder::new()
        .name("spotify-pause".into())
        .spawn(move || {
            run_pause_worker(
                rx,
                move || status(&player),
                || thread::sleep(SAMPLE),
                move || spotify.pause_if_playing(),
            );
        })
}

/// Handle each queued request in turn. A request can wait behind a slow call,
/// and Spotify may have started and paused Znicz in the meantime, so a request
/// pauses Spotify only while the engine plays.
///
/// `Stopped` is also the short gap between two tracks, so it is read again
/// after one `wait` before the request is dropped. `Paused` is dropped at once.
fn run_pause_worker(
    rx: Receiver<()>,
    status: impl Fn() -> PlaybackStatus,
    wait: impl Fn(),
    pause: impl Fn(),
) {
    for () in rx.iter() {
        let playing = match status() {
            PlaybackStatus::Playing => true,
            PlaybackStatus::Paused => false,
            PlaybackStatus::Stopped => {
                wait();
                status() == PlaybackStatus::Playing
            }
        };
        if playing {
            pause();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::sync::atomic::AtomicUsize;
    use znicz_core::PlaybackStatus::{Paused, Playing, Stopped};

    /// Feed `samples` after `initial` (the watcher's first sample) and collect
    /// what each one reports.
    fn observe_all(initial: PlaybackStatus, samples: &[PlaybackStatus]) -> Vec<bool> {
        let mut detector = StartDetector::new(initial);
        samples.iter().map(|&now| detector.observe(now)).collect()
    }

    #[test]
    fn an_engine_that_starts_out_playing_does_not_trigger() {
        assert_eq!(observe_all(Playing, &[Playing, Playing]), [false, false]);
    }

    #[test]
    fn stopped_stopped_playing_triggers_on_the_playing_sample() {
        assert_eq!(observe_all(Stopped, &[Stopped, Playing]), [false, true]);
        assert_eq!(observe_all(Paused, &[Paused, Playing]), [false, true]);
    }

    #[test]
    fn a_track_change_gap_does_not_trigger() {
        // Playing, Stopped (the one-sample gap), Playing.
        assert_eq!(observe_all(Playing, &[Stopped, Playing]), [false, false]);
    }

    #[test]
    fn a_start_is_counted_once() {
        // Stopped, then Playing twice: only the first Playing is a start.
        assert_eq!(observe_all(Stopped, &[Playing, Playing]), [true, false]);
    }

    #[test]
    fn a_pause_by_the_engine_is_not_a_start() {
        assert_eq!(observe_all(Playing, &[Playing, Paused]), [false, false]);
    }

    /// Run one queued request against a fake sequence of engine samples and
    /// return how many Spotify pauses it made. Each `status` read pops the next
    /// sample; the wait between samples is counted, not slept.
    fn pauses_for(samples: &[PlaybackStatus]) -> usize {
        let (tx, rx) = crossbeam_channel::bounded::<()>(1);
        request_pause(&tx);
        drop(tx);
        let samples = std::cell::RefCell::new(samples.iter().copied().collect::<VecDeque<_>>());
        let pauses = AtomicUsize::new(0);
        run_pause_worker(
            rx,
            || {
                samples
                    .borrow_mut()
                    .pop_front()
                    .expect("a sample for each read")
            },
            || {},
            || {
                pauses.fetch_add(1, Ordering::SeqCst);
            },
        );
        pauses.load(Ordering::SeqCst)
    }

    #[test]
    fn a_queued_pause_is_skipped_at_once_when_the_engine_is_paused() {
        assert_eq!(pauses_for(&[Paused]), 0);
    }

    #[test]
    fn a_queued_pause_runs_at_once_while_znicz_plays() {
        assert_eq!(pauses_for(&[Playing]), 1);
    }

    #[test]
    fn a_queued_pause_survives_a_stopped_gap_between_tracks() {
        // The gap reads Stopped; one sample later the next track plays.
        assert_eq!(pauses_for(&[Stopped, Playing]), 1);
    }

    #[test]
    fn a_queued_pause_is_skipped_once_znicz_has_stopped() {
        assert_eq!(pauses_for(&[Stopped, Stopped]), 0);
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
}
