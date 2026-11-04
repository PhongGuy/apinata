//! Export to an OpenAPI 3.1 JSON document: one operation per HTTP request, keyed by the
//! request URL's templated path, with a note for everything the document has no place for.

mod content;
mod responses;
mod security;
mod servers;
mod url;

use super::postman::insert_description;
use super::{ExportNote, OpenApiExport};
use crate::AuthSpec;
use crate::Collection;
use crate::collection::Folder;
use crate::js::script_code;
use crate::request::RequestFile;
use crate::script::ScriptPhase;
use crate::sequence::sort_by_name_then_seq;
use security::{Mapped, Schemes};
use serde_json::{Map, Value, json};
use std::collections::{HashMap, HashSet};

/// The methods an OpenAPI 3.1 path item has a field for.
const METHODS: [&str; 8] = [
    "get", "put", "post", "delete", "options", "head", "patch", "trace",
];

/// The members counted into one `collection / <kind>` note each, in note order.
const KINDS: [&str; 5] = ["scripts", "tests", "assertions", "variables", "headers"];
const SCRIPTS: usize = 0;
const TESTS: usize = 1;
const ASSERTIONS: usize = 2;
const VARIABLES: usize = 3;
const HEADERS: usize = 4;

/// The OpenAPI 3.1 document for `collection`, with notes on everything not carried over.
pub fn openapi(collection: &Collection) -> OpenApiExport {
    let mut info = Map::new();
    info.insert("title".into(), collection.name().into());
    info.insert("version".into(), "1.0.0".into());
    insert_description(&mut info, collection.file().docs());
    let mut walker = Walker {
        collection,
        notes: Vec::new(),
        paths: Map::new(),
        seen: HashMap::new(),
        keys: HashMap::new(),
        ids: HashSet::new(),
        tags: Map::new(),
        servers: HashMap::new(),
        first_origin: None,
        schemes: Schemes::default(),
        top: None,
        counts: [0; KINDS.len()],
    };
    let file = collection.file();
    walker.count_scripts(|phase| file.script(phase));
    walker.count(HEADERS, !file.headers().is_empty());
    // The collection's own auth is the document-wide requirement and names the first scheme.
    for detail in security::credential_url_notes(&collection.file().auth()) {
        walker
            .notes
            .push(ExportNote::new("collection / auth", detail));
    }
    match security::map_auth(&collection.file().auth()) {
        Mapped::Scheme {
            scheme,
            base,
            scopes,
        } => walker.top = Some(json!({ walker.schemes.name(scheme, base): scopes })),
        Mapped::None => {}
        Mapped::Unmappable(detail) => walker
            .notes
            .push(ExportNote::new("collection / auth", detail)),
    }
    walker.folder(collection.tree(), &[], None);
    for (kind, n) in KINDS.iter().zip(walker.counts) {
        let detail = match n {
            0 => continue,
            1 => format!("1 item has {kind}; OpenAPI has no place for them"),
            n => format!("{n} items have {kind}; OpenAPI has no place for them"),
        };
        walker
            .notes
            .push(ExportNote::new(format!("collection / {kind}"), detail));
    }
    let mut root = Map::new();
    root.insert("openapi".into(), "3.1.0".into());
    root.insert("info".into(), info.into());
    if let Some(origin) = &walker.first_origin {
        root.insert("servers".into(), walker.servers[origin].clone().into());
    }
    if !walker.tags.is_empty() {
        root.insert("tags".into(), walker.tags.into_values().collect());
    }
    root.insert("paths".into(), walker.paths.into());
    if let Some(components) = walker.schemes.into_components() {
        root.insert("components".into(), components);
    }
    if let Some(top) = walker.top {
        root.insert("security".into(), json!([top]));
    }
    let mut json =
        serde_json::to_string_pretty(&Value::Object(root)).expect("a JSON value serializes");
    json.push('\n');
    OpenApiExport {
        json,
        notes: walker.notes,
    }
}

/// State gathered over one tree walk; later document parts add fields here.
struct Walker<'a> {
    collection: &'a Collection,
    notes: Vec<ExportNote>,
    paths: Map<String, Value>,
    /// Method plus path with template names blanked, to the first request's source.
    seen: HashMap<String, String>,
    /// Path with template names blanked, to the first-seen path text, which is its key.
    keys: HashMap<String, String>,
    ids: HashSet<String>,
    /// Top-level folder name to its tag object, first-seen order.
    tags: Map<String, Value>,
    /// Origin as written to its server list, computed once per origin.
    servers: HashMap<String, Vec<Value>>,
    /// The first exported operation's origin, whose servers are the document's.
    first_origin: Option<String>,
    schemes: Schemes,
    /// The collection auth's requirement object, when it maps to a scheme.
    top: Option<Value>,
    /// Per `KINDS` entry, how many items carry that member.
    counts: [usize; KINDS.len()],
}

