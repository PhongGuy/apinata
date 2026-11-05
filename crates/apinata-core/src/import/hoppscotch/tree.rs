//! Collections and folders to the import tree, and each request's own members.

use super::model::{self, Collection, GqlRequest, Pair, Request, Text, text};
use super::template::rewrite;
use super::{REQUEST_VERSION, RequestKind, collection_version_note};
use super::{auth, body};
use crate::edit::{AuthSpec, BodySpec, KeyValue, Param, ParamKind, VarValue, Variable};
use crate::import::{
    DocumentError, ImportNote, ImportedCollection, ImportedFolder, ImportedItem, ImportedRequest,
    ImportedRequestItem, NoteKind,
};
use crate::url_params::build_query_string;
use serde_json::{Map, Value};

/// A root collection, whose `folders` and `requests` lists the caller has checked.
pub(super) fn root(value: Value, budget: &mut Budget) -> Result<ImportedCollection, DocumentError> {
    let version = collection_version_note(value.get("v"));
    let mut c: Collection = serde_json::from_value(value).map_err(|e| DocumentError::Invalid {
        message: format!("not a Hoppscotch collection: {e}"),
    })?;
    let (folders, requests, name) = (c.folders.take(), c.requests.take(), name(c.name.take()));
    let (own, notes) = own_members(c, version, &[], budget)?;
    let mut unwritten = Vec::new();
    let items = children(folders, requests, &mut Vec::new(), &mut unwritten, budget)?;
    Ok(ImportedCollection {
        scripts: Default::default(),
        name,
        docs: own.docs,
        auth: match own.auth {
            AuthSpec::Inherit => AuthSpec::None,
            other => other,
        },
        headers: own.headers,
        vars: own.vars,
        items,
        notes,
        unwritten,
        spec_source: false,
    })
}

/// Folders first, then requests, each in input order; `seq` counts only the written ones.
fn children(
    folders: Option<Vec<Value>>,
    requests: Option<Vec<Value>>,
    trail: &mut Vec<String>,
    unwritten: &mut Vec<ImportNote>,
    budget: &mut Budget,
) -> Result<Vec<ImportedItem>, DocumentError> {
    let mut items = Vec::new();
    for (i, value) in folders.unwrap_or_default().into_iter().enumerate() {
        let seq = items.len() as u64 + 1;
        if !value.is_object() {
            let at = format!("folders[{i}]");
            let detail = "not a folder object; not imported";
            unwritten.push(budget.member(trail, &at, NoteKind::Dropped, detail)?);
        } else if let Some(f) = folder(value, seq, trail, unwritten, budget)? {
            items.push(ImportedItem::Folder(f));
        }
    }
    for (i, value) in requests.unwrap_or_default().into_iter().enumerate() {
        let seq = items.len() as u64 + 1;
        if !value.is_object() {
            let at = format!("requests[{i}]");
            let detail = "not a request object; not imported";
            unwritten.push(budget.member(trail, &at, NoteKind::Dropped, detail)?);
        } else if let Some(r) = request(value, seq, trail, unwritten, budget)? {
            items.push(ImportedItem::Request(r));
        }
    }
    Ok(items)
}

/// A folder, or `None` with an unwritten note when its members have the wrong types. The
/// recursion is bounded by the loader's nesting limit.
fn folder(
    value: Value,
    seq: u64,
    trail: &mut Vec<String>,
    unwritten: &mut Vec<ImportNote>,
    budget: &mut Budget,
) -> Result<Option<ImportedFolder>, DocumentError> {
    let own = raw_name(&value);
    let version = collection_version_note(value.get("v"));
    let mut c: Collection = match serde_json::from_value(value) {
        Ok(c) => c,
        Err(e) => {
            let detail = format!("not imported: {e}");
            unwritten.push(budget.note(trail, &[&own], NoteKind::Dropped, detail)?);
            return Ok(None);
        }
    };
    budget.entry()?;
    let (folders, requests, name) = (c.folders.take(), c.requests.take(), name(c.name.take()));
    trail.push(name.clone());
    let folder = own_members(c, version, trail, budget).and_then(|(own, notes)| {
        let items = children(folders, requests, trail, unwritten, budget)?;
        Ok(ImportedFolder {
            scripts: Default::default(),
            name,
            seq,
            docs: own.docs,
            auth: own.auth,
            headers: own.headers,
            vars: own.vars,
            items,
            notes,
        })
    });
    trail.pop();
    folder.map(Some)
}

