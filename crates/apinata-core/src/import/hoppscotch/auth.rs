//! Auth objects and legacy auth strings to `AuthSpec`.

use super::model::{Auth, Mapped};
use super::tree::Scope;
use crate::edit::{ApiKeyPlacement, AuthSpec};
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
