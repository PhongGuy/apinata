//! Atlassian ASAP auth as the app edits it, and its YAML projection.

use super::{Redacted, text};
use std::fmt;
use yaml_serde::{Mapping, Value};

/// ASAP: a short-lived token signed with a private key on every send and sent as a bearer
/// token. Every string may hold `{{variables}}`.
#[derive(Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct AsapSpec {
    /// The signing algorithm: RS, PS or ES with 256, 384 or 512.
    pub algorithm: String,
    /// The `iss` claim.
    pub issuer: String,
    /// The `aud` claim.
    pub audience: String,
    /// The `kid` header; blank takes the key id of a data URI key.
    pub key_id: String,
    /// The `sub` claim; blank sends the issuer.
    pub subject: String,
    /// Seconds until expiry; blank is one hour.
    pub expires_in: String,
    /// Extra claims as JSON object text, which override the standard ones; blank for none.
    pub additional_claims: String,
    /// The private key: PEM text, a `data:application/pkcs8` URI or base64 DER, inline only.
    pub private_key: String,
}

/// A new ASAP auth as the app creates it: RS256, nothing else set.
impl Default for AsapSpec {
    fn default() -> Self {
        AsapSpec {
            algorithm: "RS256".to_owned(),
            issuer: String::new(),
            audience: String::new(),
            key_id: String::new(),
            subject: String::new(),
            expires_in: String::new(),
            additional_claims: String::new(),
            private_key: String::new(),
        }
    }
}

impl AsapSpec {
    /// Reads an auth mapping whose `type` is `asap`. A missing string is empty and a missing
    /// or blank algorithm is `RS256`.
    pub(crate) fn from_yaml(map: &Value) -> AsapSpec {
        let algorithm = text(map, "algorithm");
        AsapSpec {
            algorithm: if algorithm.is_empty() {
                "RS256".to_owned()
            } else {
                algorithm
            },
            issuer: text(map, "issuer"),
            audience: text(map, "audience"),
            key_id: text(map, "keyId"),
            subject: text(map, "subject"),
            expires_in: text(map, "expiresIn"),
            additional_claims: text(map, "additionalClaims"),
            private_key: text(map, "privateKey"),
        }
    }

    /// The whole auth mapping with every key, in the shape's order.
    pub(crate) fn to_yaml(&self) -> Value {
        let mut map = Mapping::new();
        map.insert(Value::from("type"), Value::from("asap"));
        for (key, value) in [
            ("algorithm", &self.algorithm),
            ("issuer", &self.issuer),
            ("audience", &self.audience),
            ("keyId", &self.key_id),
            ("subject", &self.subject),
            ("expiresIn", &self.expires_in),
            ("additionalClaims", &self.additional_claims),
            ("privateKey", &self.private_key),
        ] {
            map.insert(Value::from(key), Value::from(value.as_str()));
        }
        Value::Mapping(map)
    }
}

/// Shows the private key and the claims by size only.
impl fmt::Debug for AsapSpec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AsapSpec")
            .field("algorithm", &self.algorithm)
            .field("issuer", &self.issuer)
            .field("audience", &self.audience)
            .field("key_id", &self.key_id)
            .field("subject", &self.subject)
            .field("expires_in", &self.expires_in)
            .field("additional_claims", &Redacted(self.additional_claims.len()))
            .field("private_key", &Redacted(self.private_key.len()))
            .finish()
    }
}
