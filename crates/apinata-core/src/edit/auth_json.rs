//! The JSON auth shape the CLI's `--auth-json` and MCP's write tools share, and its parser.
//!
//! The shape is `{"type": <mode>, ...}` with snake_case keys; enum-like values keep the file's
//! spellings. Errors name the problem, the field path or position where known, but never
//! repeat an input value, since auth JSON is full of credentials.

use super::{
    ApiKeyPlacement, AuthSpec, ClientAuth, EdgeGridSpec, OAuth1KeyKind, OAuth1Placement,
    OAuth1PrivateKey, OAuth1SignatureMethod, OAuth1Spec, OAuth2Flow, OAuth2Param,
    OAuth2ParamPlacement, OAuth2Spec, OAuth2Stage, TokenPlacement, TokenSource,
};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

/// Where in the text a problem is, when serde knows: field errors inside a mode lose it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct Position {
    /// One-based line.
    pub line: usize,
    /// One-based column.
    pub column: usize,
}

/// ` (line L, column C)` for a known position, nothing otherwise.
fn at(position: Option<&Position>) -> String {
    position
        .map(|p| format!(" (line {}, column {})", p.line, p.column))
        .unwrap_or_default()
}

/// Why an auth JSON document was refused. No variant carries a value from the input other
/// than a field name, a field path or the mode tag.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum AuthJsonError {
    /// The text is not well-formed JSON.
    #[error("the auth JSON is not well-formed{}", at(position.as_ref()))]
    Malformed {
        /// Where the text stops being JSON.
        position: Option<Position>,
    },
    /// `type` names no auth mode.
    #[error("unknown auth type `{mode}`{}", at(position.as_ref()))]
    UnknownType {
        /// The `type` given.
        mode: String,
        /// Where, when known.
        position: Option<Position>,
    },
    /// A key the mode does not take.
    #[error("unknown auth field `{field}`{}", at(position.as_ref()))]
    UnknownField {
        /// The key given.
        field: String,
        /// Where, when known.
        position: Option<Position>,
    },
    /// A key the mode requires is absent.
    #[error("the auth JSON lacks the field `{field}`{}", at(position.as_ref()))]
    MissingField {
        /// The key required.
        field: String,
        /// Where, when known.
        position: Option<Position>,
    },
    /// A value of the wrong JSON type, or not one of the values its field allows. `field` is
    /// its key path, such as `params[1].placement`; `None` when the document itself is not
    /// an object.
    #[error(
        "{}an auth JSON value has the wrong type or is not an allowed value{}{}",
        field.as_ref().map(|f| format!("`{f}`: ")).unwrap_or_default(),
        expected.as_ref().map(|e| format!(", expected {e}")).unwrap_or_default(),
        at(position.as_ref())
    )]
    InvalidValue {
        /// The key path of the value.
        field: Option<String>,
        /// What the field takes, from the parser's own description, never from the input.
        expected: Option<String>,
        /// Where, when known.
        position: Option<Position>,
    },
}

/// Parse an auth in the shared JSON shape into the spec the file setters write.
pub fn parse_auth_json(text: &str) -> Result<AuthSpec, AuthJsonError> {
    // A valid document is read once, straight into its mode's fields. Any refusal is rebuilt
    // by the slow path, which alone knows the field path, so every error stays the same.
    fast(text).map_or_else(|| slow(text), Ok)
}

/// The mode's fields read from the text with no intermediate value; None on any refusal.
fn fast(text: &str) -> Option<AuthSpec> {
    #[derive(Deserialize)]
    struct Tag {
        #[serde(rename = "type")]
        mode: String,
    }
    let Tag { mode } = serde_json::from_str(text).ok()?;
    spec(mode, Text(text)).ok()
}

/// One parse to a value, then one path-tracking pass over the mode's fields: linear in the
/// document, however late a bad value sits.
fn slow(text: &str) -> Result<AuthSpec, AuthJsonError> {
    let doc: Value = serde_json::from_str(text).map_err(|e| AuthJsonError::Malformed {
        position: (e.line() > 0).then(|| Position {
            line: e.line(),
            column: e.column(),
        }),
    })?;
    let Value::Object(mut map) = doc else {
        return Err(invalid(None, Some("an object".into())));
    };
    let mode = match map.remove("type") {
        Some(Value::String(mode)) => mode,
        Some(_) => return Err(invalid(Some("type".into()), Some("a string".into()))),
        None => {
            return Err(AuthJsonError::MissingField {
                field: "type".into(),
                position: None,
            });
        }
    };
    spec(mode, Value::Object(map))
}

