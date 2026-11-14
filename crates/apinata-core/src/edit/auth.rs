//! A request's `http.auth` as the app edits it.

use super::edgegrid::EdgeGridSpec;
use super::oauth1::{OAuth1Placement, OAuth1Spec};
use super::oauth2::{OAuth2Spec, TokenPlacement};
use super::{Redacted, text};
use crate::document::Document;
use crate::js;
use std::fmt;
use yaml_serde::{Mapping, Value};

/// Where an API key is sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ApiKeyPlacement {
    /// As a request header.
    Header,
    /// As a query parameter.
    Query,
}

/// A request's authentication.
#[derive(Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum AuthSpec {
    /// No authentication.
    None,
    /// Use the folder's or collection's authentication.
    Inherit,
    /// HTTP basic authentication.
    Basic {
        /// The user name.
        username: String,
        /// The password.
        password: String,
    },
    /// A bearer token.
    Bearer {
        /// The token.
        token: String,
    },
    /// An API key.
    ApiKey {
        /// The header or parameter name.
        key: String,
        /// The key's value.
        value: String,
        /// Where the key is sent.
        placement: ApiKeyPlacement,
    },
    /// OAuth 1.0.
    OAuth1(OAuth1Spec),
    /// OAuth 2.0.
    OAuth2(OAuth2Spec),
    /// HTTP digest authentication.
    Digest {
        /// The user name.
        username: String,
        /// The password.
        password: String,
    },
    /// AWS Signature Version 4.
    AwsV4 {
        /// The access key id.
        access_key_id: String,
        /// The secret access key.
        secret_access_key: String,
        /// The session token, empty for long-term keys.
        session_token: String,
        /// The signing service, empty to infer it from the host.
        service: String,
        /// The signing region, empty to infer it from the host.
        region: String,
        /// The shared-files profile that supplies keys left blank.
        profile_name: String,
    },
    /// WSSE UsernameToken.
    Wsse {
        /// The user name.
        username: String,
        /// The password.
        password: String,
    },
    /// NTLM challenge-response.
    Ntlm {
        /// The user name.
        username: String,
        /// The password.
        password: String,
        /// The domain, empty for none.
        domain: String,
    },
    /// Akamai EdgeGrid.
    EdgeGrid(EdgeGridSpec),
    /// A mode this editor does not model; it is shown but never written.
    Unsupported {
        /// The mode as written in the file, or `unknown`.
        mode: String,
    },
}

/// Shows modes, key names and placement only, since the rest is credentials.
impl fmt::Debug for AuthSpec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let hidden = |s: &String| Redacted(s.len());
        match self {
            AuthSpec::None => f.write_str("None"),
            AuthSpec::Inherit => f.write_str("Inherit"),
            AuthSpec::Basic { username, password } => f
                .debug_struct("Basic")
                .field("username", &hidden(username))
                .field("password", &hidden(password))
                .finish(),
            AuthSpec::Bearer { token } => f
                .debug_struct("Bearer")
                .field("token", &hidden(token))
                .finish(),
            AuthSpec::ApiKey {
                key,
                value,
                placement,
            } => f
                .debug_struct("ApiKey")
                .field("key", key)
                .field("value", &hidden(value))
                .field("placement", placement)
                .finish(),
            AuthSpec::OAuth1(spec) => f.debug_tuple("OAuth1").field(spec).finish(),
            AuthSpec::OAuth2(spec) => f.debug_tuple("OAuth2").field(spec).finish(),
            AuthSpec::Digest { username, password } => f
                .debug_struct("Digest")
                .field("username", username)
                .field("password", &hidden(password))
                .finish(),
            AuthSpec::AwsV4 {
                access_key_id,
                secret_access_key,
                session_token,
                service,
                region,
                profile_name,
            } => f
                .debug_struct("AwsV4")
                .field("access_key_id", &hidden(access_key_id))
                .field("secret_access_key", &hidden(secret_access_key))
                .field("session_token", &hidden(session_token))
                .field("service", service)
                .field("region", region)
                .field("profile_name", profile_name)
                .finish(),
            AuthSpec::Wsse { username, password } => f
                .debug_struct("Wsse")
                .field("username", username)
                .field("password", &hidden(password))
                .finish(),
            AuthSpec::Ntlm {
                username,
                password,
                domain,
            } => f
                .debug_struct("Ntlm")
                .field("username", username)
                .field("password", &hidden(password))
                .field("domain", domain)
                .finish(),
            AuthSpec::EdgeGrid(spec) => f.debug_tuple("EdgeGrid").field(spec).finish(),
            AuthSpec::Unsupported { mode } => {
                f.debug_struct("Unsupported").field("mode", mode).finish()
            }
        }
    }
}

