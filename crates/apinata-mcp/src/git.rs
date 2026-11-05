//! The `git_status` and `git_diff` tools, which pass core's git state and file diff through
//! unchanged, and the local `git_commit` tool, so the agent sees the same JSON as `apinata git`.

use std::path::{Component, Path, PathBuf};

use apinata_core::{
    FileDiff, FileStatus, GitError, GitOptions, GitState, StageState, collection_status_with,
    commit, file_diff_with, stage,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

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

/// Arguments of `git_commit`.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct CommitArgs {
    /// The commit message; its first line is the subject.
    pub message: String,
    /// Collection-relative files or folders to stage first, `["."]` for everything. Leave out to
    /// commit what is already staged.
    #[serde(default)]
    pub paths: Option<Vec<String>>,
}

/// The commit that was recorded.
#[derive(Serialize, JsonSchema)]
pub(crate) struct CommitDoc {
    /// The abbreviated id of the new commit.
    pub short_id: String,
    /// The first line of the message.
    pub subject: String,
}

/// Stage `args.paths` (when given) and commit what is staged. Refuses before staging when the
/// paths would not be the whole commit, because other files are already staged.
pub(crate) fn commit_paths(
    root: &Path,
    args: &CommitArgs,
    options: &GitOptions,
) -> Result<CommitDoc, String> {
    if args.message.trim().is_empty() {
        return Err("the commit message is empty".to_owned());
    }
    let state = collection_status_with(root, options).map_err(|e| e.to_string())?;
    let repo = match state {
        GitState::Repo(repo) => repo,
        GitState::Unavailable => {
            return Err("git was not found on PATH, so nothing can be committed.".to_owned());
        }
        _ => return Err(format!("{} is not in a git repository.", root.display())),
    };
    if let Some(paths) = &args.paths {
        if paths.is_empty() {
            return Err("paths must not be empty; omit it to commit what is staged".to_owned());
        }
        if repo.truncated {
            return Err(
                "too many changes to check for other staged files; commit them in \
                        the app or a terminal"
                    .to_owned(),
            );
        }
        let wanted: Vec<PathBuf> = paths.iter().map(|p| plain(p)).collect();
        let named = |file: &Path| {
            wanted
                .iter()
                .any(|w| w == Path::new(".") || file.starts_with(w))
        };
        // A staged rename also removes its original path, so that must be named too.
        let others: Vec<String> = repo
            .stage
            .iter()
            .filter(|(_, s)| !matches!(s, StageState::Unstaged))
            .filter(|(file, _)| {
                let from = match repo.files.get(*file) {
                    Some(FileStatus::Renamed { from }) => Some(from),
                    _ => None,
                };
                !named(file) || from.is_some_and(|f| !named(f))
            })
            .map(|(file, _)| match repo.files.get(file) {
                Some(FileStatus::Renamed { from }) if !named(from) => {
                    format!("{} (renamed from {})", file.display(), from.display())
                }
                _ => file.display().to_string(),
            })
            .collect();
        if !others.is_empty() {
            return Err(format!(
                "other files are already staged and would be committed too: {}. Commit them \
                 first, unstage them, or leave out `paths`.",
                others.join(", ")
            ));
        }
        stage(root, &wanted, options).map_err(|e| e.to_string())?;
    }
    // A file staged between the status read and the commit would still be committed; the window
    // is narrow and the commit only ever holds collection files.
    let done = commit(root, &args.message, options).map_err(|e| e.to_string())?;
    Ok(CommitDoc {
        short_id: done.short_id,
        subject: done.subject,
    })
}

/// `path` rebuilt without a leading `./` or doubled slashes, as the diff tool reads it. The
/// collection itself stays `.`.
fn plain(path: &str) -> PathBuf {
    let rebuilt: PathBuf = Path::new(path)
        .components()
        .filter(|c| !matches!(c, Component::CurDir))
        .collect();
    if rebuilt.as_os_str().is_empty() && !path.is_empty() {
        PathBuf::from(".")
    } else {
        rebuilt
    }
}