/// A collection's or folder's own members and notes, its version note first.
fn own_members(
    c: Collection,
    version: Option<String>,
    trail: &[String],
    budget: &mut Budget,
) -> Result<(Members, Vec<ImportNote>), DocumentError> {
    let mut scope = Scope::new(budget, trail);
    if let Some(detail) = version {
        scope.note("v", NoteKind::Converted, detail)?;
    }
    let own = members(c, &mut scope)?;
    Ok((own, scope.notes))
}

/// The members a collection and a folder share, apart from their children and name.
struct Members {
    docs: String,
    auth: AuthSpec,
    headers: Vec<KeyValue>,
    vars: Vec<Variable>,
}

fn members(c: Collection, scope: &mut Scope) -> Result<Members, DocumentError> {
    scope.rest(c.rest, String::new)?;
    let headers = pairs(c.headers, "headers", scope)?;
    let vars = collection_vars(c.variables, scope)?;
    let auth = auth::auth(c.auth, auth::Legacy::default(), scope)?;
    scope.script("preRequestScript", c.pre_request_script)?;
    scope.script("testScript", c.test_script)?;
    Ok(Members {
        docs: text(c.description),
        auth,
        headers,
        vars,
    })
}

/// Collection and folder variables take the initial value, else the current one; a secret is
/// written without its value and needs input.
fn collection_vars(
    vars: Option<Vec<model::Variable>>,
    scope: &mut Scope,
) -> Result<Vec<Variable>, DocumentError> {
    let mut out = Vec::new();
    for (i, v) in vars.unwrap_or_default().into_iter().enumerate() {
        let name = text(v.key);
        if name.is_empty() {
            let detail = "the variable has no name; not imported";
            scope.note(&format!("variables[{i}]"), NoteKind::Dropped, detail)?;
            continue;
        }
        scope.budget.entry()?;
        scope.rest(v.rest, || format!("variables[{i}]."))?;
        let value = if v.secret == Some(true) {
            let detail = "a secret variable; its value is not imported, so supply it";
            scope.note(&format!("variables[{i}]"), NoteKind::NeedsInput, detail)?;
            String::new()
        } else {
            let initial = text(v.initial_value);
            let current = text(v.current_value);
            let (value, member) = if initial.is_empty() {
                (current, "currentValue")
            } else {
                if !current.is_empty() && current != initial {
                    let at = format!("variables[{i}].currentValue");
                    let detail =
                        "the current value differs from the initial value and is not imported";
                    scope.note(&at, NoteKind::Dropped, detail)?;
                }
                (initial, "initialValue")
            };
            scope.value(value, || format!("variables[{i}].{member}"))?
        };
        out.push(Variable::new(name, VarValue::Text(value)));
    }
    Ok(out)
}

/// A REST or GraphQL request, or `None` with an unwritten note for an unknown version or
/// members of the wrong types.
fn request(
    value: Value,
    seq: u64,
    trail: &mut Vec<String>,
    unwritten: &mut Vec<ImportNote>,
    budget: &mut Budget,
) -> Result<Option<ImportedRequestItem>, DocumentError> {
    let own = raw_name(&value);
    let kind = RequestKind::of(&value);
    let newer = match kind {
        RequestKind::GraphQl => return graphql_request(value, own, seq, trail, unwritten, budget),
        RequestKind::Unknown => {
            let v = value.get("v").map(Value::to_string).unwrap_or_default();
            let detail = format!("version {v} is not a request version; not imported");
            unwritten.push(budget.note(trail, &[&own], NoteKind::Dropped, detail)?);
            return Ok(None);
        }
        RequestKind::Legacy => false,
        RequestKind::Rest { newer } => newer,
    };
    let r: Request = match serde_json::from_value(value) {
        Ok(r) => r,
        Err(e) => {
            let detail = format!("not imported: {e}");
            unwritten.push(budget.note(trail, &[&own], NoteKind::Dropped, detail)?);
            return Ok(None);
        }
    };
    budget.entry()?;
    trail.push(own);
    let mut scope = Scope::new(budget, trail);
    let request = request_members(r, kind, newer, &mut scope);
    let notes = scope.notes;
    let name = trail.pop().unwrap_or_default();
    Ok(Some(ImportedRequestItem {
        name,
        seq,
        request: request?,
        notes,
    }))
}

