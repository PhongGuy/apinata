//! Keeping imported credentials out of collection files: each literal password, token, key or
//! credential header becomes a `{{name}}` reference to a secret variable, and its value is
//! handed to the caller to keep in an environment and the secret store.

use super::har::secret_names;
use super::{ImportNote, ImportedCollection, ImportedEnvironment, ImportedItem, NoteKind};
use crate::edit::{AuthSpec, KeyValue, VarValue};
use crate::env_file::EnvVariable;
use std::collections::{HashMap, HashSet};

/// A literal taken out of a file: the variable that now holds it and the value.
pub(crate) struct Found {
    /// Where it was, such as `request.auth.password`, after the item's trail.
    pub(crate) member: String,
    /// The secret variable the file now refers to.
    pub(crate) name: String,
    /// The value, never written to a file.
    pub(crate) value: String,
}

/// Picks predictable names `<base>`, `<base>_2`, ... that no taken variable already uses; the
/// same value under the same base reuses its name.
pub(crate) struct SecretNames {
    taken: HashSet<String>,
    named: HashMap<(String, String), String>,
}

impl SecretNames {
    pub(crate) fn new(taken: impl IntoIterator<Item = String>) -> SecretNames {
        SecretNames {
            taken: taken.into_iter().collect(),
            named: HashMap::new(),
        }
    }

    fn name_for(&mut self, base: &str, value: &str) -> String {
        let key = (base.to_owned(), value.to_owned());
        if let Some(name) = self.named.get(&key) {
            return name.clone();
        }
        let name = (1..)
            .map(|n| match n {
                1 => base.to_owned(),
                n => format!("{base}_{n}"),
            })
            .find(|name| !self.taken.contains(name))
            .unwrap_or_else(|| base.to_owned());
        self.taken.insert(name.clone());
        self.named.insert(key, name.clone());
        name
    }
}

/// `text` with each run of characters that are not ASCII alphanumeric as one `_`.
pub(crate) fn slug(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('_') {
            out.push('_');
        }
    }
    out
}

/// A value worth hiding: not blank and not already (partly) a variable reference.
fn is_literal(value: &str) -> bool {
    !value.trim().is_empty() && !value.contains("{{")
}

/// Replace each literal secret of `auth` and `headers` with a reference named after `owner`,
/// pushing what was taken onto `found`. `at` prefixes each member, such as `request.`.
pub(crate) fn protect_parts(
    owner: &str,
    at: &str,
    auth: &mut AuthSpec,
    headers: &mut [KeyValue],
    names: &mut SecretNames,
    found: &mut Vec<Found>,
) {
    let owner = match slug(owner).trim_matches('_') {
        "" => "request".to_owned(),
        trimmed => trimmed.to_owned(),
    };
    let mut take = |member: String, kind: &str, value: &mut String| {
        if !is_literal(value) {
            return;
        }
        let name = names.name_for(&format!("{owner}_{kind}"), value);
        found.push(Found {
            member,
            name: name.clone(),
            value: std::mem::replace(value, format!("{{{{{name}}}}}")),
        });
    };
    for (kind, value) in auth_secrets(auth) {
        take(format!("{at}auth.{kind}"), kind, value);
    }
    for (i, header) in headers.iter_mut().enumerate() {
        let lower = header.name.to_ascii_lowercase();
        let credential = matches!(
            lower.as_str(),
            "authorization" | "proxy-authorization" | "cookie"
        ) || secret_names::is_header(&header.name);
        if credential {
            let kind = slug(&header.name);
            take(format!("{at}headers[{i}]"), &kind, &mut header.value);
        }
    }
}

/// The fields of `auth` that hold a credential, each with the name kind it is stored under.
fn auth_secrets(auth: &mut AuthSpec) -> Vec<(&'static str, &mut String)> {
    match auth {
        AuthSpec::Basic { password, .. }
        | AuthSpec::Digest { password, .. }
        | AuthSpec::Wsse { password, .. }
        | AuthSpec::Ntlm { password, .. } => vec![("password", password)],
        AuthSpec::Bearer { token } => vec![("token", token)],
        AuthSpec::ApiKey { value, .. } => vec![("api_key", value)],
        AuthSpec::AwsV4 {
            secret_access_key,
            session_token,
            ..
        } => vec![
            ("secret_access_key", secret_access_key),
            ("session_token", session_token),
        ],
        AuthSpec::OAuth1(spec) => [
            ("consumer_secret", spec.consumer_secret.as_mut()),
            ("access_token", spec.access_token.as_mut()),
            ("access_token_secret", spec.access_token_secret.as_mut()),
        ]
        .into_iter()
        .filter_map(|(kind, value)| value.map(|v| (kind, v)))
        .collect(),
        AuthSpec::OAuth2(spec) => vec![
            ("client_secret", &mut spec.client_secret),
            ("password", &mut spec.password),
        ],
        AuthSpec::EdgeGrid(spec) => vec![
            ("access_token", &mut spec.access_token),
            ("client_token", &mut spec.client_token),
            ("client_secret", &mut spec.client_secret),
        ],
        _ => Vec::new(),
    }
}

