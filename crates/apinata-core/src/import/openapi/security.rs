//! Security schemes: the first requirement becomes auth whose credentials are placeholders,
//! later added to every environment as valueless variables (the OAuth client id is plain, the
//! rest are secrets). Covers apiKey, http basic, bearer and digest, oauth2 and openIdConnect.

use super::Doc;
use super::operations::Operation;
use super::refs::RefKind;
use super::version::Version;
use crate::edit::{ApiKeyPlacement, AuthSpec};
use crate::edit::{OAuth2Flow, OAuth2Spec, VarValue};
use crate::env_file::EnvVariable;
use crate::import::{DocumentError, ImportNote, ImportedEnvironment, NoteKind};
use serde_json::Value;

/// Placeholder names used so far, in first-use order, each with whether it is a secret.
#[derive(Debug, Default)]
pub(crate) struct Placeholders(Vec<(&'static str, bool)>);

impl Placeholders {
    fn add(&mut self, name: &'static str) {
        self.record(name, true);
    }

    /// A placeholder that is not a secret, such as an OAuth client id.
    fn add_plain(&mut self, name: &'static str) {
        self.record(name, false);
    }

    fn record(&mut self, name: &'static str, secret: bool) {
        if !self.0.iter().any(|(n, _)| *n == name) {
            self.0.push((name, secret));
        }
    }
}

/// The collection's auth from the root `security`; absent or empty is `None`.
pub(crate) fn collection_auth(
    doc: &mut Doc<'_>,
    notes: &mut Vec<ImportNote>,
) -> Result<AuthSpec, DocumentError> {
    let root = doc.root;
    let auth = match root.get("security") {
        Some(list) => from_requirements(doc, list, "security", notes)?,
        None => AuthSpec::None,
    };
    notes.extend(doc.take_ref_notes());
    Ok(auth)
}

/// An operation's auth from its own `security`: absent inherits, empty is `None`.
pub(crate) fn operation_auth(
    doc: &mut Doc<'_>,
    op: &Operation<'_>,
    notes: &mut Vec<ImportNote>,
) -> Result<AuthSpec, DocumentError> {
    let auth = match op.op().get("security") {
        Some(list) => {
            let source = format!("{}.security", op.source);
            from_requirements(doc, list, &source, notes)?
        }
        None => AuthSpec::Inherit,
    };
    notes.extend(doc.take_ref_notes());
    Ok(auth)
}

/// Add each placeholder in `doc.placeholders` as a valueless variable to every environment
/// lacking it, creating one named `Environment` when there is none; each variable is charged.
pub(crate) fn add_secrets(
    doc: &mut Doc<'_>,
    envs: &mut Vec<ImportedEnvironment>,
) -> Result<(), DocumentError> {
    let names = &doc.placeholders.0;
    if names.is_empty() {
        return Ok(());
    }
    let source = definitions_key(doc.version);
    if envs.is_empty() {
        doc.budget.charge(1)?;
        doc.budget.charge_text("Environment")?;
        envs.push(ImportedEnvironment {
            name: "Environment".to_owned(),
            variables: Vec::new(),
            notes: Vec::new(),
            sources: Vec::new(),
        });
    }
    for env in envs {
        for (name, secret) in names {
            if env.variables.iter().any(|v| v.name == *name) {
                continue;
            }
            doc.budget.charge(1)?;
            doc.budget
                .charge_bytes((name.len() + source.len()) as u64)?;
            env.variables.push(if *secret {
                EnvVariable::secret(*name)
            } else {
                EnvVariable::new(*name, VarValue::Text(String::new()))
            });
            env.sources.push(source.to_owned());
        }
    }
    Ok(())
}

/// The document's scheme definitions, when declared.
pub(crate) fn definitions(root: &Value, version: Version) -> Option<&Value> {
    if version == Version::Swagger2 {
        root.get("securityDefinitions")
    } else {
        root.get("components")
            .and_then(|c| c.get("securitySchemes"))
    }
}

/// Where the document declares its schemes.
pub(crate) fn definitions_key(version: Version) -> &'static str {
    if version == Version::Swagger2 {
        "securityDefinitions"
    } else {
        "components.securitySchemes"
    }
}

/// Decision 6: the first requirement's first scheme, with a `Converted` note when others are
/// ignored; an empty list or an empty first requirement is `None`.
fn from_requirements(
    doc: &mut Doc<'_>,
    list: &Value,
    source: &str,
    notes: &mut Vec<ImportNote>,
) -> Result<AuthSpec, DocumentError> {
    // An empty list means no auth; a list that is not one of requirement objects is noted.
    let first = match list.as_array().map(|l| l.first()) {
        Some(None) => return Ok(AuthSpec::None),
        Some(Some(first)) if first.is_object() => first,
        _ => {
            note(doc, notes, source, NoteKind::Dropped, &[], || {
                "not a list of security requirements, so no auth was set".to_owned()
            })?;
            return Ok(AuthSpec::None);
        }
    };
    let more_requirements = list.as_array().is_some_and(|l| l.len() > 1);
    let Some(name) = first.as_object().and_then(|r| r.keys().next()) else {
        if more_requirements {
            note(doc, notes, source, NoteKind::Converted, &[], || {
                "only the first security requirement is used, and it is empty".to_owned()
            })?;
        }
        return Ok(AuthSpec::None);
    };
    let more_schemes = first.as_object().is_some_and(|r| r.len() > 1);
    if more_requirements || more_schemes {
        note(
            doc,
            notes,
            source,
            NoteKind::Converted,
            &[name.as_str()],
            || format!("only the first security requirement's first scheme, `{name}`, is used"),
        )?;
    }
    let definitions = definitions(doc.root, doc.version);
    let scheme_source = format!("{}.{name}", definitions_key(doc.version));
    let Some(raw) = definitions.and_then(|d| d.get(name)) else {
        note(
            doc,
            notes,
            source,
            NoteKind::Dropped,
            &[name.as_str()],
            || format!("`{name}` is not a defined security scheme, so no auth was set"),
        )?;
        return Ok(AuthSpec::None);
    };
    let scheme = doc.resolve(raw, RefKind::Object, &scheme_source)?;
    let field = |key: &str| scheme.get(key).and_then(Value::as_str).unwrap_or_default();
    let kind = field("type");
    let auth = match kind {
        "http" if field("scheme").eq_ignore_ascii_case("basic") => Some(basic(doc)?),
        "http" if field("scheme").eq_ignore_ascii_case("bearer") => {
            doc.budget.charge_text("{{token}}")?;
            doc.placeholders.add("token");
            Some(AuthSpec::Bearer {
                token: "{{token}}".to_owned(),
            })
        }
        "http" if field("scheme").eq_ignore_ascii_case("digest") => {
            doc.budget.charge_text("{{username}}")?;
            doc.budget.charge_text("{{password}}")?;
            doc.placeholders.add("username");
            doc.placeholders.add("password");
            Some(AuthSpec::Digest {
                username: "{{username}}".to_owned(),
                password: "{{password}}".to_owned(),
            })
        }
        "basic" if doc.version == Version::Swagger2 => Some(basic(doc)?),
        "oauth2" => {
            let requested = requested_scopes(first, name);
            oauth2(doc, &scheme, name, &requested, source, notes)?
        }
        "openIdConnect" => {
            let requested = requested_scopes(first, name);
            Some(open_id_connect(
                doc,
                field("openIdConnectUrl"),
                name,
                &requested,
                source,
                notes,
            )?)
        }
        "apiKey" => {
            let placement = match field("in") {
                "header" => Some(ApiKeyPlacement::Header),
                "query" => Some(ApiKeyPlacement::Query),
                _ => None,
            };
            match (placement, field("name")) {
                (Some(placement), key) if !key.is_empty() => {
                    doc.budget.charge_text(key)?;
                    doc.budget.charge_text("{{apiKey}}")?;
                    doc.placeholders.add("apiKey");
                    Some(AuthSpec::ApiKey {
                        key: key.to_owned(),
                        value: "{{apiKey}}".to_owned(),
                        placement,
                    })
                }
                _ => None,
            }
        }
        _ => None,
    };
    if let Some(auth) = auth {
        return Ok(auth);
    }
    // The detail names what decided it: a missing type or key name, else the type and, where
    // it decides support, the scheme or location.
    let detail = || match kind {
        "" => format!("`{name}` has no `type`, so no auth was set"),
        "oauth2" => {
            format!("`{name}` is an `oauth2` scheme with no usable flow, so no auth was set")
        }
        "apiKey" if field("name").is_empty() => {
            format!("`{name}` is an `apiKey` scheme with no `name`, so no auth was set")
        }
        _ => {
            let qualifier = match kind {
                "http" => format!(" `{}`", field("scheme")),
                "apiKey" => format!(" in `{}`", field("in")),
                _ => String::new(),
            };
            format!(
                "`{name}` is a `{kind}`{qualifier} scheme, which is not imported, so no auth was set"
            )
        }
    };
    let quoted = [name.as_str(), kind, field("scheme"), field("in")];
    note(doc, notes, source, NoteKind::Dropped, &quoted, detail)?;
    Ok(AuthSpec::None)
}

/// Basic auth with the username and password placeholders, charged and recorded.
fn basic(doc: &mut Doc<'_>) -> Result<AuthSpec, DocumentError> {
    let (username, password) = ("{{username}}", "{{password}}");
    doc.budget.charge_text(username)?;
    doc.budget.charge_text(password)?;
    doc.placeholders.add("username");
    doc.placeholders.add("password");
    Ok(AuthSpec::Basic {
        username: username.to_owned(),
        password: password.to_owned(),
    })
}

/// The scope names a requirement lists for the scheme `name`.
fn requested_scopes<'a>(requirement: &'a Value, name: &str) -> Vec<&'a str> {
    let list = requirement.get(name).and_then(Value::as_array);
    list.into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect()
}