/// A GraphQL request, or `None` with an unwritten note when its members have the wrong types.
fn graphql_request(
    value: Value,
    own: String,
    seq: u64,
    trail: &mut Vec<String>,
    unwritten: &mut Vec<ImportNote>,
    budget: &mut Budget,
) -> Result<Option<ImportedRequestItem>, DocumentError> {
    let r: GqlRequest = match serde_json::from_value(value) {
        Ok(r) => r,
        Err(e) => {
            let detail = format!("not imported: {e}");
            unwritten.push(budget.note(trail, &[&own], NoteKind::Dropped, detail)?);
            return Ok(None);
        }
    };
    budget.entry()?;
    trail.push(own);
    let mut scope = Scope::new(budget, trail);
    let request = graphql_members(r, &mut scope);
    let notes = scope.notes;
    let name = trail.pop().unwrap_or_default();
    Ok(Some(ImportedRequestItem {
        name,
        seq,
        request: request?,
        notes,
    }))
}

/// A GraphQL request's own members. Auth missing before version 2 means none, as its
/// migration sets it.
fn graphql_members(r: GqlRequest, scope: &mut Scope) -> Result<ImportedRequest, DocumentError> {
    scope.rest(r.rest, String::new)?;
    let url = scope.value(text(r.url), || "url".to_owned())?;
    let headers = pairs(r.headers, "headers", scope)?;
    let query = scope.value(text(r.query), || "query".to_owned())?;
    let variables = scope.value(text(r.variables), || "variables".to_owned())?;
    let auth = match r.auth {
        None => AuthSpec::None,
        member => auth::auth(member, auth::Legacy::default(), scope)?,
    };
    scope.script("preRequestScript", r.pre_request_script)?;
    scope.script("testScript", r.test_script)?;
    if r.responses.as_ref().is_some_and(|v| !is_empty(v)) {
        let detail = "saved responses are not imported";
        scope.note("responses", NoteKind::Dropped, detail)?;
    }
    let mut request = ImportedRequest::new("POST", url);
    request.kind = crate::request::RequestKind::Graphql;
    request.headers = headers;
    request.body = BodySpec::Graphql { query, variables };
    request.auth = auth;
    request.docs = text(r.description);
    Ok(request)
}

