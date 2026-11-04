//! The documents read tools return: serde mirrors of core's edit types, which are not serde.
//! A core variant added later maps to an "unsupported in MCP" error instead of a guess.

use apinata_core::{
    ApiKeyPlacement, Assertion, AuthSpec, BodySpec, ClientAuth, EnvVariable, KeyValue,
    MultipartValue, OAuth1KeyKind, OAuth2Flow, OAuth2ParamPlacement, OAuth2Spec, OAuth2Stage,
    Param, ParamKind, RequestFile, RequestSettings, Timeout, TokenPlacement, TokenSource, VarValue,
    Variable,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

fn unsupported(what: &str) -> String {
    format!("{what} unsupported in MCP")
}

/// A request file's editable sections.
#[derive(Serialize, Deserialize, JsonSchema, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub(crate) struct RequestDoc {
    pub name: String,
    pub method: String,
    pub url: String,
    pub params: Vec<ParamDoc>,
    pub headers: Vec<KeyValueDoc>,
    pub body: BodyDoc,
    pub auth: AuthDoc,
    /// Request variables, set before the request is sent.
    pub vars: Vec<VarDoc>,
    pub assertions: Vec<AssertionDoc>,
    pub settings: SettingsDoc,
    pub docs: String,
    pub tags: Vec<String>,
    pub seq: Option<u64>,
    /// Present when the URL's query and the enabled query params disagree; read only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub query_mismatch: Option<QueryMismatchDoc>,
    /// The saved response examples, in file order; read only.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub examples: Vec<ExampleDoc>,
}

/// One saved example as `read_request` lists it: no headers or body.
#[derive(Serialize, Deserialize, JsonSchema, Clone, PartialEq)]
pub(crate) struct ExampleDoc {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<u16>,
}

/// Which side of a URL and params query mismatch a send uses.
pub(crate) const QUERY_MISMATCH_NOTE: &str = "the URL's query is sent";

