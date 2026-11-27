use super::poll::{FAST_POLL, SLOW_POLL, poll_interval};

use std::sync::atomic::AtomicUsize;
use std::sync::mpsc;
use std::time::Instant;

use super::*;
use crate::http::test_server::{Response, Seen, serve};
use crate::token::{self, Token};
use znicz_core::{SpotifyErrorKind, SpotifyPlayback, ZniczError};

const CLIENT_ID: &str = "0123456789abcdef0123456789abcdef";

#[test]
fn polls_fast_while_watched_and_not_at_all_when_logged_out() {
    assert_eq!(poll_interval(false, 0, 0), None);
    assert_eq!(poll_interval(true, 59_999, 0), Some(FAST_POLL));
    assert_eq!(poll_interval(true, 60_000, 0), Some(SLOW_POLL));
}

struct Fixture {
    service: SpotifyService,
    seen: Seen,
    starts: Arc<AtomicUsize>,
    _dir: tempfile::TempDir,
}

fn fixture<F>(logged_in: bool, device: &str, handler: F) -> Fixture
where
    F: Fn(&crate::http::test_server::Request) -> Response + Send + 'static,
{
    fixture_with_clock(logged_in, device, handler, system_clock())
}

fn fixture_with_clock<F>(logged_in: bool, device: &str, handler: F, clock: Clock) -> Fixture
where
    F: Fn(&crate::http::test_server::Request) -> Response + Send + 'static,
{
    let token = logged_in.then(|| Token {
        access_token: "a".into(),
        refresh_token: "r".into(),
        expires_at: u64::MAX / 2,
        scope: String::new(),
        client_id: CLIENT_ID.into(),
    });
    fixture_with_token(token, CLIENT_ID, device, handler, clock)
}

/// A service whose config holds `client_id` and whose token file, when
/// there is a token, is `token`.
fn fixture_with_token<F>(
    token: Option<Token>,
    client_id: &str,
    device: &str,
    handler: F,
    clock: Clock,
) -> Fixture
where
    F: Fn(&crate::http::test_server::Request) -> Response + Send + 'static,
{
    let (origin, seen) = serve(handler);
    let dir = tempfile::tempdir().unwrap();
    let token_path = dir.path().join("spotify_token.json");
    if let Some(token) = &token {
        token::save(&token_path, token).unwrap();
    }
    let starts = Arc::new(AtomicUsize::new(0));
    let counter = starts.clone();
    let config = SpotifyConfig {
        client_id: client_id.into(),
        redirect_port: free_port(),
        device: device.into(),
    };
    let options = ServiceOptions {
        token_path,
        on_start_playback: Box::new(move || {
            counter.fetch_add(1, Ordering::SeqCst);
        }),
    };
    let endpoints = Endpoints {
        api: format!("{origin}/v1"),
        accounts: origin,
    };
    let service = SpotifyService::start_with(
        config,
        options,
        endpoints,
        clock,
        (Duration::from_millis(400), Duration::from_millis(20)),
    );
    Fixture {
        service,
        seen,
        starts,
        _dir: dir,
    }
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn api_default(request: &crate::http::test_server::Request) -> Response {
    match request.path() {
        "/api/token" => Response::json(
            200,
            r#"{"access_token":"a","expires_in":3600,"refresh_token":"r"}"#,
        ),
        "/v1/me" => Response::json(
            200,
            r#"{"id":"u","display_name":"Eugene","product":"premium"}"#,
        ),
        "/v1/me/player" if request.method == "GET" => Response::json(
            200,
            r#"{"is_playing":true,"progress_ms":5,"currently_playing_type":"track",
                "item":{"type":"track","uri":"spotify:track:t","name":"So What","duration_ms":9,
                        "artists":[],"album":{"name":"Kind of Blue","images":[]}}}"#,
        ),
        "/v1/me/player/devices" => Response::json(
            200,
            r#"{"devices":[{"id":"k1","name":"Kitchen","type":"Speaker","is_active":false}]}"#,
        ),
        _ => Response::empty(204),
    }
}

fn wait_for(service: &SpotifyService, what: impl Fn(&SpotifyState) -> bool) -> SpotifyState {
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let state = service.state();
        if what(&state) || Instant::now() > deadline {
            return state;
        }
        thread::sleep(Duration::from_millis(20));
    }
}

fn requests(seen: &Seen) -> Vec<String> {
    seen.lock()
        .unwrap()
        .iter()
        .map(|r| format!("{} {}", r.method, r.target))
        .collect()
}

