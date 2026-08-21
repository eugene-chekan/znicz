pub mod audio;
pub mod error;
pub mod player;

pub use audio::output::AudioOutput;
pub use error::{Result, ZniczError};
pub use player::commands::{Command, PlayerEvent};
pub use player::engine::{spawn_player, AudioConfig, PlayerHandle};
pub use player::state::{AudioDeviceInfo, PlaybackStatus, PlayerState, TrackInfo};