/// The query param names whose values differ between the URL and the params list, URL names
/// first; `note` says which side a send uses.
#[derive(Serialize, Deserialize, JsonSchema, Clone, PartialEq, Debug)]
#[serde(deny_unknown_fields)]
pub(crate) struct QueryMismatchDoc {
    pub names: Vec<String>,
    pub note: String,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub(crate) struct KeyValueDoc {
    pub name: String,
    pub value: String,
    pub enabled: bool,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, PartialEq)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum ParamKindDoc {
    Query,
    Path,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub(crate) struct ParamDoc {
    pub name: String,
    pub value: String,
    pub kind: ParamKindDoc,
    pub enabled: bool,
}

/// A request, folder or collection variable. `type` is set for typed values such as `number`.
#[derive(Serialize, Deserialize, JsonSchema, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub(crate) struct VarDoc {
    pub name: String,
    pub value: String,
    #[serde(rename = "type", default, skip_serializing_if = "Option::is_none")]
    pub value_type: Option<String>,
    pub enabled: bool,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum BodyDoc {
    None,
    Json {
        text: String,
    },
    Xml {
        text: String,
    },
    Text {
        text: String,
    },
    Sparql {
        text: String,
    },
    FormUrlencoded {
        fields: Vec<KeyValueDoc>,
    },
    Multipart {
        fields: Vec<MultipartFieldDoc>,
    },
    File {
        files: Vec<FileEntryDoc>,
    },
    /// A GraphQL request's query and variables text; the only body a GraphQL request takes.
    Graphql {
        query: String,
        variables: String,
    },
    /// A body mode Apinata does not model; shown, never written.
    Unsupported {
        mode: String,
    },
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub(crate) struct MultipartFieldDoc {
    pub name: String,
    pub value: MultipartValueDoc,
    pub content_type: String,
    pub enabled: bool,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum MultipartValueDoc {
    Text { text: String },
    Files { paths: Vec<String> },
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub(crate) struct FileEntryDoc {
    pub path: String,
    pub content_type: String,
    pub selected: bool,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, PartialEq, Default)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum AuthDoc {
    #[default]
    None,
    Inherit,
    Basic {
        username: String,
        password: String,
    },
    Bearer {
        token: String,
    },
    Apikey {
        key: String,
        value: String,
        placement: PlacementDoc,
    },
    Digest {
        username: String,
        password: String,
    },
    /// AWS Signature Version 4; a blank service or region is inferred from the host, and blank
    /// keys come from `profile_name` in the AWS shared files.
    #[serde(rename = "awsv4")]
    Awsv4 {
        access_key_id: String,
        secret_access_key: String,
        session_token: String,
        service: String,
        region: String,
        profile_name: String,
    },
    /// OAuth 1.0. A `null` string is left out of the file and an empty one is written as "";
    /// `signature_method` and `placement` are the file's spellings; file values are shown as written.
    Oauth1 {
        consumer_key: Option<String>,
        consumer_secret: Option<String>,
        access_token: Option<String>,
        access_token_secret: Option<String>,
        callback_url: Option<String>,
        verifier: Option<String>,
        signature_method: String,
        private_key: Option<Oauth1KeyDoc>,
        timestamp: Option<String>,
        nonce: Option<String>,
        version: String,
        realm: Option<String>,
        placement: String,
        include_body_hash: bool,
    },
    /// OAuth 2.0. Every flow's fields are listed; only `flow`'s own are written to the file.
    Oauth2(Oauth2Doc),
    /// WSSE UsernameToken; the header is built at send time.
    Wsse {
        username: String,
        password: String,
    },
    /// NTLM; the handshake runs at send time. The domain may be empty.
    Ntlm {
        username: String,
        password: String,
        domain: String,
    },
    /// Akamai EdgeGrid, signed per hop; a blank nonce or timestamp is generated, and
    /// `max_body_size` `null` means the default limit.
    Edgegrid {
        access_token: String,
        client_token: String,
        client_secret: String,
        nonce: String,
        timestamp: String,
        base_url: String,
        headers_to_sign: String,
        max_body_size: Option<MaxBodySize>,
    },
    /// An auth mode Apinata does not model; shown, never written.
    Unsupported {
        mode: String,
    },
}

/// OAuth 2.0 settings. `flow`, `client_auth` (`null` when unset), `token_source` and the
/// params' `stage` and `placement` are the file's spellings.
#[derive(Serialize, Deserialize, JsonSchema, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub(crate) struct Oauth2Doc {
    pub flow: Oauth2FlowDoc,
    pub authorization_url: String,
    pub access_token_url: String,
    pub refresh_token_url: String,
    pub callback_url: String,
    pub client_id: String,
    pub client_secret: String,
    pub client_auth: Option<String>,
    pub username: String,
    pub password: String,
    pub scope: String,
    pub state: String,
    pub pkce: bool,
    pub token_id: String,
    pub token_placement: TokenPlacementDoc,
    pub token_source: String,
    pub auto_fetch: bool,
    pub auto_refresh: bool,
    pub params: Vec<Oauth2ParamDoc>,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, PartialEq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Oauth2FlowDoc {
    ClientCredentials,
    ResourceOwnerPasswordCredentials,
    AuthorizationCode,
    Implicit,
}

/// Where the fetched token goes: after `prefix` in the Authorization header, as query param
/// `key`, or `unset` when the file does not say.
#[derive(Serialize, Deserialize, JsonSchema, Clone, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum TokenPlacementDoc {
    Header { prefix: String },
    Query { key: String },
    Unset,
}

/// An extra parameter sent with one request of the flow.
#[derive(Serialize, Deserialize, JsonSchema, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub(crate) struct Oauth2ParamDoc {
    pub stage: Oauth2StageDoc,
    pub name: String,
    pub value: String,
    pub placement: Oauth2ParamPlacementDoc,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, PartialEq)]
pub(crate) enum Oauth2StageDoc {
    #[serde(rename = "authorizationRequest")]
    Authorization,
    #[serde(rename = "accessTokenRequest")]
    Token,
    #[serde(rename = "refreshTokenRequest")]
    Refresh,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, PartialEq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Oauth2ParamPlacementDoc {
    Header,
    Query,
    Body,
}

/// An EdgeGrid max body size: a finite value is a JSON number, and infinity or NaN, which JSON
/// has no number for, is the JavaScript spelling `"Infinity"`, `"-Infinity"` or `"NaN"`.
/// Equality is bitwise, so an unedited NaN compares equal to itself.
#[derive(Clone, Copy, Debug)]
pub(crate) struct MaxBodySize(pub f64);

impl PartialEq for MaxBodySize {
    fn eq(&self, other: &Self) -> bool {
        self.0.to_bits() == other.0.to_bits()
    }
}

impl Serialize for MaxBodySize {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self.0 {
            v if v.is_finite() => s.serialize_f64(v),
            v if v.is_nan() => s.serialize_str("NaN"),
            v if v > 0.0 => s.serialize_str("Infinity"),
            _ => s.serialize_str("-Infinity"),
        }
    }
}

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

impl JsonSchema for MaxBodySize {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "MaxBodySize".into()
    }

    fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({
            "anyOf": [
                {"type": "number"},
                {"type": "string", "enum": ["Infinity", "-Infinity", "NaN"]}
            ]
        })
    }
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, PartialEq)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum PlacementDoc {
    Header,
    Query,
}

