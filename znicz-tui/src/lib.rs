//! The Znicz terminal interface.
//!
//! Layout: library home with an overlay queue drawer, a transport cover slot at
//! the bottom (including the signal path on tall terminals), and hint lines.
//! Player state is read fresh every frame from the engine (local in tests, or
//! `znicz player` over IPC in production), so the interface owns no copy of it.

pub mod app;
pub mod config_file;
pub mod cover;
pub mod cursor;
pub mod folder_complete;
pub mod footer_hits;
pub mod format;
pub mod help_state;
pub mod hit;
pub mod keys;
pub mod layout;
pub mod library_pane;
pub mod line_edit;
pub mod marquee;
pub mod meta;
pub mod playing;
pub mod scan_status;
pub mod theme;
pub mod toast;
pub mod tui_config;
pub mod update;
pub mod views;

pub use app::{
    App, Focus, Modal, PlaylistPrompt, RadioPrompt, Resume, RunOutcome, StationField, View,
};
pub use config_file::{ConfigFile, ConfigProblem};
pub use tui_config::{CoverProtocol, IconSet, LibraryLayout, TuiConfig};
pub use update::{Release, Updates};
