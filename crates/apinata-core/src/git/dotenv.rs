//! Keeping a collection's `.env`, which holds secret values, out of git.

use std::io;
use std::path::Path;

use super::{GitError, GitOptions, check_root, is_not_a_repo, run, show_prefix};
use crate::collection::{append_dotenv_rule, refuse_special_file};

/// Where a collection's `.env` stands with git, as [`ignore_dotenv`] found or left it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum DotenvGit {
    /// Git is missing or the collection is outside a repository; nothing was written.
    Outside,
    /// A rule git reads already ignores `.env`.
    Ignored,
    /// No rule ignored `.env`, so a `.env` line was appended to the collection's `.gitignore`.
    Added,
    /// Git tracks `.env`, which no ignore rule can change; nothing was written.
    Tracked,
}

/// Make git ignore `<root>/.env`, judged as `git check-ignore` judges it, so a rule anywhere
/// git reads counts and one undone by a later `!` rule does not. A missing rule is appended to
/// `<root>/.gitignore`. A pipe, socket or device at that `.gitignore` is `Io` of kind
/// `InvalidInput` before git runs; other git failures are errors.
pub fn ignore_dotenv(root: &Path, options: &GitOptions) -> Result<DotenvGit, GitError> {
    check_root(root)?;
    let gitignore = root.join(".gitignore");
    // Git would wait forever reading rules from a pipe.
    refuse_special_file(&gitignore)?;
    match show_prefix(root, options) {
        Ok(_) => {}
        Err(e) if is_not_a_repo(&e) => return Ok(DotenvGit::Outside),
        Err(GitError::Io(e)) if e.kind() == io::ErrorKind::NotFound => {
            return Ok(DotenvGit::Outside);
        }
        Err(e) => return Err(e),
    }
    let args = ["ls-files", "-z", "--cached", "--", ".env"];
    let out = run::git(&args, root, options)?;
    if !out.status.success() {
        return Err(out.failed(&args));
    }
    if !out.stdout.is_empty() {
        return Ok(DotenvGit::Tracked);
    }
    // Exit 0 is ignored and 1 is not; anything else is git failing.
    let args = ["check-ignore", "-q", "--", ".env"];
    let out = run::git(&args, root, options)?;
    match out.status.code() {
        Some(0) => Ok(DotenvGit::Ignored),
        Some(1) => {
            append_dotenv_rule(&gitignore)?;
            Ok(DotenvGit::Added)
        }
        _ => Err(out.failed(&args)),
    }
}
