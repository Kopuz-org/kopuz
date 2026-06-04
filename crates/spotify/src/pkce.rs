//! PKCE helpers for Spotify's Authorization Code with PKCE flow.

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use rand::RngCore;
use sha2::{Digest, Sha256};

/// Spotify allows verifiers of 43..=128 unreserved characters. We use 64
/// random bytes encoded as base64url without padding, which yields a
/// well-formed verifier of length 86.
pub fn generate_verifier() -> String {
    let mut buf = [0u8; 64];
    rand::thread_rng().fill_bytes(&mut buf);
    URL_SAFE_NO_PAD.encode(buf)
}

/// S256 challenge: base64url(SHA256(verifier)).
pub fn challenge_s256(verifier: &str) -> String {
    let digest = Sha256::digest(verifier.as_bytes());
    URL_SAFE_NO_PAD.encode(digest)
}

/// High-entropy random state value used to bind authorization request and
/// callback. The exact length is not specified by Spotify; 32 bytes is plenty.
pub fn generate_state() -> String {
    let mut buf = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut buf);
    URL_SAFE_NO_PAD.encode(buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verifier_length_in_range() {
        let v = generate_verifier();
        assert!(
            v.len() >= 43 && v.len() <= 128,
            "verifier length {}",
            v.len()
        );
    }

    #[test]
    fn verifier_is_unreserved_charset() {
        let v = generate_verifier();
        for c in v.chars() {
            assert!(
                c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '~'),
                "disallowed char: {c}"
            );
        }
    }

    #[test]
    fn verifiers_are_unique() {
        let a = generate_verifier();
        let b = generate_verifier();
        assert_ne!(a, b);
    }

    #[test]
    fn challenge_s256_known_vector() {
        // RFC 7636 example
        let v = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        let c = challenge_s256(v);
        assert_eq!(c, "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM");
    }

    #[test]
    fn state_unique_and_safe() {
        let a = generate_state();
        let b = generate_state();
        assert_ne!(a, b);
        for c in a.chars() {
            assert!(c.is_ascii_alphanumeric() || matches!(c, '-' | '_'));
        }
    }
}