/// Where a mode's fields come from: the whole text for the fast path, or the document
/// without its `type` for the slow one.
trait Fields {
    fn read<T: DeserializeOwned>(self) -> Result<T, AuthJsonError>;
}

impl Fields for Value {
    fn read<T: DeserializeOwned>(self) -> Result<T, AuthJsonError> {
        fields(self)
    }
}

struct Text<'a>(&'a str);

impl Fields for Text<'_> {
    fn read<T: DeserializeOwned>(self) -> Result<T, AuthJsonError> {
        // The fast path discards its errors, so any variant stands in for them.
        serde_json::from_str::<WithoutType<T>>(self.0)
            .map(|w| w.0)
            .map_err(|_| AuthJsonError::Malformed { position: None })
    }
}

/// The spec for `mode` from its fields in `rest`.
fn spec(mode: String, rest: impl Fields) -> Result<AuthSpec, AuthJsonError> {
    Ok(match mode.as_str() {
        "none" => rest.read::<NoFields>().map(|_| AuthSpec::None)?,
        "inherit" => rest.read::<NoFields>().map(|_| AuthSpec::Inherit)?,
        "basic" => {
            let UserPass { username, password } = rest.read()?;
            AuthSpec::Basic { username, password }
        }
        "digest" => {
            let UserPass { username, password } = rest.read()?;
            AuthSpec::Digest { username, password }
        }
        "wsse" => {
            let UserPass { username, password } = rest.read()?;
            AuthSpec::Wsse { username, password }
        }
        "bearer" => AuthSpec::Bearer {
            token: rest.read::<BearerJson>()?.token,
        },
        "apikey" => rest.read::<ApiKeyJson>()?.into_spec(),
        "awsv4" => rest.read::<AwsV4Json>()?.into_spec(),
        "oauth1" => rest.read::<OAuth1Json>()?.into_spec(),
        "oauth2" => AuthSpec::OAuth2(rest.read::<OAuth2Json>()?.into_spec()?),
        "ntlm" => {
            let NtlmJson {
                username,
                password,
                domain,
            } = rest.read()?;
            AuthSpec::Ntlm {
                username,
                password,
                domain,
            }
        }
        "edgegrid" => rest.read::<EdgeGridJson>()?.into_spec(),
        _ => {
            return Err(AuthJsonError::UnknownType {
                mode,
                position: None,
            });
        }
    })
}

/// `T` read from an object whose `type` key is skipped, so a mode's own struct keeps
/// refusing every other unknown key.
struct WithoutType<T>(T);

impl<'de, T: Deserialize<'de>> Deserialize<'de> for WithoutType<T> {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct Visitor<T>(std::marker::PhantomData<T>);
        impl<'de, T: Deserialize<'de>> serde::de::Visitor<'de> for Visitor<T> {
            type Value = WithoutType<T>;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("an object")
            }
            fn visit_map<A: serde::de::MapAccess<'de>>(
                self,
                map: A,
            ) -> Result<Self::Value, A::Error> {
                T::deserialize(serde::de::value::MapAccessDeserializer::new(SkipType(map)))
                    .map(WithoutType)
            }
        }
        d.deserialize_map(Visitor(std::marker::PhantomData))
    }
}

/// A map's entries with every `type` entry left out.
struct SkipType<A>(A);

impl<'de, A: serde::de::MapAccess<'de>> serde::de::MapAccess<'de> for SkipType<A> {
    type Error = A::Error;

    fn next_key_seed<K: serde::de::DeserializeSeed<'de>>(
        &mut self,
        seed: K,
    ) -> Result<Option<K::Value>, A::Error> {
        use serde::de::IntoDeserializer;
        loop {
            let Some(key) = self.0.next_key::<String>()? else {
                return Ok(None);
            };
            if key == "type" {
                self.0.next_value::<serde::de::IgnoredAny>()?;
                continue;
            }
            return seed.deserialize(key.into_deserializer()).map(Some);
        }
    }

    fn next_value_seed<V: serde::de::DeserializeSeed<'de>>(
        &mut self,
        seed: V,
    ) -> Result<V::Value, A::Error> {
        self.0.next_value_seed(seed)
    }
}