/// A request's own members, with notes at `<trail> / <member>`.
fn request_members(
    r: Request,
    kind: RequestKind,
    newer: bool,
    scope: &mut Scope,
) -> Result<ImportedRequest, DocumentError> {
    if newer {
        let detail = format!("a newer request version; read as version {REQUEST_VERSION}");
        scope.note("v", NoteKind::Converted, detail)?;
    }
    scope.rest(r.rest, String::new)?;
    let params = pairs(r.params, "params", scope)?;
    let params: Vec<Param> = params.into_iter().map(to_param).collect();
    let base = match kind {
        // A legacy request carrying a later `endpoint` falls back to it when it has no URL of
        // its own; otherwise the unused endpoint is noted rather than lost silently.
        RequestKind::Legacy => {
            let joined = text(r.url) + &text(r.path);
            let endpoint = text(r.endpoint);
            if joined.is_empty() && !endpoint.is_empty() {
                scope.value(endpoint, || "endpoint".to_owned())?
            } else {
                if !endpoint.is_empty() {
                    let detail = "a legacy request's URL is `url` and `path`; not imported";
                    scope.note("endpoint", NoteKind::Dropped, detail)?;
                }
                scope.value(joined, || "url".to_owned())?
            }
        }
        _ => scope.value(text(r.endpoint), || "endpoint".to_owned())?,
    };
    let headers = pairs(r.headers, "headers", scope)?;
    let vars = pairs(r.request_variables, "requestVariables", scope)?;
    let legacy_body = body::Legacy {
        content_type: text(r.content_type),
        raw_params: r.raw_params.map(|t| t.0),
    };
    let body = body::body(r.body, legacy_body, scope)?;
    if r.body_params.as_ref().is_some_and(|v| !is_empty(v)) {
        let detail = "legacy form params are not imported";
        scope.note("bodyParams", NoteKind::Dropped, detail)?;
    }
    // A legacy request without auth sends none; a later one inherits.
    let auth = match (kind, r.auth) {
        (RequestKind::Legacy, None) => AuthSpec::None,
        (_, member) => {
            let legacy = auth::Legacy {
                user: text(r.http_user),
                password: text(r.http_password),
                token: text(r.bearer_token),
            };
            auth::auth(member, legacy, scope)?
        }
    };
    scope.script("preRequestScript", r.pre_request_script)?;
    scope.script("testScript", r.test_script)?;
    if r.responses.as_ref().is_some_and(|v| !is_empty(v)) {
        let detail = "saved responses are not imported";
        scope.note("responses", NoteKind::Dropped, detail)?;
    }
    Ok(ImportedRequest {
        scripts: Default::default(),
        kind: crate::request::RequestKind::Http,
        method: r.method.map_or_else(|| "GET".to_owned(), |m| m.0),
        url: with_query(base, &params),
        params,
        headers,
        body,
        auth,
        docs: text(r.description),
        settings: None,
        omitted_headers: Vec::new(),
        vars: vars.into_iter().map(to_variable).collect(),
        tags: Vec::new(),
        secrets: Vec::new(),
        examples: Vec::new(),
    })
}

/// Params, headers or request variables in order, `active` as enabled; names stay verbatim
/// and values have their templates rewritten.
fn pairs(
    pairs: Option<Vec<Pair>>,
    member: &str,
    scope: &mut Scope,
) -> Result<Vec<KeyValue>, DocumentError> {
    let mut out = Vec::new();
    for (i, p) in pairs.unwrap_or_default().into_iter().enumerate() {
        scope.budget.entry()?;
        scope.rest(p.rest, || format!("{member}[{i}]."))?;
        let value = scope.value(text(p.value), || format!("{member}[{i}].value"))?;
        let mut entry = KeyValue::new(text(p.key), value);
        entry.enabled = p.active.unwrap_or(true);
        entry.description = description(p.description);
        out.push(entry);
    }
    Ok(out)
}

fn to_param(entry: KeyValue) -> Param {
    let mut param = Param::new(entry.name, entry.value, ParamKind::Query);
    param.enabled = entry.enabled;
    param.description = entry.description;
    param
}

fn to_variable(entry: KeyValue) -> Variable {
    let mut var = Variable::new(entry.name, VarValue::Text(entry.value));
    var.enabled = entry.enabled;
    var
}

/// Whether a member holds nothing: `null`, or an empty string, list or object.
fn is_empty(value: &Value) -> bool {
    match value {
        Value::Null => true,
        Value::String(s) => s.is_empty(),
        Value::Array(a) => a.is_empty(),
        Value::Object(o) => o.is_empty(),
        _ => false,
    }
}

/// The notes of one collection, folder or request, sourced at its `trail`.
pub(super) struct Scope<'a> {
    budget: &'a mut Budget,
    trail: &'a [String],
    notes: Vec<ImportNote>,
}

