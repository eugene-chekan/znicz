//! The Spotify mode rules as pure functions: which key does what, the seek and
//! volume arithmetic, the mute plan, the labels and the error plan. No IO.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use znicz_core::{RepeatMode, SpotifyErrorKind, SpotifyOp, SpotifyRepeat, SpotifyState};

pub const SEEK_SMALL_SECS: i64 = 5;
pub const SEEK_LARGE_SECS: i64 = 30;
pub const VOLUME_STEP_PERCENT: i16 = 1;
/// The percent `m` restores to when no volume was remembered.
pub const UNMUTE_DEFAULT_PERCENT: u8 = 50;
pub const NOT_IN_MODE: &str = "not in Spotify mode";
pub const NOT_LOGGED_IN: &str = "Spotify: log in under Settings → Spotify (S, 6)";
/// The toast when the login is lost while Spotify mode is on; the mode is left.
pub const LOGGED_OUT: &str = "Spotify: logged out; O again after logging in";
pub const NO_STOP: &str = "Spotify has no stop: Space pauses";
pub const NOTHING_PLAYING: &str = "Nothing is playing on Spotify";
pub const ONE_TRACK_AT_A_TIME: &str = "Spotify's queue takes one track at a time";
pub const NO_VOLUME: &str = "Spotify does not report this device's volume";
/// The toast when a request has no connection to Spotify in this session.
pub const NO_LANE: &str = "Spotify is not connected in this session";
/// The message row when a change or a read did not answer in time.
pub const NETWORK_TEXT: &str =
    "Spotify did not answer: the change may still happen; the next status read shows the truth";
/// Whether a key keeps its global meaning in Spotify mode: `P`, `R`, `S`, `M`, `?`, `q`, `v`, `i`, `O`.
pub fn keeps_global(key: KeyEvent) -> bool {
    matches!(
        key.code,
        KeyCode::Char('q' | '?' | 'S' | 'P' | 'R' | 'M' | 'v' | 'i' | 'O')
    )
}

/// Keys the library owns and Spotify mode refuses.
pub const LIBRARY_ONLY_KEYS: [char; 5] = ['A', 'L', 'F', 'U', 'J'];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpotifyKey {
    PlayPause,
    Next,
    Previous,
    /// Seconds, signed.
    Seek(i64),
    /// Percent points, signed.
    Volume(i16),
    Repeat,
    Shuffle,
    Mute,
    Devices,
    QueueAdd,
    QueueDrawer,
    Search,
    Stop,
    Reload,
    LibraryOnly,
}

/// The transport keys of Spotify mode, or `None` for a key that is not Spotify's.
/// Navigation (`j`, `k`, arrows without seek, `Enter`, `h`, `l`, `g`, `G`, paging)
/// is the browser's and returns `None` here.
pub fn key_action(key: KeyEvent) -> Option<SpotifyKey> {
    if key
        .modifiers
        .intersects(KeyModifiers::ALT | KeyModifiers::CONTROL)
    {
        return None;
    }
    let big = key.modifiers.contains(KeyModifiers::SHIFT);
    let seek = if big {
        SEEK_LARGE_SECS
    } else {
        SEEK_SMALL_SECS
    };
    match key.code {
        KeyCode::Char(' ') => Some(SpotifyKey::PlayPause),
        KeyCode::Char('n') => Some(SpotifyKey::Next),
        KeyCode::Char('p') => Some(SpotifyKey::Previous),
        KeyCode::Right => Some(SpotifyKey::Seek(seek)),
        KeyCode::Left => Some(SpotifyKey::Seek(-seek)),
        KeyCode::Char('+' | '=') => Some(SpotifyKey::Volume(VOLUME_STEP_PERCENT)),
        KeyCode::Char('-' | '_') => Some(SpotifyKey::Volume(-VOLUME_STEP_PERCENT)),
        KeyCode::Char('e') => Some(SpotifyKey::Repeat),
        KeyCode::Char('z') => Some(SpotifyKey::Shuffle),
        KeyCode::Char('m') => Some(SpotifyKey::Mute),
        KeyCode::Char(',') => Some(SpotifyKey::Devices),
        KeyCode::Char('a') => Some(SpotifyKey::QueueAdd),
        KeyCode::Char(']') => Some(SpotifyKey::QueueDrawer),
        KeyCode::Char('/') => Some(SpotifyKey::Search),
        KeyCode::Char('s') => Some(SpotifyKey::Stop),
        KeyCode::Char('r') => Some(SpotifyKey::Reload),
        KeyCode::Char(c) if LIBRARY_ONLY_KEYS.contains(&c) => Some(SpotifyKey::LibraryOnly),
        _ => None,
    }
}

/// The seek target, clamped to `[0, duration]`. A zero duration is unknown: the
/// target is clamped at zero only.
pub fn seek_target(progress_ms: u64, delta_secs: i64, duration_ms: u64) -> u64 {
    let target = progress_ms as i64 + delta_secs * 1_000;
    if duration_ms == 0 {
        return target.max(0) as u64;
    }
    target.clamp(0, duration_ms as i64) as u64
}

