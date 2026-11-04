//! Akamai EdgeGrid auth as the app edits it, and its YAML projection.

use super::{Redacted, text};
use std::fmt;
use yaml_serde::{Mapping, Value};

/// Akamai EdgeGrid credentials and signing options; every string may hold `{{variables}}`.
#[derive(Clone, Default)]
#[non_exhaustive]
pub struct EdgeGridSpec {
    /// The access token.
    pub access_token: String,
    /// The client token.
    pub client_token: String,
    /// The client secret.
    pub client_secret: String,
    /// A fixed nonce, empty for a fresh one per signature.
    pub nonce: String,
    /// A fixed timestamp, empty for the current time.
    pub timestamp: String,
    /// The base URL whose host is signed, empty for the request's own.
    pub base_url: String,
    /// Comma-separated header names to sign.
    pub headers_to_sign: String,
    /// The most body bytes hashed, `None` for the default.
    pub max_body_size: Option<f64>,
}

/// Largest magnitude below which every integral `f64` is exact, so it is written as an integer.
pub(crate) const MAX_EXACT: f64 = 9_007_199_254_740_992.0;

impl EdgeGridSpec {
    /// Reads the mapping; a missing string is empty and a non-number `maxBodySize` is `None`.
    pub(crate) fn from_yaml(map: &Value) -> EdgeGridSpec {
        EdgeGridSpec {
            access_token: text(map, "accessToken"),
            client_token: text(map, "clientToken"),
            client_secret: text(map, "clientSecret"),
            nonce: text(map, "nonce"),
            timestamp: text(map, "timestamp"),
            base_url: text(map, "baseURL"),
            headers_to_sign: text(map, "headersToSign"),
            max_body_size: match map.get("maxBodySize") {
                Some(Value::Number(n)) => n.as_f64(),
                _ => None,
            },
        }
    }

    /// The whole auth mapping in the reference writer's key order. An integral `maxBodySize`
    /// is written as an integer so an unchanged read stays byte-equal; `None` leaves it out.
    pub(crate) fn to_yaml(&self) -> Value {
        let mut map = Mapping::new();
        map.insert(Value::from("type"), Value::from("akamai-edgegrid"));
        for (key, value) in [
            ("accessToken", &self.access_token),
            ("clientToken", &self.client_token),
            ("clientSecret", &self.client_secret),
            ("nonce", &self.nonce),
            ("timestamp", &self.timestamp),
            ("baseURL", &self.base_url),
            ("headersToSign", &self.headers_to_sign),
        ] {
            map.insert(Value::from(key), Value::from(value.as_str()));
        }
        if let Some(max) = self.max_body_size {
            let value = if max.fract() == 0.0 && max.abs() <= MAX_EXACT {
                Value::from(max as i64)
            } else {
                Value::from(max)
            };
            map.insert(Value::from("maxBodySize"), value);
        }
        Value::Mapping(map)
    }
}

/// Compares `max_body_size` by its bits, so a NaN equals itself and `Eq` holds.
impl PartialEq for EdgeGridSpec {
    fn eq(&self, other: &Self) -> bool {
        fn strings(s: &EdgeGridSpec) -> [&String; 7] {
            [
                &s.access_token,
                &s.client_token,
                &s.client_secret,
                &s.nonce,
                &s.timestamp,
                &s.base_url,
                &s.headers_to_sign,
            ]
        }
        strings(self) == strings(other)
            && self.max_body_size.map(f64::to_bits) == other.max_body_size.map(f64::to_bits)
    }
}

impl Eq for EdgeGridSpec {}

/// Shows the three credentials by size only.
impl fmt::Debug for EdgeGridSpec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EdgeGridSpec")
            .field("access_token", &Redacted(self.access_token.len()))
            .field("client_token", &Redacted(self.client_token.len()))
            .field("client_secret", &Redacted(self.client_secret.len()))
            .field("nonce", &self.nonce)
            .field("timestamp", &self.timestamp)
            .field("base_url", &self.base_url)
            .field("headers_to_sign", &self.headers_to_sign)
            .field("max_body_size", &self.max_body_size)
            .finish()
    }
}
