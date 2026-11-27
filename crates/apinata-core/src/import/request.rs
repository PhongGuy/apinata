//! Writing an imported request into a request file.

use super::protect::rename_reference;
use super::{ImportError, ImportNote, ImportedRequest, ImportedScripts, NoteKind};
use crate::collection::Collection;
use crate::edit::{AuthSpec, BodySpec, KeyValue};
use crate::env_file::EnvVariable;
use crate::error::{EditError, TreeError};
use crate::item_path::ItemPath;
use crate::js;
use crate::request::{RequestFile, RequestKind};
use crate::secrets::{SecretError, SecretStore};
use serde_json::Value;
use std::collections::{BTreeSet, HashSet};

/// Create `name` under `parent`, apply `request`, save. When applying or saving fails, the new
/// file is deleted again and that error returned. Fails with `Tree` for an unknown parent or
/// unusable name.
pub fn add_request(
    collection: &mut Collection,
    parent: &ItemPath,
    name: &str,
    request: &ImportedRequest,
) -> Result<ItemPath, ImportError> {
    let path = collection.create_request_of_kind(parent, name, request.kind)?;
    let file = collection
        .request_mut(&path)
        .ok_or_else(|| TreeError::NotFound { item: path.clone() })?;
    let written = request
        .apply_to(file)
        .map_err(ImportError::from)
        .and_then(|_notes| file.save().map_err(ImportError::from));
    // Undo only the new file: siblings keep their seqs and unsaved edits. A failed cleanup is
    // ignored, since the apply or save error is the one the caller can act on.
    if let Err(e) = written {
        let _ = collection.discard_request(&path);
        return Err(e);
    }
    Ok(path)
}

/// Rename each secret of `request`, and its references, whose name `collection` already uses
/// for something else: a plain variable, a variable of another environment, or a secret of the
/// target environment (chosen as [`store_request_secrets`] chooses it) holding another value
/// or none. A secret already holding the same value keeps its name. Call it before writing
/// the request, so a later import never takes over an earlier request's secret.
pub fn claim_secret_names(
    collection: &Collection,
    request: &mut ImportedRequest,
    prefer: Option<&str>,
    store: &dyn SecretStore,
) {
    claim_secret_names_besides(collection, request, prefer, store, &BTreeSet::new());
}

/// [`claim_secret_names`], with the `held` names taken too: names promised to secrets that
/// are not stored yet, such as those of an unsaved draft.
pub fn claim_secret_names_besides(
    collection: &Collection,
    request: &mut ImportedRequest,
    prefer: Option<&str>,
    store: &dyn SecretStore,
    held: &BTreeSet<String>,
) {
    let Ok(env) = target_environment(collection, prefer) else {
        return;
    };
    let target = collection
        .environment(&env)
        .map(|e| e.variables())
        .unwrap_or_default();
    let mut taken: HashSet<String> = collection
        .environments()
        .iter()
        .flat_map(|e| e.variables().into_iter().map(|v| v.name))
        .chain(collection.file().vars().into_iter().map(|v| v.name))
        .chain(held.iter().cloned())
        .collect();
    for i in 0..request.secrets.len() {
        let (base, value) = (&request.secrets[i].name, &request.secrets[i].value);
        let fits = |name: &str| match target.iter().find(|v| v.name == name) {
            Some(v) if v.secret => matches!(
                store.get(&collection.secret_key(&env, name)),
                Ok(Some(stored)) if stored == *value
            ),
            Some(_) => false,
            None => !taken.contains(name),
        };
        let name = (1..)
            .map(|n| match n {
                1 => base.clone(),
                n => format!("{base}_{n}"),
            })
            .find(|name| fits(name))
            .unwrap_or_else(|| base.clone());
        if name != *base {
            let old = base.clone();
            rename_reference(request, &old, &name);
            request.secrets[i].name = name.clone();
        }
        // A later secret of the same request must not land on this one.
        taken.insert(name);
    }
}

