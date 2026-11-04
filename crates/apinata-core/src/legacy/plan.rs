//! Plans a legacy collection's conversion: what it creates, copies and removes. Writes nothing.

use super::bru::parse_bruno_json;
use super::convert::{hash_stream, open_regular};
use super::error::{BruError, ConvertError};
use super::yml::{FileError, ROOT_COLLECTION, ignored_names, is_bru, plan_walked, walk_files};
use crate::document::read_regular;
use crate::yaml::emit::to_bruno_yaml;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

const BRUNO_JSON: &str = "bruno.json";
const ROOT_FILE: &str = "opencollection.yml";

/// Whether `root` holds a legacy collection: `bruno.json` is a file and `opencollection.yml`
/// is not.
pub fn is_legacy_collection(root: &Path) -> bool {
    root.join(BRUNO_JSON).is_file() && !root.join(ROOT_FILE).is_file()
}

/// The full outcome of converting one legacy collection, computed without writing anything.
#[derive(Clone)]
#[non_exhaustive]
pub struct ConversionPlan {
    pub(super) root: PathBuf,
    name: String,
    /// Each YAML document to write, (path relative to `root`, text), sorted by path.
    pub(super) files: Vec<(PathBuf, String)>,
    pub(super) copies: Vec<PathBuf>,
    pub(super) removals: Vec<PathBuf>,
    /// SHA-256 of every source the conversion consumes, taken before it was read for planning.
    pub(super) hashes: Vec<(PathBuf, Vec<u8>)>,
    /// SHA-256 of every file in `copies`, kept apart from `hashes` because an in-place
    /// conversion never copies. A copy with no digest here is treated as changed.
    pub(super) copy_hashes: Vec<(PathBuf, Vec<u8>)>,
}

/// Lists paths only: the rendered documents can hold literal auth values from the sources.
impl fmt::Debug for ConversionPlan {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ConversionPlan")
            .field("root", &self.root)
            .field("name", &self.name)
            .field("created", &self.created())
            .field("copied", &self.copies)
            .field("removed_in_place", &self.removals)
            .finish_non_exhaustive()
    }
}

impl ConversionPlan {
    /// The legacy collection's root, as given to [`plan_conversion`].
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The collection name: `bruno.json`'s `name` when it is a string, else the root's
    /// directory name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The YAML files the conversion creates, relative to the root, sorted.
    pub fn created(&self) -> Vec<PathBuf> {
        self.files.iter().map(|(path, _)| path.clone()).collect()
    }

    /// The files an in-place conversion deletes: every walked `.bru` file plus `bruno.json`,
    /// relative to the root, sorted.
    pub fn removed_in_place(&self) -> Vec<PathBuf> {
        self.removals.clone()
    }

    /// Every other walked file, which a conversion into a new folder copies, relative to the
    /// root, sorted.
    pub fn copied(&self) -> Vec<PathBuf> {
        self.copies.clone()
    }

    /// After an in-place conversion failed with `error`: when it failed deleting one of
    /// [`removed_in_place`](Self::removed_in_place), the YAML collection is complete and this
    /// answers the legacy files still present (relative, sorted). `None` for any other error,
    /// which the conversion rolled back.
    pub fn left_over(&self, error: &ConvertError) -> Option<Vec<PathBuf>> {
        let ConvertError::Io { path, .. } = error else {
            return None;
        };
        if !self.removals.iter().any(|rel| self.root.join(rel) == *path) {
            return None;
        }
        let present = |rel: &&PathBuf| fs::symlink_metadata(self.root.join(rel)).is_ok();
        Some(self.removals.iter().filter(present).cloned().collect())
    }
}

