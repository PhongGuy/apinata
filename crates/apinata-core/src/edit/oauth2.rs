//! OAuth 2.0 auth as the app edits it: the typed spec and its reader.

use super::{Redacted, text};
use crate::js;
use std::fmt;
use yaml_serde::{Mapping, Value};

/// The OAuth 2.0 grant a request uses to get its token.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum OAuth2Flow {
    /// The client credentials grant.
    ClientCredentials,
    /// The resource owner password credentials grant.
    Password,
    /// The authorization code grant, optionally with PKCE.
    AuthorizationCode,
    /// The implicit grant.
    Implicit,
}

impl OAuth2Flow {
    /// The flow as written in the file's `flow` key.
    fn from_name(name: &str) -> Option<OAuth2Flow> {
        Some(match name {
            "client_credentials" => OAuth2Flow::ClientCredentials,
            "resource_owner_password_credentials" => OAuth2Flow::Password,
            "authorization_code" => OAuth2Flow::AuthorizationCode,
            "implicit" => OAuth2Flow::Implicit,
            _ => return None,
        })
    }

    /// The flow's `flow` key value.
    pub(crate) fn name(self) -> &'static str {
        match self {
            OAuth2Flow::ClientCredentials => "client_credentials",
            OAuth2Flow::Password => "resource_owner_password_credentials",
            OAuth2Flow::AuthorizationCode => "authorization_code",
            OAuth2Flow::Implicit => "implicit",
        }
    }

    /// The parameter stages this flow sends, in writer order.
    pub fn stages(self) -> &'static [OAuth2Stage] {
        use OAuth2Stage::*;
        match self {
            OAuth2Flow::ClientCredentials | OAuth2Flow::Password => &[Token, Refresh],
            OAuth2Flow::AuthorizationCode => &[Authorization, Token, Refresh],
            OAuth2Flow::Implicit => &[Authorization],
        }
    }
}

/// How the client authenticates to the token endpoint.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ClientAuth {
    /// The client id and secret in the request body.
    Body,
    /// The client id and secret as a basic `Authorization` header.
    BasicHeader,
    /// A placement this editor does not model, kept as written.
    Other(String),
}

/// Where the fetched token is sent on the request.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum TokenPlacement {
    /// In the `Authorization` header after `prefix`.
    Header {
        /// The scheme written before the token, such as `Bearer`.
        prefix: String,
    },
    /// As the query parameter `key`.
    Query {
        /// The parameter name.
        key: String,
    },
    /// Not set in the file.
    Unset,
}

/// Which token of the token response is sent.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum TokenSource {
    /// The access token.
    AccessToken,
    /// The ID token.
    IdToken,
    /// A source this editor does not model, kept as written.
    Other(String),
}

/// The request of the flow an additional parameter is sent with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum OAuth2Stage {
    /// The authorization request.
    Authorization,
    /// The access token request.
    Token,
    /// The refresh token request.
    Refresh,
}

impl OAuth2Stage {
    /// The stage's key under `additionalParameters`.
    fn key(self) -> &'static str {
        match self {
            OAuth2Stage::Authorization => "authorizationRequest",
            OAuth2Stage::Token => "accessTokenRequest",
            OAuth2Stage::Refresh => "refreshTokenRequest",
        }
    }
}

/// Where an additional parameter is sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum OAuth2ParamPlacement {
    /// As a header.
    Header,
    /// As a query parameter.
    Query,
    /// In the form body.
    Body,
}

/// An extra parameter sent with one of the flow's requests.
#[derive(Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct OAuth2Param {
    /// The request it is sent with.
    pub stage: OAuth2Stage,
    /// The parameter name.
    pub name: String,
    /// The value, possibly with `{{variables}}`.
    pub value: String,
    /// Where it is sent.
    pub placement: OAuth2ParamPlacement,
}

impl OAuth2Param {
    /// A parameter for `stage`.
    pub fn new(
        stage: OAuth2Stage,
        name: impl Into<String>,
        value: impl Into<String>,
        placement: OAuth2ParamPlacement,
    ) -> OAuth2Param {
        OAuth2Param {
            stage,
            name: name.into(),
            value: value.into(),
            placement,
        }
    }
}

/// Shows the value's size only, since parameters often carry credentials.
impl fmt::Debug for OAuth2Param {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OAuth2Param")
            .field("stage", &self.stage)
            .field("name", &self.name)
            .field("value", &Redacted(self.value.len()))
            .field("placement", &self.placement)
            .finish()
    }
}

