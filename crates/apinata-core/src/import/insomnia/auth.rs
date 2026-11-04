//! Request and folder authentication.

use super::model::{Member, text};
use super::template::apply;
use super::{Budget, Crumbs};
use crate::edit::{ApiKeyPlacement, AuthSpec, KeyValue};
use crate::import::{DocumentError, ImportNote, NoteKind};
use serde_json::{Map, Value};

/// Map an `authentication` member. `trail` ends with the item's name and prefixes every note
/// source; a bearer token with a custom prefix is sent as an `Authorization` pushed to `headers`.
/// Missing, malformed, typeless or disabled auth inherits; a type with no counterpart is `None`.
pub(super) fn auth(
    member: Option<Member>,
    trail: &[String],
    headers: &mut Vec<KeyValue>,
    notes: &mut Vec<ImportNote>,
    budget: &mut Budget,
) -> Result<AuthSpec, DocumentError> {
    let Some(Member { value, path }) = member else {
        return Ok(AuthSpec::Inherit);
    };
    let mut auth = match value {
        Value::Object(auth) => auth,
        Value::Null => return Ok(AuthSpec::Inherit),
        _ => {
            let detail = "not an object; not imported";
            notes.push(budget.note(trail, &path, NoteKind::Dropped, detail)?);
            return Ok(AuthSpec::Inherit);
        }
    };
    let kind = text(auth.remove("type").as_ref());
    let disabled = auth.remove("disabled");
    if kind.is_empty() || disabled == Some(Value::Bool(true)) {
        return Ok(AuthSpec::Inherit);
    }
    // Credential values may hold templates; the other members are plain settings.
    let take = |auth: &mut Map<String, Value>, key: &str| text(auth.remove(key).as_ref());
    let templated = |auth: &mut Map<String, Value>,
                     key: &str,
                     notes: &mut Vec<ImportNote>,
                     budget: &mut Budget| {
        let at = format!("{path}.{key}");
        apply(&text(auth.remove(key).as_ref()), trail, &at, notes, budget)
    };
    let spec = match kind.as_str() {
        "none" => AuthSpec::None,
        "basic" => {
            let spec = AuthSpec::Basic {
                username: templated(&mut auth, "username", notes, budget)?,
                password: templated(&mut auth, "password", notes, budget)?,
            };
            if auth.remove("useISO88591") == Some(Value::Bool(true)) {
                let at = format!("{path}.useISO88591");
                let detail = "ISO-8859-1 encoding is not imported; credentials are sent as UTF-8";
                notes.push(budget.note(trail, &at, NoteKind::Converted, detail)?);
            }
            spec
        }
        "bearer" => {
            let token = templated(&mut auth, "token", notes, budget)?;
            let prefix = take(&mut auth, "prefix");
            if prefix.is_empty() || prefix == "Bearer" {
                AuthSpec::Bearer { token }
            } else {
                // Only the `Bearer` scheme has a counterpart, so another one is sent by hand.
                let value = if prefix == "NO_PREFIX" {
                    token
                } else {
                    format!("{prefix} {token}")
                };
                let at = format!("{path}.prefix");
                // Insomnia sends an explicit `Authorization` header in place of the auth.
                let explicit = headers
                    .iter()
                    .any(|h| h.name.eq_ignore_ascii_case("Authorization"));
                if explicit {
                    let detail =
                        "an explicit `Authorization` header is sent instead; auth is set to none";
                    notes.push(budget.note(trail, &at, NoteKind::Dropped, detail)?);
                } else {
                    headers.push(KeyValue::new("Authorization", value));
                    let detail =
                        "the token is sent as an `Authorization` header; auth is set to none";
                    notes.push(budget.note(trail, &at, NoteKind::Converted, detail)?);
                }
                AuthSpec::None
            }
        }
        "apikey" => {
            let placement = match take(&mut auth, "addTo").as_str() {
                "" | "header" => Some(ApiKeyPlacement::Header),
                "queryParams" => Some(ApiKeyPlacement::Query),
                _ => None,
            };
            match placement {
                Some(placement) => AuthSpec::ApiKey {
                    key: take(&mut auth, "key"),
                    value: templated(&mut auth, "value", notes, budget)?,
                    placement,
                },
                None => {
                    let detail = "an API key sent other than in a header or query is not imported; set to none";
                    return set_none(&path, trail, notes, budget, detail);
                }
            }
        }
        other => {
            let detail = format!("`{other}` authentication is not supported; set to none");
            return set_none(&path, trail, notes, budget, detail);
        }
    };
    unread(&auth, &path, trail, notes, budget)?;
    Ok(spec)
}

/// No auth for a kind core cannot hold, noted as a change at `path`.
fn set_none(
    path: &str,
    trail: &[String],
    notes: &mut Vec<ImportNote>,
    budget: &mut Budget,
    detail: impl Into<String>,
) -> Result<AuthSpec, DocumentError> {
    notes.push(budget.note(trail, path, NoteKind::Converted, detail)?);
    Ok(AuthSpec::None)
}

/// Note each member of a mapped auth object that was not read.
fn unread(
    rest: &Map<String, Value>,
    path: &str,
    trail: &[String],
    notes: &mut Vec<ImportNote>,
    budget: &mut Budget,
) -> Result<(), DocumentError> {
    for key in rest.keys() {
        let at = format!("{path}.{key}");
        let detail = "this authentication setting is not imported";
        notes.push(budget.note(trail, &at, NoteKind::Dropped, detail)?);
    }
    Ok(())
}
