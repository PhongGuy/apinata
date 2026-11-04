//! The migration walk: every YAML file a legacy collection converts to, computed in memory.

use super::{map_collection, map_environment, map_folder, map_request};
use crate::document::read_regular_text;
use crate::legacy::bru::{
    BruError, parse_bruno_json, parse_collection, parse_environment, parse_request,
};
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use yaml_serde::Mapping;

/// A legacy file that could not be read or parsed; `line` is 0 when no line applies.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FileError {
    pub(crate) path: PathBuf,
    pub(crate) line: usize,
    pub(crate) message: String,
}

impl FileError {
    pub(crate) fn new(path: &Path, line: usize, message: impl Into<String>) -> Self {
        Self {
            path: path.to_path_buf(),
            line,
            message: message.into(),
        }
    }
}

/// The root file whose settings merge into `opencollection.yml`.
pub(crate) const ROOT_COLLECTION: &str = "collection.bru";

/// Directory and file names the walk never enters, besides `bruno.json`'s `ignore` list.
const IGNORED_NAMES: [&str; 2] = ["node_modules", ".git"];

/// Every YAML file the reference migration writes for the legacy collection at `root`, as
/// (target relative to `root`, document), sorted by target. Reads only. Any failure returns
/// every failing file, paths relative to `root`, sorted by path.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn plan_files(root: &Path) -> Result<Vec<(PathBuf, Mapping)>, Vec<FileError>> {
    // Without a readable, valid `bruno.json` the folder is not a legacy root: stop at once.
    let bruno_json_path = Path::new("bruno.json");
    let bruno_json = read_regular_text(&root.join(bruno_json_path))
        .map_err(|e| vec![FileError::new(bruno_json_path, 0, e.to_string())])
        .and_then(|text| {
            parse_bruno_json(&text)
                .map_err(|e| vec![FileError::new(bruno_json_path, e.line, e.message)])
        })?;
    let files = walk_files(root, &ignored_names(&bruno_json)).map_err(|e| vec![e])?;
    plan_walked(root, &bruno_json, &files)
}

/// The names the walk skips at every depth: `bruno.json`'s `ignore` list plus the fixed ones.
pub(crate) fn ignored_names(bruno_json: &Value) -> Vec<&str> {
    bruno_json
        .get("ignore")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .chain(IGNORED_NAMES)
        .collect()
}

/// [`plan_files`] over an already parsed `bruno.json` and the output of [`walk_files`]; only
/// the `.bru` files among `files` are read.
pub(crate) fn plan_walked(
    root: &Path,
    bruno_json: &Value,
    files: &[PathBuf],
) -> Result<Vec<(PathBuf, Mapping)>, Vec<FileError>> {
    // Convert each file independently so that one bad file never hides another.
    let mut planned = Vec::new();
    let mut errors = Vec::new();
    // The reference reads the root `collection.bru` before and apart from the walk, following
    // a symlink and ignoring the ignore list, so it merges even when the walk never lists it.
    let collection_root = match read_root_collection(root) {
        Ok(parsed) => parsed,
        Err(error) => {
            errors.push(error);
            None
        }
    };
    for rel in files
        .iter()
        .filter(|rel| is_bru(rel) && rel.as_path() != Path::new(ROOT_COLLECTION))
    {
        let parsed = read_regular_text(&root.join(rel))
            .map_err(|e| FileError::new(rel, 0, e.to_string()))
            .and_then(|text| {
                convert(rel, &text).map_err(|e| FileError::new(rel, e.line, e.message))
            });
        match parsed {
            Ok((target, doc)) => planned.push((target, doc)),
            Err(error) => errors.push(error),
        }
    }
    // The collection document comes from `bruno.json`, so its failures name that file.
    match map_collection(collection_root.as_ref(), bruno_json) {
        Ok(doc) => planned.push((PathBuf::from("opencollection.yml"), doc)),
        Err(message) => errors.push(FileError::new(Path::new("bruno.json"), 0, message)),
    }
    if !errors.is_empty() {
        errors.sort_by(|a, b| a.path.cmp(&b.path));
        return Err(errors);
    }
    planned.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(planned)
}

/// Whether `rel` is a legacy `.bru` file by its extension.
pub(crate) fn is_bru(rel: &Path) -> bool {
    rel.extension().is_some_and(|e| e == "bru")
}

/// The root `collection.bru` that merges into `opencollection.yml`; absent (or a dangling
/// link) is no root, an unreadable or unparsable one is a failure naming it.
fn read_root_collection(root: &Path) -> Result<Option<Value>, FileError> {
    let rel = Path::new(ROOT_COLLECTION);
    let text = match read_regular_text(&root.join(rel)) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(FileError::new(rel, 0, e.to_string())),
    };
    parse_collection(&text)
        .map(Some)
        .map_err(|e| FileError::new(rel, e.line, e.message))
}

/// Routes a walked `rel` as the migration does: a file directly in `environments/` is an
/// environment, any other `folder.bru` is `folder.yml`, and everything else is a request.
fn convert(rel: &Path, text: &str) -> Result<(PathBuf, Mapping), BruError> {
    let dir = rel.parent().unwrap_or(Path::new(""));
    let name = rel.file_name().and_then(|n| n.to_str());
    // The reference lets the collection file silently overwrite this request; refuse instead.
    if rel == Path::new("opencollection.bru") {
        return Err(BruError {
            line: 0,
            message: "this request would become opencollection.yml, which the legacy \
                      collection file also becomes; rename it first"
                .to_owned(),
        });
    }
    let (target, doc) = if dir == Path::new("environments") {
        (
            rel.with_extension("yml"),
            map_environment(&parse_environment(text)?),
        )
    } else if name == Some("folder.bru") {
        (dir.join("folder.yml"), map_folder(&parse_collection(text)?))
    } else {
        (
            rel.with_extension("yml"),
            map_request(&parse_request(text)?),
        )
    };
    Ok((target, doc))
}

/// `collectBruFilesForMigration`, generalised to every file: the regular files under `root`,
/// relative to it and sorted, skipping `ignored` names at every depth and symlinks. A
/// directory that cannot be read fails the walk, so no file is silently left out.
pub(crate) fn walk_files(root: &Path, ignored: &[&str]) -> Result<Vec<PathBuf>, FileError> {
    let mut out = Vec::new();
    collect_files(root, Path::new(""), ignored, &mut out)?;
    out.sort();
    Ok(out)
}

fn collect_files(
    root: &Path,
    rel: &Path,
    ignored: &[&str],
    out: &mut Vec<PathBuf>,
) -> Result<(), FileError> {
    let fail = |e: std::io::Error| FileError::new(rel, 0, format!("cannot read directory: {e}"));
    for entry in fs::read_dir(root.join(rel)).map_err(fail)? {
        let entry = entry.map_err(fail)?;
        let name = entry.file_name();
        if name.to_str().is_some_and(|n| ignored.contains(&n)) {
            continue;
        }
        // `DirEntry::file_type` does not follow links, so a symlink is neither dir nor file.
        let kind = entry.file_type().map_err(fail)?;
        let path = rel.join(&name);
        if kind.is_dir() {
            collect_files(root, &path, ignored, out)?;
        } else if kind.is_file() {
            out.push(path);
        }
    }
    Ok(())
}