/// The parser's fast and slow paths, so a test can hold them equal; not a public API.
#[doc(hidden)]
pub mod paths {
    use super::{AuthJsonError, AuthSpec};

    /// The typed parse alone; None on any refusal.
    pub fn fast(text: &str) -> Option<AuthSpec> {
        super::fast(text)
    }

    /// The value-then-path-tracking parse that builds every error.
    pub fn slow(text: &str) -> Result<AuthSpec, AuthJsonError> {
        super::slow(text)
    }
}

fn invalid(field: Option<String>, expected: Option<String>) -> AuthJsonError {
    AuthJsonError::InvalidValue {
        field,
        expected,
        position: None,
    }
}

/// A mode's fields from `rest`, the document without its `type`. serde's message may quote
/// the input, so only a leading backquoted field name and the static text after its last
/// `, expected ` are kept.
fn fields<T: DeserializeOwned>(rest: Value) -> Result<T, AuthJsonError> {
    serde_path_to_error::deserialize(rest).map_err(|e| {
        let path = e.path().to_string();
        let message = e.into_inner().to_string();
        let quoted = |prefix: &str| {
            message
                .strip_prefix(prefix)
                .and_then(|rest| rest.split_once('`'))
                .map(|(name, _)| name.to_owned())
        };
        if let Some(field) = quoted("missing field `") {
            return AuthJsonError::MissingField {
                field,
                position: None,
            };
        }
        if let Some(field) = quoted("unknown field `") {
            return AuthJsonError::UnknownField {
                field,
                position: None,
            };
        }
        let field = (path != ".").then_some(path);
        invalid(field, expected_of(&message))
    })
}

