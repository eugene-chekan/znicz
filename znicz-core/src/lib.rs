pub mod audio;
mod cover_fetch;
pub mod error;
pub mod metadata;
pub mod player;
pub mod playlist;
pub mod session;
pub mod station;

pub use audio::output::AudioOutput;
pub use audio::source::probe_track;
pub use error::{Result, ZniczError};
pub use cover_fetch::fetch_cover;
pub use metadata::{
    is_audio_file, read_cover, read_metadata, read_tags, title_from_path, AudioProperties,
    CoverArt, FileMetadata, TrackTags,
};
pub use player::commands::{Command, PlayerEvent};
pub use player::engine::{spawn_player, AudioConfig, PlayerHandle, PlayerOps};
pub use player::ipc::{try_state as ipc_try_state, ClientRole, IpcClient, IpcServer};
pub use player::state::{
    AudioDeviceInfo, OutputInfo, PlaybackStatus, PlayerState, QueueItem, RepeatMode, TrackInfo,
};
pub use playlist::{
    apply_to_player, copy_saved, list_saved, load_path, parse, remove_saved, rename_saved,
    sanitize_stem, saved_path, skipped_notice, write_path, write_text, LoadResult,
};
pub use session::{
    apply as apply_session, load as load_session, restore as restore_session, save as save_session,
    save_from_player as save_session_from_player, Session, SESSION_SAVE_DEBOUNCE,
};
pub use station::{
    add as add_station, copy as copy_station, find as find_station, load as load_stations,
    play_station, remove as remove_station, rename as rename_station, save as save_stations,
    set_url as set_station_url, update as update_station, validate_name, validate_url, Station,
};
