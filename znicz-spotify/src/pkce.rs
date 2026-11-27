//! PKCE (RFC 7636): a random verifier, its S256 challenge, and a random
//! `state` that ties a redirect to the login that asked for it.

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use sha2::{Digest, Sha256};

/// 48 random bytes in base64url: 64 characters, all unreserved.
pub(crate) fn random_verifier() -> String {
    URL_SAFE_NO_PAD.encode(random_bytes::<48>())
}

pub(crate) fn random_state() -> String {
    URL_SAFE_NO_PAD.encode(random_bytes::<16>())
}

/// The S256 challenge: base64url of the verifier's SHA-256, no padding.
pub(crate) fn challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

fn random_bytes<const N: usize>() -> [u8; N] {
    let mut bytes = [0u8; N];
    getrandom::fill(&mut bytes).expect("the OS random source is available");
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn challenge_matches_rfc_7636_appendix_b() {
        assert_eq!(
            challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    #[test]
    fn verifiers_are_64_unreserved_characters() {
        let verifier = random_verifier();
        assert_eq!(verifier.len(), 64);
        assert!(
            verifier
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'),
            "{verifier}"
        );
    }

    #[test]
    fn states_differ() {
        assert_ne!(random_state(), random_state());
    }
}
