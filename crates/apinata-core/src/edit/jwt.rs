//! JWT bearer auth as the app edits it, and its YAML projection.

use super::oauth1::{OAuth1KeyKind, OAuth1PrivateKey};
use super::{Redacted, text};
use std::fmt;
use yaml_serde::{Mapping, Value};

/// How a JWT auth token is signed.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
// Each variant is the algorithm's own registered name; a doc line apiece would only repeat it.
#[allow(missing_docs)]
pub enum JwtAlgorithm {
    Hs256,
    Hs384,
    Hs512,
    Rs256,
    Rs384,
    Rs512,
    Ps256,
    Ps384,
    Ps512,
    Es256,
    Es384,
    Es512,
    /// Any other name, kept as written.
    Other(String),
}

/// Every named algorithm with its spelling, in picker order.
const ALGORITHMS: [(JwtAlgorithm, &str); 12] = [
    (JwtAlgorithm::Hs256, "HS256"),
    (JwtAlgorithm::Hs384, "HS384"),
    (JwtAlgorithm::Hs512, "HS512"),
    (JwtAlgorithm::Rs256, "RS256"),
    (JwtAlgorithm::Rs384, "RS384"),
    (JwtAlgorithm::Rs512, "RS512"),
    (JwtAlgorithm::Ps256, "PS256"),
    (JwtAlgorithm::Ps384, "PS384"),
    (JwtAlgorithm::Ps512, "PS512"),
    (JwtAlgorithm::Es256, "ES256"),
    (JwtAlgorithm::Es384, "ES384"),
    (JwtAlgorithm::Es512, "ES512"),
];

impl JwtAlgorithm {
    /// The name as written in YAML and in the token's `alg`.
    pub fn as_str(&self) -> &str {
        match self {
            JwtAlgorithm::Other(name) => name,
            known => ALGORITHMS
                .iter()
                .find(|(alg, _)| alg == known)
                .map_or("", |(_, name)| name),
        }
    }

    /// The algorithm named `name`; an unknown name is `Other`.
    pub fn parse(name: &str) -> Self {
        ALGORITHMS
            .iter()
            .find(|(_, spelling)| *spelling == name)
            .map_or_else(
                || JwtAlgorithm::Other(name.to_owned()),
                |(alg, _)| alg.clone(),
            )
    }

    /// Whether the algorithm signs with a shared secret (HS) rather than a private key.
    pub fn uses_secret(&self) -> bool {
        matches!(
            self,
            JwtAlgorithm::Hs256 | JwtAlgorithm::Hs384 | JwtAlgorithm::Hs512
        )
    }
}

/// Where the signed token is sent.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum JwtPlacement {
    /// In the `Authorization` header, after the prefix.
    Header,
    /// As a query parameter.
    Query,
    /// Any other name, kept as written.
    Other(String),
}

impl JwtPlacement {
    /// The name as written in YAML.
    pub fn as_str(&self) -> &str {
        match self {
            JwtPlacement::Header => "header",
            JwtPlacement::Query => "query",
            JwtPlacement::Other(name) => name,
        }
    }

    /// The placement named `name`; an unknown name is `Other`.
    pub fn parse(name: &str) -> Self {
        match name {
            "header" => JwtPlacement::Header,
            "query" => JwtPlacement::Query,
            other => JwtPlacement::Other(other.to_owned()),
        }
    }
}

/// JWT auth: a token signed fresh on every send. Strings may hold `{{variables}}`.
#[derive(Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct JwtSpec {
    /// The signing algorithm, always the token's `alg`.
    pub algorithm: JwtAlgorithm,
    /// The HS secret.
    pub secret: String,
    /// Whether the secret is base64 text to decode before signing.
    pub secret_base64: bool,
    /// The PEM private key of the RS, PS and ES algorithms.
    pub private_key: Option<OAuth1PrivateKey>,
    /// Extra token header keys as JSON object text; blank for none.
    pub headers: String,
    /// The claims as JSON object text; blank for `{}`.
    pub payload: String,
    /// Seconds until expiry; blank adds no `iat` or `exp`.
    pub expires_in: String,
    /// Where the token is sent.
    pub placement: JwtPlacement,
    /// The text before the token in the header; `None` (absent) sends `Bearer`.
    pub header_prefix: Option<String>,
    /// The query parameter name; blank sends `token`.
    pub query_param_key: String,
}

