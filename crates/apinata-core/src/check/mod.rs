//! Checking a collection without sending anything.
//!
//! The check opens the collection, reports what failed to load and what is
//! structurally wrong, and returns a sorted report. It never sends a request,
//! runs a script, and reads a secret store only when given one, to see which
//! secrets have a value.

mod files;
mod report;
mod unclosed;
mod vars;

use std::collections::HashSet;
use std::path::Path;
use std::sync::Arc;

use crate::collection::Folder;
use crate::error::{HintShell, OpenError};
use crate::script::Segment;
use crate::script::compile::{Problem, syntax_problems};
use crate::secrets::SecretStore;
use crate::{Collection, FileError};

pub use report::{CheckReport, Finding, FindingKind, Severity};

/// Why a collection could not be checked at all.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum CheckError {
    /// The directory is not an openable collection.
    #[error(transparent)]
    Open(#[from] OpenError),
    /// `env` names an environment the collection does not have.
    #[error(
        "{}",
        crate::not_found::not_found_text(
            "environment",
            "this collection has no environments",
            name,
            available
        )
    )]
    EnvironmentNotFound {
        /// The unknown name.
        name: String,
        /// The environments the collection does have, by name.
        available: Vec<String>,
    },
}

/// What to check against.
#[derive(Clone, Default)]
#[non_exhaustive]
pub struct CheckOptions {
    env: Option<String>,
    env_vars: Vec<(String, String)>,
    global_env_vars: Vec<(String, String)>,
    secrets: Option<Arc<dyn SecretStore>>,
    keychain_not_used: bool,
    hint_shell: HintShell,
}

// Variable values may be secrets, so Debug shows names only.
impl std::fmt::Debug for CheckOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let names = |vars: &[(String, String)]| -> Vec<String> {
            vars.iter().map(|(k, _)| k.clone()).collect()
        };
        f.debug_struct("CheckOptions")
            .field("env", &self.env)
            .field("env_vars", &names(&self.env_vars))
            .field("global_env_vars", &names(&self.global_env_vars))
            .field("secrets", &self.secrets.is_some())
            .field("keychain_not_used", &self.keychain_not_used)
            .finish()
    }
}

impl CheckOptions {
    /// Check only this environment instead of all of them.
    #[must_use]
    pub fn env(mut self, name: impl Into<String>) -> Self {
        self.env = Some(name.into());
        self
    }

    /// Supply a value for a variable, as `--env-var` does.
    #[must_use]
    pub fn env_var(mut self, k: impl Into<String>, v: impl Into<String>) -> Self {
        self.env_vars.push((k.into(), v.into()));
        self
    }

    /// Supply a value for a global environment variable.
    #[must_use]
    pub fn global_env_var(mut self, k: impl Into<String>, v: impl Into<String>) -> Self {
        self.global_env_vars.push((k.into(), v.into()));
        self
    }

    /// Say the caller's runs read no secret store, so a valueless secret needs no keychain note.
    #[must_use]
    pub fn keychain_not_used(mut self) -> Self {
        self.keychain_not_used = true;
        self
    }

    /// Word the findings' remedies for this front end; the command line when unset.
    #[must_use]
    pub fn hint_shell(mut self, shell: HintShell) -> Self {
        self.hint_shell = shell;
        self
    }

    /// Look up secret values in `store`, read-only, so a secret that has a value is not reported.
    #[must_use]
    pub fn secrets(mut self, store: Arc<dyn SecretStore>) -> Self {
        self.secrets = Some(store);
        self
    }
}

/// Check the collection at `root` and report every problem found.
///
/// Fails only when the directory is not a collection or `opts` names an
/// unknown environment; every other problem is a finding.
pub fn check_collection(root: &Path, opts: &CheckOptions) -> Result<CheckReport, CheckError> {
    check_collection_with(root, opts, &syntax_problems)
}

/// The script syntax pass, as a parameter so a test can make the engine fail to start.
type SyntaxCheck<'a> = &'a dyn Fn(&[Segment]) -> Result<Vec<Problem>, rquickjs::Error>;

/// `check_collection` with the syntax check supplied by the caller.
fn check_collection_with(
    root: &Path,
    opts: &CheckOptions,
    syntax: SyntaxCheck<'_>,
) -> Result<CheckReport, CheckError> {
    let collection = Collection::open(root)?;
    let environments: Vec<String> = match &opts.env {
        Some(name) => {
            if collection.environment(name).is_none() {
                return Err(CheckError::EnvironmentNotFound {
                    name: name.clone(),
                    available: collection
                        .environments()
                        .iter()
                        .map(|e| e.name().to_owned())
                        .collect(),
                });
            }
            vec![name.clone()]
        }
        None => collection
            .environments()
            .iter()
            .map(|e| e.name().to_owned())
            .collect(),
    };

    let mut findings: Vec<Finding> = collection
        .load_errors()
        .iter()
        .map(|e| load_finding(&collection, e))
        .collect();
    duplicate_sequences(&collection, collection.tree(), &mut findings);
    // Read from the raw files, so it holds whatever the environments or resolution say.
    unclosed::check_unclosed(&collection, collection.tree(), &mut findings);
    script_syntax(
        &collection,
        environments.first().map(String::as_str),
        syntax,
        &mut findings,
    );
    let unresolved = vars::check_variables(&collection, opts, &environments, &mut findings);
    let items_checked = count_requests(collection.tree());
    Ok(CheckReport::build(
        findings,
        items_checked,
        unresolved,
        environments,
    ))
}

