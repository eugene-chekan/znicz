use std::path::Path;
use std::sync::{Arc, RwLock};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, RecvTimeoutError, Sender, bounded, unbounded};

use crate::audio::convert::RateConverter;
use crate::audio::crossfade::{Crossfade, DecodedTrack, next_packet, output_frames};
use crate::audio::feeder::{DecodeStep, Feeder, PumpOutcome};
use crate::audio::http::HttpStreamSource;
use crate::audio::icy::{IcyTitle, IcyUrl, apply_icy_to_track, apply_icy_url_to_track};
use crate::audio::output::AudioOutput;
use crate::audio::source::{AudioDecoder, AudioSource, LocalFileSource};
use crate::error::{Result, ZniczError};
use crate::player::commands::{Command, CommandEnvelope, PlayerEvent};
use crate::player::fade::{self, FadeOut, FadeTimes, Playback, fade_out_time, runs_during_fade};
use crate::player::gapless::{PendingTrack, PendingTracks, may_chain, resolve_position};
use crate::player::state::{
    OutputInfo, PlaybackStatus, PlayerState, QueueItem, RepeatMode, TrackInfo,
};

/// Commands and snapshots used by the TUI, MCP, and the localhost IPC attach.
pub trait PlayerOps: Send {
    fn send_blocking(&self, command: Command) -> Result<()>;
    fn state(&self) -> PlayerState;
    fn drain_events(&self) -> Vec<PlayerEvent> {
        Vec::new()
    }
}

/// Non-zero starting point for the shuffle generator.
fn seed_from_clock() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0x9E3779B97F4A7C15)
        | 1
}

#[derive(Debug, Clone)]
pub struct AudioConfig {
    pub device_id: Option<String>,
    pub volume: f32,
    pub bit_perfect: bool,
    pub fades: FadeTimes,
}

impl Default for AudioConfig {
    fn default() -> Self {
        Self {
            device_id: None,
            volume: 1.0,
            bit_perfect: true,
            fades: FadeTimes::default(),
        }
    }
}

/// How long `send_blocking` waits for the engine before giving up.
pub const COMMAND_TIMEOUT: Duration = Duration::from_secs(20);

#[derive(Clone)]
pub struct PlayerHandle {
    command_tx: Sender<CommandEnvelope>,
    state: Arc<RwLock<PlayerState>>,
    event_rx: Receiver<PlayerEvent>,
}

impl PlayerHandle {
    /// Queue a command without waiting. State may still be stale right after
    /// this returns, so only use it where a later redraw picks up the change.
    pub fn send(&self, command: Command) -> Result<()> {
        self.command_tx
            .send(CommandEnvelope::new(command))
            .map_err(|e| ZniczError::Player(e.to_string()))?;
        Ok(())
    }

    /// Queue a command and wait until the engine has applied it.
    ///
    /// Returns the engine's own result, so a failure (missing file, unusable
    /// device) reaches the caller instead of only being logged as an event.
    /// After this returns `Ok`, [`PlayerHandle::state`] reflects the command.
    pub fn send_blocking(&self, command: Command) -> Result<()> {
        self.send_blocking_timeout(command, COMMAND_TIMEOUT)
    }

    pub fn send_blocking_timeout(&self, command: Command, timeout: Duration) -> Result<()> {
        let (ack_tx, ack_rx) = bounded(1);
        self.command_tx
            .send(CommandEnvelope::with_ack(command, ack_tx))
            .map_err(|e| ZniczError::Player(e.to_string()))?;

        ack_rx
            .recv_timeout(timeout)
            .map_err(|_| ZniczError::Player("player did not answer in time".into()))?
    }

    pub fn state(&self) -> PlayerState {
        self.state.read().unwrap().clone()
    }

    pub fn state_arc(&self) -> Arc<RwLock<PlayerState>> {
        self.state.clone()
    }

    pub fn try_recv_event(&self) -> Option<PlayerEvent> {
        self.event_rx.try_recv().ok()
    }

    pub fn drain_events(&self) -> Vec<PlayerEvent> {
        let mut events = Vec::new();
        while let Ok(event) = self.event_rx.try_recv() {
            events.push(event);
        }
        events
    }
}

impl PlayerOps for PlayerHandle {
    fn send_blocking(&self, command: Command) -> Result<()> {
        PlayerHandle::send_blocking(self, command)
    }

    fn state(&self) -> PlayerState {
        PlayerHandle::state(self)
    }

    fn drain_events(&self) -> Vec<PlayerEvent> {
        PlayerHandle::drain_events(self)
    }
}

pub fn spawn_player(config: AudioConfig) -> (PlayerHandle, JoinHandle<()>) {
    let (command_tx, command_rx) = unbounded();
    let (event_tx, event_rx) = unbounded();
    let state = Arc::new(RwLock::new(PlayerState {
        volume: config.volume,
        device_id: config.device_id.clone(),
        ..PlayerState::default()
    }));

    let handle = PlayerHandle {
        command_tx,
        state: state.clone(),
        event_rx,
    };

    let thread = thread::Builder::new()
        .name("znicz-player".into())
        .spawn(move || {
            let mut engine = PlayerEngine::new(config, state, command_rx, event_tx);
            engine.run();
        })
        .expect("failed to spawn player thread");

    (handle, thread)
}