/// Every variable name the collection's tree declares, so a new secret never shadows one.
fn tree_names(collection: &ImportedCollection) -> Vec<String> {
    fn walk(items: &[ImportedItem], out: &mut Vec<String>) {
        for item in items {
            match item {
                ImportedItem::Folder(f) => {
                    out.extend(f.vars.iter().map(|v| v.name.clone()));
                    walk(&f.items, out);
                }
                ImportedItem::Request(r) => {
                    out.extend(r.request.vars.iter().map(|v| v.name.clone()));
                }
            }
        }
    }
    let mut out: Vec<String> = collection.vars.iter().map(|v| v.name.clone()).collect();
    walk(&collection.items, &mut out);
    out
}

/// A secret taken out of an imported collection, with the note source it came from.
pub(crate) struct Taken {
    pub(crate) source: String,
    pub(crate) name: String,
    pub(crate) value: String,
}

/// Take every literal credential out of `collection`, avoiding the names in `taken` and those
/// its tree declares; the root's own auth and headers only `with_root`. Each one gets a
/// `Converted` note on the item it came from.
pub(crate) fn protect_collection(
    collection: &mut ImportedCollection,
    taken: impl IntoIterator<Item = String>,
    with_root: bool,
) -> Vec<Taken> {
    let mut names = SecretNames::new(taken.into_iter().chain(tree_names(collection)));
    let mut out = Vec::new();
    if with_root {
        let mut found = Vec::new();
        let owner = collection.name.clone();
        protect_parts(
            &owner,
            "",
            &mut collection.auth,
            &mut collection.headers,
            &mut names,
            &mut found,
        );
        record("collection / ", &mut found, &mut collection.notes, &mut out);
    }
    protect_items(&mut collection.items, "", &mut names, &mut out);
    out
}

fn protect_items(
    items: &mut [ImportedItem],
    trail: &str,
    names: &mut SecretNames,
    out: &mut Vec<Taken>,
) {
    let mut found = Vec::new();
    for item in items {
        match item {
            ImportedItem::Folder(f) => {
                let trail = format!("{trail}{} / ", f.name);
                protect_parts(&f.name, "", &mut f.auth, &mut f.headers, names, &mut found);
                record(&trail, &mut found, &mut f.notes, out);
                protect_items(&mut f.items, &trail, names, out);
            }
            ImportedItem::Request(r) => {
                let trail = format!("{trail}{} / ", r.name);
                let request = &mut r.request;
                protect_parts(
                    &r.name,
                    "request.",
                    &mut request.auth,
                    &mut request.headers,
                    names,
                    &mut found,
                );
                record(&trail, &mut found, &mut r.notes, out);
            }
        }
    }
}

/// Note each of `found` on its item and move it to `out`.
fn record(trail: &str, found: &mut Vec<Found>, notes: &mut Vec<ImportNote>, out: &mut Vec<Taken>) {
    for f in found.drain(..) {
        let source = format!("{trail}{}", f.member);
        notes.push(ImportNote::new(
            source.clone(),
            NoteKind::Converted,
            format!(
                "the value is kept as the secret variable `{}`, not in the file",
                f.name
            ),
        ));
        out.push(Taken {
            source,
            name: f.name,
            value: f.value,
        });
    }
}

/// Declare each of `taken` as a secret variable with its value in every one of `envs`, adding
/// an `Environment` when there is none; the environment writer stores the values.
pub(crate) fn add_to_environments(envs: &mut Vec<ImportedEnvironment>, taken: &[Taken]) {
    if taken.is_empty() {
        return;
    }
    if envs.is_empty() {
        envs.push(ImportedEnvironment {
            name: "Environment".to_owned(),
            variables: Vec::new(),
            notes: Vec::new(),
            sources: Vec::new(),
        });
    }
    let names: Vec<String> = taken.iter().map(|t| format!("`{}`", t.name)).collect();
    let detail = format!(
        "the imported credentials are the secret variables {}; their values are kept in the \
         secret store, not in the file",
        names.join(", ")
    );
    for env in envs {
        env.notes.push(ImportNote::new(
            "imported credentials",
            NoteKind::Converted,
            detail.clone(),
        ));
        // Sources pair with variables by index, so a short list is padded first.
        let fallback = (env.sources.len()..env.variables.len()).map(|j| format!("variables[{j}]"));
        let padded: Vec<String> = fallback.collect();
        env.sources.extend(padded);
        for t in taken {
            let mut variable = EnvVariable::secret(t.name.clone());
            variable.value = VarValue::Text(t.value.clone());
            env.variables.push(variable);
            env.sources.push(t.source.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_skip_taken_ones_and_reuse_a_known_value() {
        let mut names = SecretNames::new(["a_password".to_owned()]);
        assert_eq!(names.name_for("a_password", "x"), "a_password_2");
        assert_eq!(names.name_for("a_password", "y"), "a_password_3");
        assert_eq!(names.name_for("a_password", "x"), "a_password_2");
    }

    #[test]
    fn references_and_blanks_are_not_literals() {
        assert!(is_literal("s3cret"));
        assert!(!is_literal("{{pw}}"));
        assert!(!is_literal("Bearer {{token}}"));
        assert!(!is_literal("  "));
    }
}