fn play() -> SpotifyOp {
    SpotifyOp::Play {
        context_uri: Some("spotify:album:a1".into()),
        uris: vec![],
        offset: None,
    }
}

#[test]
fn starts_logged_out_without_a_token_and_refuses_control() {
    let f = fixture(false, "", api_default);
    assert_eq!(f.service.state().login, SpotifyLogin::NotLoggedIn);
    let error = f.service.apply(SpotifyOp::Pause).unwrap_err();
    assert_eq!(error.to_string(), "not logged in to Spotify");
}

#[test]
fn an_accepted_play_pauses_znicz() {
    let f = fixture(true, "", api_default);
    f.service.apply(play()).unwrap();
    assert_eq!(f.starts.load(Ordering::SeqCst), 1);
    assert!(requests(&f.seen).contains(&"PUT /v1/me/player/play".to_string()));
    f.service.apply(SpotifyOp::Pause).unwrap();
    assert_eq!(f.starts.load(Ordering::SeqCst), 1, "pause does not count");
}

#[test]
fn no_active_device_falls_back_to_the_preferred_one() {
    let f = fixture(true, "Kitchen", |request| {
        if request.target == "/v1/me/player/play" {
            Response::json(
                404,
                r#"{"error":{"status":404,"message":"No active device found","reason":"NO_ACTIVE_DEVICE"}}"#,
            )
        } else {
            api_default(request)
        }
    });
    f.service.apply(play()).unwrap();
    let seen = requests(&f.seen);
    assert!(
        seen.contains(&"GET /v1/me/player/devices".to_string()),
        "{seen:?}"
    );
    assert!(
        seen.contains(&"PUT /v1/me/player/play?device_id=k1".to_string()),
        "{seen:?}"
    );
}

#[test]
fn no_active_device_without_a_preference_is_reported() {
    let f = fixture(true, "", |request| {
        if request.path() == "/v1/me/player/play" {
            Response::json(404, r#"{"error":{"reason":"NO_ACTIVE_DEVICE"}}"#)
        } else {
            api_default(request)
        }
    });
    let error = f.service.apply(play()).unwrap_err();
    assert_eq!(error.to_string(), SpotifyError::NoActiveDevice.to_string());
    assert_eq!(f.service.state().last_error, Some(error.to_string()));
}

#[test]
fn a_429_waits_and_retries_once() {
    let hits = Arc::new(AtomicUsize::new(0));
    let counter = hits.clone();
    let f = fixture(true, "", move |request| {
        if request.path() == "/v1/me/player/next" {
            if counter.fetch_add(1, Ordering::SeqCst) == 0 {
                return Response::empty(429).with_header("Retry-After", "1");
            }
            return Response::empty(204);
        }
        api_default(request)
    });
    let started = Instant::now();
    f.service.apply(SpotifyOp::Next).unwrap();
    assert!(started.elapsed() >= Duration::from_millis(900));
    assert_eq!(hits.load(Ordering::SeqCst), 2);
}

#[test]
fn login_by_paste() {
    let f = fixture(false, "", api_default);
    let url = match f.service.apply(SpotifyOp::BeginLogin).unwrap() {
        SpotifyReply::AuthorizeUrl { url, .. } => url,
        other => panic!("{other:?}"),
    };
    assert_eq!(f.service.state().login, SpotifyLogin::LoggingIn);
    let state = url
        .split("state=")
        .nth(1)
        .unwrap()
        .split('&')
        .next()
        .unwrap();
    assert!(
        f.service
            .apply(SpotifyOp::CompleteLogin {
                redirect_url: "code=x&state=nope".into()
            })
            .is_err()
    );
    assert_eq!(
        f.service.state().login,
        SpotifyLogin::LoggingIn,
        "a wrong paste keeps waiting"
    );
    f.service
        .apply(SpotifyOp::CompleteLogin {
            redirect_url: format!("code=x&state={state}"),
        })
        .unwrap();
    let state = f.service.state();
    assert_eq!(state.login, SpotifyLogin::LoggedIn);
    assert_eq!(state.account.unwrap().user, "Eugene");
    assert_eq!(state.last_error, None);
}

#[test]
fn login_by_callback() {
    let f = fixture(false, "", api_default);
    let url = match f.service.apply(SpotifyOp::BeginLogin).unwrap() {
        SpotifyReply::AuthorizeUrl { url, .. } => url,
        other => panic!("{other:?}"),
    };
    let state = url
        .split("state=")
        .nth(1)
        .unwrap()
        .split('&')
        .next()
        .unwrap()
        .to_string();
    let port = f.service.redirect_port();
    let mut stream = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
    use std::io::{Read, Write};
    write!(
        stream,
        "GET /callback?code=x&state={state} HTTP/1.1\r\n\r\n"
    )
    .unwrap();
    let mut page = String::new();
    stream.read_to_string(&mut page).unwrap();
    assert!(page.contains("You can close this tab"), "{page}");
    let state = wait_for(&f.service, |s| s.login == SpotifyLogin::LoggedIn);
    assert_eq!(state.login, SpotifyLogin::LoggedIn);
}

#[test]
fn touch_polls_playback_at_once() {
    let f = fixture(true, "", api_default);
    f.service.apply(SpotifyOp::Touch).unwrap();
    let state = wait_for(&f.service, |s| s.playback.is_some());
    let playback = state.playback.unwrap();
    assert!(playback.is_playing);
    assert_eq!(playback.item.unwrap().title, "So What");
    assert_eq!(state.devices.len(), 1);
}

#[test]
fn pause_if_playing_only_pauses_a_playing_spotify() {
    let playing = Arc::new(AtomicBool::new(false));
    let flag = playing.clone();
    let f = fixture(true, "", move |request| {
        if request.method == "GET" && request.path() == "/v1/me/player" {
            let body = if flag.load(Ordering::SeqCst) {
                r#"{"is_playing":true,"progress_ms":5,"currently_playing_type":"track",
                    "item":{"type":"track","uri":"spotify:track:t","name":"So What","duration_ms":9,
                            "artists":[],"album":{"name":"Kind of Blue","images":[]}}}"#
            } else {
                r#"{"is_playing":false,"progress_ms":5,"currently_playing_type":"track","item":null}"#
            };
            return Response::json(200, body);
        }
        api_default(request)
    });
    f.service.pause_if_playing();
    assert!(!requests(&f.seen).contains(&"PUT /v1/me/player/pause".to_string()));
    // Started on another device after the last poll: no Touch, no wait for a poll.
    playing.store(true, Ordering::SeqCst);
    f.service.pause_if_playing();
    assert!(requests(&f.seen).contains(&"PUT /v1/me/player/pause".to_string()));
}