struct PlayerEngine {
    config: AudioConfig,
    state: Arc<RwLock<PlayerState>>,
    command_rx: Receiver<CommandEnvelope>,
    event_tx: Sender<PlayerEvent>,
    output: AudioOutput,
    decoder: Option<AudioDecoder>,
    feeder: Feeder,
    converter: Option<RateConverter>,
    /// Chained tracks already in the buffer but not yet heard (gapless).
    pending: PendingTracks,
    draining_since: Option<Instant>,
    last_position_tick: Instant,
    /// Seed for shuffle picks.
    rng: u64,
    /// How long each fade takes; `SetFades` replaces it.
    fades: FadeTimes,
    /// A fade-out in progress, and the commands waiting for it.
    fade_out: Option<FadeOut>,
    /// A crossfade in progress. It owns both decoders while it runs.
    crossfade: Option<Crossfade>,
    /// Frame where the current decoder starts a crossfade, if it does.
    crossfade_from: Option<u64>,
    /// Stated length of the current track, for working out a crossfade.
    track_duration: Option<Duration>,
}

/// Keep some headroom before decoding another packet.
const MIN_WRITE_SLOTS: usize = 4096;
/// Bound on work per loop so commands stay responsive.
const MAX_PACKETS_PER_PUMP: usize = 32;

impl PlayerEngine {
    fn new(
        config: AudioConfig,
        state: Arc<RwLock<PlayerState>>,
        command_rx: Receiver<CommandEnvelope>,
        event_tx: Sender<PlayerEvent>,
    ) -> Self {
        let output = AudioOutput::new();
        output.set_volume(config.volume);
        let fades = config.fades;

        Self {
            config,
            state,
            command_rx,
            event_tx,
            output,
            decoder: None,
            feeder: Feeder::new(),
            converter: None,
            pending: PendingTracks::default(),
            draining_since: None,
            last_position_tick: Instant::now(),
            rng: seed_from_clock(),
            fades,
            fade_out: None,
            crossfade: None,
            crossfade_from: None,
            track_duration: None,
        }
    }

