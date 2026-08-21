use thiserror::Error;

#[derive(Debug, Error)]
pub enum LibraryError {
    #[error("database error: {0}")]
    Database(#[from] rusqlite::Error),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("{0} is not a directory")]
    NotADirectory(String),
}

pub type Result<T> = std::result::Result<T, LibraryError>;
