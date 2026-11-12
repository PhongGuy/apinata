//! Getters and setters for the parts of a request file the app edits. Each setter writes
//! Bruno's form for what it changes and leaves every other key alone.

use crate::assertion::Assertion;
use crate::edit::auth::write_auth;
use crate::edit::body::BODY_ORDER;
use crate::edit::settings as edit_settings;
use crate::edit::{
    AuthSpec, BodySpec, KeyValue, Param, ParamKind, RequestSettings, Variable, headers_to_yaml,
};
use crate::error::EditError;
use crate::js;
use crate::order;
use crate::post_response::{PostResponseVar, merge_actions};
use crate::request::{RequestFile, RequestKind};
use crate::script::ScriptPhase;
use crate::script::edit::{script_in, with_script};
use crate::url_params::{
    QueryPart, build_query_string, escape_query_part, parse_path_params, parse_query_params,
    split_on_first,
};
use yaml_serde::Value;

const HTTP: &[&[&str]] = &[order::TOP, order::HTTP];
const RUNTIME: &[&[&str]] = &[order::TOP, order::RUNTIME];
const INFO: &[&[&str]] = &[order::TOP, order::INFO];

impl RequestFile {
    fn list(&self, keys: &[&str]) -> Vec<Value> {
        self.document()
            .get(keys)
            .and_then(Value::as_sequence)
            .cloned()
            .unwrap_or_default()
    }

    /// The root's `headers` in file order, disabled ones included.
    pub fn headers(&self) -> Vec<KeyValue> {
        self.list(&[self.root_key(), "headers"])
            .iter()
            .map(KeyValue::from_yaml)
            .collect()
    }

    /// Replace the root's `headers`; no headers removes the key. A name with a line break is
    /// [`EditError::InvalidHeaderName`] and leaves the file unchanged.
    pub fn set_headers(&mut self, headers: Vec<KeyValue>) -> Result<(), EditError> {
        let key = self.root_key();
        let items = headers_to_yaml(&headers)?;
        self.document_mut().set_list(&[key, "headers"], items, HTTP);
        Ok(())
    }

    /// `info.tags`, skipping entries that are not scalars.
    pub fn tags(&self) -> Vec<String> {
        self.list(&["info", "tags"])
            .iter()
            .filter_map(js::yaml_to_string)
            .collect()
    }

    /// Replace `info.tags` with the normalised `tags`; none removes the key.
    pub fn set_tags(&mut self, tags: Vec<String>) {
        let tags = crate::tags::normalize(tags);
        // Unchanged tags keep their original scalars, such as a numeric tag.
        if crate::tags::normalize(self.tags()) == tags {
            return;
        }
        let items = tags.into_iter().map(Value::String).collect();
        self.document_mut().set_list(&["info", "tags"], items, INFO);
    }

    /// The top-level `docs` text; empty when absent.
    pub fn docs(&self) -> String {
        self.document()
            .get(&["docs"])
            .and_then(js::yaml_to_string)
            .unwrap_or_default()
    }

    /// Set `docs`; blank text removes the key, as Bruno writes only non-blank docs.
    pub fn set_docs(&mut self, docs: impl Into<String>) {
        let docs = docs.into();
        // Unchanged docs keep their original scalar, such as a numeric one.
        if self.docs() == docs {
            return;
        }
        if js::js_trim(&docs).is_empty() {
            self.document_mut().remove(&["docs"]);
        } else {
            self.document_mut()
                .set(&["docs"], Value::String(docs), &[order::TOP]);
        }
    }

    /// `runtime.variables` in file order, disabled ones included.
    pub fn vars(&self) -> Vec<Variable> {
        self.list(&["runtime", "variables"])
            .iter()
            .map(Variable::from_yaml)
            .collect()
    }

    /// Replace `runtime.variables`; none removes the key, and an emptied `runtime` with it.
    pub fn set_vars(&mut self, vars: Vec<Variable>) {
        let items = vars.iter().map(Variable::to_yaml).collect();
        self.set_runtime_list("variables", items);
    }

    /// Replace `runtime.assertions`; none removes the key, and an emptied `runtime` with it.
    pub fn set_assertions(&mut self, assertions: Vec<Assertion>) {
        let items = assertions.iter().map(Assertion::to_yaml).collect();
        self.set_runtime_list("assertions", items);
    }

    /// Replace the after-response `set-variable` entries of `runtime.actions`, keeping other
    /// actions in place; an empty result removes the key and an emptied `runtime`.
    pub fn set_post_response_vars(&mut self, vars: Vec<PostResponseVar>) {
        let items = merge_actions(self.document().get(&["runtime", "actions"]), &vars);
        self.set_runtime_list("actions", items);
    }

