//! Keeping imported credentials out of collection files: each literal password, token, key or
//! credential header becomes a `{{name}}` reference to a secret variable, and its value is
//! handed to the caller to keep in an environment and the secret store.

use super::har::secret_names;
use super::{
    ImportBundle, ImportNote, ImportedCollection, ImportedEnvironment, ImportedItem,
    ImportedRequest, NoteKind,
};
use crate::collection::Collection;
use crate::edit::{
    AuthSpec, BodySpec, KeyValue, MultipartValue, OAuth1KeyKind, ParamKind, VarValue,
};
use crate::env_file::EnvVariable;
use crate::secrets::{MemoryStore, SecretError, SecretStore};
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
/// same value under the same base reuses its name. A held name fits only the value it holds.
pub(crate) struct SecretNames {
    taken: HashSet<String>,
    held: Held,
    named: HashMap<(String, String), String>,
}

/// Names a collection already uses: `Some(value)` for a secret holding `value`, `None` for a
/// name no new value may take.
pub(crate) type Held = HashMap<String, Option<String>>;

impl SecretNames {
    pub(crate) fn new(taken: impl IntoIterator<Item = String>) -> SecretNames {
        SecretNames::holding(taken, Held::new())
    }

    pub(crate) fn holding(taken: impl IntoIterator<Item = String>, held: Held) -> SecretNames {
        SecretNames {
            taken: taken.into_iter().collect(),
            held,
            named: HashMap::new(),
        }
    }

    fn fits(&self, name: &str, value: &str) -> bool {
        !self.taken.contains(name)
            && self
                .held
                .get(name)
                .is_none_or(|h| h.as_deref() == Some(value))
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
            .find(|name| self.fits(name, value))
            .unwrap_or_else(|| base.to_owned());
        self.taken.insert(name.clone());
        self.named.insert(key, name.clone());
        name
    }
}

/// The secret variable name a credential of `owner` (a host or an item's name) starts from,
/// such as `get_user_password` for the `password` of `Get user`.
pub fn secret_name_base(owner: &str, kind: &str) -> String {
    let owner = slug(owner);
    match owner.trim_matches('_') {
        "" => kind.to_owned(),
        owner => format!("{owner}_{kind}"),
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

/// A value worth hiding: not blank, and not just variable references, optionally after an
/// auth scheme word such as `Bearer`, which hold nothing secret themselves.
fn is_literal(value: &str) -> bool {
    if value.trim().is_empty() {
        return false;
    }
    let mut rest = String::new();
    let mut tail = value;
    while let Some(open) = tail.find("{{") {
        rest.push_str(&tail[..open]);
        match tail[open..].find("}}") {
            Some(close) => tail = &tail[open + close + 2..],
            None => {
                rest.push_str(&tail[open..]);
                tail = "";
            }
        }
    }
    rest.push_str(tail);
    let rest = rest.trim();
    let scheme = ["bearer", "basic", "token", "digest"]
        .iter()
        .any(|word| rest.eq_ignore_ascii_case(word));
    !(rest.is_empty() || (scheme && value.contains("{{")))
}

/// Names one owner's secrets `<owner>_<kind>` and records each one taken.
struct Taker<'a> {
    owner: String,
    names: &'a mut SecretNames,
    found: &'a mut Vec<Found>,
}

impl Taker<'_> {
    fn new<'a>(owner: &str, names: &'a mut SecretNames, found: &'a mut Vec<Found>) -> Taker<'a> {
        let owner = match slug(owner).trim_matches('_') {
            "" => "request".to_owned(),
            trimmed => trimmed.to_owned(),
        };
        Taker {
            owner,
            names,
            found,
        }
    }

    /// The reference for `value`, taking it as a secret when it is a literal.
    fn take(&mut self, member: String, kind: &str, value: &mut String) {
        if !is_literal(value) {
            return;
        }
        let name = self
            .names
            .name_for(&format!("{}_{kind}", self.owner), value);
        self.found.push(Found {
            member,
            name: name.clone(),
            value: std::mem::replace(value, format!("{{{{{name}}}}}")),
        });
    }
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
    let mut taker = Taker::new(owner, names, found);
    for (kind, value) in auth_secrets(auth) {
        taker.take(format!("{at}auth.{kind}"), kind, value);
    }
    for (i, header) in headers.iter_mut().enumerate() {
        if is_credential_header(&header.name) {
            let kind = slug(&header.name);
            taker.take(format!("{at}headers[{i}]"), &kind, &mut header.value);
        }
    }
}

