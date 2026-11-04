//! A per-install key that digests header values, so a value kept masked can still be compared
//! with another without keeping the value itself.

use crate::sealed;
use crate::secrets::SecretStore;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;
use std::fmt;

/// An HMAC-SHA256 key kept in a secret store, with a short public id naming it.
#[non_exhaustive]
#[derive(Clone)]
pub struct DigestKey {
    key: [u8; 32],
    id: String,
}

impl DigestKey {
    /// The per-install key under `SecretKey::new("", "", name)` in `keys`, created when missing.
    /// A malformed stored key is an error and is left in place.
    pub fn load(keys: &dyn SecretStore, name: &str) -> Result<DigestKey, String> {
        let key = sealed::file_key(keys, name, "kept-send digest")?;
        let id = mac(&key, &[b"apinata key id"])[..8]
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        Ok(DigestKey { key, id })
    }

    /// Lowercase hex of the first 8 bytes of HMAC-SHA256(key, "apinata key id"): 16 chars.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Standard padded base64 of HMAC-SHA256(key, lowercase(name) || 0x00 || value).
    pub fn digest(&self, name: &str, value: &str) -> String {
        let name = name.to_ascii_lowercase();
        STANDARD.encode(mac(&self.key, &[name.as_bytes(), &[0], value.as_bytes()]))
    }
}

fn mac(key: &[u8], parts: &[&[u8]]) -> Vec<u8> {
    let mut mac =
        <Hmac<Sha256> as KeyInit>::new_from_slice(key).expect("HMAC takes any key length");
    for part in parts {
        mac.update(part);
    }
    mac.finalize().into_bytes().to_vec()
}

// The key bytes never print; the id is enough to tell two keys apart.
impl fmt::Debug for DigestKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DigestKey").field("id", &self.id).finish()
    }
}
