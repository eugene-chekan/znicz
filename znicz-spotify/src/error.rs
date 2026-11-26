//! Every way a Spotify request can fail, worded for the user.

use znicz_core::ZniczError;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SpotifyError {
    #[error("Spotify is not configured: set a client ID in Settings → Spotify")]
    NotConfigured,
    #[error("not logged in to Spotify")]
    NotLoggedIn,
    #[error("no Spotify login is in progress")]
    NoLogin,
    #[error("login expired, press Enter to start again")]
    LoginExpired,
    #[error("that address is not from this login")]
    WrongAddress,
    #[error("Spotify login was declined: {0}")]
    Declined(String),
    #[error(
        "Spotify refused the client ID or redirect URI: register exactly {redirect_uri} in your Spotify app"
    )]
    InvalidClient { redirect_uri: String },
    #[error("Spotify login expired")]
    Revoked,
    #[error("Spotify rejected the access token")]
    Unauthorized,
    #[error("needs Spotify Premium")]
    PremiumRequired,
    #[error("{0}")]
    Unavailable(String),
    #[error("Spotify has no active device: open Spotify somewhere, or pick one with ,")]
    NoActiveDevice,
    #[error("Spotify is rate limiting requests; try again in {retry_after_secs} s")]
    RateLimited { retry_after_secs: u64 },
    #[error("Spotify error {status}: {message}")]
    Http { status: u16, message: String },
    #[error("could not reach Spotify: {0}")]
    Network(String),
    #[error("unexpected Spotify response: {0}")]
    Parse(String),
    #[error("{0}")]
    Io(String),
}

pub type Result<T> = std::result::Result<T, SpotifyError>;

impl From<SpotifyError> for ZniczError {
    fn from(error: SpotifyError) -> Self {
        ZniczError::Spotify(error.to_string())
    }
}

impl From<std::io::Error> for SpotifyError {
    fn from(error: std::io::Error) -> Self {
        SpotifyError::Io(error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_match_the_spec() {
        assert_eq!(
            SpotifyError::NotConfigured.to_string(),
            "Spotify is not configured: set a client ID in Settings → Spotify"
        );
        assert_eq!(
            SpotifyError::PremiumRequired.to_string(),
            "needs Spotify Premium"
        );
        assert_eq!(
            SpotifyError::InvalidClient {
                redirect_uri: "http://127.0.0.1:8898/callback".into()
            }
            .to_string(),
            "Spotify refused the client ID or redirect URI: register exactly \
             http://127.0.0.1:8898/callback in your Spotify app"
        );
    }

    #[test]
    fn converts_to_a_core_error_with_the_same_text() {
        let core: ZniczError = SpotifyError::NoActiveDevice.into();
        assert_eq!(core.to_string(), SpotifyError::NoActiveDevice.to_string());
    }
}
