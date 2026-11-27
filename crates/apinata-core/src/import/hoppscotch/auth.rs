//! Auth objects and legacy auth strings to `AuthSpec`.

use super::model::{Auth, JwtMembers, Mapped};
use super::tree::Scope;
use crate::edit::{
    ApiKeyPlacement, AuthSpec, JwtAlgorithm, JwtPlacement, JwtSpec, OAuth1KeyKind, OAuth1PrivateKey,
};
use crate::import::{DocumentError, NoteKind};

/// A legacy request's credentials, read when its auth is a string.
#[derive(Default)]
pub(super) struct Legacy {
    pub(super) user: String,
    pub(super) password: String,
    pub(super) token: String,
}

/// `auth` as Apinata models it; missing is `Inherit`, which the caller adjusts for a root or
/// legacy request. Templates in its values are rewritten; what is lost gets a note.
pub(super) fn auth(
    auth: Option<Auth>,
    legacy: Legacy,
    scope: &mut Scope,
) -> Result<AuthSpec, DocumentError> {
    let (mapped, rest) = match auth {
        None => return Ok(AuthSpec::Inherit),
        Some(Auth::Off) => return Ok(AuthSpec::None),
        Some(Auth::Legacy(kind)) => return legacy_auth(&kind, legacy, scope),
        Some(Auth::Unsupported(kind)) => {
            let detail = format!("`{kind}` auth is not supported; imported as no auth");
            scope.note("auth", NoteKind::Converted, detail)?;
            return Ok(AuthSpec::None);
        }
        Some(Auth::Mapped(mapped, rest)) => (mapped, rest),
    };
    scope.rest(rest, || "auth.".to_owned())?;
    Ok(match mapped {
        Mapped::None => AuthSpec::None,
        Mapped::Inherit => AuthSpec::Inherit,
        Mapped::Basic { username, password } => AuthSpec::Basic {
            username: scope.value(username, || "auth.username".to_owned())?,
            password: scope.value(password, || "auth.password".to_owned())?,
        },
        Mapped::Bearer { token } => AuthSpec::Bearer {
            token: scope.value(token, || "auth.token".to_owned())?,
        },
        Mapped::ApiKey { key, value, add_to } => {
            let placement = match add_to.as_deref() {
                None | Some("HEADERS" | "Headers") => ApiKeyPlacement::Header,
                Some("QUERY_PARAMS" | "Query params") => ApiKeyPlacement::Query,
                Some(other) => {
                    let detail = format!("unknown placement `{other}`; sent as a header");
                    scope.note("auth.addTo", NoteKind::Converted, detail)?;
                    ApiKeyPlacement::Header
                }
            };
            AuthSpec::ApiKey {
                key,
                value: scope.value(value, || "auth.value".to_owned())?,
                placement,
            }
        }
        Mapped::Jwt(members) => AuthSpec::Jwt(jwt(*members, scope)?),
    })
}

/// A JWT auth with its templates rewritten. The default prefix `"Bearer "` loses its space,
/// which the sender adds; a blank algorithm is HS256 and a blank parameter name `token`.
fn jwt(m: JwtMembers, scope: &mut Scope) -> Result<JwtSpec, DocumentError> {
    let placement = match m.add_to.as_deref() {
        None | Some("HEADERS") => JwtPlacement::Header,
        Some("QUERY_PARAMS") => JwtPlacement::Query,
        Some(other) => {
            let detail = format!("unknown placement `{other}`; sent as a header");
            scope.note("auth.addTo", NoteKind::Converted, detail)?;
            JwtPlacement::Header
        }
    };
    let algorithm = scope.value(m.algorithm, || "auth.algorithm".to_owned())?;
    let private_key = scope.value(m.private_key, || "auth.privateKey".to_owned())?;
    let prefix = match m.header_prefix {
        Some(p) => scope.value(p, || "auth.headerPrefix".to_owned())?,
        None => "Bearer".to_owned(),
    };
    let param = scope.value(m.param_name, || "auth.paramName".to_owned())?;
    Ok(JwtSpec {
        algorithm: if algorithm.is_empty() {
            JwtAlgorithm::Hs256
        } else {
            JwtAlgorithm::parse(&algorithm)
        },
        secret: scope.value(m.secret, || "auth.secret".to_owned())?,
        secret_base64: m.secret_base64,
        private_key: (!private_key.is_empty())
            .then(|| OAuth1PrivateKey::new(OAuth1KeyKind::Text, private_key)),
        headers: scope.value(m.headers, || "auth.jwtHeaders".to_owned())?,
        payload: scope.value(m.payload, || "auth.payload".to_owned())?,
        expires_in: String::new(),
        placement,
        header_prefix: Some(prefix.trim().to_owned()),
        query_param_key: if param.is_empty() {
            "token".to_owned()
        } else {
            param
        },
    })
}

/// A version 0 auth string with the request's own credential members.
fn legacy_auth(kind: &str, legacy: Legacy, scope: &mut Scope) -> Result<AuthSpec, DocumentError> {
    Ok(match kind {
        "None" => AuthSpec::None,
        "Basic Auth" => AuthSpec::Basic {
            username: scope.value(legacy.user, || "httpUser".to_owned())?,
            password: scope.value(legacy.password, || "httpPassword".to_owned())?,
        },
        "Bearer Token" => AuthSpec::Bearer {
            token: scope.value(legacy.token, || "bearerToken".to_owned())?,
        },
        other => {
            let detail = format!("`{other}` auth is not supported; imported as no auth");
            scope.note("auth", NoteKind::Converted, detail)?;
            AuthSpec::None
        }
    })
}