/// `percent + delta`, clamped to 0..=100.
pub fn percent_after(percent: u8, delta: i16) -> u8 {
    (i16::from(percent) + delta).clamp(0, 100) as u8
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MuteAction {
    /// Remember `remember` and set volume 0.
    Mute { remember: u8 },
    /// Set volume to this percent and forget the remembered one.
    Unmute(u8),
    /// Nothing to do: the device reports no volume, so there is nothing to mute or restore.
    Nothing,
}

/// Decide `m`. `volume` is the device's percent (`None` when unknown); `restore` is the remembered percent.
pub fn mute_action(volume: Option<u8>, restore: Option<u8>) -> MuteAction {
    match (volume, restore) {
        (Some(percent), _) if percent > 0 => MuteAction::Mute { remember: percent },
        (Some(0), Some(percent)) | (None, Some(percent)) => MuteAction::Unmute(percent),
        (Some(0), None) => MuteAction::Unmute(UNMUTE_DEFAULT_PERCENT),
        _ => MuteAction::Nothing,
    }
}

/// Unix time in milliseconds: the clock the seek reads, to place the playhead.
pub fn now_unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

pub fn repeat_mode(repeat: SpotifyRepeat) -> RepeatMode {
    match repeat {
        SpotifyRepeat::Off => RepeatMode::Off,
        SpotifyRepeat::Context => RepeatMode::All,
        SpotifyRepeat::Track => RepeatMode::One,
    }
}

/// `♫ Spotify: <title> — <artist> · O to switch`, when something plays with an item.
pub fn now_playing_line(state: &SpotifyState) -> Option<String> {
    let playback = state.playback.as_ref()?;
    if !playback.is_playing {
        return None;
    }
    let item = playback.item.as_ref()?;
    let artist = item.artist_line();
    if artist.is_empty() {
        Some(format!("♫ Spotify: {} · O to switch", item.title))
    } else {
        Some(format!(
            "♫ Spotify: {} — {} · O to switch",
            item.title, artist
        ))
    }
}

/// The message row text for an error of a change or a read, and whether it stops polling.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ErrorPlan {
    pub text: String,
    pub stops_polling: bool,
    pub reread: bool,
}