#[test]
fn a_playback_answer_tells_whether_the_account_has_premium() {
    let f = fixture(true, "", |request| match request.path() {
        "/v1/me" => Response::json(200, r#"{"id":"u","display_name":"Eugene"}"#),
        "/v1/me/player/next" => Response::json(
            403,
            r#"{"error":{"status":403,"message":"Player command failed: Premium required","reason":"PREMIUM_REQUIRED"}}"#,
        ),
        _ => api_default(request),
    });
    f.service.apply(SpotifyOp::Touch).unwrap();
    let state = wait_for(&f.service, |s| s.account.is_some());
    assert_eq!(
        state.account.unwrap().premium,
        None,
        "unknown until a refusal"
    );
    f.service.apply(SpotifyOp::Next).unwrap_err();
    assert_eq!(f.service.state().account.unwrap().premium, Some(false));
    f.service.apply(SpotifyOp::Pause).unwrap();
    assert_eq!(f.service.state().account.unwrap().premium, Some(true));
}

#[test]
fn logout_clears_the_state() {
    let f = fixture(true, "", api_default);
    f.service.apply(SpotifyOp::Touch).unwrap();
    wait_for(&f.service, |s| s.playback.is_some());
    f.service.apply(SpotifyOp::Logout).unwrap();
    let state = f.service.state();
    assert_eq!(state.login, SpotifyLogin::NotLoggedIn);
    assert_eq!(state.playback, None);
    assert_eq!(state.account, None);
}

#[test]
fn configure_without_a_client_id_is_not_configured() {
    let f = fixture(true, "", api_default);
    f.service
        .apply(SpotifyOp::Configure {
            config: SpotifyConfig::default(),
            replace_client_id: true,
        })
        .unwrap();
    assert_eq!(f.service.state().login, SpotifyLogin::NotConfigured);
    let error = f.service.apply(SpotifyOp::Devices).unwrap_err();
    assert_eq!(error.to_string(), SpotifyError::NotConfigured.to_string());
}

#[test]
fn a_revoked_token_during_a_command_logs_out() {
    let f = fixture(true, "", |request| match request.path() {
        "/v1/me/player/next" => Response::empty(401),
        "/api/token" => Response::json(400, r#"{"error":"invalid_grant"}"#),
        _ => api_default(request),
    });
    f.service.apply(SpotifyOp::Next).unwrap_err();
    assert_eq!(f.service.state().login, SpotifyLogin::NotLoggedIn);
}

#[test]
fn pasting_after_the_login_finished_keeps_it() {
    let f = fixture(true, "", api_default);
    assert!(
        f.service
            .apply(SpotifyOp::CompleteLogin {
                redirect_url: "code=x&state=y".into()
            })
            .is_err()
    );
    assert_eq!(f.service.state().login, SpotifyLogin::LoggedIn);
}

#[test]
fn logging_in_twice_does_not_report_a_busy_port() {
    let f = fixture(false, "", api_default);
    f.service.apply(SpotifyOp::BeginLogin).unwrap();
    match f.service.apply(SpotifyOp::BeginLogin).unwrap() {
        SpotifyReply::AuthorizeUrl { listening, .. } => assert!(listening),
        other => panic!("{other:?}"),
    }
    assert_eq!(f.service.state().last_error, None);
}

#[test]
fn begin_login_says_whether_it_listens() {
    let f = fixture(false, "", api_default);
    match f.service.apply(SpotifyOp::BeginLogin).unwrap() {
        SpotifyReply::AuthorizeUrl {
            redirect_uri,
            listening,
            ..
        } => {
            assert!(listening);
            assert!(redirect_uri.ends_with("/callback"), "{redirect_uri}");
        }
        other => panic!("{other:?}"),
    }

    let f = fixture(false, "", api_default);
    let port = f.service.redirect_port();
    let _busy = std::net::TcpListener::bind(("127.0.0.1", port)).unwrap();
    match f.service.apply(SpotifyOp::BeginLogin).unwrap() {
        SpotifyReply::AuthorizeUrl { listening, .. } => assert!(!listening),
        other => panic!("{other:?}"),
    }
    let error = f.service.state().last_error.unwrap_or_default();
    assert!(error.contains("could not listen"), "{error}");
}

#[test]
fn configure_with_the_same_port_keeps_the_login_listener() {
    let f = fixture(false, "", api_default);
    let url = match f.service.apply(SpotifyOp::BeginLogin).unwrap() {
        SpotifyReply::AuthorizeUrl { url, .. } => url,
        other => panic!("{other:?}"),
    };
    let state = url
        .split("state=")
        .nth(1)
        .unwrap()
        .split('&')
        .next()
        .unwrap()
        .to_string();
    let port = f.service.redirect_port();
    f.service
        .apply(SpotifyOp::Configure {
            config: SpotifyConfig {
                client_id: CLIENT_ID.into(),
                redirect_port: port,
                device: "Kitchen".into(),
            },
            replace_client_id: false,
        })
        .unwrap();
    let mut stream = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
    use std::io::Write;
    write!(
        stream,
        "GET /callback?code=x&state={state} HTTP/1.1\r\n\r\n"
    )
    .unwrap();
    let state = wait_for(&f.service, |s| s.login == SpotifyLogin::LoggedIn);
    assert_eq!(state.login, SpotifyLogin::LoggedIn);
}

const OTHER_CLIENT_ID: &str = "fedcba9876543210fedcba9876543210";

#[test]
fn a_legacy_token_stays_logged_in_when_an_empty_player_gets_its_id() {
    let legacy = Token {
        access_token: "a".into(),
        refresh_token: "r".into(),
        expires_at: u64::MAX / 2,
        scope: String::new(),
        client_id: String::new(),
    };
    let f = fixture_with_token(Some(legacy), "", "", api_default, system_clock());
    assert_eq!(f.service.state().login, SpotifyLogin::NotConfigured);
    f.service
        .apply(SpotifyOp::Configure {
            config: SpotifyConfig {
                client_id: CLIENT_ID.into(),
                redirect_port: f.service.redirect_port(),
                device: String::new(),
            },
            replace_client_id: false,
        })
        .unwrap();
    assert!(f._dir.path().join("spotify_token.json").exists());
    assert_eq!(f.service.state().login, SpotifyLogin::LoggedIn);
}

#[test]
fn a_different_client_id_without_the_flag_is_refused_and_changes_nothing() {
    let f = fixture(true, "", api_default);
    let before = f.service.shared.client.lock().unwrap().config().clone();
    let error = f
        .service
        .apply(SpotifyOp::Configure {
            config: SpotifyConfig {
                client_id: OTHER_CLIENT_ID.into(),
                redirect_port: before.redirect_port + 1,
                device: "Kitchen".into(),
            },
            replace_client_id: false,
        })
        .unwrap_err();
    match error {
        ZniczError::Spotify { kind, message } => {
            assert_eq!(kind, SpotifyErrorKind::ClientIdDiffers);
            assert_eq!(message, "the player uses a different Spotify client ID");
        }
        other => panic!("{other:?}"),
    }
    assert!(f._dir.path().join("spotify_token.json").exists());
    assert_eq!(f.service.state().login, SpotifyLogin::LoggedIn);
    assert_eq!(*f.service.shared.client.lock().unwrap().config(), before);
}

#[test]
fn a_different_client_id_with_the_flag_logs_out() {
    let f = fixture(true, "", api_default);
    f.service
        .apply(SpotifyOp::Configure {
            config: SpotifyConfig {
                client_id: OTHER_CLIENT_ID.into(),
                redirect_port: f.service.redirect_port(),
                device: String::new(),
            },
            replace_client_id: true,
        })
        .unwrap();
    assert!(!f._dir.path().join("spotify_token.json").exists());
    assert_eq!(f.service.state().login, SpotifyLogin::NotLoggedIn);
    assert_eq!(
        f.service.shared.client.lock().unwrap().config().client_id,
        OTHER_CLIENT_ID
    );
}

#[test]
fn the_same_client_id_with_a_new_port_stays_logged_in() {
    let f = fixture(true, "", api_default);
    let port = f.service.redirect_port() + 1;
    f.service
        .apply(SpotifyOp::Configure {
            config: SpotifyConfig {
                client_id: CLIENT_ID.into(),
                redirect_port: port,
                device: "Kitchen".into(),
            },
            replace_client_id: false,
        })
        .unwrap();
    assert_eq!(f.service.redirect_port(), port);
    assert_eq!(f.service.state().login, SpotifyLogin::LoggedIn);
    assert!(f._dir.path().join("spotify_token.json").exists());
}

#[test]
fn the_first_client_id_needs_no_flag() {
    let f = fixture(false, "", api_default);
    // Clear the ID the fixture starts with, so the first real one is "first".
    let mut empty = f.service.shared.client.lock().unwrap().config().clone();
    empty.client_id.clear();
    f.service
        .shared
        .client
        .lock()
        .unwrap()
        .set_config(empty, false);
    f.service
        .apply(SpotifyOp::Configure {
            config: SpotifyConfig {
                client_id: CLIENT_ID.into(),
                redirect_port: f.service.redirect_port(),
                device: String::new(),
            },
            replace_client_id: false,
        })
        .unwrap();
    assert_eq!(
        f.service.shared.client.lock().unwrap().config().client_id,
        CLIENT_ID
    );
    assert_eq!(f.service.state().login, SpotifyLogin::NotLoggedIn);
}

#[test]
fn play_while_logged_out_does_not_pause_znicz() {
    let f = fixture(false, "", api_default);
    f.service.apply(play()).unwrap_err();
    assert_eq!(f.starts.load(Ordering::SeqCst), 0);
}

#[test]
fn a_refused_play_does_not_pause_znicz() {
    let f = fixture(true, "", |request| {
        if request.path() == "/v1/me/player/play" {
            Response::json(
                404,
                r#"{"error":{"status":404,"message":"No active device found","reason":"NO_ACTIVE_DEVICE"}}"#,
            )
        } else {
            api_default(request)
        }
    });
    f.service.apply(play()).unwrap_err();
    assert_eq!(f.starts.load(Ordering::SeqCst), 0);
}

#[test]
fn pause_if_playing_after_a_revoke_logs_out() {
    let f = fixture(true, "", |request| match request.path() {
        "/v1/me/player/pause" => Response::empty(401),
        "/api/token" => Response::json(400, r#"{"error":"invalid_grant"}"#),
        _ => api_default(request),
    });
    // No Touch and no wait: the fresh read says playing, then the pause gets a 401.
    f.service.pause_if_playing();
    assert_eq!(f.service.state().login, SpotifyLogin::NotLoggedIn);
}

#[test]
fn an_abandoned_login_expires() {
    let now = Arc::new(AtomicU64::new(1_000_000));
    let clock_now = now.clone();
    let clock: Clock = Arc::new(move || clock_now.load(Ordering::SeqCst));
    let f = fixture_with_clock(false, "", api_default, clock);
    f.service.apply(SpotifyOp::BeginLogin).unwrap();
    assert_eq!(f.service.state().login, SpotifyLogin::LoggingIn);
    now.fetch_add(crate::client::LOGIN_TTL_MS + 1, Ordering::SeqCst);
    let state = wait_for(&f.service, |s| matches!(s.login, SpotifyLogin::Failed(_)));
    assert!(
        matches!(state.login, SpotifyLogin::Failed(_)),
        "{:?}",
        state.login
    );
    // The listener is dropped after the state changes; give it time to let go of the port.
    thread::sleep(Duration::from_millis(300));
    assert!(
        std::net::TcpStream::connect(("127.0.0.1", f.service.redirect_port())).is_err(),
        "the login listener still holds the port"
    );
}

#[test]
fn a_failing_account_read_is_not_retried_every_poll() {
    let f = fixture(true, "", |request| match request.path() {
        "/v1/me" => Response::json(
            403,
            r#"{"error":{"status":403,"message":"not in user list"}}"#,
        ),
        _ => api_default(request),
    });
    for _ in 0..3 {
        f.service.apply(SpotifyOp::Touch).unwrap();
        thread::sleep(Duration::from_millis(1200));
    }
    let reads = requests(&f.seen)
        .iter()
        .filter(|r| r.as_str() == "GET /v1/me")
        .count();
    assert!(reads <= 2, "{reads} account reads");
}

#[test]
fn play_marks_playback_playing_at_once() {
    let played = Arc::new(AtomicBool::new(false));
    let handler_played = played.clone();
    let (release_tx, release_rx) = mpsc::channel::<()>();
    let f = fixture(true, "", move |request| {
        match (request.method.as_str(), request.path()) {
            ("PUT", "/v1/me/player/play") => {
                handler_played.store(true, Ordering::SeqCst);
                Response::empty(204)
            }
            // The poll that the play wakes is held here, so only the
            // command's own update can show playing until the test releases it.
            ("GET", "/v1/me/player") if handler_played.load(Ordering::SeqCst) => {
                let _ = release_rx.recv();
                Response::json(200, PAUSED)
            }
            ("GET", "/v1/me/player") => Response::json(200, PAUSED),
            _ => api_default(request),
        }
    });
    // No Touch, so no poll is due: Spotify is set to paused directly.
    f.service.shared.state.lock().unwrap().playback = Some(SpotifyPlayback::default());
    f.service.apply(play()).unwrap();
    assert!(f.service.state().is_playing());
    release_tx.send(()).unwrap();
}

const PAUSED: &str =
    r#"{"is_playing":false,"progress_ms":5,"currently_playing_type":"track","item":null}"#;

/// Poll `what` for up to 3 s.
fn wait_until(what: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while !what() {
        assert!(Instant::now() < deadline, "timed out");
        thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn a_429_during_a_poll_skips_polls_until_it_passes() {
    let now = Arc::new(AtomicU64::new(1_000_000));
    let clock_now = now.clone();
    let clock: Clock = Arc::new(move || clock_now.load(Ordering::SeqCst));
    let reads = Arc::new(AtomicUsize::new(0));
    let counter = reads.clone();
    let f = fixture_with_clock(
        true,
        "",
        move |request| {
            if request.method == "GET" && request.path() == "/v1/me/player" {
                let read = counter.fetch_add(1, Ordering::SeqCst);
                if read == 0 {
                    return Response::empty(429).with_header("Retry-After", "10");
                }
            }
            api_default(request)
        },
        clock,
    );
    f.service.apply(SpotifyOp::Touch).unwrap();
    wait_until(|| reads.load(Ordering::SeqCst) == 1);
    // Backoff runs for 10 s from the 429. Watching polls are skipped until then.
    now.fetch_add(9_000, Ordering::SeqCst);
    f.service.apply(SpotifyOp::Touch).unwrap();
    thread::sleep(Duration::from_millis(300));
    assert_eq!(
        reads.load(Ordering::SeqCst),
        1,
        "a poll ran during the backoff"
    );
    now.fetch_add(2_000, Ordering::SeqCst);
    f.service.apply(SpotifyOp::Touch).unwrap();
    wait_until(|| reads.load(Ordering::SeqCst) >= 2);
}

#[test]
fn a_poll_error_clears_on_a_good_poll_but_not_a_command_error() {
    let failing = Arc::new(AtomicBool::new(true));
    let flag = failing.clone();
    let f = fixture(true, "", move |request| {
        match (request.method.as_str(), request.path()) {
            ("GET", "/v1/me/player") if flag.load(Ordering::SeqCst) => {
                Response::json(500, r#"{"error":{"status":500,"message":"boom"}}"#)
            }
            (_, "/v1/me/player/next") => {
                Response::json(404, r#"{"error":{"reason":"NO_ACTIVE_DEVICE"}}"#)
            }
            _ => api_default(request),
        }
    });
    let shared = &f.service.shared;
    f.service.apply(SpotifyOp::Touch).unwrap();
    wait_until(|| f.service.state().last_error.is_some());
    let error = f.service.state().last_error.unwrap_or_default();
    assert!(error.contains("500"), "{error}");

    failing.store(false, Ordering::SeqCst);
    f.service.apply(SpotifyOp::Touch).unwrap();
    wait_until(|| f.service.state().playback.is_some());
    assert_eq!(f.service.state().last_error, None);

    failing.store(true, Ordering::SeqCst);
    f.service.apply(SpotifyOp::Touch).unwrap();
    wait_until(|| shared.poll.last_error.lock().unwrap().is_some());
    let command_error = f.service.apply(SpotifyOp::Next).unwrap_err().to_string();
    assert_eq!(f.service.state().last_error, Some(command_error.clone()));

    failing.store(false, Ordering::SeqCst);
    f.service.apply(SpotifyOp::Touch).unwrap();
    wait_until(|| shared.poll.last_error.lock().unwrap().is_none());
    assert_eq!(f.service.state().last_error, Some(command_error));
}

#[test]
fn a_read_from_an_earlier_login_cannot_write_after_a_logout() {
    let f = fixture(true, "", api_default);
    let shared = &f.service.shared;
    let epoch = shared.login_epoch.load(Ordering::SeqCst);
    f.service.apply(SpotifyOp::Logout).unwrap();
    let wrote = shared.update_if_current(epoch, |state| state.devices.clear());
    assert!(!wrote);
    let now = shared.login_epoch.load(Ordering::SeqCst);
    assert!(shared.update_if_current(now, |state| state.devices.clear()));
}

#[test]
fn a_logout_during_a_poll_leaves_no_playback() {
    let reads = Arc::new(AtomicUsize::new(0));
    let counter = reads.clone();
    let (release_tx, release_rx) = mpsc::channel::<()>();
    let f = fixture(true, "", move |request| {
        if request.method == "GET" && request.path() == "/v1/me/player" {
            // The second read is held until the test releases it.
            if counter.fetch_add(1, Ordering::SeqCst) == 1 {
                let _ = release_rx.recv();
            }
        }
        api_default(request)
    });
    f.service.apply(SpotifyOp::Touch).unwrap();
    wait_until(|| f.service.state().playback.is_some());
    f.service.apply(SpotifyOp::Touch).unwrap();
    wait_until(|| reads.load(Ordering::SeqCst) >= 2);
    thread::scope(|scope| {
        let logout = scope.spawn(|| f.service.apply(SpotifyOp::Logout));
        // Logout waits for the poll's client lock, which the held read keeps.
        thread::sleep(Duration::from_millis(100));
        release_tx.send(()).unwrap();
        logout.join().unwrap().unwrap();
    });
    assert_eq!(f.service.state().playback, None);
}

fn transfer(device_id: &str, play: bool) -> SpotifyOp {
    SpotifyOp::Transfer {
        device_id: device_id.into(),
        play,
    }
}

/// Playback on `device_id`, as `GET /me/player` answers it.
fn playback_on(device_id: &str) -> String {
    format!(
        r#"{{"is_playing":true,"progress_ms":5,"currently_playing_type":"track",
            "device":{{"id":"{device_id}","name":"Phone","type":"Smartphone","is_active":true}},
            "item":{{"type":"track","uri":"spotify:track:t","name":"So What","duration_ms":9,
                    "artists":[],"album":{{"name":"Kind of Blue","images":[]}}}}}}"#
    )
}

#[test]
fn a_transfer_that_takes_over_is_done() {
    let f = fixture(true, "", |request| {
        match (request.method.as_str(), request.path()) {
            ("PUT", "/v1/me/player") => Response::empty(204),
            ("GET", "/v1/me/player") => Response::json(200, &playback_on("target")),
            _ => api_default(request),
        }
    });
    f.service.apply(transfer("target", false)).unwrap();
    assert!(requests(&f.seen).contains(&"PUT /v1/me/player".to_string()));
    let playback = f.service.state().playback.expect("playback");
    assert_eq!(playback.device.map(|d| d.id).as_deref(), Some("target"));
}

#[test]
fn a_transfer_the_device_never_takes_is_reported() {
    let f = fixture(true, "", |request| {
        match (request.method.as_str(), request.path()) {
            ("PUT", "/v1/me/player") => Response::empty(204),
            ("GET", "/v1/me/player") => Response::json(200, &playback_on("other")),
            _ => api_default(request),
        }
    });
    let error = f.service.apply(transfer("target", false)).unwrap_err();
    assert!(error.to_string().contains("did not take over"), "{error}");
    assert!(
        matches!(
            error,
            ZniczError::Spotify {
                kind: SpotifyErrorKind::Unavailable,
                ..
            }
        ),
        "{error:?}"
    );
}

#[test]
fn a_play_transfer_the_device_never_takes_does_not_pause_znicz() {
    let f = fixture(true, "", |request| {
        match (request.method.as_str(), request.path()) {
            ("PUT", "/v1/me/player") => Response::empty(204),
            ("GET", "/v1/me/player") => Response::json(200, &playback_on("other")),
            _ => api_default(request),
        }
    });
    f.service.apply(transfer("target", true)).unwrap_err();
    assert_eq!(f.starts.load(Ordering::SeqCst), 0);
}

#[test]
fn a_transfer_whose_playback_cannot_be_read_says_so() {
    let f = fixture(true, "", |request| {
        match (request.method.as_str(), request.path()) {
            ("PUT", "/v1/me/player") => Response::empty(204),
            ("GET", "/v1/me/player") => Response::json(500, r#"{"error":{"status":500}}"#),
            _ => api_default(request),
        }
    });
    let error = f.service.apply(transfer("target", false)).unwrap_err();
    assert!(!error.to_string().contains("did not take over"), "{error}");
    assert!(error.to_string().contains("500"), "{error}");
}

#[test]
fn a_transfer_read_that_fails_once_then_shows_the_target_is_done() {
    let reads = Arc::new(AtomicUsize::new(0));
    let counter = reads.clone();
    let f = fixture(true, "", move |request| {
        match (request.method.as_str(), request.path()) {
            ("PUT", "/v1/me/player") => Response::empty(204),
            ("GET", "/v1/me/player") => {
                if counter.fetch_add(1, Ordering::SeqCst) == 0 {
                    Response::json(500, r#"{"error":{"status":500}}"#)
                } else {
                    Response::json(200, &playback_on("target"))
                }
            }
            _ => api_default(request),
        }
    });
    f.service.apply(transfer("target", false)).unwrap();
    assert_eq!(reads.load(Ordering::SeqCst), 2);
}

#[test]
fn a_rate_limited_transfer_read_is_returned_at_once() {
    let reads = Arc::new(AtomicUsize::new(0));
    let counter = reads.clone();
    let f = fixture(true, "", move |request| {
        match (request.method.as_str(), request.path()) {
            ("PUT", "/v1/me/player") => Response::empty(204),
            ("GET", "/v1/me/player") => {
                counter.fetch_add(1, Ordering::SeqCst);
                Response::empty(429).with_header("Retry-After", "30")
            }
            _ => api_default(request),
        }
    });
    let error = f.service.apply(transfer("target", false)).unwrap_err();
    assert!(
        matches!(
            error,
            ZniczError::Spotify {
                kind: SpotifyErrorKind::RateLimited,
                ..
            }
        ),
        "{error:?}"
    );
    assert_eq!(reads.load(Ordering::SeqCst), 1);
}

#[test]
fn a_played_transfer_counts_one_start() {
    let f = fixture(true, "", |request| {
        match (request.method.as_str(), request.path()) {
            ("PUT", "/v1/me/player") => Response::empty(204),
            ("GET", "/v1/me/player") => Response::json(200, &playback_on("target")),
            _ => api_default(request),
        }
    });
    f.service.apply(transfer("target", true)).unwrap();
    assert_eq!(f.starts.load(Ordering::SeqCst), 1);
}

#[test]
fn a_transfer_without_play_does_not_count_a_start() {
    let f = fixture(true, "", |request| {
        match (request.method.as_str(), request.path()) {
            ("PUT", "/v1/me/player") => Response::empty(204),
            ("GET", "/v1/me/player") => Response::json(200, &playback_on("target")),
            _ => api_default(request),
        }
    });
    f.service.apply(transfer("target", false)).unwrap();
    assert_eq!(f.starts.load(Ordering::SeqCst), 0);
}