/// An OAuth 1.0 RSA private key: inline PEM text or a file path.
#[derive(Serialize, Deserialize, JsonSchema, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub(crate) struct Oauth1KeyDoc {
    #[serde(rename = "type")]
    pub kind: KeyKindDoc,
    pub value: String,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, PartialEq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum KeyKindDoc {
    Text,
    File,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub(crate) struct AssertionDoc {
    pub expression: String,
    pub operator: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub(crate) struct SettingsDoc {
    pub encode_url: bool,
    pub timeout: TimeoutDoc,
    pub follow_redirects: bool,
    pub max_redirects: u32,
    pub forward_authorization_header: bool,
}

/// `inherit` uses the caller's default timeout: the app's Preferences value, while the CLI and
/// MCP set none, so there is no limit; `unlimited` waits forever.
#[derive(Serialize, Deserialize, JsonSchema, Clone, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum TimeoutDoc {
    Inherit,
    Unlimited,
    Millis { ms: u64 },
}

/// The sections a collection or folder file shares with its requests.
#[derive(Serialize, Deserialize, JsonSchema, Clone, PartialEq, Default)]
#[serde(deny_unknown_fields)]
pub(crate) struct ConfigDoc {
    pub headers: Vec<KeyValueDoc>,
    pub auth: AuthDoc,
    pub vars: Vec<VarDoc>,
    pub docs: String,
}

/// An environment variable. A secret never carries its value, only that it is secret.
#[derive(Serialize, Deserialize, JsonSchema, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub(crate) struct EnvVarDoc {
    pub name: String,
    /// Absent for a secret variable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    #[serde(rename = "type", default, skip_serializing_if = "Option::is_none")]
    pub value_type: Option<String>,
    pub enabled: bool,
    pub secret: bool,
}

/// One environment file's variables in file order, disabled ones included.
#[derive(Serialize, JsonSchema)]
pub(crate) struct EnvironmentDoc {
    pub name: String,
    pub variables: Vec<EnvVarDoc>,
}

/// Mirror a collection or folder file's shared sections.
pub(crate) fn config_doc(
    headers: Vec<KeyValue>,
    auth: AuthSpec,
    vars: Vec<Variable>,
    docs: String,
) -> Result<ConfigDoc, String> {
    Ok(ConfigDoc {
        headers: headers.iter().map(key_value).collect(),
        auth: self::auth(auth)?,
        vars: vars.iter().map(var).collect::<Result<_, _>>()?,
        docs,
    })
}

/// Mirror `v`, dropping a secret's value whatever core holds for it.
pub(crate) fn env_var(v: &EnvVariable) -> Result<EnvVarDoc, String> {
    let (value, value_type) = var_value(&v.value)?;
    Ok(EnvVarDoc {
        name: v.name.clone(),
        value: (!v.secret).then_some(value),
        value_type,
        enabled: v.enabled,
        secret: v.secret,
    })
}

