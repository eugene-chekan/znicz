//! Spotify control through the Web API: login (OAuth PKCE), browse and
//! playback calls, and a polling service the player process hosts.
//!
//! Spotify plays the audio; Znicz only sends commands and reads state.

mod api;
mod auth;
mod client;
mod error;
mod http;
mod listener;
mod model;
mod pkce;
mod redirect;
mod service;
mod token;

pub use error::{Result, SpotifyError};
pub use service::{ServiceOptions, SpotifyService};
pub use token::default_token_path;
