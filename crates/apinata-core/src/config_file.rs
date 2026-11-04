//! `folder.yml` and `opencollection.yml` as typed views over a [`Document`].

use crate::document::{DiskState, Document, as_seq};
use crate::edit::auth::write_auth;
use crate::edit::{AuthSpec, KeyValue, Variable, headers_to_yaml};
use crate::error::{EditError, SaveError};
use crate::import::SpecBaseline;
use crate::item_path::ItemPath;
use crate::js;
use crate::network::{
    ClientCertificate, CollectionNetwork, CollectionProxy, HostOverride, read_network,
    write_certificates, write_host_overrides, write_proxy,
};
use crate::post_response::{PostResponseVar, merge_actions, post_response_vars_in};
use crate::presets::CollectionPresets;
use crate::script::ScriptPhase;
use crate::script::edit::{script_in, with_script};
use std::path::{Path, PathBuf};
use yaml_serde::{Mapping, Value};

/// Bruno's writer key order for `folder.yml` (`stringifyFolder.ts`).
const FOLDER_TOP: &[&str] = &["info", "request", "docs"];
const FOLDER_INFO: &[&str] = &["name", "type", "seq", "tags"];
/// Bruno's writer key order for `opencollection.yml` (`stringifyCollection.ts`).
pub(crate) const COLLECTION_TOP: &[&str] = &[
    "opencollection",
    "info",
    "config",
    "request",
    "docs",
    "bundled",
    "extensions",
];
const COLLECTION_INFO: &[&str] = &["name", "version"];
/// Writer key orders shared by both files' `request` and `docs` mappings.
const REQUEST: &[&str] = &["headers", "auth", "variables", "actions", "scripts"];
const DOCS: &[&str] = &["content", "type"];

/// The document and writer orders both file types share, so each method exists once.
#[derive(Debug)]
struct ConfigDoc {
    doc: Document,
    top: &'static [&'static str],
    info: &'static [&'static str],
}

impl ConfigDoc {
    fn path(&self) -> &Path {
        self.doc.path()
    }

    fn name(&self) -> Option<&str> {
        self.doc.get(&["info", "name"]).and_then(Value::as_str)
    }

    fn set_name(&mut self, name: String) {
        self.set_info("name", Value::String(name));
    }

    fn set_info(&mut self, key: &str, value: Value) {
        self.doc.set(&["info", key], value, &[self.top, self.info]);
    }

    fn list(&self, key: &str) -> Vec<Value> {
        self.doc
            .get(&["request", key])
            .and_then(Value::as_sequence)
            .cloned()
            .unwrap_or_default()
    }

    /// Bruno writes `request` only while it holds a default, so a drained one goes.
    fn set_request_list(&mut self, key: &str, items: Vec<Value>) {
        self.doc
            .set_list(&["request", key], items, &[self.top, REQUEST]);
        self.doc.remove_if_empty(&["request"]);
    }

    fn set_auth(&mut self, auth: &AuthSpec) {
        write_auth(
            &mut self.doc,
            &["request", "auth"],
            &[self.top, REQUEST],
            auth,
        );
        self.doc.remove_if_empty(&["request"]);
    }

    /// Bruno reads `docs` as plain text or as `{content}`.
    fn docs(&self) -> String {
        let docs = self.doc.get(&["docs"]);
        docs.and_then(|d| d.get("content"))
            .or(docs)
            .and_then(js::yaml_to_string)
            .unwrap_or_default()
    }

    /// Blank text removes `docs`; a mapping keeps its unknown keys, anything else is replaced.
    fn set_docs(&mut self, docs: String) {
        if js::js_trim(&docs).is_empty() {
            self.doc.remove(&["docs"]);
        } else if self.doc.get(&["docs"]).is_some_and(Value::is_mapping) {
            let order: &[&[&str]] = &[self.top, DOCS];
            self.doc
                .set(&["docs", "content"], Value::String(docs), order);
            self.doc
                .set(&["docs", "type"], Value::from("text/markdown"), order);
        } else {
            let mut map = Mapping::new();
            map.insert(Value::from("content"), Value::String(docs));
            map.insert(Value::from("type"), Value::from("text/markdown"));
            self.doc.set(&["docs"], Value::Mapping(map), &[self.top]);
        }
    }

