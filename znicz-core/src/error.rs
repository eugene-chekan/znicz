use thiserror::Error;

use crate::spotify_host::SpotifyErrorKind;

#[derive(Debug, Error)]
pub enum ZniczError {
    #[error("audio error: {0}")]
    Audio(String),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("decode error: {0}")]
    Decode(String),

    #[error("player error: {0}")]
    Player(String),

    /// A library write was refused or failed; the message is shown as is.
    #[error("{0}")]
    Library(String),

    /// A Spotify request failed. `message` is shown as is; `kind` says what to do about it.
    #[error("{message}")]
    Spotify {
        kind: SpotifyErrorKind,
        message: String,
    },

    #[error("not implemented: {0}")]
    NotImplemented(String),
}

pub type Result<T> = std::result::Result<T, ZniczError>;