    fn run(&mut self) {
        loop {
            // Waiting on the channel replaces a plain sleep: commands are
            // picked up the moment they arrive, so an acknowledged command
            // does not wait out a sleep interval.
            let busy =
                self.fade_out.is_some() || self.has_source() || self.draining_since.is_some();
            let idle = if busy {
                Duration::from_millis(5)
            } else {
                Duration::from_millis(50)
            };

            match self.command_rx.recv_timeout(idle) {
                Ok(envelope) => {
                    self.apply(envelope);
                    // Take anything else already queued before pumping audio.
                    while let Ok(envelope) = self.command_rx.try_recv() {
                        self.apply(envelope);
                    }
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return,
            }

            self.finish_fade_out();
            self.pump_decode();
            self.advance_pending();

            // A track that ends during a fade-out waits for it: the waiting
            // command (pause, stop, next) applies to this track, not the next.
            if self.fade_out.is_none() && self.drain_complete() {
                self.finish_track();
            }

            self.tick_position();
        }
    }

    /// Run one command, or hold it while a fade-out finishes.
    fn apply(&mut self, envelope: CommandEnvelope) {
        if let Some(fade) = self.fade_out.as_mut() {
            if runs_during_fade(&envelope.command) {
                self.run_command(envelope);
            } else if matches!(envelope.command, Command::Resume) && fade.only_a_pause() {
                self.cancel_pause(envelope);
            } else {
                fade.wait(envelope);
            }
            return;
        }
        match fade_out_time(&envelope.command, &self.fades, self.playback()) {
            Some(time) => {
                let generation = self.output.fade_to(0.0, time);
                self.fade_out = Some(FadeOut::new(envelope, generation, time, Instant::now()));
            }
            None => self.run_command(envelope),
        }
    }

    /// Run a command now and answer its caller.
    fn run_command(&mut self, envelope: CommandEnvelope) {
        let CommandEnvelope { command, ack } = envelope;
        let result = self.handle_command(command);

        if let Err(e) = &result {
            self.emit_error(e.to_string());
        }

        if let Some(ack) = ack {
            // A caller that gave up waiting is fine; ignore the send error.
            ack.send(result).ok();
        }
    }

    /// What fading needs to know about the engine now.
    fn playback(&self) -> Playback {
        let state = self.state.read().unwrap();
        Playback {
            audible: state.status == PlaybackStatus::Playing
                && (self.has_source() || self.draining_since.is_some()),
            position: state.queue_position,
            decoding: self.has_source(),
        }
    }

    /// Resume during a pause's fade-out: the pause never happens; the sound
    /// comes back up from where the fade got to.
    fn cancel_pause(&mut self, resume: CommandEnvelope) {
        let Some(fade) = self.fade_out.take() else {
            return;
        };
        self.output.fade_to(1.0, self.fades.play);
        for envelope in fade.into_waiting().into_iter().chain([resume]) {
            if let Some(ack) = envelope.ack {
                ack.send(Ok(())).ok();
            }
        }
    }

    /// Once a fade-out landed, run the commands that waited for it, in order.
    /// A command that starts another fade-out takes the rest along.
    fn finish_fade_out(&mut self) {
        let due = self.fade_out.as_ref().is_some_and(|fade| {
            fade.is_due(self.output.fade_landed(fade.generation), Instant::now())
        });
        if !due {
            return;
        }
        let mut waiting = self
            .fade_out
            .take()
            .map(FadeOut::into_waiting)
            .unwrap_or_default();
        // The fade for the first command already happened.
        if let Some(first) = waiting.pop_front() {
            self.run_command(first);
        }
        while let Some(envelope) = waiting.pop_front() {
            self.apply(envelope);
            if let Some(fade) = self.fade_out.as_mut() {
                for rest in waiting.drain(..) {
                    fade.wait(rest);
                }
            }
        }
    }

    fn handle_command(&mut self, command: Command) -> Result<()> {
        match command {
            Command::Play(item) => self.play_item(item, true)?,
            Command::Pause => {
                self.output.set_paused(true);
                self.update_status(PlaybackStatus::Paused);
                self.emit_state_changed();
            }
            Command::Resume => {
                // Only a paused track fades back in; Resume while playing
                // must not dip the sound.
                if self.state.read().unwrap().status == PlaybackStatus::Paused {
                    // Silent before unpausing, so no frame plays loud; the
                    // ramp starts once the stream runs again.
                    self.output.set_fade_now(0.0);
                    self.output.set_paused(false);
                    self.output.fade_in(self.fades.play);
                }
                self.update_status(PlaybackStatus::Playing);
                self.emit_state_changed();
            }
            Command::Stop => self.stop()?,
            Command::Seek(pos) => self.seek(pos)?,
            Command::SetVolume(vol) => {
                let v = vol.clamp(0.0, 1.0);
                let (before, effective) = {
                    let mut state = self.state.write().unwrap();
                    let before = state.effective_volume();
                    state.volume = v;
                    (before, state.effective_volume())
                };
                // Unchanged (volume keys while muted): leave a running mute
                // fade alone instead of cutting it short with a 20 ms ramp.
                if effective != before {
                    self.output.set_volume(effective);
                }
                self.emit_state_changed();
            }
            Command::SetMuted(muted) => {
                let effective = {
                    let mut state = self.state.write().unwrap();
                    state.muted = muted;
                    state.effective_volume()
                };
                self.output.set_gain(effective, self.fades.mute);
                self.emit_state_changed();
            }
            Command::NextTrack => self.next_track()?,
            Command::PreviousTrack => self.previous_track()?,
            Command::QueueAdd(items) => {
                let mut state = self.state.write().unwrap();
                state.queue.items_mut().extend(items);
                self.event_tx.send(PlayerEvent::QueueChanged).ok();
                self.emit_state_changed();
            }
            Command::QueueClear => {
                let mut state = self.state.write().unwrap();
                state.queue.items_mut().clear();
                state.queue_position = 0;
                self.event_tx.send(PlayerEvent::QueueChanged).ok();
                self.emit_state_changed();
            }
            Command::QueuePlayIndex(index) => self.play_queue_index(index, true)?,
            Command::QueueRemove(index) => self.remove_from_queue(index)?,
            Command::QueueMove { from, to } => self.move_in_queue(from, to)?,
            Command::ReplaceQueue { items, position } => self.replace_queue(items, position)?,
            Command::SetRepeat(mode) => {
                self.state.write().unwrap().repeat = mode;
                self.emit_state_changed();
            }
            Command::SetShuffle(on) => {
                self.state.write().unwrap().shuffle = on;
                self.emit_state_changed();
            }
            Command::SetDevice(id) => {
                self.config.device_id = Some(id);
                if let Some((rate, ch)) = self.source_format() {
                    let playing = self.state.read().unwrap().status == PlaybackStatus::Playing;
                    // A reopened stream starts from the fade gain it is given:
                    // silent when it fades in, full when paused (Resume fades).
                    self.output.set_fade_now(if playing { 0.0 } else { 1.0 });
                    self.output
                        .open_stream(rate, ch, self.config.device_id.as_deref())?;
                    if playing {
                        self.output.fade_in(self.fades.play);
                    }
                    // The reopened stream starts empty: the chained tracks in
                    // the old buffer are gone, so show the decoder's track.
                    self.start_all_pending();
                }
                self.emit_state_changed();
            }
            Command::SetFades(times) => {
                self.fades = times;
                self.arm_crossfade(self.track_duration);
            }
        }
        Ok(())
    }

    /// Open `item` and start it. `fade_in` fades the sound up from silence;
    /// a track that follows another without a command does not.
    fn play_item(&mut self, item: crate::player::state::QueueItem, fade_in: bool) -> Result<()> {
        // Stop first so a failed open cannot leave the previous file playing.
        self.stop()?;

        let source: Box<dyn AudioSource> = match &item {
            crate::player::state::QueueItem::File { path } => {
                Box::new(LocalFileSource::new(path.clone()))
            }
            crate::player::state::QueueItem::Stream { name, url } => {
                Box::new(HttpStreamSource::new(name.clone(), url.clone()))
            }
        };
        let (decoder, track_info) = AudioDecoder::open(source.as_ref())?;
        let sample_rate = decoder.sample_rate();
        let channels = decoder.channels();

        // Set before the stream opens: a stream starts from this gain.
        self.output.set_fade_now(if fade_in { 0.0 } else { 1.0 });
        self.output
            .open_stream(sample_rate, channels, self.config.device_id.as_deref())?;
        self.output.set_paused(false);
        if fade_in {
            self.output.fade_in(self.fades.play);
        }
        self.output
            .set_volume(self.state.read().unwrap().effective_volume());

        {
            let mut state = self.state.write().unwrap();
            state.device_id = self.config.device_id.clone();
            state.device_name = self.output.device_name().map(str::to_string);
        }

        let output_rate = self.output.sample_rate();
        let output_channels = self.output.channels().max(1);

        if output_channels != channels {
            tracing::info!(
                file_channels = channels,
                device_channels = output_channels,
                "device channel count differs from file, remapping channels"
            );
        }

        // A rate mismatch would play the track at the wrong speed, so convert.
        self.converter = if output_rate != sample_rate {
            tracing::warn!(
                file_rate = sample_rate,
                device_rate = output_rate,
                "device refused the file sample rate, resampling (not bit perfect)"
            );
            Some(RateConverter::new(
                sample_rate,
                output_rate,
                output_channels as usize,
            ))
        } else {
            None
        };

        self.decoder = Some(decoder);
        self.arm_crossfade(track_info.duration);
        self.draining_since = None;
        self.feeder.reset();
        self.last_position_tick = Instant::now();

        let output_info = OutputInfo {
            sample_rate: output_rate,
            channels: output_channels,
            sample_format: self.output.sample_format().unwrap_or("?").to_string(),
            bit_perfect: output_rate == sample_rate && output_channels == channels,
        };

        let mut state = self.state.write().unwrap();
        state.output = Some(output_info);

        if state.queue.is_empty() {
            state.queue.replace(vec![item.clone()]);
            state.queue_position = 0;
        } else if let Some(pos) = state.queue.iter().position(|row| row == &item) {
            state.queue_position = pos;
        } else {
            state.queue.items_mut().push(item.clone());
            state.queue_position = state.queue.len() - 1;
        }

        state.current_track = Some(track_info.clone());
        state.status = PlaybackStatus::Playing;
        state.position = Duration::ZERO;

        self.event_tx
            .send(PlayerEvent::TrackStarted(Box::new(track_info)))
            .ok();
        self.emit_state_changed();
        Ok(())
    }

    fn stop(&mut self) -> Result<()> {
        self.decoder = None;
        self.converter = None;
        self.crossfade = None;
        self.crossfade_from = None;
        self.track_duration = None;
        self.draining_since = None;
        self.pending.clear();
        self.output.stop_stream();
        self.feeder.reset();

        let mut state = self.state.write().unwrap();
        state.status = PlaybackStatus::Stopped;
        state.position = Duration::ZERO;
        state.current_track = None;
        state.output = None;
        drop(state);

        self.emit_state_changed();
        Ok(())
    }

    fn seek(&mut self, position: Duration) -> Result<()> {
        if self.queue_row_is_stream() {
            return Err(ZniczError::Player("radio cannot seek".into()));
        }

        self.end_heard_crossfade();
        if !self.pending.is_empty() {
            // The decoder already reads the next track; reopen this one.
            self.replay_current()?;
        }

        if let Some(decoder) = &mut self.decoder {
            decoder.seek(position)?;
            self.output.request_flush();
            self.output.wait_flush();

            // Anything still buffered belongs to the old position.
            self.feeder.reset();
            if let Some(converter) = self.converter.as_mut() {
                converter.reset();
            }

            let rate = self.output.sample_rate() as f64;
            let channels = self.output.channels().max(1) as f64;
            self.feeder
                .set_pushed_samples((position.as_secs_f64() * rate * channels) as u64);

            self.state.write().unwrap().position = position;
            self.emit_state_changed();
        }
        Ok(())
    }

    /// Reopen the playing row from the start, keeping it paused if it was.
    fn replay_current(&mut self) -> Result<()> {
        let (index, paused, row_exists) = {
            let state = self.state.read().unwrap();
            (
                state.queue_position,
                state.status == PlaybackStatus::Paused,
                state.queue_position < state.queue.len(),
            )
        };
        if !row_exists {
            // The queue was cleared under the chained audio: nothing to reopen.
            return Err(ZniczError::Player("nothing to seek in".into()));
        }
        self.play_queue_index(index, false)?;
        if paused {
            self.output.set_paused(true);
            self.update_status(PlaybackStatus::Paused);
        }
        Ok(())
    }

    fn queue_row_is_stream(&self) -> bool {
        let state = self.state.read().unwrap();
        state
            .queue
            .get(state.queue_position)
            .map(|item| item.is_stream())
            .unwrap_or(false)
    }

    fn next_track(&mut self) -> Result<()> {
        match self.pick_next(false) {
            Some(index) => self.play_queue_index(index, true),
            None => Ok(()),
        }
    }

    /// Which queue entry to play after the playing one, or `None` to stop.
    fn pick_next(&mut self, auto: bool) -> Option<usize> {
        let current = self.state.read().unwrap().queue_position;
        self.pick_next_after(current, auto)
    }

    /// Which queue entry follows `current`, or `None` to stop.
    ///
    /// `auto` marks the end of a track, where "repeat one" replays it. Pressing
    /// next always moves on instead.
    fn pick_next_after(&mut self, current: usize, auto: bool) -> Option<usize> {
        let (len, repeat, shuffle) = {
            let state = self.state.read().unwrap();
            (state.queue.len(), state.repeat, state.shuffle)
        };

        if len == 0 {
            return None;
        }
        if auto && repeat == RepeatMode::One {
            return Some(current);
        }
        if shuffle {
            return Some(self.random_index(len, current));
        }
        if current + 1 < len {
            return Some(current + 1);
        }
        if repeat == RepeatMode::All {
            return Some(0);
        }
        None
    }

    /// Open the track after the one that just finished decoding, if it can
    /// follow it in the same stream: a local file with the same sample rate
    /// and channel count. `None` means drain and reopen as before; a file that
    /// fails to open is reported by that path, not here.
    fn chain_next(&mut self, finished: &AudioDecoder) -> Option<AudioDecoder> {
        if !may_chain(finished.is_stream()) {
            return None;
        }
        let (track, decoder) = self.open_next_row(finished)?;
        self.pending.push(track);
        Some(decoder)
    }

    /// The queue row after the last chained track (or after the playing row),
    /// opened if it is a local file that matches `finished`. Shared by gapless
    /// chaining and crossfades; the caller queues the track as pending.
    fn open_next_row(&mut self, finished: &AudioDecoder) -> Option<(PendingTrack, AudioDecoder)> {
        let after = {
            let state = self.state.read().unwrap();
            match self.pending.last() {
                // Pick after the last chained row where it is now; if it was
                // removed, the caller gives up and the normal path picks.
                Some(last) => resolve_position(&state.queue, last.index, &last.item)?,
                None => state.queue_position,
            }
        };
        let index = self.pick_next_after(after, true)?;
        let item = self.state.read().unwrap().queue.get(index).cloned()?;
        let QueueItem::File { path } = &item else {
            return None;
        };
        let (decoder, info) = open_matching(path, finished)?;
        let track = PendingTrack {
            index,
            item,
            info,
            starts_at: self.feeder.pushed_samples(),
        };
        Some((track, decoder))
    }

    /// What the engine does when the playing decoder runs out of packets.
    fn on_decoder_finished(&mut self, decoder: AudioDecoder, converter: Option<RateConverter>) {
        self.publish_stream_bitrate(&decoder);
        self.publish_stream_title(&decoder);
        // The track ended before its crossfade point: the end is the join.
        self.crossfade_from = None;
        match self.chain_next(&decoder) {
            Some(next) => {
                // Same stream, buffer and converter: the next track's first
                // sample follows this one's last. The converter's few frames
                // of filter latency still belong to the old track, so the
                // switch lands a few milliseconds early; that is inaudible.
                let duration = self.pending.last().and_then(|track| track.info.duration);
                self.decoder = Some(next);
                self.converter = converter;
                self.arm_crossfade(duration);
            }
            None => {
                // The buffer still holds audio. Let it play out, otherwise
                // the last seconds of the track are cut off.
                self.converter = None;
                self.draining_since = Some(Instant::now());
            }
        }
    }

    /// A queue slot other than the current one, so shuffle does not repeat a
    /// track straight away.
    fn random_index(&mut self, len: usize, current: usize) -> usize {
        if len <= 1 {
            return 0;
        }
        // xorshift64: plenty for picking tracks, and keeps the dependency list short.
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;

        let pick = (self.rng % (len as u64 - 1)) as usize;
        if pick >= current { pick + 1 } else { pick }
    }

    fn play_queue_index(&mut self, index: usize, fade_in: bool) -> Result<()> {
        let item = {
            let state = self.state.read().unwrap();
            match state.queue.get(index) {
                Some(item) => item.clone(),
                None => return Ok(()),
            }
        };
        self.state.write().unwrap().queue_position = index;
        self.play_item(item, fade_in)?;
        let mut state = self.state.write().unwrap();
        if index < state.queue.len() {
            state.queue_position = index;
        }
        Ok(())
    }

    fn replace_queue(&mut self, items: Vec<QueueItem>, position: usize) -> Result<()> {
        self.stop()?;
        let mut state = self.state.write().unwrap();
        let len = items.len();
        state.queue.replace(items);
        state.queue_position = if len == 0 { 0 } else { position.min(len - 1) };
        drop(state);
        self.event_tx.send(PlayerEvent::QueueChanged).ok();
        self.emit_state_changed();
        Ok(())
    }

    fn move_in_queue(&mut self, from: usize, to: usize) -> Result<()> {
        {
            let mut state = self.state.write().unwrap();
            let len = state.queue.len();
            if from >= len || to >= len {
                return Err(ZniczError::Player("queue index out of range".into()));
            }
            if from == to {
                return Ok(());
            }
            let items = state.queue.items_mut();
            let item = items.remove(from);
            items.insert(to, item);
            state.queue_position = moved_position(state.queue_position, from, to);
        }
        self.event_tx.send(PlayerEvent::QueueChanged).ok();
        self.emit_state_changed();
        Ok(())
    }

    fn remove_from_queue(&mut self, index: usize) -> Result<()> {
        let playing_removed = {
            let mut state = self.state.write().unwrap();
            if index >= state.queue.len() {
                return Ok(());
            }
            let playing = state.status != PlaybackStatus::Stopped;
            let was_playing_row = index == state.queue_position && playing;
            state.queue.items_mut().remove(index);
            if index < state.queue_position {
                state.queue_position -= 1;
            }
            was_playing_row
        };

        self.event_tx.send(PlayerEvent::QueueChanged).ok();

        if playing_removed {
            self.stop()?;
            let (pos, len) = {
                let state = self.state.read().unwrap();
                (state.queue_position, state.queue.len())
            };
            if pos < len {
                self.play_queue_index(pos, true)?;
            } else {
                self.state.write().unwrap().queue_position = len.saturating_sub(1);
            }
        }

        self.emit_state_changed();
        Ok(())
    }

    fn previous_track(&mut self) -> Result<()> {
        let state = self.state.read().unwrap();
        if state.queue.is_empty() {
            return Ok(());
        }
        let prev_pos = if state.queue_position > 0 {
            state.queue_position - 1
        } else {
            0
        };
        let item = state.queue[prev_pos].clone();
        drop(state);
        self.state.write().unwrap().queue_position = prev_pos;
        self.play_item(item, true)
    }

    fn pump_decode(&mut self) {
        if self.state.read().unwrap().status == PlaybackStatus::Paused {
            return;
        }

        if let Some(crossfade) = self.crossfade.take() {
            self.pump_crossfade(crossfade);
            return;
        }

        let Some(mut decoder) = self.decoder.take() else {
            return;
        };

        let mut converter = self.converter.take();
        let output_channels = self.output_channels();
        let crossfade_from = self.crossfade_from;

        let mut decode = || {
            // The crossfade point: stop here, the engine starts the crossfade.
            if crossfade_from.is_some_and(|from| decoder.position_frames() >= from) {
                return DecodeStep::Yield;
            }
            match next_packet(&mut decoder, converter.as_mut(), output_channels) {
                Ok(Some(samples)) => DecodeStep::Samples(samples),
                Ok(None) => DecodeStep::End,
                Err(e) => DecodeStep::Failed(e.to_string()),
            }
        };

        let outcome = self.feeder.pump(
            &mut self.output,
            MIN_WRITE_SLOTS,
            MAX_PACKETS_PER_PUMP,
            &mut decode,
        );

        match outcome {
            PumpOutcome::SinkFull => {
                self.publish_stream_bitrate(&decoder);
                self.publish_stream_title(&decoder);
                self.decoder = Some(decoder);
                self.converter = converter;
            }
            PumpOutcome::Yielded => self.start_crossfade(decoder, converter),
            PumpOutcome::Finished => self.on_decoder_finished(decoder, converter),
            PumpOutcome::Failed(message) => {
                self.emit_error(message);
                let _ = self.stop();
            }
        }
    }

    /// Pump a running crossfade in place of the main decoder.
    fn pump_crossfade(&mut self, mut crossfade: Crossfade) {
        let output_channels = self.output_channels();
        let mut step = || crossfade.next_step(output_channels);
        let outcome = self.feeder.pump(
            &mut self.output,
            MIN_WRITE_SLOTS,
            MAX_PACKETS_PER_PUMP,
            &mut step,
        );

        match outcome {
            PumpOutcome::SinkFull => self.crossfade = Some(crossfade),
            // A crossfade step only yields once it is done; a `Finished` would
            // mean the same: the incoming track is what plays on.
            PumpOutcome::Yielded | PumpOutcome::Finished => self.hand_over(crossfade),
            PumpOutcome::Failed(message) => {
                self.emit_error(message);
                let _ = self.stop();
            }
        }
    }

    /// The playing track reached its crossfade point: open the next row and
    /// mix the two. When that is not possible the track plays on, and its end
    /// joins or drains as usual.
    fn start_crossfade(&mut self, outgoing: AudioDecoder, converter: Option<RateConverter>) {
        let file_rate = outgoing.sample_rate();
        let length = fade::crossfade_length(
            self.fades.crossfade,
            self.track_duration,
            outgoing.position_frames(),
            file_rate,
        );
        let Some((track, incoming)) = self.open_crossfade_track(&outgoing, length) else {
            // No retry for this track: its end joins or drains as usual.
            self.crossfade_from = None;
            self.decoder = Some(outgoing);
            self.converter = converter;
            return;
        };

        let output_rate = self.output.sample_rate();
        let output_channels = self.output_channels();
        // Same format as the outgoing track (checked when it was opened), so
        // the incoming side needs a converter exactly when the file's rate
        // differs from the output's.
        let incoming_converter = (file_rate != output_rate)
            .then(|| RateConverter::new(file_rate, output_rate, output_channels));
        let duration = track.info.duration;

        self.pending.push(track);
        self.crossfade_from = None;
        self.crossfade = Some(Crossfade::new(
            DecodedTrack {
                decoder: outgoing,
                converter,
            },
            DecodedTrack {
                decoder: incoming,
                converter: incoming_converter,
            },
            duration,
            output_frames(length, file_rate, output_rate),
            output_channels,
        ));
    }

    /// The next row, opened, if a crossfade of `length` frames (in the file's
    /// rate) fits: a local file of the same format, long enough for it.
    fn open_crossfade_track(
        &mut self,
        outgoing: &AudioDecoder,
        length: u64,
    ) -> Option<(PendingTrack, AudioDecoder)> {
        if length == 0 {
            return None;
        }
        let (track, incoming) = self.open_next_row(outgoing)?;
        fade::incoming_long_enough(track.info.duration, length, outgoing.sample_rate())
            .then_some((track, incoming))
    }

    /// The crossfade is over: the incoming track becomes the current decoder.
    fn hand_over(&mut self, crossfade: Crossfade) {
        let (incoming, duration) = crossfade.into_incoming();
        self.decoder = Some(incoming.decoder);
        self.converter = incoming.converter;
        self.arm_crossfade(duration);
    }

    /// A crossfade whose incoming track is already playing ends now, so that
    /// a seek applies to that track. Before it is heard, the reopen in `seek`
    /// drops the crossfade instead.
    fn end_heard_crossfade(&mut self) {
        if !self.pending.is_empty() {
            return;
        }
        if let Some(crossfade) = self.crossfade.take() {
            self.hand_over(crossfade);
        }
    }

    /// Work out where the current decoder's crossfade starts, if it has one.
    /// The decoder must already be set.
    fn arm_crossfade(&mut self, duration: Option<Duration>) {
        self.track_duration = duration;
        self.crossfade_from = self.decoder.as_ref().and_then(|decoder| {
            fade::crossfade_from(
                self.fades.crossfade,
                duration,
                decoder.sample_rate(),
                decoder.is_stream(),
            )
        });
    }

    /// A source is open: a decoder, or a crossfade that owns both decoders.
    fn has_source(&self) -> bool {
        self.decoder.is_some() || self.crossfade.is_some()
    }

    /// Sample rate and channel count of the open source.
    fn source_format(&self) -> Option<(u32, u16)> {
        if let Some(crossfade) = &self.crossfade {
            return Some(crossfade.format());
        }
        self.decoder
            .as_ref()
            .map(|decoder| (decoder.sample_rate(), decoder.channels()))
    }

    fn output_channels(&self) -> usize {
        self.output.channels().max(1) as usize
    }

    /// True once the buffered tail of a finished track has been played.
    fn drain_complete(&self) -> bool {
        let Some(since) = self.draining_since else {
            return false;
        };
        let nearly_empty = self.output.queued_samples() <= self.output.channels() as usize * 64;
        let paused = self.state.read().unwrap().status == PlaybackStatus::Paused;
        nearly_empty || (!paused && since.elapsed() > Duration::from_secs(10))
    }

    /// Called once the finished track has fully played. Advances the queue.
    fn finish_track(&mut self) {
        // A chain whose last track was tiny may end before its boundary
        // check ran; show those tracks so none is skipped.
        self.start_all_pending();
        self.draining_since = None;
        self.feeder.reset();
        self.converter = None;

        self.state.write().unwrap().status = PlaybackStatus::Stopped;
        self.event_tx.send(PlayerEvent::TrackEnded).ok();
        self.emit_state_changed();

        if let Some(next) = self.pick_next(true) {
            if let Err(e) = self.play_queue_index(next, false) {
                self.emit_error(e.to_string());
            }
        }
    }

    /// Samples (all channels) the speakers have played of the current track.
    ///
    /// Decoding runs ahead of the speakers, so what is still in the ring is
    /// subtracted. The device's own buffer (tens of milliseconds) is not
    /// counted, so the position and a gapless switch run that much early.
    fn played_samples(&self) -> u64 {
        self.feeder
            .pushed_samples()
            .saturating_sub(self.output.queued_samples() as u64)
    }

    /// Show each chained track whose first sample has been played.
    fn advance_pending(&mut self) {
        while let Some(track) = self.pending.take_started(self.played_samples()) {
            self.start_chained(track);
        }
    }

    /// Show every chained track now: the buffer holding them is gone or done.
    fn start_all_pending(&mut self) {
        while let Some(track) = self.pending.take_started(u64::MAX) {
            self.start_chained(track);
        }
    }

    /// Make a chained track the playing one: events, queue row, position.
    fn start_chained(&mut self, track: PendingTrack) {
        // Count the position from the new track's first sample.
        let pushed = self.feeder.pushed_samples();
        self.feeder
            .set_pushed_samples(pushed.saturating_sub(track.starts_at));

        self.event_tx.send(PlayerEvent::TrackEnded).ok();
        {
            let mut state = self.state.write().unwrap();
            if let Some(position) = resolve_position(&state.queue, track.index, &track.item) {
                state.queue_position = position;
            }
            state.current_track = Some(track.info.clone());
            state.position = Duration::ZERO;
        }
        self.event_tx
            .send(PlayerEvent::TrackStarted(Box::new(track.info)))
            .ok();
        self.emit_state_changed();
    }

    fn tick_position(&mut self) {
        if self.last_position_tick.elapsed() < Duration::from_millis(200) {
            return;
        }
        self.last_position_tick = Instant::now();

        if !self.has_source() && self.draining_since.is_none() {
            return;
        }

        let rate = self.output.sample_rate() as u64;
        let channels = self.output.channels().max(1) as u64;
        if rate == 0 {
            return;
        }

        let played = self.played_samples();
        let position = Duration::from_secs_f64(played as f64 / (rate * channels) as f64);

        self.state.write().unwrap().position = position;
        self.event_tx.send(PlayerEvent::PositionTick(position)).ok();
    }

    fn publish_stream_bitrate(&self, decoder: &crate::audio::source::AudioDecoder) {
        if !decoder.is_stream() {
            return;
        }
        let Some(kbps) = decoder.measured_bitrate_kbps() else {
            return;
        };
        let mut state = self.state.write().unwrap();
        if let Some(track) = state.current_track.as_mut() {
            track.bitrate_kbps = Some(kbps);
        }
    }

    fn publish_stream_title(&self, decoder: &AudioDecoder) {
        let icy = decoder.icy_title();
        let icy_url = decoder.icy_url();
        if matches!(icy, IcyTitle::Unset) && matches!(icy_url, IcyUrl::Unset) {
            return;
        }
        let station_name = {
            let state = self.state.read().unwrap();
            match state.queue.get(state.queue_position) {
                Some(QueueItem::Stream { name, .. }) => name.clone(),
                _ => return,
            }
        };
        let mut state = self.state.write().unwrap();
        let Some(track) = state.current_track.as_mut() else {
            return;
        };
        let title_changed = if matches!(icy, IcyTitle::Unset) {
            false
        } else {
            apply_icy_to_track(track, &icy, &station_name)
        };
        let url_changed = apply_icy_url_to_track(track, &icy_url);
        if title_changed || url_changed {
            drop(state);
            self.emit_state_changed();
        }
    }

    fn update_status(&mut self, status: PlaybackStatus) {
        self.state.write().unwrap().status = status;
    }

    fn emit_state_changed(&self) {
        self.event_tx.send(PlayerEvent::StateChanged).ok();
    }

    fn emit_error(&self, message: String) {
        self.event_tx.send(PlayerEvent::Error(message)).ok();
    }
}

/// Open `path` if it can join `finished` in the same stream: same sample rate
/// and channel count.
fn open_matching(path: &Path, finished: &AudioDecoder) -> Option<(AudioDecoder, TrackInfo)> {
    let (decoder, info) = match AudioDecoder::open(&LocalFileSource::new(path.to_path_buf())) {
        Ok(opened) => opened,
        Err(e) => {
            tracing::debug!(error = %e, "next track did not open, no gapless join");
            return None;
        }
    };
    let same_format = decoder.sample_rate() == finished.sample_rate()
        && decoder.channels() == finished.channels();
    same_format.then_some((decoder, info))
}

/// Where the playing row sits after the row at `from` moves to `to`.
pub(crate) fn moved_position(position: usize, from: usize, to: usize) -> usize {
    if from == position {
        to
    } else if from < position && to >= position {
        position - 1
    } else if from > position && to <= position {
        position + 1
    } else {
        position
    }
}

#[cfg(test)]
mod tests {
    use super::{AudioConfig, PlayerHandle, moved_position, spawn_player};
    use crate::player::commands::Command;
    use crate::player::state::QueueItem;

