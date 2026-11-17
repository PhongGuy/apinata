//! What a pulled range did to the environment files, and their bytes at either end.

use std::path::Path;

use super::{GitError, GitOptions, run};

/// The environment files a range touched, each by its file stem.
#[derive(Debug, Default)]
pub(crate) struct EnvChanges {
    pub deleted: Vec<String>,
    pub added: Vec<String>,
    pub modified: Vec<String>,
    /// Pairs git matched by content, old stem first.
    pub renamed: Vec<(String, String)>,
}

/// The environment-file changes between commits `from` and `to`, seen from the collection at
/// `root`; `None` when either commit is not in the repository.
pub(crate) fn env_changes(
    root: &Path,
    from: &str,
    to: &str,
    opts: &GitOptions,
) -> Result<Option<EnvChanges>, GitError> {
    for rev in [from, to] {
        let commit = format!("{rev}^{{commit}}");
        let out = run::git(
            &["rev-parse", "--verify", "-q", commit.as_str()],
            root,
            opts,
        )?;
        if !out.status.success() {
            return Ok(None);
        }
    }
    let args = [
        "diff",
        "--name-status",
        "-z",
        "-M",
        "--relative",
        from,
        to,
        "--",
        "environments/",
    ];
    let out = run::git(&args, root, opts)?;
    if !out.status.success() {
        return Err(out.failed(&args));
    }
    parse_name_status(&out.stdout).map(Some)
}

/// The bytes of `rel`, relative to the collection at `root`, in commit `rev`; `None` when that
/// commit has no such file.
pub(crate) fn blob_at(
    root: &Path,
    rev: &str,
    rel: &str,
    opts: &GitOptions,
) -> Result<Option<Vec<u8>>, GitError> {
    // The `./` makes the path relative to the collection, not the repository top.
    let object = format!("{rev}:./{rel}");
    let out = run::git(&["cat-file", "blob", object.as_str()], root, opts)?;
    Ok(out.status.success().then_some(out.stdout))
}

/// Read `-z` name-status output: `R<score>\0old\0new\0` for a rename, `<letter>\0path\0` for
/// anything else. Only files directly in `environments/` are kept.
fn parse_name_status(stdout: &[u8]) -> Result<EnvChanges, GitError> {
    let mut fields = stdout
        .split(|&b| b == 0)
        .filter(|f| !f.is_empty())
        .map(|f| String::from_utf8_lossy(f).into_owned());
    let mut changes = EnvChanges::default();
    while let Some(status) = fields.next() {
        let missing = || GitError::Malformed(format!("git diff gave `{status}` without its paths"));
        let first = fields.next().ok_or_else(missing)?;
        let kind = status.bytes().next().unwrap_or_default();
        if matches!(kind, b'R' | b'C') {
            let second = fields.next().ok_or_else(missing)?;
            match (env_stem(&first), env_stem(&second)) {
                (Some(old), Some(new)) if kind == b'R' => changes.renamed.push((old, new)),
                (Some(old), None) if kind == b'R' => changes.deleted.push(old),
                (_, Some(new)) => changes.added.push(new),
                _ => {}
            }
            continue;
        }
        let Some(stem) = env_stem(&first) else {
            continue;
        };
        match kind {
            b'A' => changes.added.push(stem),
            b'D' => changes.deleted.push(stem),
            b'M' | b'T' => changes.modified.push(stem),
            _ => {}
        }
    }
    Ok(changes)
}

/// The stem of `environments/<stem>.yml`, `None` for any other path.
fn env_stem(rel: &str) -> Option<String> {
    let name = rel.strip_prefix("environments/")?;
    let stem = name.strip_suffix(".yml")?;
    (!stem.is_empty() && !stem.contains('/')).then(|| stem.to_owned())
}