    /// The code of the first `runtime.scripts` entry of `phase`'s type; empty when none.
    pub fn script(&self, phase: ScriptPhase) -> String {
        script_in(self.document().get(&["runtime", "scripts"]), phase)
    }

    /// Set that entry's code to [`crate::script_code`] of `code`; blank removes the entry, no
    /// entries removes `scripts` and an emptied `runtime`. Unchanged code changes nothing.
    pub fn set_script(&mut self, phase: ScriptPhase, code: impl Into<String>) {
        let code = code.into();
        // A hand-written block scalar ends in a newline, so both sides compare trimmed.
        if js::script_code(&code) == js::script_code(&self.script(phase)) {
            return;
        }
        let items = with_script(self.document().get(&["runtime", "scripts"]), phase, &code);
        self.set_runtime_list("scripts", items);
    }

    fn set_runtime_list(&mut self, key: &str, items: Vec<Value>) {
        let doc = self.document_mut();
        doc.set_list(&["runtime", key], items, RUNTIME);
        doc.remove_if_empty(&["runtime"]);
    }

    /// The request's own settings, with Bruno's defaults and coercions.
    pub fn settings(&self) -> RequestSettings {
        RequestSettings::from_yaml(self.document().get(&["settings"]))
    }

    /// Write the settings that differ from [`RequestFile::settings`]; other keys are left alone,
    /// so a file without a `settings` block gains only what changed. A key set back to its saved
    /// value gets the saved node back (or is removed if the saved file lacked it, with an emptied
    /// block it lacked too); an ill-typed saved node is written canonically instead.
    pub fn set_settings(&mut self, settings: RequestSettings) {
        let saved_block = self.document().get_saved(&["settings"]).cloned();
        let differs_from_saved: Vec<&str> = settings
            .changes_from(&RequestSettings::from_yaml(saved_block.as_ref()))
            .into_iter()
            .map(|(key, _)| key)
            .collect();
        for (key, value) in settings.changes_from(&self.settings()) {
            let saved_node = saved_block.as_ref().and_then(|block| block.get(key));
            let value = match saved_node {
                _ if differs_from_saved.contains(&key) => value,
                None => {
                    self.document_mut().remove(&["settings", key]);
                    continue;
                }
                Some(node) if edit_settings::well_typed(key, node) => node.clone(),
                Some(_) => value,
            };
            self.document_mut()
                .set(&["settings", key], value, edit_settings::ORDER);
        }
        if saved_block.is_none() {
            self.document_mut().remove_if_empty(&["settings"]);
        }
    }

    /// The default headers the request omits, from `settings.omitHeaders`: strings, trimmed,
    /// blanks dropped, as Bruno reads them.
    pub fn omitted_headers(&self) -> Vec<String> {
        edit_settings::omitted_names(self.document().get(&["settings"]))
    }