    fn is_dirty(&self) -> bool {
        self.doc.is_dirty()
    }

    fn save(&mut self) -> Result<(), SaveError> {
        self.doc.save()
    }
}

/// Public methods both file types share, delegating to the private wrapper.
macro_rules! shared_methods {
    ($ty:ident) => {
        impl $ty {
            /// The file's path.
            pub fn path(&self) -> &Path {
                self.inner.path()
            }

            /// `info.name`, when it is a string.
            pub fn name(&self) -> Option<&str> {
                self.inner.name()
            }

            /// Set `info.name`.
            pub fn set_name(&mut self, name: impl Into<String>) {
                self.inner.set_name(name.into());
            }

            /// Whether an edit has changed the file since it was loaded or last saved.
            pub fn is_dirty(&self) -> bool {
                self.inner.is_dirty()
            }

            /// What the disk holds for this file; only a dirty file outlives it.
            pub fn disk_state(&self) -> DiskState {
                self.inner.doc.disk_state()
            }

            /// Write pending edits in Bruno's format. Does nothing when clean, and fails with
            /// [`SaveError::Conflict`] when the file changed on disk since it was loaded, or
            /// appeared on disk when this one was not yet saved.
            pub fn save(&mut self) -> Result<(), SaveError> {
                self.inner.save()
            }

            /// `request.headers` in file order, disabled ones included.
            pub fn headers(&self) -> Vec<KeyValue> {
                self.inner
                    .list("headers")
                    .iter()
                    .map(KeyValue::from_yaml)
                    .collect()
            }

            /// Replace `request.headers`; none removes the key. A name with a line break is
            /// [`EditError::InvalidHeaderName`] and leaves the file unchanged.
            pub fn set_headers(&mut self, headers: Vec<KeyValue>) -> Result<(), EditError> {
                let items = headers_to_yaml(&headers)?;
                self.inner.set_request_list("headers", items);
                Ok(())
            }

            /// `request.auth`; none is [`AuthSpec::None`].
            pub fn auth(&self) -> AuthSpec {
                AuthSpec::from_yaml(self.inner.doc.get(&["request", "auth"]))
            }

            /// Set `request.auth` as a request's auth is set.
            pub fn set_auth(&mut self, auth: AuthSpec) {
                self.inner.set_auth(&auth);
            }

            /// `request.variables` in file order, disabled ones included.
            pub fn vars(&self) -> Vec<Variable> {
                self.inner
                    .list("variables")
                    .iter()
                    .map(Variable::from_yaml)
                    .collect()
            }

            /// Replace `request.variables`; none removes the key.
            pub fn set_vars(&mut self, vars: Vec<Variable>) {
                let items = vars.iter().map(Variable::to_yaml).collect();
                self.inner.set_request_list("variables", items);
            }

            /// `request.actions`' post-response variables in file order, disabled ones included.
            pub fn post_response_vars(&self) -> Vec<PostResponseVar> {
                post_response_vars_in(self.inner.doc.get(&["request", "actions"]))
            }

            /// Replace them as `RequestFile::set_post_response_vars` does, under `request.actions`.
            pub fn set_post_response_vars(&mut self, vars: Vec<PostResponseVar>) {
                let items = merge_actions(self.inner.doc.get(&["request", "actions"]), &vars);
                self.inner.set_request_list("actions", items);
            }

            /// The docs text, from `docs.content` or a plain-text `docs`; empty when absent.
            pub fn docs(&self) -> String {
                self.inner.docs()
            }

            /// Write `docs` as `{content, type: text/markdown}`; blank text removes it.
            pub fn set_docs(&mut self, docs: impl Into<String>) {
                self.inner.set_docs(docs.into());
            }

            /// The code of the first `request.scripts` entry of `phase`'s type; empty when none.
            pub fn script(&self, phase: ScriptPhase) -> String {
                script_in(self.inner.doc.get(&["request", "scripts"]), phase)
            }

            /// Set it as `RequestFile::set_script` does, under `request.scripts`; an emptied
            /// `request` is removed.
            pub fn set_script(&mut self, phase: ScriptPhase, code: impl Into<String>) {
                let code = code.into();
                if js::script_code(&code) == js::script_code(&self.script(phase)) {
                    return;
                }
                let items = with_script(self.inner.doc.get(&["request", "scripts"]), phase, &code);
                self.inner.set_request_list("scripts", items);
            }

            pub(crate) fn document(&self) -> &Document {
                &self.inner.doc
            }
        }
    };
}

