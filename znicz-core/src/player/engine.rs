use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, Sender, unbounded};

use crate::audio::convert::{RateConverter, adapt_channels};
use crate::audio::feeder::{DecodeStep, Feeder, PumpOutcome};
use crate::audio::output::AudioOutput;
use crate::audio::source::AudioDecoder;
use crate::error::{Result, ZniczError};
use crate::player::commands::{Command, PlayerEvent};
use crate::player::state::{PlaybackStatus, PlayerState};

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

#[derive(Clone)]
pub struct PlayerHandle {
    command_tx: Sender<Command>,
    state: Arc<RwLock<PlayerState>>,
    event_rx: Receiver<PlayerEvent>,
}

impl PlayerHandle {
    pub fn send(&self, command: Command) -> Result<()> {
        self.command_tx
            .send(command)
            .map_err(|e| ZniczError::Player(e.to_string()))?;
        Ok(())
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
    command_rx: Receiver<Command>,
    event_tx: Sender<PlayerEvent>,
    output: AudioOutput,
    decoder: Option<AudioDecoder>,
    feeder: Feeder,
    converter: Option<RateConverter>,
    draining_since: Option<Instant>,
    last_position_tick: Instant,
}

/// Keep some headroom before decoding another packet.
const MIN_WRITE_SLOTS: usize = 4096;
/// Bound on work per loop so commands stay responsive.
const MAX_PACKETS_PER_PUMP: usize = 32;

impl PlayerEngine {
    fn new(
        config: AudioConfig,
        state: Arc<RwLock<PlayerState>>,
        command_rx: Receiver<Command>,
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
        }
    }

    fn run(&mut self) {
        loop {
            while let Ok(cmd) = self.command_rx.try_recv() {
                if let Err(e) = self.handle_command(cmd) {
                    self.emit_error(e.to_string());
                }
            }

            self.pump_decode();

            if self.drain_complete() {
                self.finish_track();
            }

            self.tick_position();

            if self.decoder.is_some() || self.draining_since.is_some() {
                thread::sleep(Duration::from_millis(5));
            } else {
                thread::sleep(Duration::from_millis(50));
            }
        }
    }

    fn handle_command(&mut self, command: Command) -> Result<()> {
        match command {
            Command::Play(path) => self.play_path(path)?,
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
                self.output.set_volume(v);
                self.state.write().unwrap().volume = v;
                self.emit_state_changed();
            }
            Command::NextTrack => self.next_track()?,
            Command::PreviousTrack => self.previous_track()?,
            Command::QueueAdd(paths) => {
                let mut state = self.state.write().unwrap();
                state.queue.extend(paths);
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
            Command::SetDevice(id) => {
                self.config.device_id = Some(id);
                if let Some(decoder) = &self.decoder {
                    let rate = decoder.sample_rate();
                    let ch = decoder.channels();
                    self.output.open_stream(rate, ch, self.config.device_id.as_deref())?;
                }
                self.emit_state_changed();
            }
        }
        Ok(())
    }

    fn play_path(&mut self, path: PathBuf) -> Result<()> {
        let (decoder, track_info) = AudioDecoder::open(&path)?;
        let sample_rate = decoder.sample_rate();
        let channels = decoder.channels();

        self.output.open_stream(
            sample_rate,
            channels,
            self.config.device_id.as_deref(),
        )?;
        self.output.set_paused(false);
        self.output.set_volume(self.state.read().unwrap().volume);

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

        let mut state = self.state.write().unwrap();
        if state.queue.is_empty() {
            state.queue = vec![path.clone()];
            state.queue_position = 0;
        } else if !state.queue.contains(&path) {
            state.queue.push(path.clone());
        } else {
            state.queue_position = state
                .queue
                .iter()
                .position(|p| p == &path)
                .unwrap_or(state.queue_position);
        }
        state.current_track = Some(track_info.clone());
        state.status = PlaybackStatus::Playing;
        state.position = Duration::ZERO;

        self.event_tx.send(PlayerEvent::TrackStarted(track_info)).ok();
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

        self.emit_state_changed();
        Ok(())
    }

    fn seek(&mut self, position: Duration) -> Result<()> {
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

    fn next_track(&mut self) -> Result<()> {
        let state = self.state.read().unwrap();
        if state.queue.is_empty() {
            return Ok(());
        }
        let next_pos = if state.queue_position + 1 < state.queue.len() {
            state.queue_position + 1
        } else {
            return Ok(());
        };
        let path = state.queue[next_pos].clone();
        drop(state);
        self.state.write().unwrap().queue_position = next_pos;
        self.play_path(path)?;
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
        let path = state.queue[prev_pos].clone();
        drop(state);
        self.state.write().unwrap().queue_position = prev_pos;
        self.play_path(path)?;
        Ok(())
    }

    fn pump_decode(&mut self) {
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
                self.decoder = Some(decoder);
                self.converter = converter;
            }
            PumpOutcome::Finished => {
                // The buffer still holds audio. Let it play out, otherwise the
                // last seconds of the track are cut off.
                self.converter = None;
                self.draining_since = Some(Instant::now());
            }
            PumpOutcome::Failed(message) => {
                self.emit_error(message);
                self.feeder.reset();
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

        let mut state = self.state.write().unwrap();
        state.status = PlaybackStatus::Stopped;
        self.event_tx.send(PlayerEvent::TrackEnded).ok();
        self.emit_state_changed();

        if state.queue_position + 1 < state.queue.len() {
            let next = state.queue[state.queue_position + 1].clone();
            state.queue_position += 1;
            drop(state);
            if let Err(e) = self.play_path(next) {
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
