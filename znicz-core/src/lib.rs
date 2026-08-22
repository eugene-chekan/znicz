pub mod audio;
pub mod error;
pub mod metadata;
pub mod player;

pub use audio::output::AudioOutput;
pub use audio::source::probe_track;
pub use error::{Result, ZniczError};
pub use metadata::{
    AudioProperties, FileMetadata, TrackTags, is_audio_file, read_metadata, read_tags,
    title_from_path,
};
pub use player::commands::{Command, PlayerEvent};
pub use player::engine::{AudioConfig, PlayerHandle, spawn_player};
pub use player::state::{
    AudioDeviceInfo, OutputInfo, PlaybackStatus, PlayerState, RepeatMode, TrackInfo,
};
