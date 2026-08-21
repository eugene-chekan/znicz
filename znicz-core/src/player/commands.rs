use std::path::PathBuf;
use std::time::Duration;

use crossbeam_channel::Sender;

use crate::error::Result;

/// A command plus an optional channel to report back on.
///
/// The engine applies the command and then, if an `ack` is present, sends the
/// outcome. Callers that need an accurate answer (the MCP server) wait for that
/// outcome instead of reading state that has not changed yet.
pub struct CommandEnvelope {
    pub command: Command,
    pub ack: Option<Sender<Result<()>>>,
}

impl CommandEnvelope {
    /// Fire and forget. Used by the TUI, which redraws on its own tick.
    pub fn new(command: Command) -> Self {
        Self { command, ack: None }
    }

    pub fn with_ack(command: Command, ack: Sender<Result<()>>) -> Self {
        Self {
            command,
            ack: Some(ack),
        }
    }
}

#[derive(Debug, Clone)]
pub enum Command {
    Play(PathBuf),
    Pause,
    Resume,
    Stop,
    Seek(Duration),
    SetVolume(f32),
    NextTrack,
    PreviousTrack,
    QueueAdd(Vec<PathBuf>),
    QueueClear,
    SetDevice(String),
}

#[derive(Debug, Clone)]
pub enum PlayerEvent {
    /// Boxed because track info with tags is much larger than the other
    /// variants, and every event travels through the same channel.
    TrackStarted(Box<crate::player::state::TrackInfo>),
    PositionTick(Duration),
    TrackEnded,
    QueueChanged,
    Error(String),
    StateChanged,
}