/// A new JWT auth as the app creates it.
impl Default for JwtSpec {
    fn default() -> Self {
        JwtSpec {
            algorithm: JwtAlgorithm::Hs256,
            secret: String::new(),
            secret_base64: false,
            private_key: Some(OAuth1PrivateKey::new(OAuth1KeyKind::Text, "")),
            headers: String::new(),
            payload: String::new(),
            expires_in: String::new(),
            placement: JwtPlacement::Header,
            header_prefix: Some("Bearer".to_owned()),
            query_param_key: "token".to_owned(),
        }
    }
}

impl JwtSpec {
    /// Read an auth mapping whose `type` is `jwt`. A missing string is empty; a missing
    /// algorithm or placement falls back to `HS256` and `header`.
    pub(crate) fn from_yaml(map: &Value) -> JwtSpec {
        let present = |key: &str| map.get(key).and_then(crate::js::yaml_to_string);
        let private_key = match map.get("privateKey") {
            Some(key @ Value::Mapping(_)) => {
                let kind = match key.get("type").and_then(Value::as_str) {
                    Some("file") => OAuth1KeyKind::File,
                    _ => OAuth1KeyKind::Text,
                };
                Some(OAuth1PrivateKey::new(kind, text(key, "value")))
            }
            Some(Value::String(pem)) => Some(OAuth1PrivateKey::new(OAuth1KeyKind::Text, pem)),
            _ => None,
        };
        JwtSpec {
            algorithm: present("algorithm")
                .filter(|s| !s.is_empty())
                .map_or(JwtAlgorithm::Hs256, |name| JwtAlgorithm::parse(&name)),
            secret: text(map, "secret"),
            // A hand-quoted flag reads as the word it spells, as request settings do.
            secret_base64: match map.get("secretBase64") {
                Some(Value::Bool(on)) => *on,
                Some(Value::String(word)) => word == "true",
                _ => false,
            },
            private_key,
            headers: text(map, "headers"),
            payload: text(map, "payload"),
            expires_in: text(map, "expiresIn"),
            placement: present("placement")
                .filter(|s| !s.is_empty())
                .map_or(JwtPlacement::Header, |name| JwtPlacement::parse(&name)),
            header_prefix: present("headerPrefix"),
            query_param_key: text(map, "queryParamKey"),
        }
    }

    /// The whole auth mapping in the shape's key order. The key and prefix are left out when
    /// absent, and a blank expiry is left out; a whole-number expiry is written as a number.
    pub(crate) fn to_yaml(&self) -> Value {
        let mut map = Mapping::new();
        let mut put = |key: &str, value: Value| {
            map.insert(Value::from(key), value);
        };
        put("type", Value::from("jwt"));
        put("algorithm", Value::from(self.algorithm.as_str()));
        put("secret", Value::from(self.secret.as_str()));
        put("secretBase64", Value::Bool(self.secret_base64));
        if let Some(key) = &self.private_key {
            let mut m = Mapping::new();
            let kind = match key.kind {
                OAuth1KeyKind::File => "file",
                OAuth1KeyKind::Text => "text",
            };
            m.insert(Value::from("type"), Value::from(kind));
            m.insert(Value::from("value"), Value::from(key.value.as_str()));
            put("privateKey", Value::Mapping(m));
        }
        put("headers", Value::from(self.headers.as_str()));
        put("payload", Value::from(self.payload.as_str()));
        let expires = self.expires_in.trim();
        if !expires.is_empty() {
            let value = match expires.parse::<u64>() {
                Ok(n) if n.to_string() == expires => Value::from(n),
                _ => Value::from(self.expires_in.as_str()),
            };
            put("expiresIn", value);
        }
        put("placement", Value::from(self.placement.as_str()));
        if let Some(prefix) = &self.header_prefix {
            put("headerPrefix", Value::from(prefix.as_str()));
        }
        put("queryParamKey", Value::from(self.query_param_key.as_str()));
        Value::Mapping(map)
    }
}

/// Shows the secret and an inline key by size only.
impl fmt::Debug for JwtSpec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("JwtSpec")
            .field("algorithm", &self.algorithm)
            .field("secret", &Redacted(self.secret.len()))
            .field("secret_base64", &self.secret_base64)
            .field("private_key", &self.private_key)
            .field("headers", &self.headers)
            .field("payload", &Redacted(self.payload.len()))
            .field("expires_in", &self.expires_in)
            .field("placement", &self.placement)
            .field("header_prefix", &self.header_prefix)
            .field("query_param_key", &self.query_param_key)
            .finish()
    }
}