    fn player() -> PlayerHandle {
        // Queue bookkeeping does not need a sound card; the engine opens the
        // device lazily, so this works on machines without one.
        spawn_player(AudioConfig::default()).0
    }

    fn files(names: &[&str]) -> Vec<QueueItem> {
        names
            .iter()
            .map(|name| QueueItem::file(format!("/music/{name}.flac")))
            .collect()
    }

    fn revision(player: &PlayerHandle) -> u64 {
        player.state().queue.revision()
    }

    #[test]
    fn queue_edits_raise_the_revision() {
        let player = player();
        let mut last = revision(&player);

        let steps = [
            Command::QueueAdd(files(&["a", "b", "c"])),
            Command::QueueMove { from: 0, to: 2 },
            Command::QueueRemove(0),
            Command::ReplaceQueue {
                items: files(&["x", "y"]),
                position: 0,
            },
            Command::QueueClear,
        ];
        for command in steps {
            let label = format!("{command:?}");
            player.send_blocking(command).expect("queue command");
            let now = revision(&player);
            assert!(now > last, "{label} must raise the revision");
            last = now;
        }
    }

    #[test]
    fn a_move_raises_the_revision_once_and_reorders() {
        let player = player();
        player
            .send_blocking(Command::QueueAdd(files(&["a", "b", "c"])))
            .expect("add");
        let before = revision(&player);

        player
            .send_blocking(Command::QueueMove { from: 0, to: 2 })
            .expect("move");

        assert_eq!(revision(&player), before + 1);
        assert_eq!(player.state().queue, files(&["b", "c", "a"]));
    }