/// Key order inside a basic auth mapping.
pub(crate) const BASIC_ORDER: &[&str] = &["type", "username", "password"];
/// Key order inside a bearer auth mapping.
pub(crate) const BEARER_ORDER: &[&str] = &["type", "token"];
/// Key order inside an API key auth mapping.
pub(crate) const APIKEY_ORDER: &[&str] = &["type", "key", "value", "placement"];
/// Key order inside a digest auth mapping.
pub(crate) const DIGEST_ORDER: &[&str] = &["type", "username", "password"];
/// Key order inside a WSSE auth mapping.
pub(crate) const WSSE_ORDER: &[&str] = &["type", "username", "password"];
/// Key order inside an NTLM auth mapping.
pub(crate) const NTLM_ORDER: &[&str] = &["type", "username", "password", "domain"];
/// Key order inside an AWS SigV4 auth mapping.
pub(crate) const AWSV4_ORDER: &[&str] = &[
    "type",
    "accessKeyId",
    "secretAccessKey",
    "sessionToken",
    "service",
    "region",
    "profileName",
];

/// Paths, relative to an auth mapping in any mode, whose scalar values are credentials.
pub(crate) const CREDENTIAL_PATHS: &[&[&str]] = &[
    &["password"],
    &["token"],
    &["value"],
    &["consumerSecret"],
    &["accessToken"],
    &["accessTokenSecret"],
    &["privateKey"],
    &["privateKey", "value"],
    &["clientSecret"],
    &["clientToken"],
    &["credentials", "clientSecret"],
    &["resourceOwner", "password"],
    &["secretAccessKey"],
    &["sessionToken"],
];
/// The OAuth 2 key holding each stage's extra parameter rows; a row's `value` is a credential.
pub(crate) const PARAM_ROWS: &str = "additionalParameters";

impl AuthSpec {
    /// The header this auth writes when the request is sent, replacing a same-named header the
    /// request sets; `None` for an auth that writes no header or one not modelled here.
    /// An API key's header is its key as written, before variables are resolved.
    pub fn sets_header(&self) -> Option<&str> {
        const AUTHORIZATION: &str = "Authorization";
        match self {
            AuthSpec::Basic { .. }
            | AuthSpec::Bearer { .. }
            | AuthSpec::AwsV4 { .. }
            | AuthSpec::EdgeGrid(_) => Some(AUTHORIZATION),
            // Sending tests the raw key and value: either one empty sets no header.
            AuthSpec::ApiKey {
                key,
                value,
                placement: ApiKeyPlacement::Header,
            } if !key.is_empty() && !value.is_empty() => Some(key),
            AuthSpec::OAuth1(spec) if spec.placement == OAuth1Placement::Header => {
                Some(AUTHORIZATION)
            }
            AuthSpec::Wsse { .. } => Some("X-WSSE"),
            AuthSpec::OAuth2(spec) => match spec.token_placement {
                TokenPlacement::Query { .. } => None,
                _ => Some(AUTHORIZATION),
            },
            _ => None,
        }
    }

    pub(crate) fn from_yaml(value: Option<&Value>) -> AuthSpec {
        match value {
            None | Some(Value::Null) => AuthSpec::None,
            Some(map @ Value::Mapping(_)) => match text(map, "type").as_str() {
                "basic" => AuthSpec::Basic {
                    username: text(map, "username"),
                    password: text(map, "password"),
                },
                "bearer" => AuthSpec::Bearer {
                    token: text(map, "token"),
                },
                "apikey" => AuthSpec::ApiKey {
                    key: text(map, "key"),
                    value: text(map, "value"),
                    placement: if text(map, "placement") == "query" {
                        ApiKeyPlacement::Query
                    } else {
                        ApiKeyPlacement::Header
                    },
                },
                "digest" => AuthSpec::Digest {
                    username: text(map, "username"),
                    password: text(map, "password"),
                },
                "awsv4" => AuthSpec::AwsV4 {
                    access_key_id: text(map, "accessKeyId"),
                    secret_access_key: text(map, "secretAccessKey"),
                    session_token: text(map, "sessionToken"),
                    service: text(map, "service"),
                    region: text(map, "region"),
                    profile_name: text(map, "profileName"),
                },
                "wsse" => AuthSpec::Wsse {
                    username: text(map, "username"),
                    password: text(map, "password"),
                },
                "ntlm" => AuthSpec::Ntlm {
                    username: text(map, "username"),
                    password: text(map, "password"),
                    domain: text(map, "domain"),
                },
                "akamai-edgegrid" => AuthSpec::EdgeGrid(EdgeGridSpec::from_yaml(map)),
                "oauth1" => AuthSpec::OAuth1(OAuth1Spec::from_yaml(map)),
                "oauth2" => OAuth2Spec::from_yaml(map).map_or_else(
                    || AuthSpec::Unsupported {
                        mode: "oauth2".to_owned(),
                    },
                    AuthSpec::OAuth2,
                ),
                "" => AuthSpec::Unsupported {
                    mode: "unknown".to_owned(),
                },
                other => AuthSpec::Unsupported {
                    mode: other.to_owned(),
                },
            },
            Some(scalar) => match js::yaml_to_string(scalar).as_deref() {
                Some("none") => AuthSpec::None,
                Some("inherit") => AuthSpec::Inherit,
                other => AuthSpec::Unsupported {
                    mode: other.unwrap_or("unknown").to_owned(),
                },
            },
        }
    }

