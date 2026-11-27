//! A background connection for the Spotify calls that can be slow (up to about
//! 40 s). The UI thread sends a request and drains the replies each tick, so
//! no frame waits on Spotify. Generic over the tag, so Part 5 can reuse it.

use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};

use crate::spotify_browse::BrowseList;
use znicz_core::{Result, SpotifyHost, SpotifyOp, SpotifyReply};

/// What asked for a request. Settings' rows and the browser's lists go to the
/// browse lane; playback goes to the transport lane (`spotify_mode::lane`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpotifyAsk {
    Configure,
    BeginLogin,
    CompleteLogin,
    CancelLogin,
    Logout,
    Devices,
    /// Spotify mode's browse request. `generation` is the browser's; a reply with an old one is dropped.
    Browse {
        generation: u64,
        list: BrowseList,
        offset: u32,
    },
    /// The queue, for the drawer.
    Queue,
    /// Any transport op: play, pause, next, previous, seek, volume, repeat, shuffle.
    Transport,
    /// A track added to Spotify's queue. Its success re-reads the drawer at once.
    QueueAdd,
    /// A move to a Spotify Connect device. `name` is the device's, for the toast.
    Transfer {
        name: String,
    },
}

/// The answer to one request, with the tag that asked for it.
#[derive(Debug)]
pub struct Done<T> {
    pub tag: T,
    pub result: Result<SpotifyReply>,
}

/// The worker thread's two channels. Dropping the worker closes the request
/// channel, and the thread ends after the request it is running.
pub struct SpotifyWorker<T> {
    requests: Sender<(T, SpotifyOp)>,
    replies: Receiver<Done<T>>,
}

impl<T: Send + 'static> SpotifyWorker<T> {
    /// Start the thread that owns `host`. Requests run in the order sent.
    pub fn spawn(host: Arc<dyn SpotifyHost>) -> std::io::Result<Self> {
        let (requests, inbox) = channel::<(T, SpotifyOp)>();
        let (outbox, replies) = channel::<Done<T>>();
        std::thread::Builder::new()
            .name("spotify-worker".into())
            .spawn(move || {
                while let Ok((tag, op)) = inbox.recv() {
                    let result = host.apply(op);
                    if outbox.send(Done { tag, result }).is_err() {
                        break;
                    }
                }
            })?;
        Ok(Self { requests, replies })
    }

    /// Queue a request. False when the thread has stopped.
    pub fn send(&self, tag: T, op: SpotifyOp) -> bool {
        self.requests.send((tag, op)).is_ok()
    }

    /// The next finished request, without waiting.
    pub fn try_recv(&self) -> Option<Done<T>> {
        self.replies.try_recv().ok()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;
    use std::sync::mpsc::{Receiver, Sender, channel};
    use std::time::{Duration, Instant};

    use super::*;
    use znicz_core::{SpotifyState, ZniczError};

    /// Records each op; `Devices` blocks until the test sends a release.
    struct Host {
        seen: Mutex<Vec<SpotifyOp>>,
        gate: Mutex<Receiver<()>>,
        entered: Sender<()>,
    }

    impl SpotifyHost for Host {
        fn apply(&self, op: SpotifyOp) -> Result<SpotifyReply> {
            self.seen.lock().unwrap().push(op.clone());
            if op == SpotifyOp::Devices {
                let _ = self.entered.send(());
                self.gate.lock().unwrap().recv().unwrap();
                return Ok(SpotifyReply::Devices {
                    devices: Vec::new(),
                });
            }
            if op == SpotifyOp::Logout {
                return Err(ZniczError::Player("nope".into()));
            }
            Ok(SpotifyReply::Done)
        }
        fn state(&self) -> SpotifyState {
            SpotifyState::default()
        }
    }

    fn host() -> (Arc<Host>, Sender<()>, Receiver<()>) {
        let (release, gate) = channel();
        let (entered, started) = channel();
        let host = Arc::new(Host {
            seen: Mutex::new(Vec::new()),
            gate: Mutex::new(gate),
            entered,
        });
        (host, release, started)
    }

    fn wait_for(worker: &SpotifyWorker<SpotifyAsk>) -> Done<SpotifyAsk> {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(done) = worker.try_recv() {
                return done;
            }
            assert!(Instant::now() < deadline, "no reply from the worker");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn replies_come_back_tagged_in_request_order() {
        let (host, release, _started) = host();
        release.send(()).unwrap();
        let worker = SpotifyWorker::spawn(host.clone()).unwrap();
        assert!(worker.send(SpotifyAsk::Configure, SpotifyOp::Touch));
        assert!(worker.send(SpotifyAsk::Logout, SpotifyOp::Logout));
        let first = wait_for(&worker);
        assert_eq!(first.tag, SpotifyAsk::Configure);
        assert!(first.result.is_ok());
        let second = wait_for(&worker);
        assert_eq!(second.tag, SpotifyAsk::Logout);
        assert!(second.result.is_err());
        assert_eq!(
            host.seen.lock().unwrap().as_slice(),
            [SpotifyOp::Touch, SpotifyOp::Logout]
        );
    }

    #[test]
    fn a_slow_op_does_not_block_try_recv() {
        let (host, release, started) = host();
        let worker = SpotifyWorker::spawn(host).unwrap();
        worker.send(SpotifyAsk::Devices, SpotifyOp::Devices);
        started.recv_timeout(Duration::from_secs(5)).unwrap();
        let before = Instant::now();
        assert!(worker.try_recv().is_none());
        assert!(before.elapsed() < Duration::from_millis(50));
        release.send(()).unwrap();
        assert_eq!(wait_for(&worker).tag, SpotifyAsk::Devices);
    }

    #[test]
    fn dropping_the_worker_ends_its_thread() {
        let (host, release, _started) = host();
        release.send(()).unwrap();
        let worker = SpotifyWorker::<SpotifyAsk>::spawn(host.clone()).unwrap();
        assert!(Arc::strong_count(&host) >= 2, "the thread holds the host");
        drop(worker);
        let deadline = Instant::now() + Duration::from_secs(5);
        while Arc::strong_count(&host) > 1 {
            assert!(Instant::now() < deadline, "the thread did not end");
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}