/// Mirror every section of `r`; fails naming the first core variant MCP does not model.
pub(crate) fn request_doc(r: &RequestFile) -> Result<RequestDoc, String> {
    Ok(RequestDoc {
        name: r.name().to_owned(),
        method: r.method().to_owned(),
        url: r.url().to_owned(),
        params: r
            .shown_params()
            .iter()
            .map(param)
            .collect::<Result<_, _>>()?,
        headers: r.headers().iter().map(key_value).collect(),
        body: body(r.body())?,
        auth: auth(r.auth())?,
        vars: r.vars().iter().map(var).collect::<Result<_, _>>()?,
        assertions: r.assertions().iter().map(assertion).collect(),
        settings: settings(r.settings())?,
        docs: r.docs(),
        tags: r.tags(),
        seq: r.seq(),
        query_mismatch: query_mismatch_doc(r),
        examples: r
            .examples()
            .into_iter()
            .map(|e| ExampleDoc {
                name: e.name,
                description: e.description,
                status: e.response.and_then(|r| r.status),
            })
            .collect(),
    })
}

/// `r`'s URL and params disagreement as the tools report it, or `None` when they agree.
pub(crate) fn query_mismatch_doc(r: &RequestFile) -> Option<QueryMismatchDoc> {
    r.query_mismatch().map(|m| QueryMismatchDoc {
        names: m.names,
        note: QUERY_MISMATCH_NOTE.to_owned(),
    })
}

pub(crate) fn assertion(a: &Assertion) -> AssertionDoc {
    AssertionDoc {
        expression: a.expression().to_owned(),
        operator: a.operator().to_owned(),
        value: a.value().map(str::to_owned),
        enabled: a.enabled(),
        description: a.description().map(str::to_owned),
    }
}

pub(crate) fn key_value(kv: &KeyValue) -> KeyValueDoc {
    KeyValueDoc {
        name: kv.name.clone(),
        value: kv.value.clone(),
        enabled: kv.enabled,
    }
}

pub(crate) fn param(p: &Param) -> Result<ParamDoc, String> {
    let kind = match p.kind {
        ParamKind::Query => ParamKindDoc::Query,
        ParamKind::Path => ParamKindDoc::Path,
        _ => return Err(unsupported("param kind")),
    };
    Ok(ParamDoc {
        name: p.name.clone(),
        value: p.value.clone(),
        kind,
        enabled: p.enabled,
    })
}

fn var_value(v: &VarValue) -> Result<(String, Option<String>), String> {
    match v {
        VarValue::Text(text) => Ok((text.clone(), None)),
        VarValue::Typed { kind, data } => Ok((data.clone(), Some(kind.clone()))),
        _ => Err(unsupported("variable value")),
    }
}

pub(crate) fn var(v: &Variable) -> Result<VarDoc, String> {
    let (value, value_type) = var_value(&v.value)?;
    Ok(VarDoc {
        name: v.name.clone(),
        value,
        value_type,
        enabled: v.enabled,
    })
}

pub(crate) fn body(b: BodySpec) -> Result<BodyDoc, String> {
    Ok(match b {
        BodySpec::None => BodyDoc::None,
        BodySpec::Json(text) => BodyDoc::Json { text },
        BodySpec::Xml(text) => BodyDoc::Xml { text },
        BodySpec::Text(text) => BodyDoc::Text { text },
        BodySpec::Sparql(text) => BodyDoc::Sparql { text },
        BodySpec::FormUrlEncoded(fields) => BodyDoc::FormUrlencoded {
            fields: fields.iter().map(key_value).collect(),
        },
        BodySpec::Multipart(fields) => BodyDoc::Multipart {
            fields: fields
                .into_iter()
                .map(|f| {
                    let value = match f.value {
                        MultipartValue::Text(text) => MultipartValueDoc::Text { text },
                        MultipartValue::Files(paths) => MultipartValueDoc::Files { paths },
                        _ => return Err(unsupported("multipart value")),
                    };
                    Ok(MultipartFieldDoc {
                        name: f.name,
                        value,
                        content_type: f.content_type,
                        enabled: f.enabled,
                    })
                })
                .collect::<Result<_, _>>()?,
        },
        BodySpec::File(files) => BodyDoc::File {
            files: files
                .into_iter()
                .map(|f| FileEntryDoc {
                    path: f.path,
                    content_type: f.content_type,
                    selected: f.selected,
                })
                .collect(),
        },
        BodySpec::Graphql { query, variables } => BodyDoc::Graphql { query, variables },
        BodySpec::Unsupported { kind } => BodyDoc::Unsupported { mode: kind },
        _ => return Err(unsupported("body mode")),
    })
}

