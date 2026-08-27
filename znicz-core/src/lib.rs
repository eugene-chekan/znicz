pub mod audio;
pub mod error;
pub mod metadata;
pub mod player;
pub mod playlist;

pub use audio::output::AudioOutput;
pub use audio::source::probe_track;
pub use error::{Result, ZniczError};
pub use metadata::{
    is_audio_file, read_metadata, read_tags, title_from_path, AudioProperties, FileMetadata,
    TrackTags,
};
pub use player::commands::{Command, PlayerEvent};
pub use player::engine::{spawn_player, AudioConfig, PlayerHandle};
pub use player::state::{
    AudioDeviceInfo, OutputInfo, PlaybackStatus, PlayerState, RepeatMode, TrackInfo,
};
pub use playlist::{
    apply_to_player, list_saved, load_path, parse, sanitize_stem, saved_path, skipped_notice,
    write_path, write_text, LoadResult,
};
