//! Spotify control through the Web API: login (OAuth PKCE), browse and
//! playback calls, and a polling service the player process hosts.
//!
//! Spotify plays the audio; Znicz only sends commands and reads state.

mod auth;
mod error;
mod http;
mod pkce;
mod redirect;
mod token;

pub use error::{Result, SpotifyError};
pub use token::default_token_path;