/// An `oauth2` auth: every flow's fields, of which only the current flow's are written.
#[derive(Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct OAuth2Spec {
    /// The grant.
    pub flow: OAuth2Flow,
    /// The authorization endpoint.
    pub authorization_url: String,
    /// The token endpoint.
    pub access_token_url: String,
    /// The refresh endpoint; empty means the token endpoint.
    pub refresh_token_url: String,
    /// The redirect URI.
    pub callback_url: String,
    /// The client id.
    pub client_id: String,
    /// The client secret.
    pub client_secret: String,
    /// How the client authenticates; `None` when the file does not say.
    pub client_auth: Option<ClientAuth>,
    /// The resource owner's user name.
    pub username: String,
    /// The resource owner's password.
    pub password: String,
    /// The requested scope.
    pub scope: String,
    /// The `state` parameter.
    pub state: String,
    /// Whether the authorization code flow uses PKCE.
    pub pkce: bool,
    /// The name the fetched token is stored under.
    pub token_id: String,
    /// Where the token is sent.
    pub token_placement: TokenPlacement,
    /// Which token is sent.
    pub token_source: TokenSource,
    /// Whether a missing token is fetched before sending.
    pub auto_fetch: bool,
    /// Whether an expired token is refreshed before sending.
    pub auto_refresh: bool,
    /// Extra parameters for the flow's requests.
    pub params: Vec<OAuth2Param>,
}

impl OAuth2Spec {
    /// A fresh oauth2 auth with the values a new one starts with.
    pub fn new(flow: OAuth2Flow) -> OAuth2Spec {
        OAuth2Spec {
            client_auth: Some(ClientAuth::Body),
            token_id: "credentials".to_owned(),
            token_placement: TokenPlacement::Header {
                prefix: "Bearer".to_owned(),
            },
            ..OAuth2Spec::read_defaults(flow)
        }
    }

    /// What the reader assumes for every key the file leaves out.
    fn read_defaults(flow: OAuth2Flow) -> OAuth2Spec {
        OAuth2Spec {
            flow,
            authorization_url: String::new(),
            access_token_url: String::new(),
            refresh_token_url: String::new(),
            callback_url: String::new(),
            client_id: String::new(),
            client_secret: String::new(),
            client_auth: None,
            username: String::new(),
            password: String::new(),
            scope: String::new(),
            state: String::new(),
            pkce: false,
            token_id: String::new(),
            token_placement: TokenPlacement::Unset,
            token_source: TokenSource::AccessToken,
            auto_fetch: true,
            auto_refresh: false,
            params: Vec::new(),
        }
    }

    /// Read an auth mapping whose `type` is `oauth2`; `None` when `flow` is missing or unknown.
    /// Only the flow's own keys are read, as the reference reader's per-flow switch does.
    pub(crate) fn from_yaml(map: &Value) -> Option<OAuth2Spec> {
        let flow = OAuth2Flow::from_name(map.get("flow").and_then(js::yaml_to_string)?.as_str())?;
        let mut o = OAuth2Spec::read_defaults(flow);
        let credentials = map.get("credentials");
        let cred = |key| credentials.map(|c| text(c, key)).unwrap_or_default();
        let has_endpoint = flow != OAuth2Flow::Implicit;
        let has_authorize = matches!(flow, OAuth2Flow::AuthorizationCode | OAuth2Flow::Implicit);

        o.client_id = cred("clientId");
        o.scope = text(map, "scope");
        if has_endpoint {
            o.access_token_url = text(map, "accessTokenUrl");
            o.refresh_token_url = text(map, "refreshTokenUrl");
            o.client_secret = cred("clientSecret");
            o.client_auth = match cred("placement").as_str() {
                "" => None,
                "body" => Some(ClientAuth::Body),
                "basic_auth_header" => Some(ClientAuth::BasicHeader),
                other => Some(ClientAuth::Other(other.to_owned())),
            };
        }
        if has_authorize {
            o.authorization_url = text(map, "authorizationUrl");
            o.callback_url = text(map, "callbackUrl");
            o.state = text(map, "state");
        }
        if flow == OAuth2Flow::Password
            && let Some(owner) = map.get("resourceOwner")
        {
            o.username = text(owner, "username");
            o.password = text(owner, "password");
        }
        // Any present pkce value turns it on unless its `disabled` is truthy.
        if flow == OAuth2Flow::AuthorizationCode
            && let Some(pkce) = map.get("pkce").filter(|p| !p.is_null())
        {
            o.pkce = !pkce.get("disabled").is_some_and(js_truthy);
        }
        if let Some(config) = map.get("tokenConfig") {
            o.token_id = text(config, "id");
            o.token_source = match text(config, "source").as_str() {
                "" | "access_token" => TokenSource::AccessToken,
                "id_token" => TokenSource::IdToken,
                other => TokenSource::Other(other.to_owned()),
            };
            o.token_placement = token_placement(config.get("placement"));
        }
        if let Some(settings) = map.get("settings") {
            let flag = |key| settings.get(key).and_then(Value::as_bool);
            o.auto_fetch = flag("autoFetchToken").unwrap_or(o.auto_fetch);
            o.auto_refresh = flag("autoRefreshToken").unwrap_or(o.auto_refresh);
        }
        o.params = read_params(map.get("additionalParameters"), flow);
        Some(o)
    }
}

