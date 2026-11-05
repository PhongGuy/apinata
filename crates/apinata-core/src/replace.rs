//! Replacing text across a collection's requests, through the same edit setters the editor
//! uses so unchanged bytes stay byte-identical.

use crate::collection::{Collection, Folder, Sibling};
use crate::edit::{BodySpec, KeyValue, MultipartValue, Param, ParamKind};
use crate::item_path::ItemPath;
use crate::request::RequestFile;
use crate::script::ScriptPhase;
use crate::search::{Matcher, SearchError, SearchField, SearchHit, SearchOptions};
use std::collections::HashMap;

const PHASES: [ScriptPhase; 3] = [
    ScriptPhase::PreRequest,
    ScriptPhase::PostResponse,
    ScriptPhase::Tests,
];

/// How much a replace touches.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ReplaceScope {
    /// Every match in every searched field.
    All,
    /// Only the first match in the field the hit describes: its request, field and key.
    Hit(SearchHit),
}

impl ReplaceScope {
    /// Every match everywhere.
    pub fn all() -> Self {
        ReplaceScope::All
    }

    /// The first match in the field of `hit`. The hit must come from a search run after the
    /// last change to the collection; a field that no longer holds a match is left alone.
    pub fn hit(hit: SearchHit) -> Self {
        ReplaceScope::Hit(hit)
    }
}

/// What a replace did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct ReplaceReport {
    /// Matches replaced, in files that were saved.
    pub replaced: usize,
    /// Request files rewritten.
    pub files_changed: usize,
    /// Paths of the rewritten requests, in tree order; skipped and failed requests are absent.
    pub changed_paths: Vec<ItemPath>,
    /// Names of requests with matches that were left alone because they hold unsaved edits.
    pub skipped_unsaved: Vec<String>,
    /// Requests that could not be edited or saved, with the reason; each is left as on disk.
    pub failed: Vec<(String, String)>,
    /// A script changed, so its new hash asks for trust again.
    pub scripts_changed: bool,
}

impl ReplaceReport {
    /// One line for a toast or a terminal.
    pub fn summary(&self) -> String {
        let files = if self.files_changed == 1 {
            "file"
        } else {
            "files"
        };
        let mut out = format!(
            "Replaced {} in {} {files}",
            self.replaced, self.files_changed
        );
        if !self.skipped_unsaved.is_empty() {
            out.push_str(&format!(
                "; skipped {} with unsaved edits",
                self.skipped_unsaved.len()
            ));
        }
        if self.scripts_changed {
            out.push_str("; scripts changed, they will ask for trust again");
        }
        if !self.failed.is_empty() {
            out.push_str(&format!("; {} failed", self.failed.len()));
        }
        out
    }
}

/// The searched fields of one request, copied out so they can be edited and written back.
/// It mirrors the field walk in `search.rs`, which must cover exactly the same fields.
struct Model {
    name: String,
    url: String,
    params: Vec<Param>,
    headers: Vec<KeyValue>,
    body: BodySpec,
    scripts: Vec<(ScriptPhase, String)>,
    docs: String,
}

/// Which setters a pass over a model needs.
#[derive(Default)]
struct Touched {
    name: bool,
    url: bool,
    params: bool,
    headers: bool,
    body: bool,
    scripts: Vec<ScriptPhase>,
    docs: bool,
}

/// Applies one replacement to fields in search order, counting what it changed.
struct Editor<'a> {
    matcher: &'a Matcher,
    replacement: &'a str,
    target: Option<&'a SearchHit>,
    count: usize,
    done: bool,
    /// Fields passed so far in this request, per field and key, to tell repeats apart.
    seen: HashMap<(SearchField, Option<String>), usize>,
}

impl Editor<'_> {
    fn apply(&mut self, field: SearchField, key: Option<&str>, text: &mut String) -> bool {
        let slot = self
            .seen
            .entry((field, key.map(str::to_owned)))
            .or_default();
        let occurrence = *slot;
        *slot += 1;
        if self.done {
            return false;
        }
        if let Some(t) = self.target
            && (t.field != field || t.key.as_deref() != key || t.occurrence != occurrence)
        {
            return false;
        }
        let first_only = self.target.is_some();
        let Some((new, n)) = self.matcher.replace(text, self.replacement, first_only) else {
            return false;
        };
        *text = new;
        self.count += n;
        self.done = first_only;
        true
    }

    fn model(&mut self, m: &mut Model) -> Touched {
        use SearchField::*;
        let mut t = Touched {
            name: self.apply(Name, None, &mut m.name),
            url: self.apply(Url, None, &mut m.url),
            ..Touched::default()
        };
        for p in m.params.iter_mut().filter(|p| p.kind == ParamKind::Path) {
            let key = p.name.clone();
            t.params |= self.apply(PathParam, Some(&key), &mut p.name);
            t.params |= self.apply(PathParam, Some(&key), &mut p.value);
        }
        for h in &mut m.headers {
            let key = h.name.clone();
            t.headers |= self.apply(HeaderName, Some(&key), &mut h.name);
            t.headers |= self.apply(HeaderValue, Some(&key), &mut h.value);
        }
        t.body = self.body(&mut m.body);
        for (phase, code) in &mut m.scripts {
            if self.apply(Script, Some(phase.as_str()), code) {
                t.scripts.push(*phase);
            }
        }
        t.docs = self.apply(Docs, None, &mut m.docs);
        t
    }

    fn body(&mut self, body: &mut BodySpec) -> bool {
        use SearchField::Body;
        let mut changed = false;
        match body {
            BodySpec::Json(t) | BodySpec::Xml(t) | BodySpec::Text(t) | BodySpec::Sparql(t) => {
                changed |= self.apply(Body, None, t);
            }
            BodySpec::FormUrlEncoded(fields) => {
                for f in fields {
                    let key = f.name.clone();
                    changed |= self.apply(Body, Some(&key), &mut f.name);
                    changed |= self.apply(Body, Some(&key), &mut f.value);
                }
            }
            BodySpec::Multipart(fields) => {
                for f in fields {
                    let key = f.name.clone();
                    changed |= self.apply(Body, Some(&key), &mut f.name);
                    match &mut f.value {
                        MultipartValue::Text(t) => changed |= self.apply(Body, Some(&key), t),
                        // Search offers an empty value for files; counting it keeps the
                        // occurrence indexes of the two walks equal.
                        _ => {
                            self.apply(Body, Some(&key), &mut String::new());
                        }
                    }
                }
            }
            BodySpec::Graphql { query, variables } => {
                changed |= self.apply(Body, Some("query"), query);
                changed |= self.apply(Body, Some("variables"), variables);
            }
            _ => {}
        }
        changed
    }
}