/// The flow keys in priority order: OAS 3 flow key, Swagger 2 flow name, flow.
const FLOWS: [(&str, &str, OAuth2Flow); 4] = [
    (
        "authorizationCode",
        "accessCode",
        OAuth2Flow::AuthorizationCode,
    ),
    (
        "clientCredentials",
        "application",
        OAuth2Flow::ClientCredentials,
    ),
    ("password", "password", OAuth2Flow::Password),
    ("implicit", "implicit", OAuth2Flow::Implicit),
];

/// An oauth2 scheme (OAS 3.1 4.8.27 to 4.8.30, Swagger 2.0): the highest-priority flow it
/// offers, with `Converted` notes for the others. `None` when it offers no known flow.
fn oauth2(
    doc: &mut Doc<'_>,
    scheme: &Value,
    name: &str,
    requested: &[&str],
    source: &str,
    notes: &mut Vec<ImportNote>,
) -> Result<Option<AuthSpec>, DocumentError> {
    // Swagger 2 keeps one flow flat on the scheme; OAS 3 maps flow names to flow objects.
    let swagger = doc.version == Version::Swagger2;
    let flow_key = scheme
        .get("flow")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let present: Vec<(&str, OAuth2Flow, &Value)> = FLOWS
        .iter()
        .filter_map(|&(key, swagger_key, flow)| {
            let object = if swagger {
                (flow_key == swagger_key).then_some(scheme)
            } else {
                scheme.get("flows").and_then(|f| f.get(key))
            };
            object
                .filter(|o| o.is_object())
                .map(|object| (key, flow, object))
        })
        .collect();
    let Some(&(_, flow, object)) = present.first() else {
        return Ok(None);
    };
    for &(key, _, _) in &present[1..] {
        note(
            doc,
            notes,
            source,
            NoteKind::Converted,
            &[name, key],
            || format!("`{name}` also offers the `{key}` flow, which was not imported"),
        )?;
    }
    let url = |key: &str| object.get(key).and_then(Value::as_str).unwrap_or_default();
    let mut spec = OAuth2Spec::new(flow);
    spec.pkce = flow == OAuth2Flow::AuthorizationCode;
    spec.client_id = "{{clientId}}".to_owned();
    doc.budget.charge_text("{{clientId}}")?;
    doc.placeholders.add_plain("clientId");
    let mut copied = Vec::new();
    if matches!(flow, OAuth2Flow::AuthorizationCode | OAuth2Flow::Implicit) {
        spec.authorization_url = url("authorizationUrl").to_owned();
        copied.push(&spec.authorization_url);
    }
    if flow != OAuth2Flow::Implicit {
        spec.access_token_url = url("tokenUrl").to_owned();
        spec.refresh_token_url = url("refreshUrl").to_owned();
        spec.client_secret = "{{clientSecret}}".to_owned();
        copied.push(&spec.access_token_url);
        copied.push(&spec.refresh_token_url);
        doc.budget.charge_text("{{clientSecret}}")?;
        doc.placeholders.add("clientSecret");
    }
    for text in copied {
        doc.budget.charge_text(text)?;
    }
    if flow == OAuth2Flow::Password {
        spec.username = "{{username}}".to_owned();
        spec.password = "{{password}}".to_owned();
        doc.budget.charge_text("{{username}}")?;
        doc.budget.charge_text("{{password}}")?;
        doc.placeholders.add("username");
        doc.placeholders.add("password");
    }
    // A requirement with no scopes asks for everything the flow declares.
    let declared: Vec<&str> = object
        .get("scopes")
        .and_then(Value::as_object)
        .map(|m| m.keys().map(String::as_str).collect())
        .unwrap_or_default();
    let names = if requested.is_empty() {
        &declared[..]
    } else {
        requested
    };
    spec.scope = scope_string(doc, names, source, notes)?;
    note(doc, notes, source, NoteKind::Converted, &[name], || {
        format!("`{name}` uses placeholder variables: set their values in the environment")
    })?;
    Ok(Some(AuthSpec::OAuth2(spec)))
}