impl OAuth2Spec {
    /// The spec as reading back its written form gives: other flows' fields dropped.
    // `to_yaml` always writes the spec's own flow, which `from_yaml` reads back.
    #[allow(clippy::expect_used)]
    pub fn on_disk(&self) -> OAuth2Spec {
        OAuth2Spec::from_yaml(&self.to_yaml()).expect("the projection names a known flow")
    }

    /// The whole auth mapping as the reference writer projects it: `type`, `flow`, then the
    /// flow's keys in its order, each blank string and empty sub-mapping left out.
    pub(crate) fn to_yaml(&self) -> Value {
        use OAuth2Flow::*;
        let flow = self.flow;
        let mut map = Mapping::new();
        put(&mut map, "type", Some(Value::from("oauth2")));
        put(&mut map, "flow", Some(Value::from(flow.name())));
        if matches!(flow, AuthorizationCode | Implicit) {
            put(
                &mut map,
                "authorizationUrl",
                filled(&self.authorization_url),
            );
        }
        if flow != Implicit {
            put(&mut map, "accessTokenUrl", filled(&self.access_token_url));
            put(&mut map, "refreshTokenUrl", filled(&self.refresh_token_url));
        }
        if matches!(flow, AuthorizationCode | Implicit) {
            put(&mut map, "callbackUrl", filled(&self.callback_url));
        }
        put(&mut map, "credentials", self.credentials());
        if flow == Password {
            let mut owner = Mapping::new();
            put(&mut owner, "username", filled(&self.username));
            put(&mut owner, "password", filled(&self.password));
            put(&mut map, "resourceOwner", non_empty(owner));
        }
        // The code flow writes its parameters before scope and state; the others after.
        let params = self.params_yaml();
        match flow {
            ClientCredentials | Password => {
                put(&mut map, "scope", filled(&self.scope));
                put(&mut map, "additionalParameters", params);
            }
            AuthorizationCode => {
                put(&mut map, "additionalParameters", params);
                put(&mut map, "scope", filled(&self.scope));
                put(&mut map, "state", filled(&self.state));
                let mut pkce = Mapping::new();
                if !self.pkce {
                    pkce.insert(Value::from("disabled"), Value::Bool(true));
                }
                put(&mut map, "pkce", Some(Value::Mapping(pkce)));
            }
            Implicit => {
                put(&mut map, "scope", filled(&self.scope));
                put(&mut map, "state", filled(&self.state));
                put(&mut map, "additionalParameters", params);
            }
        }
        put(&mut map, "tokenConfig", Some(self.token_config()));
        let mut settings = Mapping::new();
        settings.insert(Value::from("autoFetchToken"), Value::Bool(self.auto_fetch));
        settings.insert(
            Value::from("autoRefreshToken"),
            Value::Bool(self.auto_refresh),
        );
        put(&mut map, "settings", Some(Value::Mapping(settings)));
        Value::Mapping(map)
    }

    /// `credentials`: the implicit flow sends only the client id.
    fn credentials(&self) -> Option<Value> {
        let mut c = Mapping::new();
        put(&mut c, "clientId", filled(&self.client_id));
        if self.flow != OAuth2Flow::Implicit {
            put(&mut c, "clientSecret", filled(&self.client_secret));
            let placement = match &self.client_auth {
                None => "",
                Some(ClientAuth::Body) => "body",
                Some(ClientAuth::BasicHeader) => "basic_auth_header",
                Some(ClientAuth::Other(other)) => other,
            };
            put(&mut c, "placement", filled(placement));
        }
        non_empty(c)
    }

    /// `tokenConfig`: an unset placement is written as an empty header prefix.
    fn token_config(&self) -> Value {
        let mut c = Mapping::new();
        put(&mut c, "id", filled(&self.token_id));
        let (side, text) = match &self.token_placement {
            TokenPlacement::Header { prefix } => ("header", prefix.as_str()),
            TokenPlacement::Query { key } => ("query", key.as_str()),
            TokenPlacement::Unset => ("header", ""),
        };
        let mut placement = Mapping::new();
        placement.insert(Value::from(side), Value::from(text));
        c.insert(Value::from("placement"), Value::Mapping(placement));
        let source = match &self.token_source {
            TokenSource::AccessToken => "access_token",
            TokenSource::IdToken => "id_token",
            TokenSource::Other(s) if s.is_empty() => "access_token",
            TokenSource::Other(s) => s,
        };
        c.insert(Value::from("source"), Value::from(source));
        Value::Mapping(c)
    }

