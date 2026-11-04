//! OAuth 1.0 auth as the app edits it, and its YAML projection.

use super::Redacted;
use std::fmt;
use yaml_serde::{Mapping, Value};

/// How an OAuth 1.0 request is signed.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum OAuth1SignatureMethod {
    /// `HMAC-SHA1`.
    HmacSha1,
    /// `HMAC-SHA256`.
    HmacSha256,
    /// `HMAC-SHA512`.
    HmacSha512,
    /// `RSA-SHA1`.
    RsaSha1,
    /// `RSA-SHA256`.
    RsaSha256,
    /// `RSA-SHA512`.
    RsaSha512,
    /// `PLAINTEXT`.
    Plaintext,
    /// Any other name, kept as written.
    Other(String),
}

impl OAuth1SignatureMethod {
    /// The name as written in YAML.
    pub fn as_str(&self) -> &str {
        use OAuth1SignatureMethod::*;
        match self {
            HmacSha1 => "HMAC-SHA1",
            HmacSha256 => "HMAC-SHA256",
            HmacSha512 => "HMAC-SHA512",
            RsaSha1 => "RSA-SHA1",
            RsaSha256 => "RSA-SHA256",
            RsaSha512 => "RSA-SHA512",
            Plaintext => "PLAINTEXT",
            Other(name) => name,
        }
    }

    /// The method named `name`; an unknown name is `Other`.
    pub fn parse(name: &str) -> Self {
        use OAuth1SignatureMethod::*;
        match name {
            "HMAC-SHA1" => HmacSha1,
            "HMAC-SHA256" => HmacSha256,
            "HMAC-SHA512" => HmacSha512,
            "RSA-SHA1" => RsaSha1,
            "RSA-SHA256" => RsaSha256,
            "RSA-SHA512" => RsaSha512,
            "PLAINTEXT" => Plaintext,
            other => Other(other.to_owned()),
        }
    }

    /// Whether the method signs with an RSA private key.
    pub fn is_rsa(&self) -> bool {
        use OAuth1SignatureMethod::*;
        matches!(self, RsaSha1 | RsaSha256 | RsaSha512)
    }
}

/// Where the OAuth 1.0 parameters are sent.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum OAuth1Placement {
    /// In the `Authorization` header.
    Header,
    /// In the query string.
    Query,
    /// In a form body.
    Body,
    /// Any other name, kept as written.
    Other(String),
}

impl OAuth1Placement {
    /// The name as written in YAML.
    pub fn as_str(&self) -> &str {
        match self {
            OAuth1Placement::Header => "header",
            OAuth1Placement::Query => "query",
            OAuth1Placement::Body => "body",
            OAuth1Placement::Other(name) => name,
        }
    }

    /// The placement named `name`; an unknown name is `Other`.
    pub fn parse(name: &str) -> Self {
        match name {
            "header" => OAuth1Placement::Header,
            "query" => OAuth1Placement::Query,
            "body" => OAuth1Placement::Body,
            other => OAuth1Placement::Other(other.to_owned()),
        }
    }
}

/// Whether a private key is given inline or as a file path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum OAuth1KeyKind {
    /// The PEM text itself.
    Text,
    /// A path to a PEM file.
    File,
}

/// The RSA private key of an RSA signature method.
#[derive(Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct OAuth1PrivateKey {
    /// Inline text or a file path.
    pub kind: OAuth1KeyKind,
    /// The PEM text or the path, as written.
    pub value: String,
}

impl OAuth1PrivateKey {
    /// A key of `kind` holding `value`.
    pub fn new(kind: OAuth1KeyKind, value: impl Into<String>) -> Self {
        OAuth1PrivateKey {
            kind,
            value: value.into(),
        }
    }
}

/// Shows an inline key by size only; a file path is shown.
impl fmt::Debug for OAuth1PrivateKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut d = f.debug_struct("OAuth1PrivateKey");
        d.field("kind", &self.kind);
        match self.kind {
            OAuth1KeyKind::File => d.field("value", &self.value),
            OAuth1KeyKind::Text => d.field("value", &Redacted(self.value.len())),
        };
        d.finish()
    }
}

/// OAuth 1.0 settings. `None` is a key left out of the file; `Some("")` is written as `""`.
#[derive(Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct OAuth1Spec {
    /// The consumer key.
    pub consumer_key: Option<String>,
    /// The consumer secret.
    pub consumer_secret: Option<String>,
    /// The access token.
    pub access_token: Option<String>,
    /// The access token secret.
    pub access_token_secret: Option<String>,
    /// The callback URL.
    pub callback_url: Option<String>,
    /// The verifier.
    pub verifier: Option<String>,
    /// The signature method.
    pub signature_method: OAuth1SignatureMethod,
    /// The RSA private key, used by the RSA methods only.
    pub private_key: Option<OAuth1PrivateKey>,
    /// A fixed timestamp; blank generates one at send.
    pub timestamp: Option<String>,
    /// A fixed nonce; blank generates one at send.
    pub nonce: Option<String>,
    /// The `oauth_version`.
    pub version: String,
    /// The realm.
    pub realm: Option<String>,
    /// Where the parameters are sent.
    pub placement: OAuth1Placement,
    /// Whether `oauth_body_hash` is sent.
    pub include_body_hash: bool,
}