    #[test]
    fn rejected_or_no_op_queue_edits_keep_the_revision() {
        let player = player();
        player
            .send_blocking(Command::QueueAdd(files(&["a", "b"])))
            .expect("add");
        let before = revision(&player);

        player
            .send_blocking(Command::QueueMove { from: 1, to: 1 })
            .expect("same-index move");
        player
            .send_blocking(Command::QueueRemove(9))
            .expect("remove past the end");
        let err = player
            .send_blocking(Command::QueueMove { from: 0, to: 9 })
            .expect_err("move past the end must be rejected");
        assert!(err.to_string().contains("out of range"), "got {err}");

        assert_eq!(revision(&player), before);
    }

    #[test]
    fn playback_and_mode_commands_keep_the_revision() {
        let player = player();
        player
            .send_blocking(Command::QueueAdd(files(&["a", "b"])))
            .expect("add");
        let before = revision(&player);

        player
            .send_blocking(Command::SetVolume(0.5))
            .expect("volume");
        player.send_blocking(Command::SetMuted(true)).expect("mute");
        player
            .send_blocking(Command::SetRepeat(crate::player::state::RepeatMode::All))
            .expect("repeat");
        player
            .send_blocking(Command::SetShuffle(true))
            .expect("shuffle");
        player.send_blocking(Command::Pause).expect("pause");

        assert_eq!(revision(&player), before);
    }