    /// `additionalParameters`: the flow's stages in order, each row's name trimmed and a
    /// nameless row dropped.
    fn params_yaml(&self) -> Option<Value> {
        let mut out = Mapping::new();
        for &stage in self.flow.stages() {
            let rows: Vec<Value> = self
                .params
                .iter()
                .filter(|p| p.stage == stage && filled(&p.name).is_some())
                .map(|p| {
                    let mut row = Mapping::new();
                    row.insert(Value::from("name"), Value::from(js::js_trim(&p.name)));
                    let placement = match p.placement {
                        OAuth2ParamPlacement::Header => "header",
                        OAuth2ParamPlacement::Query => "query",
                        OAuth2ParamPlacement::Body => "body",
                    };
                    row.insert(Value::from("placement"), Value::from(placement));
                    put(&mut row, "value", filled(&p.value));
                    Value::Mapping(row)
                })
                .collect();
            if !rows.is_empty() {
                out.insert(Value::from(stage.key()), Value::Sequence(rows));
            }
        }
        non_empty(out)
    }
}

/// A string the reference writes: one with a non-whitespace character, kept untrimmed.
fn filled(text: &str) -> Option<Value> {
    (!js::js_trim(text).is_empty()).then(|| Value::from(text))
}

/// A sub-mapping the reference writes: one with at least one key.
fn non_empty(map: Mapping) -> Option<Value> {
    (!map.is_empty()).then_some(Value::Mapping(map))
}

/// Append `key` when there is a value to write.
fn put(map: &mut Mapping, key: &str, value: Option<Value>) {
    if let Some(value) = value {
        map.insert(Value::from(key), value);
    }
}

/// Shows the client secret, username, password and parameter values by size only.
impl fmt::Debug for OAuth2Spec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OAuth2Spec")
            .field("flow", &self.flow)
            .field("authorization_url", &self.authorization_url)
            .field("access_token_url", &self.access_token_url)
            .field("refresh_token_url", &self.refresh_token_url)
            .field("callback_url", &self.callback_url)
            .field("client_id", &self.client_id)
            .field("client_secret", &Redacted(self.client_secret.len()))
            .field("client_auth", &self.client_auth)
            .field("username", &Redacted(self.username.len()))
            .field("password", &Redacted(self.password.len()))
            .field("scope", &self.scope)
            .field("state", &self.state)
            .field("pkce", &self.pkce)
            .field("token_id", &self.token_id)
            .field("token_placement", &self.token_placement)
            .field("token_source", &self.token_source)
            .field("auto_fetch", &self.auto_fetch)
            .field("auto_refresh", &self.auto_refresh)
            .field("params", &self.params)
            .finish()
    }
}

/// `tokenConfig.placement`: a `header` key wins over `query`; anything else is unset.
fn token_placement(placement: Option<&Value>) -> TokenPlacement {
    let Some(map @ Value::Mapping(_)) = placement else {
        return TokenPlacement::Unset;
    };
    if map.get("header").is_some() {
        TokenPlacement::Header {
            prefix: text(map, "header"),
        }
    } else if map.get("query").is_some() {
        TokenPlacement::Query {
            key: text(map, "query"),
        }
    } else {
        TokenPlacement::Unset
    }
}

/// The parameters of the stages `flow` sends, in stage order; every row is kept, and an
/// unknown placement reads as a header.
fn read_params(params: Option<&Value>, flow: OAuth2Flow) -> Vec<OAuth2Param> {
    let Some(params) = params else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for &stage in flow.stages() {
        let Some(Value::Sequence(rows)) = params.get(stage.key()) else {
            continue;
        };
        out.extend(rows.iter().map(|row| {
            let placement = match text(row, "placement").as_str() {
                "query" => OAuth2ParamPlacement::Query,
                "body" => OAuth2ParamPlacement::Body,
                _ => OAuth2ParamPlacement::Header,
            };
            OAuth2Param::new(stage, text(row, "name"), text(row, "value"), placement)
        }));
    }
    out
}

/// JavaScript truthiness of a YAML value.
fn js_truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|f| f != 0.0 && !f.is_nan()),
        Value::String(s) => !s.is_empty(),
        Value::Tagged(tagged) => js_truthy(&tagged.value),
        Value::Sequence(_) | Value::Mapping(_) => true,
    }
}