impl<'a> Scope<'a> {
    fn new(budget: &'a mut Budget, trail: &'a [String]) -> Scope<'a> {
        Scope {
            budget,
            trail,
            notes: Vec::new(),
        }
    }

    pub(super) fn note(
        &mut self,
        path: &str,
        kind: NoteKind,
        detail: impl Into<String>,
    ) -> Result<(), DocumentError> {
        let note = self.budget.member(self.trail, path, kind, detail)?;
        self.notes.push(note);
        Ok(())
    }

    /// `text` with its templates rewritten; dynamic variables core does not generate add one
    /// note at the path `at` builds.
    pub(super) fn value(
        &mut self,
        text: String,
        at: impl FnOnce() -> String,
    ) -> Result<String, DocumentError> {
        let (text, dynamic) = rewrite(&text);
        if !dynamic.is_empty() {
            let names = dynamic.join(", ");
            let detail = format!("uses the generated variables {names}; give each a value");
            self.note(&at(), NoteKind::NeedsInput, detail)?;
        }
        Ok(text)
    }

    /// A dropped note for each member no field declares, its path `prefix` then the key.
    pub(super) fn rest(
        &mut self,
        rest: Map<String, Value>,
        prefix: impl Fn() -> String,
    ) -> Result<(), DocumentError> {
        for key in rest.keys() {
            let path = prefix() + key;
            self.note(&path, NoteKind::Dropped, "not imported")?;
        }
        Ok(())
    }

    /// Charge one emitted entry, such as a form field.
    pub(super) fn entry(&mut self) -> Result<(), DocumentError> {
        self.budget.entry()
    }

    fn script(&mut self, member: &str, script: Option<Text>) -> Result<(), DocumentError> {
        if !text(script).trim().is_empty() {
            self.note(member, NoteKind::Dropped, "scripts are not imported")?;
        }
        Ok(())
    }
}

/// `url` with the enabled params added to its query, before any fragment: Hoppscotch keeps
/// them out of the endpoint, and the URL is sent as written.
fn with_query(mut url: String, params: &[Param]) -> String {
    let enabled = params.iter().filter(|p| p.enabled);
    let query = build_query_string(enabled.map(|p| (p.name.as_str(), p.value.as_str())));
    if query.is_empty() {
        return url;
    }
    let fragment = url
        .find('#')
        .map(|at| url.split_off(at))
        .unwrap_or_default();
    if !url.contains('?') {
        url.push('?');
    } else if !url.ends_with(['?', '&']) {
        url.push('&');
    }
    url.push_str(&query);
    url + &fragment
}

fn description(member: Option<Text>) -> Option<String> {
    Some(text(member)).filter(|d| !d.trim().is_empty())
}

fn name(member: Option<Text>) -> String {
    member.map_or_else(|| "Untitled".to_owned(), |n| n.0)
}

/// The name of an entry that may not deserialise, for the source of its note.
fn raw_name(value: &Value) -> String {
    let name = value.get("name").filter(|n| !n.is_null());
    name.and_then(|n| serde_json::from_value::<Text>(n.clone()).ok())
        .map_or_else(|| "Untitled".to_owned(), |n| n.0)
}

pub(super) use crate::import::budget::Budget;

/// Notes whose source is an ancestor trail plus tail names.
pub(super) trait Crumbs {
    /// A note sourced from the `trail` and `tail` names joined by ` / `, charged before its
    /// source is built.
    fn note(
        &mut self,
        trail: &[String],
        tail: &[&str],
        kind: NoteKind,
        detail: impl Into<String>,
    ) -> Result<ImportNote, DocumentError>;

    /// A note on a member of the collection or folder at `trail`; the root's members are
    /// sourced `collection / <path>`.
    fn member(
        &mut self,
        trail: &[String],
        path: &str,
        kind: NoteKind,
        detail: impl Into<String>,
    ) -> Result<ImportNote, DocumentError>;
}

impl Crumbs for Budget {
    fn note(
        &mut self,
        trail: &[String],
        tail: &[&str],
        kind: NoteKind,
        detail: impl Into<String>,
    ) -> Result<ImportNote, DocumentError> {
        let mut pieces: Vec<&str> = Vec::new();
        for (i, part) in trail
            .iter()
            .map(String::as_str)
            .chain(tail.iter().copied())
            .enumerate()
        {
            if i > 0 {
                pieces.push(" / ");
            }
            pieces.push(part);
        }
        self.note_from(&pieces, kind, detail)
    }

    fn member(
        &mut self,
        trail: &[String],
        path: &str,
        kind: NoteKind,
        detail: impl Into<String>,
    ) -> Result<ImportNote, DocumentError> {
        let tail: &[&str] = if trail.is_empty() {
            &["collection", path]
        } else {
            &[path]
        };
        Crumbs::note(self, trail, tail, kind, detail)
    }
}