/// A `folder.yml`, loaded or created unsaved for a folder that has none.
#[derive(Debug)]
#[non_exhaustive]
pub struct FolderFile {
    inner: ConfigDoc,
}

/// The collection's `opencollection.yml`.
#[derive(Debug)]
#[non_exhaustive]
pub struct CollectionFile {
    inner: ConfigDoc,
}

shared_methods!(FolderFile);
shared_methods!(CollectionFile);

impl FolderFile {
    /// `info.tags`, normalised: scalars as strings, trimmed, empties dropped, first of each kept.
    pub fn tags(&self) -> Vec<String> {
        let items = self
            .inner
            .doc
            .get(&["info", "tags"])
            .and_then(Value::as_sequence);
        crate::tags::normalize(items.into_iter().flatten().filter_map(js::yaml_to_string))
    }

    /// Replace `info.tags` with the normalised `tags`; none removes the key.
    pub fn set_tags(&mut self, tags: Vec<String>) {
        let tags = crate::tags::normalize(tags);
        // Unchanged tags keep their original scalars, such as a numeric tag.
        if self.tags() == tags {
            return;
        }
        let items = tags.into_iter().map(Value::String).collect();
        self.inner
            .doc
            .set_list(&["info", "tags"], items, &[FOLDER_TOP, FOLDER_INFO]);
    }

    pub(crate) fn document_mut(&mut self) -> &mut Document {
        &mut self.inner.doc
    }

    /// Write `info.name` through to disk, keeping unsaved edits unsaved.
    pub(crate) fn write_name(&mut self, name: &str) -> Result<(), SaveError> {
        self.inner.doc.write_through(
            &["info", "name"],
            Value::from(name),
            &[FOLDER_TOP, FOLDER_INFO],
        )
    }

    /// Write `info.seq` through to disk, keeping unsaved edits unsaved.
    pub(crate) fn write_seq(&mut self, seq: u64) -> Result<(), SaveError> {
        self.inner.doc.write_through(
            &["info", "seq"],
            Value::from(seq),
            &[FOLDER_TOP, FOLDER_INFO],
        )
    }

    pub(crate) fn from_document(doc: Document) -> FolderFile {
        FolderFile {
            inner: ConfigDoc {
                doc,
                top: FOLDER_TOP,
                info: FOLDER_INFO,
            },
        }
    }

    /// An unsaved `{info: {name: <dir_name>, type: folder}}` at `path`, as Bruno writes a folder
    /// root on its first save.
    pub(crate) fn unsaved(path: PathBuf, dir_name: &str) -> FolderFile {
        let mut info = Mapping::new();
        info.insert("name".into(), Value::String(dir_name.to_owned()));
        info.insert("type".into(), "folder".into());
        let mut root = Mapping::new();
        root.insert("info".into(), Value::Mapping(info));
        FolderFile::from_document(Document::new(path, root))
    }

    /// `info.seq`, when present and a non-negative integer.
    pub fn seq(&self) -> Option<u64> {
        self.inner.doc.get(&["info", "seq"]).and_then(as_seq)
    }