/// Plan converting the legacy collection at `root` to OpenCollection YAML. Reads only.
///
/// Fails with `NotLegacy` when `root/bruno.json` is not a file, `Parse` with every file that
/// cannot be read (only `bruno.json` when that one fails), and `TargetExists` with every
/// path the conversion would create that already exists in the tree.
pub fn plan_conversion(root: &Path) -> Result<ConversionPlan, ConvertError> {
    let bruno_json_rel = Path::new(BRUNO_JSON);
    if !root.join(bruno_json_rel).is_file() {
        return Err(ConvertError::NotLegacy {
            root: root.to_path_buf(),
        });
    }
    // The walk needs `bruno.json`'s ignore list, so a bad `bruno.json` is the only error.
    let parse_one = |error: FileError| ConvertError::Parse {
        errors: vec![bru_error(error)],
    };
    let bytes = read_regular(&root.join(bruno_json_rel))
        .map_err(|e| parse_one(FileError::new(bruno_json_rel, 0, e.to_string())))?;
    let bruno_json = match std::str::from_utf8(&bytes) {
        Err(e) => return Err(parse_one(FileError::new(bruno_json_rel, 0, e.to_string()))),
        Ok(text) => parse_bruno_json(text)
            .map_err(|e| parse_one(FileError::new(bruno_json_rel, e.line, e.message)))?,
    };

    // Hash every source before planning reads it, so an edit from here on is caught later.
    let walked = walk_files(root, &ignored_names(&bruno_json)).map_err(parse_one)?;
    let mut hashes = vec![(
        bruno_json_rel.to_path_buf(),
        Sha256::digest(&bytes).to_vec(),
    )];
    let (bru, others): (Vec<&PathBuf>, Vec<&PathBuf>) = walked.iter().partition(|p| is_bru(p));
    for rel in &bru {
        if let Ok(source) = read_regular(&root.join(rel)) {
            hashes.push(((*rel).clone(), Sha256::digest(&source).to_vec()));
        }
    }
    // Copies are hashed streaming and without following links, as the copy will read them.
    let copy_hashes: Vec<(PathBuf, Vec<u8>)> = others
        .iter()
        .filter(|p| p.as_path() != bruno_json_rel)
        .filter_map(|rel| {
            let mut file = open_regular(&root.join(rel)).ok()?;
            let digest = hash_stream(&mut file, None).ok()?;
            Some(((*rel).clone(), digest))
        })
        .collect();

    // The root `collection.bru` merges even when the walk skips it (a symlink or an ignored
    // name), so it is hashed too; it joins the removals only when the walk listed it.
    let root_collection = Path::new(ROOT_COLLECTION);
    if !hashes.iter().any(|(p, _)| p == root_collection)
        && let Ok(source) = read_regular(&root.join(root_collection))
    {
        hashes.push((
            root_collection.to_path_buf(),
            Sha256::digest(&source).to_vec(),
        ));
    }

    let documents =
        plan_walked(root, &bruno_json, &walked).map_err(|errors| ConvertError::Parse {
            errors: errors.into_iter().map(bru_error).collect(),
        })?;
    let files: Vec<(PathBuf, String)> = documents
        .into_iter()
        .map(|(path, doc)| (path, to_bruno_yaml(&doc)))
        .collect();

    // `symlink_metadata` also sees a dangling link, which a write would still collide with.
    let collisions: Vec<PathBuf> = files
        .iter()
        .map(|(path, _)| path.clone())
        .filter(|path| fs::symlink_metadata(root.join(path)).is_ok())
        .collect();
    if !collisions.is_empty() {
        return Err(ConvertError::TargetExists { paths: collisions });
    }

    let mut removals: Vec<PathBuf> = bru.into_iter().cloned().collect();
    removals.push(bruno_json_rel.to_path_buf());
    removals.sort();
    let copies = others
        .into_iter()
        .filter(|p| p.as_path() != bruno_json_rel)
        .cloned()
        .collect();
    Ok(ConversionPlan {
        root: root.to_path_buf(),
        name: collection_name(root, &bruno_json),
        files,
        copies,
        removals,
        hashes,
        copy_hashes,
    })
}

fn bru_error(error: FileError) -> BruError {
    BruError {
        path: error.path,
        line: error.line,
        message: error.message,
    }
}

/// `bruno.json`'s string `name`, else the root's directory name (resolved for `.` and `..`).
fn collection_name(root: &Path, bruno_json: &Value) -> String {
    if let Some(name) = bruno_json.get("name").and_then(Value::as_str) {
        return name.to_owned();
    }
    let real = fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    real.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}