    #[test]
    fn fade_times_are_accepted() {
        let player = player();
        player
            .send_blocking(Command::SetFades(crate::player::fade::FadeTimes::default()))
            .expect("set fades");
    }

    #[test]
    fn the_playing_row_follows_itself() {
        assert_eq!(moved_position(2, 2, 0), 0);
        assert_eq!(moved_position(2, 2, 4), 4);
    }

    #[test]
    fn rows_crossing_the_playing_row_shift_it() {
        // A row from above moves to or below the playing row: it shifts up.
        assert_eq!(moved_position(2, 0, 2), 1);
        assert_eq!(moved_position(2, 1, 4), 1);
        // A row from below moves to or above it: it shifts down.
        assert_eq!(moved_position(2, 4, 2), 3);
        assert_eq!(moved_position(2, 3, 0), 3);
    }

    #[test]
    fn edges_and_adjacent_swaps() {
        // Playing row first.
        assert_eq!(moved_position(0, 3, 0), 1);
        assert_eq!(moved_position(0, 0, 3), 3);
        assert_eq!(moved_position(0, 2, 4), 0);
        // Playing row last (index 4).
        assert_eq!(moved_position(4, 0, 4), 3);
        assert_eq!(moved_position(4, 4, 0), 0);
        assert_eq!(moved_position(4, 1, 3), 4);
        // The row just above moves down onto the playing row.
        assert_eq!(moved_position(2, 1, 2), 1);
        // The row just below moves up onto the playing row.
        assert_eq!(moved_position(2, 3, 2), 3);
    }

    #[test]
    fn rows_that_do_not_cross_leave_it_alone() {
        assert_eq!(moved_position(2, 0, 1), 2);
        assert_eq!(moved_position(2, 3, 4), 2);
    }
}