fn is_credential_header(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    matches!(
        lower.as_str(),
        "authorization" | "proxy-authorization" | "cookie"
    ) || secret_names::is_header(name)
}

/// [`protect_parts`] for a request, plus the URL's userinfo password, credential query
/// params (in the URL and the params alike) and credential form fields.
pub(crate) fn protect_request(
    owner: &str,
    at: &str,
    request: &mut ImportedRequest,
    names: &mut SecretNames,
    found: &mut Vec<Found>,
) {
    protect_parts(
        owner,
        at,
        &mut request.auth,
        &mut request.headers,
        names,
        found,
    );
    let mut taker = Taker::new(owner, names, found);
    let url_member = format!("{at}url");
    if let Some(range) = userinfo_password(&request.url) {
        let mut password = request.url[range.clone()].to_owned();
        taker.take(url_member.clone(), "url_password", &mut password);
        request.url.replace_range(range, &password);
    }
    request.url = rewrite_query(&request.url, |key, value| {
        let kind = slug(secret_names::leaf(key));
        taker.take(url_member.clone(), &kind, value);
    });
    for (i, param) in request.params.iter_mut().enumerate() {
        if param.kind == ParamKind::Query && secret_names::is_query(&param.name) {
            let kind = slug(secret_names::leaf(&param.name));
            taker.take(format!("{at}params[{i}]"), &kind, &mut param.value);
        }
    }
    match &mut request.body {
        BodySpec::FormUrlEncoded(fields) => {
            for (i, field) in fields.iter_mut().enumerate() {
                if secret_names::is_field(&field.name) {
                    let kind = slug(secret_names::leaf(&field.name));
                    taker.take(format!("{at}body[{i}]"), &kind, &mut field.value);
                }
            }
        }
        BodySpec::Multipart(fields) => {
            for (i, field) in fields.iter_mut().enumerate() {
                if let MultipartValue::Text(value) = &mut field.value
                    && secret_names::is_field(&field.name)
                {
                    let kind = slug(secret_names::leaf(&field.name));
                    taker.take(format!("{at}body[{i}]"), &kind, value);
                }
            }
        }
        _ => {}
    }
}

/// The byte range of the password in a `scheme://user:password@host` URL, when it has one.
fn userinfo_password(url: &str) -> Option<std::ops::Range<usize>> {
    let colon = url.find(':')?;
    let scheme = &url[..colon];
    let is_scheme = !scheme.is_empty()
        && scheme
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'));
    if !is_scheme {
        return None;
    }
    // The authority follows the scheme's `:` and any run of slashes, up to a path, query or
    // fragment; its last `@` ends the userinfo.
    let start = url.len() - url[colon + 1..].trim_start_matches(['/', '\\']).len();
    let end = url[start..]
        .find(['/', '\\', '?', '#'])
        .map_or(url.len(), |i| start + i);
    let at_sign = url[start..end].rfind('@').map(|i| start + i)?;
    let sep = url[start..at_sign].find(':').map(|i| start + i)?;
    (sep + 1 < at_sign).then_some(sep + 1..at_sign)
}

/// `url` with `take` given the raw value of each credential pair of its query, every other
/// byte as written.
fn rewrite_query(url: &str, mut take: impl FnMut(&str, &mut String)) -> String {
    let hash = url.find('#').unwrap_or(url.len());
    let Some(q) = url[..hash].find('?') else {
        return url.to_owned();
    };
    let pairs: Vec<String> = url[q + 1..hash]
        .split('&')
        .map(|pair| match pair.split_once('=') {
            Some((key, value)) if secret_names::is_query(key) => {
                let mut value = value.to_owned();
                take(key, &mut value);
                format!("{key}={value}")
            }
            _ => pair.to_owned(),
        })
        .collect();
    format!("{}{}{}", &url[..=q], pairs.join("&"), &url[hash..])
}

