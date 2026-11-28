//! Hawk auth as the app edits it, and its YAML projection.

use super::{Redacted, text};
use std::fmt;
use yaml_serde::{Mapping, Value};

/// Hawk credentials and signing options; every string may hold `{{variables}}`.
#[derive(Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct HawkSpec {
    /// The key identifier sent as `id`.
    pub auth_id: String,
    /// The shared key the MAC is keyed with.
    pub auth_key: String,
    /// The MAC hash, `sha256` or `sha1`.
    pub algorithm: String,
    /// Application data sent as `ext`, empty for none.
    pub ext: String,
    /// The application id sent as `app`, empty for none.
    pub app: String,
    /// The delegating application sent as `dlg`, used only with `app`.
    pub dlg: String,
    /// A fixed nonce, empty for a fresh one per signature.
    pub nonce: String,
    /// A fixed Unix timestamp in seconds, empty for the current time.
    pub timestamp: String,
    /// Whether the body's hash is signed and sent as `hash`.
    pub include_payload_hash: bool,
}

/// A new Hawk auth as the app creates it: SHA-256, nothing else set.
impl Default for HawkSpec {
    fn default() -> Self {
        HawkSpec {
            auth_id: String::new(),
            auth_key: String::new(),
            algorithm: "sha256".to_owned(),
            ext: String::new(),
            app: String::new(),
            dlg: String::new(),
            nonce: String::new(),
            timestamp: String::new(),
            include_payload_hash: false,
        }
    }
}

impl HawkSpec {
    /// Reads an auth mapping whose `type` is `hawk`. A missing string is empty, a missing or
    /// blank algorithm is `sha256`, and a quoted flag reads as the word it spells.
    pub(crate) fn from_yaml(map: &Value) -> HawkSpec {
        let algorithm = text(map, "algorithm");
        HawkSpec {
            auth_id: text(map, "authId"),
            auth_key: text(map, "authKey"),
            algorithm: if algorithm.is_empty() {
                "sha256".to_owned()
            } else {
                algorithm
            },
            ext: text(map, "ext"),
            app: text(map, "app"),
            dlg: text(map, "dlg"),
            nonce: text(map, "nonce"),
            timestamp: text(map, "timestamp"),
            include_payload_hash: match map.get("includePayloadHash") {
                Some(Value::Bool(on)) => *on,
                Some(Value::String(word)) => word == "true",
                _ => false,
            },
        }
    }

    /// The whole auth mapping with every key, in the shape's order.
    pub(crate) fn to_yaml(&self) -> Value {
        let mut map = Mapping::new();
        map.insert(Value::from("type"), Value::from("hawk"));
        for (key, value) in [
            ("authId", &self.auth_id),
            ("authKey", &self.auth_key),
            ("algorithm", &self.algorithm),
            ("ext", &self.ext),
            ("app", &self.app),
            ("dlg", &self.dlg),
            ("nonce", &self.nonce),
            ("timestamp", &self.timestamp),
        ] {
            map.insert(Value::from(key), Value::from(value.as_str()));
        }
        map.insert(
            Value::from("includePayloadHash"),
            Value::Bool(self.include_payload_hash),
        );
        Value::Mapping(map)
    }
}

/// Shows the key by size only.
impl fmt::Debug for HawkSpec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HawkSpec")
            .field("auth_id", &self.auth_id)
            .field("auth_key", &Redacted(self.auth_key.len()))
            .field("algorithm", &self.algorithm)
            .field("ext", &self.ext)
            .field("app", &self.app)
            .field("dlg", &self.dlg)
            .field("nonce", &self.nonce)
            .field("timestamp", &self.timestamp)
            .field("include_payload_hash", &self.include_payload_hash)
            .finish()
    }
}
