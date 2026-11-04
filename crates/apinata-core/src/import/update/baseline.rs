//! The field hashes recorded at import time, kept in the root file under
//! `extensions.apinata.openapi.ops`, and per folder under
//! `extensions.apinata.openapi.folders.<folder>.ops`.

use super::{ChangedField, FieldHashes};
use crate::document::Document;
use crate::item_path::ItemPath;
use crate::js;
use std::collections::BTreeMap;
use yaml_serde::{Mapping, Value};

const FILE: &str = "file";
const OPS: [&str; 4] = ["extensions", "apinata", "openapi", "ops"];
const FOLDERS: [&str; 4] = ["extensions", "apinata", "openapi", "folders"];
/// Key order inside `openapi`: the root `ops` first, then `folders`.
const OPENAPI_ORDER: &[&str] = &["ops", "folders"];

/// What each generated request's spec-owned fields hashed to when they were last written from
/// the spec, by operation key.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct SpecBaseline {
    ops: BTreeMap<String, FieldHashes>,
    files: BTreeMap<String, String>,
}

impl SpecBaseline {
    /// The hashes recorded for `key`.
    pub fn get(&self, key: &str) -> Option<&FieldHashes> {
        self.ops.get(key)
    }

    /// The item path (such as `people/getUser.yml`) the operation `key` was last written to or
    /// found at; `None` for a baseline recorded before files were kept.
    pub fn file(&self, key: &str) -> Option<&str> {
        self.files.get(key).map(String::as_str)
    }

    /// Record the item path `file` for `key`, replacing any earlier one.
    pub fn set_file(&mut self, key: String, file: String) {
        self.files.insert(key, file);
    }

    /// Record `hashes` for `key`, replacing any earlier entry.
    pub fn insert(&mut self, key: String, hashes: FieldHashes) {
        self.ops.insert(key, hashes);
    }

    /// Forget `key`; whether it was recorded.
    pub fn remove(&mut self, key: &str) -> bool {
        self.files.remove(key);
        self.ops.remove(key).is_some()
    }

    /// The recorded keys in order.
    pub fn keys(&self) -> impl Iterator<Item = &str> {
        self.ops.keys().map(String::as_str)
    }

    /// How many operations are recorded.
    pub fn len(&self) -> usize {
        self.ops.len()
    }

    /// Whether nothing is recorded.
    pub fn is_empty(&self) -> bool {
        self.ops.is_empty()
    }

    /// `None` when `ops` is absent; an empty mapping reads as an empty baseline.
    pub(crate) fn read(doc: &Document) -> Option<SpecBaseline> {
        Self::read_at(doc, &OPS)
    }

    /// The baseline of `folder`; the root folder is the root baseline.
    pub(crate) fn read_in(doc: &Document, folder: &ItemPath) -> Option<SpecBaseline> {
        if folder.is_root() {
            return Self::read(doc);
        }
        Self::read_at(doc, &folder_ops(folder))
    }

    /// Folders that have their own entry, in file order.
    pub(crate) fn folders(doc: &Document) -> Vec<ItemPath> {
        let Some(map) = doc.get(&FOLDERS).and_then(Value::as_mapping) else {
            return Vec::new();
        };
        map.keys()
            .filter_map(|k| ItemPath::new(&js::yaml_to_string(k)?))
            .filter(|p| !p.is_root())
            .collect()
    }

    fn read_at(doc: &Document, at: &[&str]) -> Option<SpecBaseline> {
        let ops = doc.get(at)?.as_mapping()?;
        let mut baseline = SpecBaseline::default();
        for (key, fields) in ops {
            let (Some(key), Some(fields)) = (js::yaml_to_string(key), fields.as_mapping()) else {
                continue;
            };
            let pairs = ChangedField::ALL.into_iter().filter_map(|f| {
                let hash = js::yaml_to_string(fields.get(f.id())?)?;
                Some((f, hash))
            });
            baseline.insert(key.clone(), FieldHashes::new(pairs));
            if let Some(file) = fields.get(FILE).and_then(js::yaml_to_string) {
                baseline.set_file(key, file);
            }
        }
        Some(baseline)
    }

    /// Hashes go in as strings, which the writer quotes when they could read as numbers.
    pub(crate) fn write(&self, doc: &mut Document, top: &[&str]) {
        self.write_in(doc, top, &ItemPath::root());
    }

    pub(crate) fn write_in(&self, doc: &mut Document, top: &[&str], folder: &ItemPath) {
        let mut ops = Mapping::new();
        for (key, hashes) in &self.ops {
            let mut fields = Mapping::new();
            for (field, hash) in hashes.iter() {
                fields.insert(Value::from(field.id()), Value::String(hash.to_owned()));
            }
            if let Some(file) = self.files.get(key) {
                fields.insert(Value::from(FILE), Value::String(file.to_owned()));
            }
            ops.insert(Value::from(key.as_str()), Value::Mapping(fields));
        }
        let order: &[&[&str]] = &[top, &[], &[], OPENAPI_ORDER];
        if folder.is_root() {
            doc.set(&OPS, Value::Mapping(ops), order);
        } else {
            doc.set(&folder_ops(folder), Value::Mapping(ops), order);
        }
    }

    /// Drop `ops` and the `openapi` and `apinata` sections it leaves empty; `extensions` stays.
    /// Folder entries are untouched.
    pub(crate) fn clear(doc: &mut Document) {
        doc.remove(&OPS);
        prune(doc);
    }

    /// Drop the entry of `folder` and any parents it leaves empty.
    pub(crate) fn clear_in(doc: &mut Document, folder: &ItemPath) {
        if folder.is_root() {
            return Self::clear(doc);
        }
        let ops = folder_ops(folder);
        doc.remove(&ops);
        doc.remove_if_empty(&ops[..5]);
        prune(doc);
    }
}

fn folder_ops(folder: &ItemPath) -> [&str; 6] {
    [
        "extensions",
        "apinata",
        "openapi",
        "folders",
        folder.as_str(),
        "ops",
    ]
}

fn prune(doc: &mut Document) {
    doc.remove_if_empty(&FOLDERS);
    doc.remove_if_empty(&OPS[..3]);
    doc.remove_if_empty(&OPS[..2]);
}