    /// Set `info.seq`.
    pub fn set_seq(&mut self, seq: u64) {
        self.inner.set_info("seq", Value::Number(seq.into()));
    }
}

impl CollectionFile {
    pub(crate) fn document_mut(&mut self) -> &mut Document {
        &mut self.inner.doc
    }

    /// The collection's `config.proxy` and `config.clientCertificates`.
    pub fn network(&self) -> CollectionNetwork {
        read_network(&self.inner.doc)
    }

    /// Set `config.proxy` as Bruno writes it; see [`CollectionProxy`].
    pub fn set_proxy(&mut self, proxy: &CollectionProxy) {
        write_proxy(&mut self.inner.doc, COLLECTION_TOP, proxy);
    }

    /// Replace `config.clientCertificates`; entries of a type this reader does not offer stay.
    pub fn set_client_certificates(&mut self, certificates: &[ClientCertificate]) {
        write_certificates(&mut self.inner.doc, COLLECTION_TOP, certificates);
    }

    /// Replace `extensions.apinata.hostOverrides`.
    pub fn set_host_overrides(&mut self, rows: &[HostOverride]) {
        write_host_overrides(&mut self.inner.doc, COLLECTION_TOP, rows);
    }

    /// The collection's `extensions.bruno.presets`; absent values are empty.
    pub fn presets(&self) -> CollectionPresets {
        CollectionPresets::read(&self.inner.doc)
    }

    /// Set the presets' request type and URL as Bruno writes them; empty values are removed and
    /// the default environment is left as it is.
    pub fn set_presets(&mut self, presets: &CollectionPresets) {
        presets.write(&mut self.inner.doc, COLLECTION_TOP);
    }

    /// The spec update baseline; `None` when `extensions.apinata.openapi.ops` is absent, and an
    /// empty baseline when it is an empty mapping.
    pub fn spec_baseline(&self) -> Option<SpecBaseline> {
        SpecBaseline::read(&self.inner.doc)
    }

    /// Write the baseline as `extensions.apinata.openapi.ops`. An empty baseline still writes an
    /// empty `ops`, so a collection whose generated requests were all pruned stays updatable.
    pub fn set_spec_baseline(&mut self, baseline: &SpecBaseline) {
        baseline.write(&mut self.inner.doc, COLLECTION_TOP);
    }

    /// Remove `ops` and the sections it leaves empty: the collection has no baseline again.
    pub fn clear_spec_baseline(&mut self) {
        SpecBaseline::clear(&mut self.inner.doc);
    }

    /// The baseline recorded for the spec imported into `folder`; the root folder is
    /// [`CollectionFile::spec_baseline`].
    pub fn spec_baseline_in(&self, folder: &ItemPath) -> Option<SpecBaseline> {
        SpecBaseline::read_in(&self.inner.doc, folder)
    }

    /// The folders that have a spec baseline entry of their own, in file order. An entry may
    /// name a folder that no longer exists on disk.
    pub fn spec_baseline_folders(&self) -> Vec<ItemPath> {
        SpecBaseline::folders(&self.inner.doc)
    }

    /// Write `baseline` as the entry of `folder`.
    pub fn set_spec_baseline_in(&mut self, folder: &ItemPath, baseline: &SpecBaseline) {
        baseline.write_in(&mut self.inner.doc, COLLECTION_TOP, folder);
    }

    /// Remove the entry of `folder` and the sections it leaves empty.
    pub fn clear_spec_baseline_in(&mut self, folder: &ItemPath) {
        SpecBaseline::clear_in(&mut self.inner.doc, folder);
    }

    pub(crate) fn from_document(doc: Document) -> CollectionFile {
        CollectionFile {
            inner: ConfigDoc {
                doc,
                top: COLLECTION_TOP,
                info: COLLECTION_INFO,
            },
        }
    }
}
