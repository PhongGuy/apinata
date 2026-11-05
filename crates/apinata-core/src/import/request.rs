//! Writing an imported request into a request file.

use super::{ImportError, ImportNote, ImportedRequest, ImportedScripts, NoteKind};
use crate::collection::Collection;
use crate::edit::{AuthSpec, BodySpec};
use crate::env_file::EnvVariable;
use crate::error::{EditError, TreeError};
use crate::item_path::ItemPath;
use crate::request::{RequestFile, RequestKind};
use crate::secrets::SecretStore;

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

/// Keep each secret of `request` as a secret variable of one environment, with its value in
/// `store`. The environment is `prefer`; a named one that does not exist stores nothing. With no
/// `prefer` it is the only environment; with none to choose, when the variable name is taken by
/// a plain variable, or when the store refuses, the secret's `NeedsInput` note stays in `notes` and says why. On success that note becomes a
/// `Converted` one naming the environment. A value already stored for the variable is replaced.
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
            (Err(why), Some(at)) => notes[at].detail.push_str(&format!(" ({why})")),
            (Err(why), None) => notes.push(ImportNote::new(
                secret.source.clone(),
                NoteKind::NeedsInput,
                format!("the secret `{}` was not stored ({why})", secret.name),
            )),
        }
    }
}

/// The environment the secret went to, or why it went nowhere.
fn keep_secret(
    collection: &mut Collection,
    name: &str,
    value: &str,
    prefer: Option<&str>,
    store: &dyn SecretStore,
) -> Result<String, String> {
    // A name the caller gave is never swapped for another environment: a secret that lands
    // somewhere unintended is worse than one that waits for input.
    let env = match prefer {
        Some(p) if collection.environment(p).is_some() => p.to_owned(),
        Some(p) => return Err(collection.environment_not_found(p).to_string()),
        None => match collection.environments() {
            [only] => only.name().to_owned(),
            [] => return Err("the collection has no environment".to_owned()),
            _ => return Err("the collection has several environments".to_owned()),
        },
    };
    let environment = collection
        .environment_mut(&env)
        .ok_or_else(|| "the environment is gone".to_owned())?;
    let before = environment.variables();
    let mut variables = before.clone();
    match variables.iter().find(|v| v.name == name) {
        Some(existing) if !existing.secret => {
            return Err(format!("`{name}` is a plain variable of `{env}`"));
        }
        Some(_) => {}
        None => {
            variables.push(EnvVariable::secret(name));
            environment.set_variables(variables);
            // A failed save leaves memory as it was, so it never disagrees with the disk.
            if let Err(e) = environment.save() {
                environment.set_variables(before);
                return Err(e.to_string());
            }
        }
    }
    // A refusing store leaves the variable declared without a value; the note tells the
    // user to enter it.
    store
        .set(&collection.secret_key(&env, name), value)
        .map_err(|e| e.to_string())?;
    Ok(env)
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
            tags: Vec::new(),
            scripts: ImportedScripts::default(),
            secrets: Vec::new(),
            examples: Vec::new(),
        }
    }

    /// Write everything but the name and sequence number into `file`, in memory. The URL and
    /// params are written as given, not derived from each other. A header the format cannot
    /// hold is [`EditError::InvalidHeaderName`], and nothing after the headers is written. Empty
    /// docs, vars, tags or examples leave the file's own, so applying to a draft never clears
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
        self.scripts
            .write_with(|phase, code| file.set_script(phase, code));
        if !self.examples.is_empty() && file.kind() == RequestKind::Http {
            super::update::write_examples(file, self)?;
        }
        Ok(notes)
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