/// A new OAuth 1.0 auth as the app creates it: every field present and blank.
impl Default for OAuth1Spec {
    fn default() -> Self {
        let blank = || Some(String::new());
        OAuth1Spec {
            consumer_key: blank(),
            consumer_secret: blank(),
            access_token: blank(),
            access_token_secret: blank(),
            callback_url: blank(),
            verifier: blank(),
            signature_method: OAuth1SignatureMethod::HmacSha1,
            private_key: Some(OAuth1PrivateKey::new(OAuth1KeyKind::Text, "")),
            timestamp: blank(),
            nonce: blank(),
            version: "1.0".to_owned(),
            realm: blank(),
            placement: OAuth1Placement::Header,
            include_body_hash: false,
        }
    }
}

impl OAuth1Spec {
    /// Read an auth mapping whose `type` is `oauth1`. A blank or non-string value reads as
    /// absent, and the method, version, placement and body-hash flag fall back to defaults.
    pub(crate) fn from_yaml(map: &Value) -> OAuth1Spec {
        let get = |key: &str| {
            map.get(key)
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
        };
        // A mapping names its kind; a plain string is inline text. A blank key drops entirely.
        let private_key = match map.get("privateKey") {
            Some(key @ Value::Mapping(_)) => {
                let kind = match key.get("type").and_then(Value::as_str) {
                    Some("file") => OAuth1KeyKind::File,
                    _ => OAuth1KeyKind::Text,
                };
                key.get("value")
                    .and_then(Value::as_str)
                    .filter(|s| !s.is_empty())
                    .map(|v| OAuth1PrivateKey::new(kind, v))
            }
            Some(Value::String(text)) if !text.is_empty() => {
                Some(OAuth1PrivateKey::new(OAuth1KeyKind::Text, text.as_str()))
            }
            _ => None,
        };
        OAuth1Spec {
            consumer_key: get("consumerKey"),
            consumer_secret: get("consumerSecret"),
            access_token: get("accessToken"),
            access_token_secret: get("accessTokenSecret"),
            callback_url: get("callbackUrl"),
            verifier: get("verifier"),
            signature_method: get("signatureMethod")
                .map_or(OAuth1SignatureMethod::HmacSha1, |name| {
                    OAuth1SignatureMethod::parse(&name)
                }),
            private_key,
            timestamp: get("timestamp"),
            nonce: get("nonce"),
            version: get("version").unwrap_or_else(|| "1.0".to_owned()),
            realm: get("realm"),
            placement: get("placement").map_or(OAuth1Placement::Header, |name| {
                OAuth1Placement::parse(&name)
            }),
            include_body_hash: map
                .get("includeBodyHash")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        }
    }

    /// The whole auth mapping in the reference writer's key order, each `None` left out.
    pub(crate) fn to_yaml(&self) -> Value {
        let mut map = Mapping::new();
        let mut put = |key: &str, value: Option<Value>| {
            if let Some(value) = value {
                map.insert(Value::from(key), value);
            }
        };
        let text = |s: &Option<String>| s.as_deref().map(Value::from);
        put("type", Some(Value::from("oauth1")));
        put("consumerKey", text(&self.consumer_key));
        put("consumerSecret", text(&self.consumer_secret));
        put("accessToken", text(&self.access_token));
        put("accessTokenSecret", text(&self.access_token_secret));
        put("callbackUrl", text(&self.callback_url));
        put("verifier", text(&self.verifier));
        put(
            "signatureMethod",
            Some(Value::from(self.signature_method.as_str())),
        );
        put(
            "privateKey",
            self.private_key.as_ref().map(|key| {
                let mut m = Mapping::new();
                let kind = match key.kind {
                    OAuth1KeyKind::File => "file",
                    OAuth1KeyKind::Text => "text",
                };
                m.insert(Value::from("type"), Value::from(kind));
                m.insert(Value::from("value"), Value::from(key.value.as_str()));
                Value::Mapping(m)
            }),
        );
        put("timestamp", text(&self.timestamp));
        put("nonce", text(&self.nonce));
        put("version", Some(Value::from(self.version.as_str())));
        put("realm", text(&self.realm));
        put("placement", Some(Value::from(self.placement.as_str())));
        put("includeBodyHash", Some(Value::Bool(self.include_body_hash)));
        Value::Mapping(map)
    }
}

/// Shows the consumer secret, both tokens and an inline key by size only.
impl fmt::Debug for OAuth1Spec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let hidden = |s: &Option<String>| s.as_ref().map(|s| Redacted(s.len()));
        f.debug_struct("OAuth1Spec")
            .field("consumer_key", &self.consumer_key)
            .field("consumer_secret", &hidden(&self.consumer_secret))
            .field("access_token", &hidden(&self.access_token))
            .field("access_token_secret", &hidden(&self.access_token_secret))
            .field("callback_url", &self.callback_url)
            .field("verifier", &self.verifier)
            .field("signature_method", &self.signature_method)
            .field("private_key", &self.private_key)
            .field("timestamp", &self.timestamp)
            .field("nonce", &self.nonce)
            .field("version", &self.version)
            .field("realm", &self.realm)
            .field("placement", &self.placement)
            .field("include_body_hash", &self.include_body_hash)
            .finish()
    }
}
