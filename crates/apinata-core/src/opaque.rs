//! Item files of a type Apinata does not model, kept so they still take part in ordering.

use crate::document::{Document, as_seq};
use crate::error::SaveError;
use crate::order;
use std::path::Path;
use yaml_serde::Value;

/// A request file of a non-`http` type, such as graphql or websocket. Only its name and `seq`
/// are read, and only `seq` can be changed; an untouched file is never rewritten.
#[derive(Debug)]
#[non_exhaustive]
pub struct OpaqueItem {
    file_name: String,
    doc: Document,
}

impl OpaqueItem {
    /// Load `path` as an opaque item: a mapping whose `info.type` is a string other than `http`.
    /// Any other file, readable or not, is `None`.
    pub(crate) fn load(path: &Path) -> Option<OpaqueItem> {
        let doc = Document::load(path).ok()?;
        let kind = doc.get(&["info", "type"])?.as_str()?;
        if kind == "http" {
            return None;
        }
        let file_name = path.file_name()?.to_string_lossy().into_owned();
        Some(OpaqueItem { file_name, doc })
    }

    /// The path the file was loaded from.
    pub fn path(&self) -> &Path {
        self.doc.path()
    }

    pub(crate) fn document(&self) -> &Document {
        &self.doc
    }

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

    /// `info.type`, such as `graphql`.
    pub fn kind(&self) -> &str {
        self.doc
            .get(&["info", "type"])
            .and_then(Value::as_str)
            .unwrap_or("")
    }

    /// `info.name`, else the file stem.
    pub fn name(&self) -> &str {
        self.doc
            .get(&["info", "name"])
            .and_then(Value::as_str)
            .or_else(|| self.doc.path().file_stem().and_then(|s| s.to_str()))
            .unwrap_or("")
    }

    /// `info.seq` when present and a non-negative integer.
    pub fn seq(&self) -> Option<u64> {
        self.doc.get(&["info", "seq"]).and_then(as_seq)
    }

    /// Set `info.seq`.
    pub fn set_seq(&mut self, seq: u64) {
        self.doc.set(
            &["info", "seq"],
            Value::Number(seq.into()),
            &[order::TOP, order::children_of("info")],
        );
    }

    /// Whether `set_seq` has changed the file since it was loaded or last saved.
    pub fn is_dirty(&self) -> bool {
        self.doc.is_dirty()
    }

    /// Write a pending `seq` edit. Does nothing when clean, and fails with
    /// [`SaveError::Conflict`] when the file changed on disk since it was loaded or last saved.
    pub fn save(&mut self) -> Result<(), SaveError> {
        self.doc.save()
    }
}