/// Keep each secret of `request` as a secret variable of one environment, with its value in
/// `store`. The environment is `prefer`; a named one that does not exist stores nothing. With no
/// `prefer` it is the only environment. With none to choose, when the variable name is taken by
/// a plain variable, or when a different value is already stored for it (never replaced), the
/// secret's `NeedsInput` note stays in `notes` and says why; secrets the store failed to keep
/// share one `NeedsInput` note, without the store's error text. On success
/// that note becomes a `Converted` one naming the environment, which becomes the collection's
/// default environment when it has none.
pub fn store_request_secrets(
    collection: &mut Collection,
    request: &ImportedRequest,
    prefer: Option<&str>,
    store: &dyn SecretStore,
    notes: &mut Vec<ImportNote>,
) {
    for secret in &request.secrets {
        let outcome = keep_secret(collection, &secret.name, &secret.value, prefer, store);
        let at = notes
            .iter()
            .position(|n| n.source == secret.source && n.kind == NoteKind::NeedsInput);
        match (outcome, at) {
            (Ok(env), Some(at)) => {
                notes[at].kind = NoteKind::Converted;
                notes[at].detail = format!(
                    "the secret is stored as the secret variable `{}` of the environment `{env}`",
                    secret.name
                );
            }
            (Ok(_), None) => {}
            (Err(NotKept::Store(e)), Some(at)) => {
                super::unstored::mark(&mut notes[at], &secret.name, &e);
            }
            (Err(NotKept::Store(e)), None) => {
                notes.push(super::unstored::note(
                    secret.source.clone(),
                    &secret.name,
                    &e,
                ));
            }
            // A note written before claiming renamed the secret is rewritten to name it.
            (Err(NotKept::Because(why)), Some(at))
                if !notes[at].detail.contains(&format!("`{}`", secret.name)) =>
            {
                notes[at].detail = format!(
                    "the value is not written to the file; it needs the secret variable `{}` \
                     ({why})",
                    secret.name
                );
            }
            (Err(NotKept::Because(why)), Some(at)) => {
                notes[at].detail.push_str(&format!(" ({why})"))
            }
            (Err(NotKept::Because(why)), None) => notes.push(ImportNote::new(
                secret.source.clone(),
                NoteKind::NeedsInput,
                format!("the secret `{}` was not stored ({why})", secret.name),
            )),
        }
    }
    super::unstored::fold(notes);
}

/// Why an imported secret went nowhere: plain words, or the store's own failure.
enum NotKept {
    Because(String),
    Store(SecretError),
}

impl From<String> for NotKept {
    fn from(why: String) -> NotKept {
        NotKept::Because(why)
    }
}

/// The environment the secret went to, or why it went nowhere.
fn keep_secret(
    collection: &mut Collection,
    name: &str,
    value: &str,
    prefer: Option<&str>,
    store: &dyn SecretStore,
) -> Result<String, NotKept> {
    let env = target_environment(collection, prefer)?;
    let environment = collection
        .environment_mut(&env)
        .ok_or_else(|| NotKept::Because("the environment is gone".to_owned()))?;
    let before = environment.variables();
    let mut variables = before.clone();
    match variables.iter().find(|v| v.name == name) {
        Some(existing) if !existing.secret => {
            return Err(format!("`{name}` is a plain variable of `{env}`").into());
        }
        Some(_) => {}
        None => {
            variables.push(EnvVariable::secret(name));
            environment.set_variables(variables);
            // A failed save leaves memory as it was, so it never disagrees with the disk.
            if let Err(e) = environment.save() {
                environment.set_variables(before);
                return Err(e.to_string().into());
            }
        }
    }
    // A value is stored only where none is: a secret another request uses keeps its own. A
    // refusing store leaves the variable declared without a value; the note tells the user to
    // enter it.
    let key = collection.secret_key(&env, name);
    match store.get(&key).map_err(NotKept::Store)? {
        Some(stored) if stored == value => {}
        Some(_) => return Err(format!("`{name}` already holds a different value").into()),
        None => store.set(&key, value).map_err(NotKept::Store)?,
    }
    super::protect::default_to(collection, &env);
    Ok(env)
}

/// The environment a secret is kept in: `prefer`, else the only one; Err says in plain words
/// why there is none.
pub fn secret_environment(collection: &Collection, prefer: Option<&str>) -> Result<String, String> {
    target_environment(collection, prefer)
}

/// The environment an imported secret goes to: `prefer`, else the only one.
fn target_environment(collection: &Collection, prefer: Option<&str>) -> Result<String, String> {
    // A name the caller gave is never swapped for another environment: a secret that lands
    // somewhere unintended is worse than one that waits for input.
    match prefer {
        Some(p) if collection.environment(p).is_some() => Ok(p.to_owned()),
        Some(p) => Err(collection.environment_not_found(p).to_string()),
        None => match collection.environments() {
            [only] => Ok(only.name().to_owned()),
            [] => Err("the collection has no environment".to_owned()),
            _ => Err("the collection has several environments".to_owned()),
        },
    }
}

impl ImportedRequest {
    /// A request with no params, headers, body, docs, vars or tags, inheriting its auth and
    /// keeping the file's settings.
    pub fn new(method: impl Into<String>, url: impl Into<String>) -> ImportedRequest {
        ImportedRequest {
            kind: RequestKind::Http,
            method: method.into(),
            url: url.into(),
            params: Vec::new(),
            headers: Vec::new(),
            body: BodySpec::None,
            auth: AuthSpec::Inherit,
            docs: String::new(),
            settings: None,
            omitted_headers: Vec::new(),
            vars: Vec::new(),
            post_response_vars: Vec::new(),
            tags: Vec::new(),
            scripts: ImportedScripts::default(),
            secrets: Vec::new(),
            examples: Vec::new(),
        }
    }