impl Model {
    fn of(r: &RequestFile) -> Self {
        Model {
            name: r.name().to_owned(),
            url: r.url().to_owned(),
            params: r.params(),
            headers: r.headers(),
            body: r.body(),
            scripts: PHASES.iter().map(|&p| (p, r.script(p))).collect(),
            docs: r.docs(),
        }
    }

    /// Write the touched parts back. Headers go first: they are the one setter that can fail,
    /// so a refusal leaves nothing half applied. Params precede the URL so a URL whose implied
    /// params changed re-derives them from the edited ones.
    fn write(self, r: &mut RequestFile, t: &Touched) -> Result<(), crate::EditError> {
        if t.headers {
            r.set_headers(self.headers)?;
        }
        if t.name {
            r.set_name(self.name);
        }
        if t.params {
            r.set_params(self.params);
        }
        if t.url {
            r.replace_url(self.url);
        }
        if t.body {
            r.set_body(self.body);
        }
        for (phase, code) in self.scripts {
            if t.scripts.contains(&phase) {
                r.set_script(phase, code);
            }
        }
        if t.docs {
            r.set_docs(self.docs);
        }
        Ok(())
    }
}

/// Request paths in tree order, folders before requests as the sidebar lists them.
fn request_paths(c: &Collection, folder: &Folder, out: &mut Vec<ItemPath>) {
    for item in folder.siblings() {
        match item {
            Sibling::Folder(f) => request_paths(c, f, out),
            Sibling::Request(r) => out.extend(c.item_path(r.path())),
            _ => {}
        }
    }
}

impl Collection {
    /// Replace `query` with `replacement` in the fields [`search_with`](Self::search_with)
    /// covers, saving each changed request through its edit setters so the rest of its bytes
    /// stay as they were. In regex mode `$1`..`$99`, `${n}`, `$&` and `$$` expand in the
    /// replacement. Requests with unsaved edits are skipped and named; a request that cannot
    /// be edited or saved is reloaded from disk and reported in `failed`. Fails only for an
    /// invalid pattern, before anything changes. A URL replace rewrites an existing `params`
    /// block only when it changed the params the URL implies, and never adds one.
    pub fn replace(
        &mut self,
        query: &str,
        replacement: &str,
        options: &SearchOptions,
        scope: ReplaceScope,
    ) -> Result<ReplaceReport, SearchError> {
        let matcher = Matcher::new(query, options)?;
        let mut report = ReplaceReport::default();
        if matcher.is_blank() {
            return Ok(report);
        }
        let target = match &scope {
            ReplaceScope::Hit(h) => Some(h),
            _ => None,
        };
        let mut paths = Vec::new();
        request_paths(self, self.tree(), &mut paths);
        for path in paths {
            if target.is_some_and(|t| t.path != path) {
                continue;
            }
            let Some(request) = self.request_mut(&path) else {
                continue;
            };
            let mut model = Model::of(request);
            let mut editor = Editor {
                matcher: &matcher,
                replacement,
                target,
                count: 0,
                done: false,
                seen: HashMap::new(),
            };
            let touched = editor.model(&mut model);
            if editor.count == 0 {
                continue;
            }
            let name = request.name().to_owned();
            if request.is_dirty() {
                report.skipped_unsaved.push(name);
                continue;
            }
            let outcome = model
                .write(request, &touched)
                .map_err(|e| e.to_string())
                .and_then(|()| request.save().map_err(|e| e.to_string()));
            match outcome {
                Ok(()) => {
                    report.replaced += editor.count;
                    report.files_changed += 1;
                    report.changed_paths.push(path);
                    report.scripts_changed |= !touched.scripts.is_empty();
                }
                Err(reason) => {
                    // Drop the half-edited model by reading the file back.
                    let reason = match RequestFile::load(request.path()) {
                        Ok(fresh) => {
                            *request = fresh;
                            reason
                        }
                        Err(e) => format!(
                            "{reason}; reloading it also failed ({e}), so memory may still hold the edit"
                        ),
                    };
                    report.failed.push((name, reason));
                }
            }
        }
        Ok(report)
    }
}