pub(crate) fn auth(a: AuthSpec) -> Result<AuthDoc, String> {
    Ok(match a {
        AuthSpec::None => AuthDoc::None,
        AuthSpec::Inherit => AuthDoc::Inherit,
        AuthSpec::Basic { username, password } => AuthDoc::Basic { username, password },
        AuthSpec::Bearer { token } => AuthDoc::Bearer { token },
        AuthSpec::ApiKey {
            key,
            value,
            placement,
        } => AuthDoc::Apikey {
            key,
            value,
            placement: match placement {
                ApiKeyPlacement::Header => PlacementDoc::Header,
                ApiKeyPlacement::Query => PlacementDoc::Query,
                _ => return Err(unsupported("API key placement")),
            },
        },
        AuthSpec::Digest { username, password } => AuthDoc::Digest { username, password },
        AuthSpec::AwsV4 {
            access_key_id,
            secret_access_key,
            session_token,
            service,
            region,
            profile_name,
        } => AuthDoc::Awsv4 {
            access_key_id,
            secret_access_key,
            session_token,
            service,
            region,
            profile_name,
        },
        AuthSpec::OAuth1(o) => AuthDoc::Oauth1 {
            private_key: match o.private_key {
                None => None,
                Some(k) => Some(Oauth1KeyDoc {
                    kind: match k.kind {
                        OAuth1KeyKind::Text => KeyKindDoc::Text,
                        OAuth1KeyKind::File => KeyKindDoc::File,
                        _ => return Err(unsupported("OAuth 1.0 key kind")),
                    },
                    value: k.value,
                }),
            },
            signature_method: o.signature_method.as_str().to_owned(),
            placement: o.placement.as_str().to_owned(),
            consumer_key: o.consumer_key,
            consumer_secret: o.consumer_secret,
            access_token: o.access_token,
            access_token_secret: o.access_token_secret,
            callback_url: o.callback_url,
            verifier: o.verifier,
            timestamp: o.timestamp,
            nonce: o.nonce,
            version: o.version,
            realm: o.realm,
            include_body_hash: o.include_body_hash,
        },
        AuthSpec::Wsse { username, password } => AuthDoc::Wsse { username, password },
        AuthSpec::EdgeGrid(e) => AuthDoc::Edgegrid {
            access_token: e.access_token,
            client_token: e.client_token,
            client_secret: e.client_secret,
            nonce: e.nonce,
            timestamp: e.timestamp,
            base_url: e.base_url,
            headers_to_sign: e.headers_to_sign,
            max_body_size: e.max_body_size.map(MaxBodySize),
        },
        AuthSpec::OAuth2(o) => AuthDoc::Oauth2(oauth2(o)?),
        AuthSpec::Ntlm {
            username,
            password,
            domain,
        } => AuthDoc::Ntlm {
            username,
            password,
            domain,
        },
        AuthSpec::Unsupported { mode } => AuthDoc::Unsupported { mode },
        _ => return Err(unsupported("auth mode")),
    })
}

