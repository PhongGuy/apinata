//! OS randomness and RFC 7636 PKCE, shared by the OAuth 2.0 browser sign-in and team single
//! sign-on.

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use rand::{TryRngCore, rngs::OsRng};
use sha2::{Digest, Sha256};

/// The operating system's random source failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct NoRandomness;

/// `N` bytes from the operating system's random source.
pub(crate) fn random<const N: usize>() -> Result<[u8; N], NoRandomness> {
    let mut bytes = [0u8; N];
    OsRng.try_fill_bytes(&mut bytes).map_err(|_| NoRandomness)?;
    Ok(bytes)
}

/// 32 OS-random bytes as unpadded base64url (43 characters).
pub(crate) fn random_b64url() -> Result<String, NoRandomness> {
    Ok(URL_SAFE_NO_PAD.encode(random::<32>()?))
}

/// An RFC 7636 code verifier and its S256 challenge.
pub(crate) struct Pkce {
    pub(crate) verifier: String,
    pub(crate) challenge: String,
}

impl Pkce {
    /// A 43-character random verifier and its S256 challenge.
    pub(crate) fn generate() -> Result<Pkce, NoRandomness> {
        let verifier = random_b64url()?;
        let challenge = Pkce::challenge_for(&verifier);
        Ok(Pkce {
            verifier,
            challenge,
        })
    }

    /// The S256 challenge: base64url of the verifier's SHA-256, unpadded.
    pub(crate) fn challenge_for(verifier: &str) -> String {
        URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pkce_uses_43_base64url_characters_and_s256() {
        let p = Pkce::generate().unwrap();
        assert_eq!(p.verifier.len(), 43);
        assert!(
            p.verifier
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        );
        assert_eq!(p.challenge, Pkce::challenge_for(&p.verifier));
        assert_ne!(Pkce::generate().unwrap().verifier, p.verifier);
        // RFC 7636 appendix B.
        assert_eq!(
            Pkce::challenge_for("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }
}