/// Point every `{{old}}` reference of `request` at `{{new}}` instead.
pub(crate) fn rename_reference(request: &mut ImportedRequest, old: &str, new: &str) {
    let (from, to) = (format!("{{{{{old}}}}}"), format!("{{{{{new}}}}}"));
    let rename = |text: &mut String| {
        if text.contains(&from) {
            *text = text.replace(&from, &to);
        }
    };
    rename(&mut request.url);
    request.params.iter_mut().for_each(|p| rename(&mut p.value));
    request
        .headers
        .iter_mut()
        .for_each(|h| rename(&mut h.value));
    auth_secrets(&mut request.auth)
        .into_iter()
        .for_each(|(_, value)| rename(value));
    match &mut request.body {
        BodySpec::FormUrlEncoded(fields) => fields.iter_mut().for_each(|f| rename(&mut f.value)),
        BodySpec::Multipart(fields) => {
            for field in fields {
                if let MultipartValue::Text(value) = &mut field.value {
                    rename(value);
                }
            }
        }
        _ => {}
    }
}

/// The fields of `auth` that hold a credential, such as a password or a token, in a fixed
/// order: each one's kind (`password`, `token`, ...) and text.
pub fn auth_secret_fields(auth: &AuthSpec) -> Vec<(&'static str, String)> {
    let mut auth = auth.clone();
    auth_secrets(&mut auth)
        .into_iter()
        .map(|(kind, value)| (kind, value.clone()))
        .collect()
}

/// Puts `to` in each credential field of `auth` whose text is exactly `from`.
pub fn replace_auth_secret(auth: &mut AuthSpec, from: &str, to: &str) {
    for (_, value) in auth_secrets(auth) {
        if value == from {
            *value = to.to_owned();
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
            (
                "private_key",
                spec.private_key
                    .as_mut()
                    .filter(|k| k.kind == OAuth1KeyKind::Text)
                    .map(|k| &mut k.value),
            ),
        ]
        .into_iter()
        .filter_map(|(kind, value)| value.map(|v| (kind, v)))
        .collect(),
        AuthSpec::OAuth2(spec) => vec![
            ("client_secret", &mut spec.client_secret),
            ("password", &mut spec.password),
        ],
        AuthSpec::Jwt(spec) => {
            let key = (spec.private_key.as_mut())
                .filter(|k| k.kind == OAuth1KeyKind::Text)
                .map(|k| ("private_key", &mut k.value));
            std::iter::once(("secret", &mut spec.secret))
                .chain(key)
                .collect()
        }
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
    /// Whether it came from the collection's own auth or headers.
    pub(crate) root: bool,
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
    held: Held,
    with_root: bool,
) -> Vec<Taken> {
    let taken = taken.into_iter().chain(tree_names(collection));
    let mut names = SecretNames::holding(taken, held);
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
                protect_request(&r.name, "request.", &mut r.request, names, &mut found);
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
            root: trail == "collection / ",
            source,
            name: f.name,
            value: f.value,
        });
    }
}

/// `bundle` with its credentials protected and named as [`write_collection`] names them on
/// a first import, from the bundle alone.
///
/// [`write_collection`]: super::write_collection
pub(crate) fn protect_bundle(bundle: &ImportBundle, held: Held) -> (ImportBundle, Vec<Taken>) {
    let mut out = bundle.clone();
    let declared: Vec<String> = bundle
        .environments
        .iter()
        .flat_map(|e| e.variables.iter().map(|v| v.name.clone()))
        .collect();
    let taken = protect_collection(&mut out.collection, declared, held, true);
    (out, taken)
}