    /// Set `settings.omitHeaders` to `names`, trimmed with blanks dropped. No names removes the
    /// key, and an emptied `settings` block goes with it. The same names
    /// as now change nothing; other settings keys are left alone.
    pub fn set_omitted_headers<I, S>(&mut self, names: I)
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let names: Vec<String> = names
            .into_iter()
            .map(Into::into)
            .map(|name| js::js_trim(&name).to_owned())
            .filter(|name| !name.is_empty())
            .collect();
        if names == self.omitted_headers() {
            return;
        }
        let items = names.into_iter().map(Value::from).collect();
        let doc = self.document_mut();
        doc.set_list(&["settings", "omitHeaders"], items, edit_settings::ORDER);
        doc.remove_if_empty(&["settings"]);
    }

    /// The request body. An HTTP request with no root `body` is [`BodySpec::None`]; a GraphQL
    /// request's body is always [`BodySpec::Graphql`], with blank fields when absent.
    pub fn body(&self) -> BodySpec {
        let body = self.document().get(&[self.root_key(), "body"]);
        match self.kind() {
            RequestKind::Graphql => BodySpec::from_graphql_yaml(body),
            RequestKind::Http => BodySpec::from_yaml(body),
        }
    }

    /// Set the body. `None` removes the root's `body` and `Unsupported` does nothing. The same mode
    /// replaces only `data`, so other body keys survive; another mode replaces the whole body.
    /// A GraphQL request takes only `Graphql` (both fields empty removes `body`) or `None`; an
    /// HTTP request ignores `Graphql`.
    pub fn set_body(&mut self, body: BodySpec) {
        let key = self.root_key();
        if body == BodySpec::None {
            self.document_mut().remove(&[key, "body"]);
            return;
        }
        if self.kind() == RequestKind::Graphql {
            if !matches!(body, BodySpec::Graphql { .. }) {
                return;
            }
            match body.to_graphql_yaml() {
                Some(mapping) => self.document_mut().set(&[key, "body"], mapping, HTTP),
                None => {
                    self.document_mut().remove(&[key, "body"]);
                }
            }
            return;
        }
        let Some((kind, data)) = body.to_yaml() else {
            return;
        };
        let same_mode = self
            .document()
            .get(&[key, "body", "type"])
            .and_then(js::yaml_to_string)
            .is_some_and(|current| current == kind);
        let doc = self.document_mut();
        if !same_mode {
            doc.set(&[key, "body"], BodySpec::fresh_mapping(kind, data), HTTP);
        } else if let Some(data) = data {
            doc.set(
                &[key, "body", "data"],
                data,
                &[order::TOP, order::HTTP, BODY_ORDER],
            );
        } else {
            doc.remove(&[key, "body", "data"]);
        }
    }

    /// The request's authentication; no root `auth` is [`AuthSpec::None`].
    pub fn auth(&self) -> AuthSpec {
        AuthSpec::from_yaml(self.document().get(&[self.root_key(), "auth"]))
    }

    /// Set the authentication. `None` removes the root's `auth`, `Inherit` writes `inherit` and
    /// `Unsupported` does nothing. The same mode rewrites only its own keys, so others survive;
    /// another mode replaces the whole mapping.
    pub fn set_auth(&mut self, auth: AuthSpec) {
        let key = self.root_key();
        write_auth(self.document_mut(), &[key, "auth"], HTTP, &auth);
    }

    /// The root's `params` in file order, disabled ones included.
    pub fn params(&self) -> Vec<Param> {
        self.list(&[self.root_key(), "params"])
            .iter()
            .map(Param::from_yaml)
            .collect()
    }

    /// Replace the root's `params` in the given order, then rebuild the URL's query from the
    /// enabled query params; the rest of the URL is kept. Delimiters in their text are escaped,
    /// so no param splits the query; a later URL edit reads them back as that URL text.
    pub fn set_params(&mut self, params: Vec<Param>) {
        let url = self.url().to_owned();
        let (base, _) = split_on_first(&url, '?');
        let escaped: Vec<(String, String)> = params
            .iter()
            .filter(|p| p.enabled && p.kind == ParamKind::Query)
            .map(|p| {
                (
                    escape_query_part(&p.name, QueryPart::Name),
                    escape_query_part(&p.value, QueryPart::Value),
                )
            })
            .collect();
        let query = build_query_string(escaped.iter().map(|(n, v)| (n.as_str(), v.as_str())));
        let url = if query.is_empty() {
            base.to_owned()
        } else {
            format!("{base}?{query}")
        };
        self.write_params(&params);
        self.write_url(url);
    }

    /// The params the editor shows: the root's `params` when the file has that key, even empty;
    /// otherwise the params the URL implies, as `set_url` would derive them. Writes nothing.
    pub fn shown_params(&self) -> Vec<Param> {
        if self.document().get(&[self.root_key(), "params"]).is_some() {
            self.params()
        } else {
            derive_params(self.url(), &[])
        }
    }

    /// Set the root's `url` and re-derive the params as Bruno's `requestUrlChanged` does, except
    /// that renaming a single path param keeps its value; the current URL does nothing.
    pub fn set_url(&mut self, url: impl Into<String>) {
        let url = url.into();
        if url == self.url() {
            return;
        }
        let params = derive_params(&url, &self.params());
        self.write_params(&params);
        self.write_url(url);
    }

    /// Set the root's `url` for a text replace, with the least churn: the `params` block is
    /// re-derived only when the new URL implies other params than the old one did, and a file
    /// without one gets none, since an absent block reads the same before and after.
    pub(crate) fn replace_url(&mut self, url: String) {
        if url == self.url() {
            return;
        }
        if self.document().get(&[self.root_key(), "params"]).is_some() {
            let current = self.params();
            let derived = derive_params(&url, &current);
            if derived != derive_params(self.url(), &current) {
                self.write_params(&derived);
            }
        }
        self.write_url(url);
    }

    /// Where the URL's query and the file's enabled query params disagree, or `None` when they
    /// carry the same pairs in any order or the file has no `params` key. Nothing is expanded.
    pub fn query_mismatch(&self) -> Option<QueryMismatch> {
        self.document().get(&[self.root_key(), "params"])?;
        // Both sides are compared as URL text: the table as `set_params` would write it, so
        // literal escape text typed in a cell never reads as a delimiter.
        let (_, query) = split_on_first(self.url(), '?');
        let from_url: Vec<(String, String)> = parse_query_params(query.unwrap_or_default())
            .into_iter()
            .map(|(name, value)| (name, value.unwrap_or_default()))
            .filter(|(name, _)| !js::js_trim(name).is_empty())
            .collect();
        let from_table = table_query_text(&self.params());
        // A name differs when the values it carries differ as a multiset; URL names lead.
        let mut names: Vec<String> = Vec::new();
        for (name, _) in from_url.iter().chain(&from_table) {
            if !names.contains(name) && values_of(&from_url, name) != values_of(&from_table, name) {
                names.push(name.clone());
            }
        }
        (!names.is_empty()).then_some(QueryMismatch { names })
    }

    /// Settle a mismatch for the URL: the table's query params become the URL's, keeping its
    /// disabled query and path params. An agreeing request is left as it is.
    pub fn use_url_query(&mut self) {
        if self.query_mismatch().is_some() {
            let params = derive_params(self.url(), &self.params());
            self.write_params(&params);
        }
    }

    /// Settle a mismatch for the table: the URL's query is rebuilt from its enabled query
    /// params. An agreeing request is left as it is.
    pub fn use_params_query(&mut self) {
        if self.query_mismatch().is_some() {
            self.set_params(self.params());
        }
    }

    /// Set the root's `url` and `params` exactly as given, deriving neither from the other, so
    /// an import keeps its source's URL and params byte for byte.
    pub(crate) fn set_url_and_params(&mut self, url: String, params: &[Param]) {
        self.write_params(params);
        self.write_url(url);
    }

    fn write_params(&mut self, params: &[Param]) {
        let key = self.root_key();
        let items = params.iter().map(Param::to_yaml).collect();
        self.document_mut().set_list(&[key, "params"], items, HTTP);
    }

    fn write_url(&mut self, url: String) {
        let key = self.root_key();
        self.document_mut()
            .set(&[key, "url"], Value::String(url), HTTP);
    }
}

