//! One place that reconciles the two ways to name a collection: the positional and `--collection`.

use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail};

/// The collection named by `positional`, by `flag`, or by neither (None, so the caller falls
/// back to the working directory). Naming it twice is fine when both name the same directory
/// and an error otherwise.
pub fn pick(positional: Option<&Path>, flag: Option<&Path>) -> anyhow::Result<Option<PathBuf>> {
    match (positional, flag) {
        (Some(a), Some(b)) => {
            if same_place(a, b) {
                Ok(Some(a.to_path_buf()))
            } else {
                bail!(
                    "the collection is named twice and the two differ: `{}` and `--collection {}`; give it once",
                    a.display(),
                    b.display()
                )
            }
        }
        (Some(p), None) | (None, Some(p)) => Ok(Some(p.to_path_buf())),
        (None, None) => Ok(None),
    }
}

/// [`pick`] for commands that cannot fall back to the working directory: `usage` names the
/// forms the command takes and is shown when neither was given.
pub fn require(
    positional: Option<&Path>,
    flag: Option<&Path>,
    usage: &str,
) -> anyhow::Result<PathBuf> {
    pick(positional, flag)?.ok_or_else(|| anyhow!("no collection given; use {usage}"))
}

/// Whether two spellings name the same place: compared canonicalized when both exist, and
/// literally otherwise.
fn same_place(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

/// Appends `hint` to a "no opencollection.yml" failure so the message says which forms the
/// command takes; any other error passes through.
pub fn with_hint<T>(result: anyhow::Result<T>, hint: &str) -> anyhow::Result<T> {
    result.map_err(|e| {
        if e.to_string().starts_with("no opencollection.yml in") {
            anyhow!("{e}; {hint}")
        } else {
            e
        }
    })
}

/// The hint for a command that takes `[COLLECTION]` or `--collection <DIR>`.
pub fn optional_hint(usage: &str) -> String {
    format!("pass the collection as `{usage}` or `--collection <DIR>`")
}