/// The part of a serde type or value error that names what was wanted, which serde writes
/// last from the visitor's static description; None for any other message.
fn expected_of(message: &str) -> Option<String> {
    let known = [
        "invalid type: ",
        "invalid value: ",
        "unknown variant ",
        "invalid length ",
    ];
    if !known.iter().any(|k| message.starts_with(k)) {
        return None;
    }
    let (_, expected) = message.rsplit_once(", expected ")?;
    Some(expected.to_owned())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NoFields {}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UserPass {
    username: String,
    password: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BearerJson {
    token: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ApiKeyJson {
    key: String,
    value: String,
    placement: ApiKeyPlacementJson,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AwsV4Json {
    access_key_id: String,
    secret_access_key: String,
    session_token: String,
    service: String,
    region: String,
    profile_name: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OAuth1Json {
    consumer_key: Option<String>,
    consumer_secret: Option<String>,
    access_token: Option<String>,
    access_token_secret: Option<String>,
    callback_url: Option<String>,
    verifier: Option<String>,
    signature_method: String,
    private_key: Option<OAuth1KeyJson>,
    timestamp: Option<String>,
    nonce: Option<String>,
    version: String,
    realm: Option<String>,
    placement: String,
    include_body_hash: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NtlmJson {
    username: String,
    password: String,
    domain: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EdgeGridJson {
    access_token: String,
    client_token: String,
    client_secret: String,
    nonce: String,
    timestamp: String,
    base_url: String,
    headers_to_sign: String,
    max_body_size: Option<MaxBodySize>,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum ApiKeyPlacementJson {
    Header,
    Query,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OAuth1KeyJson {
    #[serde(rename = "type")]
    kind: OAuth1KeyKindJson,
    value: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum OAuth1KeyKindJson {
    Text,
    File,
}

/// Every field of [`OAuth2Spec`], other flows' included, so a read round-trips exactly.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OAuth2Json {
    flow: FlowJson,
    authorization_url: String,
    access_token_url: String,
    refresh_token_url: String,
    callback_url: String,
    client_id: String,
    client_secret: String,
    client_auth: Option<String>,
    username: String,
    password: String,
    scope: String,
    state: String,
    pkce: bool,
    token_id: String,
    token_placement: TokenPlacementJson,
    token_source: String,
    auto_fetch: bool,
    auto_refresh: bool,
    params: Vec<OAuth2ParamJson>,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum FlowJson {
    ClientCredentials,
    ResourceOwnerPasswordCredentials,
    AuthorizationCode,
    Implicit,
}

/// `{"type": "header", "prefix"}`, `{"type": "query", "key"}` or `{"type": "unset"}`. Read as
/// one struct, not a tagged enum, so a bad value inside keeps its path; the keys each type
/// needs and refuses are checked after.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TokenPlacementJson {
    #[serde(rename = "type")]
    kind: TokenPlacementKind,
    #[serde(default, deserialize_with = "present")]
    prefix: Option<String>,
    #[serde(default, deserialize_with = "present")]
    key: Option<String>,
}

/// A key that is given must hold a string; only an absent key reads as None, never `null`.
fn present<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    String::deserialize(d).map(Some)
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum TokenPlacementKind {
    Header,
    Query,
    Unset,
}

impl TokenPlacementJson {
    fn into_spec(self) -> Result<TokenPlacement, AuthJsonError> {
        let missing = |field: &str| AuthJsonError::MissingField {
            field: field.into(),
            position: None,
        };
        let unknown = |field: &str| AuthJsonError::UnknownField {
            field: field.into(),
            position: None,
        };
        match (self.kind, self.prefix, self.key) {
            (TokenPlacementKind::Header, Some(prefix), None) => {
                Ok(TokenPlacement::Header { prefix })
            }
            (TokenPlacementKind::Query, None, Some(key)) => Ok(TokenPlacement::Query { key }),
            (TokenPlacementKind::Unset, None, None) => Ok(TokenPlacement::Unset),
            (TokenPlacementKind::Header, _, Some(_)) | (TokenPlacementKind::Unset, _, Some(_)) => {
                Err(unknown("key"))
            }
            (TokenPlacementKind::Query, Some(_), _) | (TokenPlacementKind::Unset, Some(_), _) => {
                Err(unknown("prefix"))
            }
            (TokenPlacementKind::Header, None, None) => Err(missing("prefix")),
            (TokenPlacementKind::Query, None, None) => Err(missing("key")),
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OAuth2ParamJson {
    stage: StageJson,
    name: String,
    value: String,
    placement: ParamPlacementJson,
}

/// The stage keys of the file's `additionalParameters`.
#[derive(Deserialize)]
enum StageJson {
    #[serde(rename = "authorizationRequest")]
    Authorization,
    #[serde(rename = "accessTokenRequest")]
    Token,
    #[serde(rename = "refreshTokenRequest")]
    Refresh,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum ParamPlacementJson {
    Header,
    Query,
    Body,
}

/// A finite number, or `"Infinity"`, `"-Infinity"` or `"NaN"`, which JSON has no number for.
struct MaxBodySize(f64);

impl<'de> Deserialize<'de> for MaxBodySize {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct Visitor;
        impl serde::de::Visitor<'_> for Visitor {
            type Value = MaxBodySize;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str(r#"a number, "Infinity", "-Infinity" or "NaN""#)
            }
            fn visit_f64<E>(self, v: f64) -> Result<MaxBodySize, E> {
                Ok(MaxBodySize(v))
            }
            fn visit_i64<E>(self, v: i64) -> Result<MaxBodySize, E> {
                Ok(MaxBodySize(v as f64))
            }
            fn visit_u64<E>(self, v: u64) -> Result<MaxBodySize, E> {
                Ok(MaxBodySize(v as f64))
            }
            fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<MaxBodySize, E> {
                match v {
                    "Infinity" => Ok(MaxBodySize(f64::INFINITY)),
                    "-Infinity" => Ok(MaxBodySize(f64::NEG_INFINITY)),
                    "NaN" => Ok(MaxBodySize(f64::NAN)),
                    _ => Err(E::invalid_value(serde::de::Unexpected::Str(v), &self)),
                }
            }
        }
        d.deserialize_any(Visitor)
    }
}

impl ApiKeyJson {
    fn into_spec(self) -> AuthSpec {
        AuthSpec::ApiKey {
            key: self.key,
            value: self.value,
            placement: match self.placement {
                ApiKeyPlacementJson::Header => ApiKeyPlacement::Header,
                ApiKeyPlacementJson::Query => ApiKeyPlacement::Query,
            },
        }
    }
}

impl AwsV4Json {
    fn into_spec(self) -> AuthSpec {
        AuthSpec::AwsV4 {
            access_key_id: self.access_key_id,
            secret_access_key: self.secret_access_key,
            session_token: self.session_token,
            service: self.service,
            region: self.region,
            profile_name: self.profile_name,
        }
    }
}

impl OAuth1Json {
    fn into_spec(self) -> AuthSpec {
        AuthSpec::OAuth1(OAuth1Spec {
            consumer_key: self.consumer_key,
            consumer_secret: self.consumer_secret,
            access_token: self.access_token,
            access_token_secret: self.access_token_secret,
            callback_url: self.callback_url,
            verifier: self.verifier,
            signature_method: OAuth1SignatureMethod::parse(&self.signature_method),
            private_key: self.private_key.map(|k| {
                let kind = match k.kind {
                    OAuth1KeyKindJson::Text => OAuth1KeyKind::Text,
                    OAuth1KeyKindJson::File => OAuth1KeyKind::File,
                };
                OAuth1PrivateKey::new(kind, k.value)
            }),
            timestamp: self.timestamp,
            nonce: self.nonce,
            version: self.version,
            realm: self.realm,
            placement: OAuth1Placement::parse(&self.placement),
            include_body_hash: self.include_body_hash,
        })
    }
}

impl EdgeGridJson {
    fn into_spec(self) -> AuthSpec {
        AuthSpec::EdgeGrid(EdgeGridSpec {
            access_token: self.access_token,
            client_token: self.client_token,
            client_secret: self.client_secret,
            nonce: self.nonce,
            timestamp: self.timestamp,
            base_url: self.base_url,
            headers_to_sign: self.headers_to_sign,
            max_body_size: self.max_body_size.map(|m| m.0),
        })
    }
}

impl OAuth2Json {
    fn into_spec(self) -> Result<OAuth2Spec, AuthJsonError> {
        Ok(OAuth2Spec {
            flow: match self.flow {
                FlowJson::ClientCredentials => OAuth2Flow::ClientCredentials,
                FlowJson::ResourceOwnerPasswordCredentials => OAuth2Flow::Password,
                FlowJson::AuthorizationCode => OAuth2Flow::AuthorizationCode,
                FlowJson::Implicit => OAuth2Flow::Implicit,
            },
            authorization_url: self.authorization_url,
            access_token_url: self.access_token_url,
            refresh_token_url: self.refresh_token_url,
            callback_url: self.callback_url,
            client_id: self.client_id,
            client_secret: self.client_secret,
            client_auth: self.client_auth.map(|c| match c.as_str() {
                "body" => ClientAuth::Body,
                "basic_auth_header" => ClientAuth::BasicHeader,
                _ => ClientAuth::Other(c),
            }),
            username: self.username,
            password: self.password,
            scope: self.scope,
            state: self.state,
            pkce: self.pkce,
            token_id: self.token_id,
            token_placement: self.token_placement.into_spec()?,
            token_source: match self.token_source.as_str() {
                "access_token" => TokenSource::AccessToken,
                "id_token" => TokenSource::IdToken,
                _ => TokenSource::Other(self.token_source),
            },
            auto_fetch: self.auto_fetch,
            auto_refresh: self.auto_refresh,
            params: self
                .params
                .into_iter()
                .map(|p| {
                    let stage = match p.stage {
                        StageJson::Authorization => OAuth2Stage::Authorization,
                        StageJson::Token => OAuth2Stage::Token,
                        StageJson::Refresh => OAuth2Stage::Refresh,
                    };
                    let placement = match p.placement {
                        ParamPlacementJson::Header => OAuth2ParamPlacement::Header,
                        ParamPlacementJson::Query => OAuth2ParamPlacement::Query,
                        ParamPlacementJson::Body => OAuth2ParamPlacement::Body,
                    };
                    OAuth2Param::new(stage, p.name, p.value, placement)
                })
                .collect(),
        })
    }
}
