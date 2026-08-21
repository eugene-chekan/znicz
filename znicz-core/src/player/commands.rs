use std::path::PathBuf;
use std::time::Duration;

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
    TrackStarted(crate::player::state::TrackInfo),
    PositionTick(Duration),
    TrackEnded,
    QueueChanged,
    Error(String),
    StateChanged,
}