impl Walker<'_> {
    fn count(&mut self, kind: usize, has: bool) {
        self.counts[kind] += usize::from(has);
    }

    /// Counts an item whose pre-request or post-response script, or tests script, is not blank.
    fn count_scripts(&mut self, script: impl Fn(ScriptPhase) -> String) {
        let filled = |phase| !script_code(&script(phase)).is_empty();
        let scripts = filled(ScriptPhase::PreRequest) || filled(ScriptPhase::PostResponse);
        self.count(SCRIPTS, scripts);
        self.count(TESTS, filled(ScriptPhase::Tests));
    }

    /// Sub-folders in display order, then requests, then a note per non-HTTP item. `tag` is
    /// the enclosing top-level folder, whose tag is added once an operation uses it.
    fn folder(&mut self, folder: &Folder, trail: &[&str], tag: Option<&Folder>) {
        if let Some(file) = folder.file().filter(|_| !trail.is_empty()) {
            let source = format!("{} / auth", trail.join(" / "));
            for detail in security::credential_url_notes(&file.auth()) {
                self.notes.push(ExportNote::new(source.clone(), detail));
            }
            if let Mapped::Unmappable(detail) = security::map_auth(&file.auth()) {
                self.notes.push(ExportNote::new(source, detail));
            }
            self.count_scripts(|phase| file.script(phase));
            let vars = !file.vars().is_empty() || !file.post_response_vars().is_empty();
            self.count(VARIABLES, vars);
            self.count(HEADERS, !file.headers().is_empty());
        }
        let folders =
            sort_by_name_then_seq(folder.folders().iter().collect(), |f| f.name(), |f| f.seq());
        for sub in folders {
            let path = [trail, &[sub.name()]].concat();
            self.folder(sub, &path, tag.or(Some(sub)));
        }
        for req in folder.requests() {
            let path = [trail, &[req.name()]].concat();
            self.request(req, &path, tag);
        }
        for opaque in folder.opaque_items() {
            let source = [trail, &[opaque.name()]].concat().join(" / ");
            let detail = format!(
                "only HTTP requests are exported; this {} item was left out",
                opaque.kind()
            );
            self.notes.push(ExportNote::new(source, detail));
        }
    }

    /// Adds `req` as an operation, or notes why it was left out: an unknown method, an
    /// inexpressible URL, or a method and path an earlier request already took.
    fn request(&mut self, req: &RequestFile, trail: &[&str], tag: Option<&Folder>) {
        // Every request counts toward the collection notes, whether or not it is exported.
        self.count_scripts(|phase| req.script(phase));
        self.count(ASSERTIONS, !req.assertions().is_empty());
        let vars = !req.vars().is_empty() || !req.post_response_vars().is_empty();
        self.count(VARIABLES, vars);
        let source = trail.join(" / ");
        let method = req.method().to_lowercase();
        let upper = req.method().to_uppercase();
        if !METHODS.contains(&method.as_str()) {
            let detail = format!(
                "OpenAPI 3.1 has no field for the {upper} method; the request was left out"
            );
            self.notes
                .push(ExportNote::new(format!("{source} / method"), detail));
            return;
        }
        let mut split = match url::split_url(req.url()) {
            Ok(split) => split,
            Err(reason) => {
                let detail = format!("{reason}; the request was left out");
                self.notes
                    .push(ExportNote::new(format!("{source} / url"), detail));
                return;
            }
        };
        let blank = blank_names(&split.path);
        let key = format!("{method} {blank}");
        if let Some(first) = self.seen.get(&key) {
            let detail = format!(
                "{upper} {} has the same method and path as {first}; this request was left out",
                split.path
            );
            self.notes.push(ExportNote::new(source, detail));
            return;
        }
        self.seen.insert(key, source.clone());
        // Paths equal but for template names are one path to OpenAPI; the first spelling wins.
        let path_key = self.keys.entry(blank).or_insert(split.path.clone()).clone();
        if let Some(origin) = url::strip_userinfo(&split.origin) {
            split.origin = origin;
            let detail = "the user name and password in the URL were left out";
            self.notes
                .push(ExportNote::new(format!("{source} / url"), detail));
        }

        let mut op = Map::new();
        if let Some(folder) = tag {
            op.insert("tags".into(), json!([folder.name()]));
            if !self.tags.contains_key(folder.name()) {
                let mut obj = Map::new();
                obj.insert("name".into(), folder.name().into());
                if let Some(file) = folder.file() {
                    insert_description(&mut obj, file.docs());
                }
                self.tags.insert(folder.name().into(), obj.into());
            }
        }
        op.insert("summary".into(), req.name().into());
        insert_description(&mut op, req.docs());
        op.insert("operationId".into(), self.operation_id(trail).into());
        // Operations on the document's origin inherit its servers; any other carries its own.
        let servers = self.servers_for(&split.origin);
        let first = self
            .first_origin
            .get_or_insert_with(|| split.origin.clone());
        if *first != split.origin {
            op.insert("servers".into(), servers.into());
        }
        let parameters = content::parameters(req, &path_key, &split.path, &source, &mut self.notes);
        if !parameters.is_empty() {
            op.insert("parameters".into(), parameters.into());
        }
        if let Some(body) = content::request_body(req, &source, &mut self.notes) {
            op.insert("requestBody".into(), body);
        }
        let responses = responses::responses(&req.examples(), &source, &mut self.notes);
        op.insert("responses".into(), responses);
        if let Some(security) = self.security(req, &source) {
            op.insert("security".into(), security);
        }
        let item = self
            .paths
            .entry(path_key)
            .or_insert_with(|| Map::new().into());
        item.as_object_mut()
            .expect("a path item is an object")
            .insert(method, op.into());
    }

    /// The operation's `security`: its effective auth's requirement when that differs from
    /// the document's, or an empty list to opt out of it. Only the request's own unmappable
    /// auth is noted here; inherited auth was noted where it is written.
    fn security(&mut self, req: &RequestFile, source: &str) -> Option<Value> {
        let mut auth = req.auth();
        let inherited = auth == AuthSpec::Inherit;
        if inherited {
            auth = self
                .collection
                .item_path(req.path())
                .map_or(AuthSpec::None, |item| {
                    self.collection.inherited_auth(&item).auth().clone()
                });
        }
        if !inherited {
            for detail in security::credential_url_notes(&auth) {
                self.notes
                    .push(ExportNote::new(format!("{source} / auth"), detail));
            }
        }
        let requirement = match security::map_auth(&auth) {
            Mapped::Scheme {
                scheme,
                base,
                scopes,
            } => json!({ self.schemes.name(scheme, base): scopes }),
            Mapped::Unmappable(detail) => {
                if !inherited {
                    self.notes
                        .push(ExportNote::new(format!("{source} / auth"), detail));
                }
                return self.top.as_ref().map(|_| json!([]));
            }
            Mapped::None => return self.top.as_ref().map(|_| json!([])),
        };
        (self.top.as_ref() != Some(&requirement)).then(|| json!([requirement]))
    }

    /// The server list for `origin`, built on first use.
    fn servers_for(&mut self, origin: &str) -> Vec<Value> {
        let collection = self.collection;
        self.servers
            .entry(origin.to_owned())
            .or_insert_with(|| servers::servers(collection, origin))
            .clone()
    }

    /// The lower camel case of the folder and request names, or `operation` when no word is
    /// left; a taken id gets the first free numeric suffix from 2.
    fn operation_id(&mut self, trail: &[&str]) -> String {
        let mut base = String::new();
        let words = trail
            .iter()
            .flat_map(|s| s.split(|c: char| !c.is_alphanumeric()))
            .filter(|w| !w.is_empty());
        for word in words {
            let word = word.to_lowercase();
            let mut chars = word.chars();
            if !base.is_empty()
                && let Some(first) = chars.next()
            {
                base.extend(first.to_uppercase());
                base.push_str(chars.as_str());
            } else {
                base.push_str(&word);
            }
        }
        if base.is_empty() {
            base = "operation".into();
        }
        let mut id = base.clone();
        let mut n = 2;
        while self.ids.contains(&id) {
            id = format!("{base}{n}");
            n += 1;
        }
        self.ids.insert(id.clone());
        id
    }
}

/// `path` with every `{name}` as `{}`, so templates differing only in names compare equal.
fn blank_names(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    let mut in_name = false;
    for c in path.chars() {
        match c {
            '{' => {
                in_name = true;
                out.push(c);
            }
            '}' => {
                in_name = false;
                out.push(c);
            }
            _ if !in_name => out.push(c),
            _ => {}
        }
    }
    out
}