/// An openIdConnect scheme: there is no OpenID variant, so it becomes an authorization code
/// grant with PKCE whose URLs the user fills in from the discovery document. Never fetched.
fn open_id_connect(
    doc: &mut Doc<'_>,
    discovery: &str,
    name: &str,
    requested: &[&str],
    source: &str,
    notes: &mut Vec<ImportNote>,
) -> Result<AuthSpec, DocumentError> {
    let mut spec = OAuth2Spec::new(OAuth2Flow::AuthorizationCode);
    spec.pkce = true;
    spec.client_id = "{{clientId}}".to_owned();
    spec.client_secret = "{{clientSecret}}".to_owned();
    doc.budget.charge_text("{{clientId}}")?;
    doc.budget.charge_text("{{clientSecret}}")?;
    doc.placeholders.add_plain("clientId");
    doc.placeholders.add("clientSecret");
    spec.scope = scope_string(doc, requested, source, notes)?;
    note(
        doc,
        notes,
        source,
        NoteKind::Converted,
        &[name, discovery],
        || {
            format!(
                "`{name}` is OpenID Connect (`{discovery}`): set the authorization and token URLs and the client variables"
            )
        },
    )?;
    Ok(AuthSpec::OAuth2(spec))
}

/// Space-joined scope names; a name that is empty or holds whitespace would split into other
/// scopes, so it is skipped with a `Dropped` note.
fn scope_string(
    doc: &mut Doc<'_>,
    names: &[&str],
    source: &str,
    notes: &mut Vec<ImportNote>,
) -> Result<String, DocumentError> {
    let mut kept: Vec<&str> = Vec::new();
    for &scope in names {
        if scope.is_empty() || scope.contains(char::is_whitespace) {
            note(doc, notes, source, NoteKind::Dropped, &[scope], || {
                format!("the scope `{scope}` is empty or holds whitespace, so it was skipped")
            })?;
        } else {
            doc.budget.charge_text(scope)?;
            kept.push(scope);
        }
    }
    Ok(kept.join(" "))
}

/// Charge, then make and keep, one note: its source, the document text it quotes and at most
/// `PROSE` bytes of fixed wording are charged before `detail` is built.
fn note(
    doc: &mut Doc<'_>,
    notes: &mut Vec<ImportNote>,
    source: &str,
    kind: NoteKind,
    quoted: &[&str],
    detail: impl FnOnce() -> String,
) -> Result<(), DocumentError> {
    const PROSE: usize = 200;
    let quoted_len: usize = quoted.iter().map(|q| q.len()).sum();
    doc.budget.charge(1)?;
    doc.budget
        .charge_bytes((source.len() + quoted_len + PROSE) as u64)?;
    let detail = detail();
    debug_assert!(detail.len() <= quoted_len + PROSE, "{detail}");
    notes.push(ImportNote::new(source, kind, detail));
    Ok(())
}
