//! Every way a Spotify request can fail, worded for the user.

use znicz_core::{SpotifyErrorKind, ZniczError};

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

impl SpotifyError {
    /// The kind that crosses IPC and reaches the MCP tools.
    pub fn kind(&self) -> SpotifyErrorKind {
        use SpotifyErrorKind as K;
        match self {
            Self::NotConfigured | Self::InvalidClient { .. } => K::NotConfigured,
            Self::NotLoggedIn | Self::Revoked | Self::Unauthorized => K::NotLoggedIn,
            Self::NoLogin | Self::WrongAddress => K::WrongAddress,
            Self::LoginExpired => K::LoginExpired,
            Self::PremiumRequired => K::PremiumRequired,
            Self::NoActiveDevice => K::NoActiveDevice,
            Self::RateLimited { .. } => K::RateLimited,
            Self::Unavailable(_) => K::Unavailable,
            Self::Network(_) => K::Network,
            Self::Declined(_) | Self::Http { .. } | Self::Parse(_) | Self::Io(_) => K::Other,
        }
    }
}

impl From<SpotifyError> for ZniczError {
    fn from(error: SpotifyError) -> Self {
        ZniczError::Spotify {
            kind: error.kind(),
            message: error.to_string(),
        }
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

    #[test]
    fn every_error_has_a_kind() {
        use znicz_core::SpotifyErrorKind as K;
        let cases = [
            (SpotifyError::NotConfigured, K::NotConfigured),
            (SpotifyError::NotLoggedIn, K::NotLoggedIn),
            (SpotifyError::NoLogin, K::WrongAddress),
            (SpotifyError::LoginExpired, K::LoginExpired),
            (SpotifyError::WrongAddress, K::WrongAddress),
            (SpotifyError::Declined("no".into()), K::Other),
            (
                SpotifyError::InvalidClient {
                    redirect_uri: "http://127.0.0.1:8898/callback".into(),
                },
                K::NotConfigured,
            ),
            (SpotifyError::Revoked, K::NotLoggedIn),
            (SpotifyError::Unauthorized, K::NotLoggedIn),
            (SpotifyError::PremiumRequired, K::PremiumRequired),
            (SpotifyError::Unavailable("x".into()), K::Unavailable),
            (SpotifyError::NoActiveDevice, K::NoActiveDevice),
            (
                SpotifyError::RateLimited {
                    retry_after_secs: 5,
                },
                K::RateLimited,
            ),
            (
                SpotifyError::Http {
                    status: 500,
                    message: "x".into(),
                },
                K::Other,
            ),
            (SpotifyError::Network("x".into()), K::Network),
            (SpotifyError::Parse("x".into()), K::Other),
            (SpotifyError::Io("x".into()), K::Other),
        ];
        for (error, kind) in cases {
            assert_eq!(error.kind(), kind, "{error}");
        }
    }

    #[test]
    fn conversion_keeps_the_kind_and_the_message() {
        let core: ZniczError = SpotifyError::NoActiveDevice.into();
        match core {
            ZniczError::Spotify { kind, message } => {
                assert_eq!(kind, znicz_core::SpotifyErrorKind::NoActiveDevice);
                assert_eq!(message, SpotifyError::NoActiveDevice.to_string());
            }
            other => panic!("{other:?}"),
        }
    }
}