/// Which query param names a request's URL and its `params` block disagree on.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct QueryMismatch {
    /// Each disagreeing name once, in first-seen order, the URL's first.
    pub names: Vec<String>,
}

/// The values `pairs` carries for `name`, sorted so order alone never counts.
fn values_of<'a>(pairs: &'a [(String, String)], name: &str) -> Vec<&'a str> {
    let mut values: Vec<&str> = pairs
        .iter()
        .filter(|(n, _)| n == name)
        .map(|(_, v)| v.as_str())
        .collect();
    values.sort_unstable();
    values
}

/// The enabled query params as the URL text `set_params` writes for them, minus the blank
/// names it skips.
fn table_query_text(params: &[Param]) -> Vec<(String, String)> {
    params
        .iter()
        .filter(|p| p.enabled && p.kind == ParamKind::Query && !js::js_trim(&p.name).is_empty())
        .map(|p| {
            (
                escape_query_part(&p.name, QueryPart::Name),
                escape_query_part(&p.value, QueryPart::Value),
            )
        })
        .collect()
}

/// The params a URL implies given the current ones: the URL's query params first, then its new
/// path params, then the disabled query params and the enabled path params the URL still uses.
/// When the URL swaps exactly one enabled path name for one new name, that param is renamed.
/// Names and values are the URL's raw text: no escape is decoded, so a typed `%26` stays `%26`.
fn derive_params(url: &str, current: &[Param]) -> Vec<Param> {
    let (base, query) = split_on_first(url, '?');
    let path_names = parse_path_params(base);
    let kept_path: Vec<&Param> = current
        .iter()
        .filter(|p| p.enabled && p.kind == ParamKind::Path)
        .collect();
    // A pair without `=` has no value in Bruno, which its writer saves as an empty string.
    let from_query = parse_query_params(query.unwrap_or_default())
        .into_iter()
        .map(|(name, value)| Param::new(name, value.unwrap_or_default(), ParamKind::Query));
    let added: Vec<&String> = path_names
        .iter()
        .filter(|name| !kept_path.iter().any(|p| &p.name == *name))
        .collect();
    let dropped: Vec<&Param> = kept_path
        .iter()
        .copied()
        .filter(|p| !path_names.contains(&p.name))
        .collect();
    // An edit that swaps exactly one path name for another is a rename: that row keeps its value
    // and place, so a typo and its undo leave the params as they were.
    let renamed = match (added.as_slice(), dropped.as_slice()) {
        ([to], [from]) => Some((from.name.as_str(), to.as_str())),
        _ => None,
    };
    let new_path = added
        .into_iter()
        .filter(|_| renamed.is_none())
        .map(|name| Param::new(name.as_str(), "", ParamKind::Path));
    let disabled_query = current
        .iter()
        .filter(|p| !p.enabled && p.kind == ParamKind::Query)
        .cloned();
    let still_used = kept_path.iter().filter_map(|p| {
        let mut p = (*p).clone();
        match renamed {
            Some((from, to)) if p.name == from => p.name = to.to_owned(),
            _ if !path_names.contains(&p.name) => return None,
            _ => {}
        }
        Some(p)
    });
    from_query
        .chain(new_path)
        .chain(disabled_query)
        .chain(still_used)
        .collect()
}