    /// The mode name as written in `type`; `None` and `Unsupported` are never written.
    pub(crate) fn mode(&self) -> Option<&'static str> {
        match self {
            AuthSpec::Basic { .. } => Some("basic"),
            AuthSpec::Bearer { .. } => Some("bearer"),
            AuthSpec::ApiKey { .. } => Some("apikey"),
            AuthSpec::Digest { .. } => Some("digest"),
            AuthSpec::AwsV4 { .. } => Some("awsv4"),
            AuthSpec::Wsse { .. } => Some("wsse"),
            AuthSpec::Ntlm { .. } => Some("ntlm"),
            AuthSpec::EdgeGrid(_) => Some("akamai-edgegrid"),
            _ => None,
        }
    }

    /// The mode's keys in writer order, after `type`.
    pub(crate) fn entries(&self) -> Vec<(&'static str, Value)> {
        let v = |s: &str| Value::from(s);
        match self {
            AuthSpec::Basic { username, password } => {
                vec![("username", v(username)), ("password", v(password))]
            }
            AuthSpec::Bearer { token } => vec![("token", v(token))],
            AuthSpec::ApiKey {
                key,
                value,
                placement,
            } => vec![
                ("key", v(key)),
                ("value", v(value)),
                (
                    "placement",
                    v(match placement {
                        ApiKeyPlacement::Header => "header",
                        ApiKeyPlacement::Query => "query",
                    }),
                ),
            ],
            AuthSpec::Digest { username, password } | AuthSpec::Wsse { username, password } => {
                vec![("username", v(username)), ("password", v(password))]
            }
            AuthSpec::Ntlm {
                username,
                password,
                domain,
            } => vec![
                ("username", v(username)),
                ("password", v(password)),
                ("domain", v(domain)),
            ],
            AuthSpec::AwsV4 {
                access_key_id,
                secret_access_key,
                session_token,
                service,
                region,
                profile_name,
            } => vec![
                ("accessKeyId", v(access_key_id)),
                ("secretAccessKey", v(secret_access_key)),
                ("sessionToken", v(session_token)),
                ("service", v(service)),
                ("region", v(region)),
                ("profileName", v(profile_name)),
            ],
            _ => Vec::new(),
        }
    }

    /// The key order of this mode's mapping.
    pub(crate) fn order(&self) -> &'static [&'static str] {
        match self {
            AuthSpec::Basic { .. } => BASIC_ORDER,
            AuthSpec::Bearer { .. } => BEARER_ORDER,
            AuthSpec::Digest { .. } => DIGEST_ORDER,
            AuthSpec::AwsV4 { .. } => AWSV4_ORDER,
            AuthSpec::Wsse { .. } => WSSE_ORDER,
            AuthSpec::Ntlm { .. } => NTLM_ORDER,
            _ => APIKEY_ORDER,
        }
    }

    /// A fresh auth mapping: `type`, then the mode's keys.
    pub(crate) fn fresh_mapping(&self, mode: &str) -> Value {
        let mut map = Mapping::new();
        map.insert(Value::from("type"), Value::from(mode));
        for (key, value) in self.entries() {
            map.insert(Value::from(key), value);
        }
        Value::Mapping(map)
    }
}

/// Write `auth` at `keys`; `order[i]` is the writer order of the mapping at depth `i` above it.
/// `None` removes the key, `Inherit` writes `inherit` and `Unsupported` does nothing. The same
/// mode rewrites only its own keys, so others survive; another mode replaces the whole mapping.
/// OAuth 1.0 and 2.0, WSSE, NTLM and EdgeGrid always replace the whole mapping with their projection,
/// dropping unknown keys as the reference writer does; an unchanged projection is an equal value
/// and leaves the file clean.
pub(crate) fn write_auth(doc: &mut Document, keys: &[&str], order: &[&[&str]], auth: &AuthSpec) {
    match auth {
        AuthSpec::None => {
            doc.remove(keys);
        }
        AuthSpec::Inherit => doc.set(keys, Value::from("inherit"), order),
        AuthSpec::Unsupported { .. } => {}
        AuthSpec::OAuth1(spec) => doc.set(keys, spec.to_yaml(), order),
        AuthSpec::OAuth2(spec) => doc.set(keys, spec.to_yaml(), order),
        AuthSpec::Wsse { .. } => doc.set(keys, auth.fresh_mapping("wsse"), order),
        AuthSpec::Ntlm { .. } => doc.set(keys, auth.fresh_mapping("ntlm"), order),
        AuthSpec::EdgeGrid(spec) => doc.set(keys, spec.to_yaml(), order),
        _ => {
            let Some(mode) = auth.mode() else { return };
            let mut path = keys.to_vec();
            path.push("type");
            let same_mode = doc
                .get(&path)
                .and_then(js::yaml_to_string)
                .is_some_and(|current| current == mode);
            if !same_mode {
                doc.set(keys, auth.fresh_mapping(mode), order);
                return;
            }
            let mut nested = order.to_vec();
            nested.push(auth.order());
            for (key, value) in auth.entries() {
                // Swap the previous leaf (first `type`, then each earlier key) for this key.
                path.pop();
                path.push(key);
                doc.set(&path, value, &nested);
            }
        }
    }
}
