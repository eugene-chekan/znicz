//! Fade-outs before a command: which commands fade, for how long, and the
//! commands waiting for a fade to finish. No audio here; the engine drives it.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::player::commands::{Command, CommandEnvelope};

/// How long each fade takes. Zero turns that fade off.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct FadeTimes {
    /// Fade in on play, resume, after a track change or a device switch.
    pub play: Duration,
    pub pause: Duration,
    pub stop: Duration,
    /// Fade out before a manual track change or a device switch.
    pub track: Duration,
    pub mute: Duration,
    /// Automatic track changes overlap by this much. Zero turns it off.
    #[serde(default)]
    pub crossfade: Duration,
}

impl Default for FadeTimes {
    fn default() -> Self {
        Self {
            play: Duration::from_millis(150),
            pause: Duration::from_millis(150),
            stop: Duration::from_millis(300),
            track: Duration::from_millis(150),
            mute: Duration::from_millis(500),
            crossfade: Duration::ZERO,
        }
    }
}

/// The frame where a crossfade into the next track starts: `length` before
/// the end, at most half the track. `None` when crossfade is off, for radio,
/// or when the track's length is unknown.
pub fn crossfade_from(
    length: Duration,
    track: Option<Duration>,
    rate: u32,
    stream: bool,
) -> Option<u64> {
    if length.is_zero() || stream {
        return None;
    }
    let total = frames(track?, rate);
    let fade = frames(length, rate).min(total / 2);
    (fade > 0).then(|| total - fade)
}

/// The next track is long enough to crossfade into: at least twice the
/// crossfade. An unknown length is given the benefit of the doubt.
pub fn incoming_long_enough(track: Option<Duration>, fade_frames: u64, rate: u32) -> bool {
    track.is_none_or(|t| frames(t, rate) >= 2 * fade_frames)
}

fn frames(time: Duration, rate: u32) -> u64 {
    (time.as_secs_f64() * f64::from(rate)).round() as u64
}

/// What the engine is doing, as far as fading is concerned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Playback {
    /// Sound is coming out: status Playing with an open stream.
    pub audible: bool,
    /// The queue row that is playing.
    pub position: usize,
    /// A decoder is open. Without one only the tail of a finished track is
    /// playing out, and a device switch has no stream to reopen.
    pub decoding: bool,
}

/// How long to fade out before `command`, or `None` to run it at once.
pub fn fade_out_time(command: &Command, times: &FadeTimes, playback: Playback) -> Option<Duration> {
    if !playback.audible {
        return None;
    }
    let time = match command {
        Command::Pause => times.pause,
        Command::Stop => times.stop,
        Command::NextTrack
        | Command::PreviousTrack
        | Command::Play(_)
        | Command::QueuePlayIndex(_)
        | Command::ReplaceQueue { .. } => times.track,
        Command::SetDevice(_) if playback.decoding => times.track,
        Command::QueueRemove(index) if *index == playback.position => times.track,
        _ => return None,
    };
    (!time.is_zero()).then_some(time)
}

/// Extra time the engine waits for the callback to report a fade landed.
pub const LANDING_GRACE: Duration = Duration::from_millis(500);

/// A fade-out in progress and the commands waiting for it, in arrival order.
pub struct FadeOut {
    pub generation: u64,
    pub deadline: Instant,
    waiting: VecDeque<CommandEnvelope>,
}

impl FadeOut {
    /// A fade-out of `time` started now for `first`.
    pub fn new(first: CommandEnvelope, generation: u64, time: Duration, now: Instant) -> Self {
        Self {
            generation,
            deadline: now + time + LANDING_GRACE,
            waiting: VecDeque::from([first]),
        }
    }

    /// Queue a command behind the waiting ones.
    pub fn wait(&mut self, envelope: CommandEnvelope) {
        self.waiting.push_back(envelope);
    }

    /// True when the only waiting command is a `Pause`: a `Resume` cancels it.
    pub fn only_a_pause(&self) -> bool {
        self.waiting.len() == 1 && matches!(self.waiting[0].command, Command::Pause)
    }

    /// The fade is done: landed, or past its deadline.
    pub fn is_due(&self, landed: bool, now: Instant) -> bool {
        landed || now >= self.deadline
    }

    /// The waiting commands, oldest first.
    pub fn into_waiting(self) -> VecDeque<CommandEnvelope> {
        self.waiting
    }
}

