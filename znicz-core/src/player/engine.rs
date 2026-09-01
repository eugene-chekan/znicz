use std::sync::{Arc, RwLock};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crossbeam_channel::{bounded, unbounded, Receiver, RecvTimeoutError, Sender};

use crate::audio::convert::{adapt_channels, RateConverter};
use crate::audio::feeder::{DecodeStep, Feeder, PumpOutcome};
use crate::audio::http::HttpStreamSource;
use crate::audio::output::AudioOutput;
use crate::audio::source::{AudioDecoder, AudioSource, LocalFileSource};
use crate::error::{Result, ZniczError};
use crate::player::commands::{Command, CommandEnvelope, PlayerEvent};
use crate::player::state::{OutputInfo, PlaybackStatus, PlayerState, RepeatMode};

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
}

impl Default for AudioConfig {
    fn default() -> Self {
        Self {
            device_id: None,
            volume: 1.0,
            bit_perfect: true,
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
    draining_since: Option<Instant>,
    last_position_tick: Instant,
    /// Seed for shuffle picks.
    rng: u64,
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

        Self {
            config,
            state,
            command_rx,
            event_tx,
            output,
            decoder: None,
            feeder: Feeder::new(),
            converter: None,
            draining_since: None,
            last_position_tick: Instant::now(),
            rng: seed_from_clock(),
        }
    }

    fn run(&mut self) {
        loop {
            // Waiting on the channel replaces a plain sleep: commands are
            // picked up the moment they arrive, so an acknowledged command
            // does not wait out a sleep interval.
            let idle = if self.decoder.is_some() || self.draining_since.is_some() {
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

            self.pump_decode();

            if self.drain_complete() {
                self.finish_track();
            }

            self.tick_position();
        }
    }

    /// Run one command and answer the caller if it asked to be told.
    fn apply(&mut self, envelope: CommandEnvelope) {
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

    fn handle_command(&mut self, command: Command) -> Result<()> {
        match command {
            Command::Play(item) => self.play_item(item)?,
            Command::Pause => {
                self.output.set_paused(true);
                self.update_status(PlaybackStatus::Paused);
                self.emit_state_changed();
            }
            Command::Resume => {
                self.output.set_paused(false);
                self.update_status(PlaybackStatus::Playing);
                self.emit_state_changed();
            }
            Command::Stop => self.stop()?,
            Command::Seek(pos) => self.seek(pos)?,
            Command::SetVolume(vol) => {
                let v = vol.clamp(0.0, 1.0);
                let effective = {
                    let mut state = self.state.write().unwrap();
                    state.volume = v;
                    state.effective_volume()
                };
                self.output.set_volume(effective);
                self.emit_state_changed();
            }
            Command::SetMuted(muted) => {
                let effective = {
                    let mut state = self.state.write().unwrap();
                    state.muted = muted;
                    state.effective_volume()
                };
                self.output.set_volume(effective);
                self.emit_state_changed();
            }
            Command::NextTrack => self.next_track()?,
            Command::PreviousTrack => self.previous_track()?,
            Command::QueueAdd(items) => {
                let mut state = self.state.write().unwrap();
                state.queue.extend(items);
                self.event_tx.send(PlayerEvent::QueueChanged).ok();
                self.emit_state_changed();
            }
            Command::QueueClear => {
                let mut state = self.state.write().unwrap();
                state.queue.clear();
                state.queue_position = 0;
                self.event_tx.send(PlayerEvent::QueueChanged).ok();
                self.emit_state_changed();
            }
            Command::QueuePlayIndex(index) => self.play_queue_index(index)?,
            Command::QueueRemove(index) => self.remove_from_queue(index)?,
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
                if let Some(decoder) = &self.decoder {
                    let rate = decoder.sample_rate();
                    let ch = decoder.channels();
                    self.output
                        .open_stream(rate, ch, self.config.device_id.as_deref())?;
                }
                self.emit_state_changed();
            }
        }
        Ok(())
    }

    fn play_item(&mut self, item: crate::player::state::QueueItem) -> Result<()> {
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

        self.output
            .open_stream(sample_rate, channels, self.config.device_id.as_deref())?;
        self.output.set_paused(false);
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
            state.queue = vec![item.clone()];
            state.queue_position = 0;
        } else if let Some(pos) = state.queue.iter().position(|row| row == &item) {
            state.queue_position = pos;
        } else {
            state.queue.push(item.clone());
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
        self.draining_since = None;
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
            Some(index) => self.play_queue_index(index),
            None => Ok(()),
        }
    }

    /// Which queue entry to play next, or `None` to stop.
    ///
    /// `auto` marks the end of a track, where "repeat one" replays it. Pressing
    /// next always moves on instead.
    fn pick_next(&mut self, auto: bool) -> Option<usize> {
        let (len, current, repeat, shuffle) = {
            let state = self.state.read().unwrap();
            (
                state.queue.len(),
                state.queue_position,
                state.repeat,
                state.shuffle,
            )
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
        if pick >= current {
            pick + 1
        } else {
            pick
        }
    }

    fn play_queue_index(&mut self, index: usize) -> Result<()> {
        let item = {
            let state = self.state.read().unwrap();
            match state.queue.get(index) {
                Some(item) => item.clone(),
                None => return Ok(()),
            }
        };
        self.state.write().unwrap().queue_position = index;
        self.play_item(item)?;
        let mut state = self.state.write().unwrap();
        if index < state.queue.len() {
            state.queue_position = index;
        }
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
            state.queue.remove(index);
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
                self.play_queue_index(pos)?;
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
        self.play_item(item)
    }

    fn pump_decode(&mut self) {
        if self.state.read().unwrap().status == PlaybackStatus::Paused {
            return;
        }

        let Some(mut decoder) = self.decoder.take() else {
            return;
        };

        let mut converter = self.converter.take();
        let source_channels = decoder.channels().max(1) as usize;
        let output_channels = self.output.channels().max(1) as usize;

        let mut decode = || match decoder.decode_next() {
            Ok(Some(samples)) => {
                let matched = adapt_channels(&samples, source_channels, output_channels);
                match converter.as_mut() {
                    Some(converter) => DecodeStep::Samples(converter.process(&matched)),
                    None => DecodeStep::Samples(matched),
                }
            }
            Ok(None) => DecodeStep::End,
            Err(e) => DecodeStep::Failed(e.to_string()),
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
                self.decoder = Some(decoder);
                self.converter = converter;
            }
            PumpOutcome::Finished => {
                self.publish_stream_bitrate(&decoder);
                // The buffer still holds audio. Let it play out, otherwise the
                // last seconds of the track are cut off.
                self.converter = None;
                self.draining_since = Some(Instant::now());
            }
            PumpOutcome::Failed(message) => {
                self.emit_error(message);
                let _ = self.stop();
            }
        }
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
        self.draining_since = None;
        self.feeder.reset();
        self.converter = None;

        self.state.write().unwrap().status = PlaybackStatus::Stopped;
        self.event_tx.send(PlayerEvent::TrackEnded).ok();
        self.emit_state_changed();

        if let Some(next) = self.pick_next(true) {
            if let Err(e) = self.play_queue_index(next) {
                self.emit_error(e.to_string());
            }
        }
    }

    fn tick_position(&mut self) {
        if self.last_position_tick.elapsed() < Duration::from_millis(200) {
            return;
        }
        self.last_position_tick = Instant::now();

        if self.decoder.is_none() && self.draining_since.is_none() {
            return;
        }

        let rate = self.output.sample_rate() as u64;
        let channels = self.output.channels().max(1) as u64;
        if rate == 0 {
            return;
        }

        // Decoding runs ahead of the speakers, so subtract what is still queued.
        let played = self
            .feeder
            .pushed_samples()
            .saturating_sub(self.output.queued_samples() as u64);
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
