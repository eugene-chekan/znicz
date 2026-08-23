//! The Znicz terminal interface.
//!
//! Layout: a tab bar, the now-playing header (including the signal path), one
//! of the list panes, and two status lines. Player state is read fresh every
//! frame from [`znicz_core::PlayerHandle`], so the interface owns no copy of it.

pub mod app;
pub mod layout;
pub mod cursor;
pub mod format;
pub mod keys;
pub mod library_pane;
pub mod meta;
pub mod theme;
pub mod toast;
pub mod views;

pub use app::{App, Pane};