/// The names `collection` already uses, read through `store`: a plain or root variable is
/// never taken, a secret only for the value it holds, and one holding nothing (or that the
/// store cannot read) is free, so later imports keep reusing it.
pub(crate) fn held_names(collection: &Collection, store: &dyn SecretStore) -> Held {
    let mut held = Held::new();
    for v in collection.file().vars() {
        held.insert(v.name, None);
    }
    for env in collection.environments() {
        for v in env.variables() {
            let value = match &v.value {
                _ if !v.secret => None,
                VarValue::Text(text) if !text.is_empty() => Some(text.clone()),
                _ => match store.get(&collection.secret_key(env.name(), &v.name)) {
                    Ok(Some(stored)) => Some(stored),
                    _ => continue,
                },
            };
            // Two environments holding different values leave no value that fits both.
            let entry = held.entry(v.name).or_insert_with(|| value.clone());
            if *entry != value {
                *entry = None;
            }
        }
    }
    held
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
    let names: Vec<String> = taken.iter().map(|t| t.name.clone()).collect();
    for env in envs {
        let mut note = ImportNote::new(
            "imported credentials",
            NoteKind::Converted,
            stored_claim(&names),
        );
        note.claims_stored = names.clone();
        env.notes.push(note);
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

/// The note saying the secret variables `names` hold the imported credentials in the store.
pub(crate) fn stored_claim(names: &[String]) -> String {
    let names: Vec<String> = names.iter().map(|n| format!("`{n}`")).collect();
    format!(
        "the imported credentials are the secret variables {}; their values are kept in the \
         secret store, not in the file",
        names.join(", ")
    )
}

/// Make `env` the collection's default environment when it has none, so a run without a
/// named environment finds the imported secrets. Best effort: a failed save leaves no default.
pub(crate) fn default_to(collection: &mut Collection, env: &str) {
    if collection.file().presets().default_environment().is_empty() {
        let file = collection.file_mut();
        file.set_default_environment(env);
        if file.save().is_err() {
            let _ = collection.revert(&collection.root().join(crate::collection::ROOT_FILE));
        }
    }
}

/// Why an imported secret went unstored: plain words, or the store's own failure.
enum NotStored {
    Because(String),
    Store(SecretError),
}

/// Keep `taken` in an existing collection: declared as secret variables of every environment
/// (of a new `Environment` when there is none), each value stored where none is yet, so a
/// secret already holding a value keeps it. `store` `None` stores nothing. What could not be
/// stored is noted as needing input and listed in `to_enter`.
pub(crate) fn keep_taken(
    collection: &mut Collection,
    taken: &[Taken],
    store: Option<&dyn SecretStore>,
    notes: &mut Vec<ImportNote>,
    to_enter: &mut Vec<(String, String)>,
) {
    if taken.is_empty() {
        return;
    }
    let mut needs = |env: &str, t: &Taken, why: NotStored| {
        let mut note = match why {
            NotStored::Because(why) => ImportNote::new(
                t.source.clone(),
                NoteKind::NeedsInput,
                format!(
                    "the secret `{}` was not stored ({why}); enter it before sending",
                    t.name
                ),
            ),
            NotStored::Store(e) => super::unstored::note(t.source.clone(), &t.name, &e),
        };
        note.file = Some(format!("environments/{env}.yml"));
        notes.push(note);
        to_enter.push((env.to_owned(), t.name.clone()));
    };
    if collection.environments().is_empty() {
        // Creating an environment writes no secret, so a throwaway store keeps the real one out.
        if let Err(e) = collection.create_environment("Environment", &MemoryStore::new()) {
            for t in taken {
                needs("Environment", t, NotStored::Because(e.to_string()));
            }
            return;
        }
    }
    let names: Vec<String> = collection
        .environments()
        .iter()
        .map(|e| e.name().to_owned())
        .collect();
    for env in &names {
        let Some(environment) = collection.environment_mut(env) else {
            continue;
        };
        let before = environment.variables();
        let mut variables = before.clone();
        let mut kept = Vec::new();
        for t in taken {
            match variables.iter().find(|v| v.name == t.name) {
                Some(v) if v.secret => kept.push(t),
                Some(_) => needs(
                    env,
                    t,
                    NotStored::Because(format!("`{}` is a plain variable of `{env}`", t.name)),
                ),
                None => {
                    variables.push(EnvVariable::secret(t.name.clone()));
                    kept.push(t);
                }
            }
        }
        if variables.len() != before.len() {
            environment.set_variables(variables);
            if let Err(e) = environment.save() {
                environment.set_variables(before);
                for t in kept {
                    needs(env, t, NotStored::Because(e.to_string()));
                }
                continue;
            }
        }
        for t in kept {
            let Some(store) = store else {
                needs(
                    env,
                    t,
                    NotStored::Because("no secret store was given".to_owned()),
                );
                continue;
            };
            let key = collection.secret_key(env, &t.name);
            match store.get(&key) {
                Ok(Some(held)) if held == t.value => {}
                Ok(Some(_)) => needs(
                    env,
                    t,
                    NotStored::Because(format!("`{}` already holds a different value", t.name)),
                ),
                Ok(None) => {
                    if let Err(e) = store.set(&key, &t.value) {
                        needs(env, t, NotStored::Store(e));
                    }
                }
                Err(e) => needs(env, t, NotStored::Store(e)),
            }
        }
    }
    if let Some(first) = names.first() {
        default_to(collection, first);
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
        assert!(is_literal("session=s3cret; csrf={{c}}"));
        assert!(is_literal("Bearer"));
        assert!(!is_literal("  "));
    }
}
