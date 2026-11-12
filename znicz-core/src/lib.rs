pub mod audio;
mod audioaddict;
pub mod cover;
mod cover_fetch;
pub mod error;
pub mod library_host;
pub mod metadata;
pub mod player;
pub mod playlist;
pub mod session;
pub mod station;

pub use audio::output::AudioOutput;
pub use audio::source::probe_track;
pub use audioaddict::{
    AudioAddictLookup, AudioAddictNetwork, audioaddict_cover_lookup, parse_audioaddict_channel,
};
pub use cover::{CoverSource, Found, Resolution, StreamCovers, StreamQuery, default_sources};
pub use cover_fetch::fetch_cover;
pub use error::{Result, ZniczError};
pub use library_host::{
    LibraryHost, LibraryOp, LibraryStatus, ScanProgress, ScanSummary, wait_for_library,
};
pub use metadata::{
    AudioProperties, CoverArt, FileMetadata, TrackTags, is_audio_file, read_cover, read_metadata,
    read_tags, title_from_path,
};
pub use player::commands::{Command, PlayerEvent};
pub use player::engine::{AudioConfig, PlayerHandle, PlayerOps, spawn_player};
pub use player::ipc::{ClientRole, IpcClient, IpcServer, try_state as ipc_try_state};
pub use player::state::{
    AudioDeviceInfo, OutputInfo, PlaybackStatus, PlayerState, Queue, QueueItem, RepeatMode,
    TrackInfo,
};
pub use playlist::{
    LoadResult, apply_to_player, copy_saved, list_saved, load_path, parse, remove_saved,
    rename_saved, sanitize_stem, saved_path, skipped_notice, write_path, write_text,
};
pub use session::{
    SESSION_SAVE_DEBOUNCE, Session, apply as apply_session, load as load_session,
    restore as restore_session, save as save_session, save_from_player as save_session_from_player,
};
pub use station::{
    Station, add as add_station, copy as copy_station, find as find_station, load as load_stations,
    play_station, remove as remove_station, rename as rename_station, save as save_stations,
    set_art as set_station_art, set_url as set_station_url, update as update_station,
    validate_name, validate_url,
};