/// Commands that never wait for a fade-out.
pub fn runs_during_fade(command: &Command) -> bool {
    matches!(
        command,
        Command::SetVolume(_) | Command::SetMuted(_) | Command::SetFades(_)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::player::state::QueueItem;

    const PLAYING: Playback = Playback {
        audible: true,
        position: 2,
        decoding: true,
    };
    const SILENT: Playback = Playback {
        audible: false,
        position: 2,
        decoding: true,
    };

    #[test]
    fn a_device_switch_during_a_tail_does_not_fade() {
        let tail = Playback {
            decoding: false,
            ..PLAYING
        };
        let t = FadeTimes::default();
        assert_eq!(
            fade_out_time(&Command::SetDevice("x".into()), &t, tail),
            None
        );
        assert_eq!(fade_out_time(&Command::Pause, &t, tail), Some(t.pause));
    }

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    #[test]
    fn transport_commands_fade_while_playing() {
        let t = FadeTimes::default();
        assert_eq!(fade_out_time(&Command::Pause, &t, PLAYING), Some(t.pause));
        assert_eq!(fade_out_time(&Command::Stop, &t, PLAYING), Some(t.stop));
        for command in [
            Command::NextTrack,
            Command::PreviousTrack,
            Command::Play(QueueItem::file("/a.flac")),
            Command::QueuePlayIndex(0),
            Command::ReplaceQueue {
                items: vec![],
                position: 0,
            },
            Command::QueueRemove(2),
            Command::SetDevice("x".into()),
        ] {
            assert_eq!(
                fade_out_time(&command, &t, PLAYING),
                Some(t.track),
                "{command:?}"
            );
        }
    }

    #[test]
    fn nothing_fades_when_nothing_is_heard() {
        let t = FadeTimes::default();
        assert_eq!(fade_out_time(&Command::Pause, &t, SILENT), None);
        assert_eq!(fade_out_time(&Command::NextTrack, &t, SILENT), None);
    }

    #[test]
    fn other_commands_and_other_rows_do_not_fade() {
        let t = FadeTimes::default();
        assert_eq!(fade_out_time(&Command::QueueRemove(0), &t, PLAYING), None);
        assert_eq!(fade_out_time(&Command::Resume, &t, PLAYING), None);
        assert_eq!(fade_out_time(&Command::Seek(ms(1000)), &t, PLAYING), None);
        assert_eq!(fade_out_time(&Command::SetVolume(0.5), &t, PLAYING), None);
        assert_eq!(fade_out_time(&Command::QueueClear, &t, PLAYING), None);
    }

    #[test]
    fn a_zero_time_means_no_fade() {
        let t = FadeTimes {
            pause: Duration::ZERO,
            ..FadeTimes::default()
        };
        assert_eq!(fade_out_time(&Command::Pause, &t, PLAYING), None);
    }

    #[test]
    fn waiting_commands_keep_their_order() {
        let now = Instant::now();
        let mut fade = FadeOut::new(
            CommandEnvelope::new(Command::ReplaceQueue {
                items: vec![],
                position: 0,
            }),
            7,
            ms(150),
            now,
        );
        fade.wait(CommandEnvelope::new(Command::QueuePlayIndex(3)));
        let order: Vec<String> = fade
            .into_waiting()
            .iter()
            .map(|e| format!("{:?}", e.command))
            .collect();
        assert!(order[0].starts_with("ReplaceQueue"));
        assert_eq!(order[1], "QueuePlayIndex(3)");
    }

    #[test]
    fn resume_can_cancel_only_a_lone_pause() {
        let now = Instant::now();
        let mut fade = FadeOut::new(CommandEnvelope::new(Command::Pause), 1, ms(150), now);
        assert!(fade.only_a_pause());
        fade.wait(CommandEnvelope::new(Command::NextTrack));
        assert!(!fade.only_a_pause());
        let stop = FadeOut::new(CommandEnvelope::new(Command::Stop), 1, ms(150), now);
        assert!(!stop.only_a_pause());
    }

    #[test]
    fn due_when_landed_or_past_the_deadline() {
        let now = Instant::now();
        let fade = FadeOut::new(CommandEnvelope::new(Command::Stop), 1, ms(300), now);
        assert!(!fade.is_due(false, now));
        assert!(fade.is_due(true, now));
        assert!(fade.is_due(false, now + ms(300) + LANDING_GRACE + ms(1)));
    }

    #[test]
    fn crossfade_is_off_by_default() {
        assert_eq!(FadeTimes::default().crossfade, Duration::ZERO);
    }

    #[test]
    fn the_crossfade_starts_its_length_before_the_end() {
        let s = |n| Duration::from_secs(n);
        // 60 s track at 1 kHz, 5 s crossfade: start at frame 55_000.
        assert_eq!(crossfade_from(s(5), Some(s(60)), 1000, false), Some(55_000));
        // Capped at half the track.
        assert_eq!(crossfade_from(s(5), Some(s(6)), 1000, false), Some(3_000));
        // Off, radio, or an unknown length: none.
        assert_eq!(
            crossfade_from(Duration::ZERO, Some(s(60)), 1000, false),
            None
        );
        assert_eq!(crossfade_from(s(5), Some(s(60)), 1000, true), None);
        assert_eq!(crossfade_from(s(5), None, 1000, false), None);
    }

    #[test]
    fn a_short_next_track_is_not_crossfaded_into() {
        let s = |n| Duration::from_secs(n);
        assert!(incoming_long_enough(Some(s(10)), 5_000, 1000));
        assert!(!incoming_long_enough(Some(s(9)), 5_000, 1000));
        assert!(
            incoming_long_enough(None, 5_000, 1000),
            "unknown length: try"
        );
    }

    #[test]
    fn volume_mute_and_fade_times_never_wait() {
        assert!(runs_during_fade(&Command::SetVolume(0.3)));
        assert!(runs_during_fade(&Command::SetMuted(true)));
        assert!(runs_during_fade(&Command::SetFades(FadeTimes::default())));
        assert!(!runs_during_fade(&Command::Seek(ms(10))));
        assert!(!runs_during_fade(&Command::QueueAdd(vec![])));
    }
}