    /// Write everything but the name and sequence number into `file`, in memory. The URL and
    /// params are written as given, not derived from each other. A header the format cannot
    /// hold is [`EditError::InvalidHeaderName`], and nothing after the headers is written. Empty
    /// docs, vars, tags, post-response variables or examples leave the file's own, so applying to a draft never clears
    /// them. A GraphQL file keeps its query and variables against an HTTP import; the returned
    /// notes say what was left out.
    pub fn apply_to(&self, file: &mut RequestFile) -> Result<Vec<ImportNote>, EditError> {
        let notes = self.notes_on(file.kind());
        file.set_method(self.method.as_str());
        file.set_url_and_params(self.url.clone(), &self.params);
        file.set_headers(self.headers.clone())?;
        if !keeps_body(file.kind(), self.kind) {
            file.set_body(self.body.clone());
        }
        file.set_auth(self.auth.clone());
        if !self.docs.is_empty() {
            file.set_docs(self.docs.as_str());
        }
        if let Some(settings) = self.settings {
            file.set_settings(settings);
        }
        if !self.omitted_headers.is_empty() {
            file.set_omitted_headers(self.omitted_headers.clone());
        }
        if !self.vars.is_empty() {
            file.set_vars(self.vars.clone());
        }
        if !self.tags.is_empty() {
            file.set_tags(self.tags.clone());
        }
        if !self.post_response_vars.is_empty() {
            file.set_post_response_vars(self.post_response_vars.clone());
        }
        self.scripts
            .write_with(|phase, code| file.set_script(phase, code));
        if !self.examples.is_empty() && file.kind() == RequestKind::Http {
            super::update::write_examples(file, self)?;
        }
        Ok(notes)
    }

    /// Removes the Content-Type headers when each one only names the type the body mode
    /// sends anyway, so a later change of body mode is not overridden by a leftover header.
    pub fn drop_restated_content_type(&mut self) {
        let is_type = |h: &KeyValue| h.name.eq_ignore_ascii_case("content-type");
        let restated = |h: &KeyValue| {
            !is_type(h) || crate::content_type_conflict::restates_body_type(&h.value, &self.body)
        };
        if self.headers.iter().any(is_type) && self.headers.iter().all(restated) {
            self.headers.retain(|h| !is_type(h));
        }
    }

    /// This request as a GraphQL request when its body is a GraphQL query: a JSON `query` and
    /// `variables` payload or an `application/graphql` body. The raw-query Content-Type goes,
    /// since a GraphQL request sends JSON. `None` for any other body.
    pub fn as_graphql(&self) -> Option<ImportedRequest> {
        if self.kind == RequestKind::Graphql {
            return Some(self.clone());
        }
        let content_type = crate::kind_convert::content_type(&self.headers);
        let (query, variables) = crate::kind_convert::graphql_payload(&self.body, content_type)?;
        let raw_query = crate::kind_convert::is_graphql_type(content_type);
        let mut graphql = self.clone();
        graphql.kind = RequestKind::Graphql;
        graphql.body = BodySpec::Graphql { query, variables };
        if raw_query {
            graphql
                .headers
                .retain(|h| !h.name.eq_ignore_ascii_case("content-type"));
        }
        Some(graphql)
    }

    /// What applying this request to a request of `kind` leaves out: a body pasted into a
    /// GraphQL request is not applied.
    pub fn notes_on(&self, kind: RequestKind) -> Vec<ImportNote> {
        (keeps_body(kind, self.kind) && self.body != BodySpec::None)
            .then(|| {
                ImportNote::new(
                    "body",
                    NoteKind::Dropped,
                    "the pasted body was not applied; a GraphQL request keeps its query and \
                     variables",
                )
            })
            .into_iter()
            .collect()
    }
}

// An HTTP import has no query to give a GraphQL request, so that request's body stays.
fn keeps_body(target: RequestKind, source: RequestKind) -> bool {
    target == RequestKind::Graphql && source != RequestKind::Graphql
}

/// A GraphQL query and variables from `{query, variables}`, or from its JSON text: anything
/// unreadable leaves both empty. Non-text variables become indented JSON.
pub(super) fn graphql_parts(source: Option<&Value>) -> (String, String) {
    let parsed;
    let source = match source {
        Some(Value::String(text)) => match serde_json::from_str::<Value>(text) {
            Ok(value) => {
                parsed = value;
                Some(&parsed)
            }
            Err(_) => None,
        },
        other => other,
    };
    let member = |key: &str| source.and_then(|s| s.get(key));
    let query = match member("query") {
        Some(Value::String(q)) => q.clone(),
        _ => String::new(),
    };
    let variables = match member("variables") {
        Some(Value::String(v)) => v.clone(),
        None | Some(Value::Null) => String::new(),
        Some(other) => js::stringify_pretty(other),
    };
    (query, variables)
}
