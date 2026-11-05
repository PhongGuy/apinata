//! A single request file.

use crate::assertion::Assertion;
use crate::document::{DiskState, Document, as_seq};
use crate::error::{FileError, SaveError};
use crate::order;
use crate::post_response::{PostResponseVar, post_response_vars_in};
use std::path::Path;
use yaml_serde::Value;

/// The protocol of a request file, named by its `info.type`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum RequestKind {
    /// A plain HTTP request under the `http` key.
    Http,
    /// A GraphQL request under the `graphql` key.
    Graphql,
}

/// A request file opened from disk.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct RequestFile {
    file_name: String,
    doc: Document,
}

impl RequestFile {
    /// Load one request file: an absent or `http` `info.type` needs an `http` mapping, `graphql`
    /// needs a `graphql` mapping, and any other type is refused by name.
    pub fn load(path: impl AsRef<Path>) -> Result<RequestFile, FileError> {
        let path = path.as_ref();
        let mut doc = Document::load(path)?;
        let key = match doc.get(&["info", "type"]) {
            None => "http",
            Some(kind) => match kind.as_str() {
                Some("http") => "http",
                Some("graphql") => "graphql",
                Some(other) => return Err(unsupported(path, other)),
                None => {
                    let shown = yaml_serde::to_string(kind)
                        .map_or_else(|_| "?".to_owned(), |s| s.trim().to_owned());
                    return Err(unsupported(path, &shown));
                }
            },
        };
        if !doc.get(&[key]).is_some_and(|v| v.is_mapping()) {
            return Err(FileError::new(
                path,
                format!("the file has no `{key}` mapping"),
            ));
        }
        doc.migrate_runtime_auth(key);
        Ok(RequestFile::from_document(doc))
    }

    /// Wrap a document already known to be an `http` or `graphql` request.
    pub(crate) fn from_document(doc: Document) -> RequestFile {
        let file_name = doc
            .path()
            .file_name()
            .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
        RequestFile { file_name, doc }
    }

    /// The request's protocol, read from `info.type` on each call so a rebuilt document cannot
    /// disagree with it.
    pub fn kind(&self) -> RequestKind {
        match self.doc.get(&["info", "type"]).and_then(Value::as_str) {
            Some("graphql") => RequestKind::Graphql,
            _ => RequestKind::Http,
        }
    }

    /// The top-level key holding the request: `http` or `graphql`, following `kind()`.
    pub(crate) fn root_key(&self) -> &'static str {
        match self.kind() {
            RequestKind::Graphql => "graphql",
            RequestKind::Http => "http",
        }
    }

    /// The path the file was loaded from.
    pub fn path(&self) -> &Path {
        self.doc.path()
    }

    /// The parsed request file.
    pub(crate) fn document(&self) -> &Document {
        &self.doc
    }

    /// The parsed request file, for an edit.
    pub(crate) fn document_mut(&mut self) -> &mut Document {
        &mut self.doc
    }

    pub(crate) fn into_document(self) -> Document {
        self.doc
    }

    /// Follow a rename on disk: the path prefix `from` became `to`.
    pub(crate) fn relocate(&mut self, from: &Path, to: &Path) {
        self.doc.relocate(from, to);
        self.file_name = self
            .doc
            .path()
            .file_name()
            .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
    }

    /// The file's name, extension included.
    pub fn file_name(&self) -> &str {
        &self.file_name
    }

    /// `info.name`, else the file stem (also used when `info.name` is not a string).
    pub fn name(&self) -> &str {
        self.doc
            .get(&["info", "name"])
            .and_then(|v| v.as_str())
            .or_else(|| self.doc.path().file_stem().and_then(|s| s.to_str()))
            .unwrap_or("")
    }

    /// `info.seq` when present and a non-negative integer.
    pub fn seq(&self) -> Option<u64> {
        self.doc.get(&["info", "seq"]).and_then(as_seq)
    }

    /// The `runtime.assertions` entries in file order, disabled ones included.
    pub fn assertions(&self) -> Vec<Assertion> {
        self.doc
            .get(&["runtime", "assertions"])
            .and_then(Value::as_sequence)
            .map_or_else(Vec::new, |list| {
                list.iter().map(Assertion::from_yaml).collect()
            })
    }

    /// The request's `runtime.actions` post-response variables in file order, disabled ones included.
    pub fn post_response_vars(&self) -> Vec<PostResponseVar> {
        post_response_vars_in(self.doc.get(&["runtime", "actions"]))
    }

    /// The root's `method`, else `GET` for HTTP and `POST` for GraphQL.
    pub fn method(&self) -> &str {
        let default = match self.kind() {
            RequestKind::Graphql => "POST",
            RequestKind::Http => "GET",
        };
        self.doc
            .get(&[self.root_key(), "method"])
            .and_then(|v| v.as_str())
            .unwrap_or(default)
    }

    /// The root's `url`, else the empty string.
    pub fn url(&self) -> &str {
        self.doc
            .get(&[self.root_key(), "url"])
            .and_then(|v| v.as_str())
            .unwrap_or("")
    }

    /// Set `info.name`.
    pub fn set_name(&mut self, name: impl Into<String>) {
        self.doc.set(
            &["info", "name"],
            Value::String(name.into()),
            &[order::TOP, order::children_of("info")],
        );
    }

    /// Set the root's `method`.
    pub fn set_method(&mut self, method: impl Into<String>) {
        let key = self.root_key();
        self.doc.set(
            &[key, "method"],
            Value::String(method.into()),
            &[order::TOP, order::children_of(key)],
        );
    }

    /// Set `info.seq`.
    pub fn set_seq(&mut self, seq: u64) {
        self.doc.set(
            &["info", "seq"],
            Value::Number(seq.into()),
            &[order::TOP, order::children_of("info")],
        );
    }

    /// Whether a setter has changed the file since it was loaded or last saved.
    pub fn is_dirty(&self) -> bool {
        self.doc.is_dirty()
    }

    /// What the disk holds for this request; only a dirty request outlives its file.
    pub fn disk_state(&self) -> DiskState {
        self.doc.disk_state()
    }

    /// Write pending edits in Bruno's format. Does nothing when clean, and fails with
    /// [`SaveError::Conflict`] when the file changed on disk since it was loaded or last saved.
    pub fn save(&mut self) -> Result<(), SaveError> {
        self.doc.save()
    }
}

fn unsupported(path: &Path, shown: &str) -> FileError {
    FileError::new(
        path,
        format!("unsupported request type `{shown}`, only `http` and `graphql` are supported"),
    )
}
