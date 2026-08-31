//! The Znicz terminal interface.
//!
//! Layout: library home with an overlay queue drawer, a two-line transport at
//! the bottom (including the signal path on tall terminals), and hint lines.
//! Player state is read fresh every frame from [`znicz_core::PlayerHandle`], so
//! the interface owns no copy of it.

pub mod app;
pub mod cursor;
pub mod format;
pub mod keys;
pub mod layout;
pub mod library_pane;
pub mod meta;
pub mod theme;
pub mod toast;
pub mod views;

pub use app::{App, Focus, Modal, RadioPrompt};