/// The path relative to the root, with `/` separators.
fn relative(collection: &Collection, path: &Path) -> String {
    let rel = path.strip_prefix(collection.root()).unwrap_or(path);
    rel.components()
        .map(|c| c.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

fn load_finding(collection: &Collection, e: &FileError) -> Finding {
    Finding::new(
        relative(collection, e.path()),
        None,
        Severity::Error,
        FindingKind::UnparsableItem,
        e.message(),
    )
}

fn count_requests(folder: &Folder) -> usize {
    folder.requests().len() + folder.folders().iter().map(count_requests).sum::<usize>()
}

/// Warn on every sibling whose non-zero `seq` an earlier sibling already uses.
fn duplicate_sequences(collection: &Collection, folder: &Folder, out: &mut Vec<Finding>) {
    let mut seen: Vec<u64> = Vec::new();
    for sibling in folder.siblings() {
        let Some(seq) = sibling.seq().filter(|n| *n != 0) else {
            continue;
        };
        if seen.contains(&seq) {
            out.push(Finding::new(
                relative(collection, sibling.path()),
                None,
                Severity::Warning,
                FindingKind::DuplicateSequence,
                format!(
                    "{} shares sequence number {seq} with an earlier sibling",
                    sibling.name()
                ),
            ));
        } else {
            seen.push(seq);
        }
    }
    for sub in folder.folders() {
        duplicate_sequences(collection, sub, out);
    }
}

/// The suffix of a script problem that only a stricter parse than the engine's reports.
/// The level that names assertion and variable expressions in a syntax check.
const EXPRESSION: &str = "expression";
const SLOPPY_ONLY: &str = " (valid only in sloppy mode, the engine may run it)";

/// Parse-check every script text of every request, resolved or not. A problem the engine
/// itself would run past is a warning. Texts do not depend on the environment, so one
/// context serves the whole pass.
fn script_syntax(
    collection: &Collection,
    env: Option<&str>,
    syntax: SyntaxCheck<'_>,
    out: &mut Vec<Finding>,
) {
    // A context that cannot be built is reported by the variable pass.
    let mut options = crate::ContextOptions::new();
    if let Some(name) = env {
        options = options.env(name);
    }
    let Ok(ctx) = crate::VarContext::new(collection, options) else {
        return;
    };
    syntax_folder(
        collection,
        &ctx,
        collection.tree(),
        syntax,
        &mut HashSet::new(),
        out,
    );
}

fn syntax_folder(
    collection: &Collection,
    ctx: &crate::VarContext,
    folder: &Folder,
    syntax: SyntaxCheck<'_>,
    seen: &mut HashSet<String>,
    out: &mut Vec<Finding>,
) {
    for req in folder.requests() {
        let item = relative(collection, req.path());
        let mut segments: Vec<Segment> = Vec::new();
        for t in crate::script_texts(req, ctx) {
            match t.kind {
                // A collection or folder script is one file's defect: report it once, not
                // once per request that inherits it.
                crate::ScriptTextKind::Script { level, .. } if level != "request" => {
                    if !seen.contains(&level) || segments.iter().any(|s| s.level == level) {
                        segments.push(Segment {
                            level,
                            code: t.text,
                        });
                    }
                }
                crate::ScriptTextKind::Script { level, .. } => segments.push(Segment {
                    level,
                    code: t.text,
                }),
                _ => segments.push(Segment {
                    level: EXPRESSION.to_owned(),
                    code: format!("return ({});", t.text),
                }),
            }
        }
        seen.extend(segments.iter().map(|s| s.level.clone()));
        match syntax(&segments) {
            Ok(problems) => {
                for p in problems {
                    let (severity, message) = if p.sloppy_only {
                        (Severity::Warning, format!("{}{SLOPPY_ONLY}", p.message))
                    } else {
                        (Severity::Error, p.message)
                    };
                    let (item, message) = if p.level == EXPRESSION {
                        let rest = message
                            .strip_prefix("expression script")
                            .unwrap_or(&message);
                        (
                            item.clone(),
                            format!("assertion or variable expression{rest}"),
                        )
                    } else if p.level == "collection" {
                        ("opencollection.yml".to_owned(), message)
                    } else if let Some(dir) = p.level.strip_prefix("folder ") {
                        (format!("{dir}/folder.yml"), message)
                    } else {
                        (item.clone(), message)
                    };
                    out.push(Finding::new(
                        item,
                        None,
                        severity,
                        FindingKind::ScriptSyntax,
                        message,
                    ));
                }
            }
            Err(_) => out.push(Finding::new(
                item,
                None,
                Severity::Error,
                FindingKind::ScriptSyntax,
                "the script engine could not start, so scripts were not checked",
            )),
        }
    }
    for sub in folder.folders() {
        syntax_folder(collection, ctx, sub, syntax, seen, out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_engine_that_cannot_start_is_one_error_finding_per_request() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(
            root.join("opencollection.yml"),
            "opencollection: 1.0.0\ninfo:\n  name: c\n",
        )
        .unwrap();
        let request = "info:\n  name: a\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: \"https://x/\"\n";
        std::fs::write(root.join("a.yml"), request).unwrap();

        let no_engine = |_: &[Segment]| Err(rquickjs::Error::Allocation);
        let report = check_collection_with(root, &CheckOptions::default(), &no_engine).unwrap();

        let found: Vec<_> = report
            .findings()
            .iter()
            .map(|f| (f.item(), f.severity(), f.kind(), f.message()))
            .collect();
        assert_eq!(
            found,
            [(
                "a.yml",
                Severity::Error,
                FindingKind::ScriptSyntax,
                "the script engine could not start, so scripts were not checked"
            )]
        );
        assert_eq!(report.errors(), 1);
    }
}
