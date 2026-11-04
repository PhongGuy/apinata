//! The read-only `git_status` and `git_diff` tools: core's git state and file diff, passed
//! through unchanged so the agent sees the same JSON as `apinata git --json`.

use std::path::{Component, Path, PathBuf};

use apinata_core::{
    FileDiff, GitError, GitOptions, GitState, collection_status_with, file_diff_with,
};
use schemars::JsonSchema;
use serde::Serialize;

/// The collection's git state, serialized exactly as core does.
#[derive(Serialize)]
#[serde(transparent)]
pub(crate) struct GitStatusDoc(pub GitState);

/// One file's diff against `HEAD`, serialized exactly as core does.
#[derive(Serialize)]
#[serde(transparent)]
pub(crate) struct GitDiffDoc(pub FileDiff);

// Core owns these shapes, so the schemas only promise an object and point at its tags.
impl JsonSchema for GitStatusDoc {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "GitStatusDoc".into()
    }

    fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({
            "type": "object",
            "description": "`state` is `unavailable`, `not_a_repo` or `repo`; a repository adds \
                            `branch`, `head`, `files` (path to a status tagged `kind`) and \
                            `truncated`."
        })
    }
}

impl JsonSchema for GitDiffDoc {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "GitDiffDoc".into()
    }

    fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({
            "type": "object",
            "description": "`path`, `status` (null when git reports none, which is not the same \
                            as unchanged), `binary`, `old`, `new`, the changed `sections` and \
                            line `hunks`."
        })
    }
}

/// The git state of the collection at `root`.
pub(crate) fn status(root: &Path, options: &GitOptions) -> Result<GitStatusDoc, String> {
    collection_status_with(root, options)
        .map(GitStatusDoc)
        .map_err(|e| e.to_string())
}

/// The diff of the collection file `path` against `HEAD`. A refused path is named as such; a
/// missing git or repository gets the CLI's sentence; any other failure passes through.
pub(crate) fn diff(root: &Path, path: &str, options: &GitOptions) -> Result<GitDiffDoc, String> {
    // Rebuilt without a leading `./` or doubled slashes, as the CLI reads it; `..` and `/` stay
    // so core still refuses them.
    let rel: PathBuf = Path::new(path)
        .components()
        .filter(|c| !matches!(c, Component::CurDir))
        .collect();
    // Core checks the path before running git, so a path error stands even without git; other
    // failures are checked against the status to tell the quiet states apart.
    let e = match file_diff_with(root, &rel, options) {
        Ok(diff) => return Ok(GitDiffDoc(diff)),
        Err(e @ GitError::Malformed(_)) => return Err(e.to_string()),
        Err(e) => e,
    };
    Err(match collection_status_with(root, options) {
        Ok(GitState::Unavailable) => {
            "git was not found on PATH, so changes cannot be shown.".to_owned()
        }
        Ok(GitState::NotARepo) => format!("{} is not in a git repository.", root.display()),
        _ => e.to_string(),
    })
}