pub fn error_plan(kind: SpotifyErrorKind, message: &str) -> ErrorPlan {
    match kind {
        SpotifyErrorKind::Network => ErrorPlan {
            text: NETWORK_TEXT.to_string(),
            stops_polling: false,
            reread: true,
        },
        SpotifyErrorKind::OutdatedPlayer | SpotifyErrorKind::NoService => ErrorPlan {
            text: message.to_string(),
            stops_polling: true,
            reread: false,
        },
        _ => ErrorPlan {
            text: message.to_string(),
            stops_polling: false,
            reread: false,
        },
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lane {
    Transport,
    Browse,
}

/// Transport ops go to the transport lane; everything else to the browse lane.
pub fn lane(op: &SpotifyOp) -> Lane {
    match op {
        SpotifyOp::Play { .. }
        | SpotifyOp::Pause
        | SpotifyOp::Next
        | SpotifyOp::Previous
        | SpotifyOp::Seek { .. }
        | SpotifyOp::SetVolume { .. }
        | SpotifyOp::SetRepeat { .. }
        | SpotifyOp::SetShuffle { .. }
        | SpotifyOp::QueueAdd { .. }
        | SpotifyOp::Transfer { .. } => Lane::Transport,
        _ => Lane::Browse,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use znicz_core::{SpotifyLogin, SpotifyPlayback, SpotifyTrack};

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn shift(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::SHIFT)
    }

    fn alt(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::ALT)
    }

    #[test]
    fn space_and_arrows_are_spotifys_and_alt_arrows_are_not() {
        assert_eq!(
            key_action(key(KeyCode::Char(' '))),
            Some(SpotifyKey::PlayPause)
        );
        assert_eq!(key_action(key(KeyCode::Right)), Some(SpotifyKey::Seek(5)));
        assert_eq!(
            key_action(shift(KeyCode::Left)),
            Some(SpotifyKey::Seek(-30))
        );
        assert_eq!(key_action(alt(KeyCode::Right)), None);
        assert_eq!(
            key_action(key(KeyCode::Down)),
            None,
            "navigation is the browser's"
        );
        assert_eq!(key_action(key(KeyCode::Enter)), None);
    }

    #[test]
    fn library_only_keys_are_named_and_the_rest_stay_global() {
        assert_eq!(
            key_action(key(KeyCode::Char('A'))),
            Some(SpotifyKey::LibraryOnly)
        );
        assert_eq!(
            key_action(key(KeyCode::Char('P'))),
            None,
            "P stays the playlists modal"
        );
    }

    #[test]
    fn the_letter_and_symbol_keys_map_to_their_actions() {
        let ctrl = KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL);
        assert_eq!(key_action(ctrl), None, "Ctrl is the browser's paging");
        assert_eq!(
            key_action(key(KeyCode::Char('+'))),
            Some(SpotifyKey::Volume(1))
        );
        assert_eq!(
            key_action(key(KeyCode::Char('_'))),
            Some(SpotifyKey::Volume(-1))
        );
        assert_eq!(
            key_action(key(KeyCode::Char('e'))),
            Some(SpotifyKey::Repeat)
        );
        assert_eq!(
            key_action(key(KeyCode::Char(']'))),
            Some(SpotifyKey::QueueDrawer)
        );
        assert_eq!(key_action(key(KeyCode::Char('s'))), Some(SpotifyKey::Stop));
    }

    #[test]
    fn seek_target_clamps_to_zero_and_duration() {
        assert_eq!(seek_target(2_000, -5, 180_000), 0);
        assert_eq!(seek_target(178_000, 30, 180_000), 180_000);
        assert_eq!(
            seek_target(10_000, 5, 0),
            15_000,
            "no duration: clamped only at zero"
        );
        assert_eq!(
            seek_target(10_000, -30, 0),
            0,
            "no duration: still not below zero"
        );
        assert_eq!(seek_target(10_000, 5, 180_000), 15_000);
    }

    #[test]
    fn volume_steps_one_percent_and_clamps() {
        assert_eq!(percent_after(100, 1), 100);
        assert_eq!(percent_after(0, -1), 0);
        assert_eq!(percent_after(40, 1), 41);
    }

    #[test]
    fn mute_remembers_the_volume_and_restores_it() {
        assert_eq!(
            mute_action(Some(40), None),
            MuteAction::Mute { remember: 40 }
        );
        assert_eq!(mute_action(Some(0), Some(40)), MuteAction::Unmute(40));
        assert_eq!(
            mute_action(Some(0), None),
            MuteAction::Unmute(UNMUTE_DEFAULT_PERCENT),
            "a silent device with nothing remembered comes back at 50 %"
        );
        assert_eq!(mute_action(Some(0), Some(0)), MuteAction::Unmute(0));
    }

    #[test]
    fn mute_with_unknown_volume_does_nothing() {
        assert_eq!(mute_action(None, None), MuteAction::Nothing);
        assert_eq!(mute_action(None, Some(30)), MuteAction::Unmute(30));
    }

    #[test]
    fn repeat_maps_to_znicz_labels() {
        assert_eq!(repeat_mode(SpotifyRepeat::Off), RepeatMode::Off);
        assert_eq!(repeat_mode(SpotifyRepeat::Context), RepeatMode::All);
        assert_eq!(repeat_mode(SpotifyRepeat::Track), RepeatMode::One);
    }

    fn state_playing(title: &str, artists: &[&str], is_playing: bool) -> SpotifyState {
        let item = SpotifyTrack {
            uri: "spotify:track:1".into(),
            title: title.into(),
            artists: artists.iter().map(|a| a.to_string()).collect(),
            ..SpotifyTrack::default()
        };
        SpotifyState {
            login: SpotifyLogin::LoggedIn,
            account: None,
            playback: Some(SpotifyPlayback {
                item: Some(item),
                unsupported: false,
                is_playing,
                progress_ms: 0,
                read_at_ms: 0,
                repeat: SpotifyRepeat::Off,
                shuffle: false,
                device: None,
                context_uri: None,
            }),
            devices: Vec::new(),
            last_error: None,
        }
    }

    #[test]
    fn the_now_playing_line_needs_a_playing_track() {
        let playing = state_playing("So What", &["Miles Davis"], true);
        assert_eq!(
            now_playing_line(&playing).as_deref(),
            Some("♫ Spotify: So What — Miles Davis · O to switch")
        );
        let paused = state_playing("So What", &["Miles Davis"], false);
        assert_eq!(now_playing_line(&paused), None);
        assert_eq!(now_playing_line(&SpotifyState::default()), None);
    }

    #[test]
    fn a_network_error_says_it_may_still_happen_and_rereads() {
        let plan = error_plan(SpotifyErrorKind::Network, "timed out");
        assert!(plan.reread && !plan.stops_polling);
        assert!(plan.text.contains("may still happen"));
    }

    #[test]
    fn an_old_player_stops_polling_and_says_why() {
        assert!(error_plan(SpotifyErrorKind::OutdatedPlayer, "old").stops_polling);
        assert!(error_plan(SpotifyErrorKind::NoService, "none").stops_polling);
    }

    #[test]
    fn a_device_error_keeps_its_message_and_does_not_stop_polling() {
        let plan = error_plan(SpotifyErrorKind::NoActiveDevice, "Pick a device with ,");
        assert_eq!(plan.text, "Pick a device with ,");
        assert!(!plan.stops_polling && !plan.reread);
    }

    #[test]
    fn transport_ops_are_on_the_transport_lane() {
        assert_eq!(lane(&SpotifyOp::Pause), Lane::Transport);
        assert_eq!(
            lane(&SpotifyOp::Transfer {
                device_id: "d".into(),
                play: true
            }),
            Lane::Transport
        );
        assert_eq!(
            lane(&SpotifyOp::Search {
                query: "x".into(),
                kinds: vec![],
                offset: 0,
                limit: 10
            }),
            Lane::Browse
        );
        assert_eq!(lane(&SpotifyOp::Devices), Lane::Browse);
    }
}