fn oauth2(o: OAuth2Spec) -> Result<Oauth2Doc, String> {
    Ok(Oauth2Doc {
        flow: match o.flow {
            OAuth2Flow::ClientCredentials => Oauth2FlowDoc::ClientCredentials,
            OAuth2Flow::Password => Oauth2FlowDoc::ResourceOwnerPasswordCredentials,
            OAuth2Flow::AuthorizationCode => Oauth2FlowDoc::AuthorizationCode,
            OAuth2Flow::Implicit => Oauth2FlowDoc::Implicit,
            _ => return Err(unsupported("OAuth 2.0 flow")),
        },
        client_auth: match o.client_auth {
            None => None,
            Some(ClientAuth::Body) => Some("body".to_owned()),
            Some(ClientAuth::BasicHeader) => Some("basic_auth_header".to_owned()),
            Some(ClientAuth::Other(other)) => Some(other),
            Some(_) => return Err(unsupported("OAuth 2.0 client auth")),
        },
        token_placement: match o.token_placement {
            TokenPlacement::Header { prefix } => TokenPlacementDoc::Header { prefix },
            TokenPlacement::Query { key } => TokenPlacementDoc::Query { key },
            TokenPlacement::Unset => TokenPlacementDoc::Unset,
            _ => return Err(unsupported("OAuth 2.0 token placement")),
        },
        token_source: match o.token_source {
            TokenSource::AccessToken => "access_token".to_owned(),
            TokenSource::IdToken => "id_token".to_owned(),
            TokenSource::Other(other) => other,
            _ => return Err(unsupported("OAuth 2.0 token source")),
        },
        params: o
            .params
            .into_iter()
            .map(|p| {
                Ok(Oauth2ParamDoc {
                    stage: match p.stage {
                        OAuth2Stage::Authorization => Oauth2StageDoc::Authorization,
                        OAuth2Stage::Token => Oauth2StageDoc::Token,
                        OAuth2Stage::Refresh => Oauth2StageDoc::Refresh,
                        _ => return Err(unsupported("OAuth 2.0 parameter stage")),
                    },
                    placement: match p.placement {
                        OAuth2ParamPlacement::Header => Oauth2ParamPlacementDoc::Header,
                        OAuth2ParamPlacement::Query => Oauth2ParamPlacementDoc::Query,
                        OAuth2ParamPlacement::Body => Oauth2ParamPlacementDoc::Body,
                        _ => return Err(unsupported("OAuth 2.0 parameter placement")),
                    },
                    name: p.name,
                    value: p.value,
                })
            })
            .collect::<Result<_, String>>()?,
        authorization_url: o.authorization_url,
        access_token_url: o.access_token_url,
        refresh_token_url: o.refresh_token_url,
        callback_url: o.callback_url,
        client_id: o.client_id,
        client_secret: o.client_secret,
        username: o.username,
        password: o.password,
        scope: o.scope,
        state: o.state,
        pkce: o.pkce,
        token_id: o.token_id,
        auto_fetch: o.auto_fetch,
        auto_refresh: o.auto_refresh,
    })
}

pub(crate) fn settings(s: RequestSettings) -> Result<SettingsDoc, String> {
    let timeout = match s.timeout {
        Timeout::Inherit => TimeoutDoc::Inherit,
        Timeout::Unlimited => TimeoutDoc::Unlimited,
        Timeout::Millis(ms) => TimeoutDoc::Millis { ms },
        _ => return Err(unsupported("timeout")),
    };
    Ok(SettingsDoc {
        encode_url: s.encode_url,
        timeout,
        follow_redirects: s.follow_redirects,
        max_redirects: s.max_redirects,
        forward_authorization_header: s.forward_authorization_header,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_timeout_doc_says_inherit_has_no_limit_outside_the_app() {
        let schema = serde_json::to_value(schemars::schema_for!(TimeoutDoc)).unwrap();
        let text = schema["description"].as_str().expect("described");
        assert!(text.contains("no limit"), "{text}");
        assert!(!text.contains("collection's timeout"), "{text}");
    }

    #[test]
    fn oauth2_auth_reads_as_its_own_mode() {
        use apinata_core::{OAuth2Flow, OAuth2Spec};
        let doc = auth(AuthSpec::OAuth2(OAuth2Spec::new(
            OAuth2Flow::ClientCredentials,
        )))
        .unwrap();
        let json = serde_json::to_value(doc).unwrap();
        assert_eq!(json["type"], "oauth2");
        assert_eq!(json["flow"], "client_credentials");
        assert_eq!(
            json["token_placement"],
            serde_json::json!({"type": "header", "prefix": "Bearer"})
        );
    }
}
