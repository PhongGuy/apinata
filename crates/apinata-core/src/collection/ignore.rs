//! Which paths a collection skips: Bruno's app watcher rule or `bruno-cli`'s walk.

use crate::document::Document;
use std::path::{Path, PathBuf};

/// Which Bruno client's idea of an ignored path a collection follows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum IgnorePolicy {
    /// `bruno-cli`'s walk: `.git` and `node_modules` directories are skipped, and a directory
    /// whose path below the root contains `node_modules` keeps only its `folder.yml`.
    #[default]
    BrunoCli,
    /// Bruno's app: also the root `mocks` directory and the paths listed under
    /// `extensions.bruno.ignore` in `opencollection.yml`.
    BrunoApp,
}

/// How to open a collection.
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct OpenOptions {
    /// The ignore rule applied to the walk, the environments and later reloads.
    pub ignore: IgnorePolicy,
}

impl OpenOptions {
    /// The defaults: the `BrunoCli` ignore policy.
    pub fn new() -> OpenOptions {
        OpenOptions::default()
    }

    /// Use `policy` to decide which paths are ignored.
    pub fn ignore(mut self, policy: IgnorePolicy) -> OpenOptions {
        self.ignore = policy;
        self
    }
}

/// One collection's ignore rule: the policy, the root it is relative to and the listed entries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Ignore {
    policy: IgnorePolicy,
    root: PathBuf,
    entries: Vec<String>,
}

impl Ignore {
    /// The rule for `root`, reading `extensions.bruno.ignore` from `doc` under `BrunoApp` only.
    /// Non-string entries are skipped; `\` is read as `/`.
    pub(crate) fn new(policy: IgnorePolicy, root: &Path, doc: &Document) -> Ignore {
        let entries = match policy {
            IgnorePolicy::BrunoApp => doc
                .get(&["extensions", "bruno", "ignore"])
                .and_then(|v| v.as_sequence())
                .map(|seq| {
                    seq.iter()
                        .filter_map(|e| e.as_str())
                        .map(|e| e.replace('\\', "/"))
                        .collect()
                })
                .unwrap_or_default(),
            IgnorePolicy::BrunoCli => Vec::new(),
        };
        Ignore {
            policy,
            root: root.to_path_buf(),
            entries,
        }
    }

    /// The same policy and root with the list read from `doc`.
    pub(crate) fn reread(&self, doc: &Document) -> Ignore {
        Ignore::new(self.policy, &self.root, doc)
    }

    /// `path` relative to the root with `/` separators; `None` outside the root.
    pub(crate) fn rel(&self, path: &Path) -> Option<String> {
        let rel = path.strip_prefix(&self.root).ok()?;
        let parts: Vec<_> = rel.iter().map(|s| s.to_string_lossy()).collect();
        Some(parts.join("/"))
    }

    /// Whether the path itself is ignored, as a file or a directory.
    pub(crate) fn matches(&self, rel: &str) -> bool {
        if rel.split('/').any(|s| s == "node_modules" || s == ".git") {
            return true;
        }
        match self.policy {
            IgnorePolicy::BrunoCli => false,
            IgnorePolicy::BrunoApp => {
                rel.split('/').next() == Some("mocks")
                    || self.entries.iter().any(|e| {
                        !e.is_empty()
                            && rel
                                .strip_prefix(e.as_str())
                                .is_some_and(|rest| rest.is_empty() || rest.starts_with('/'))
                    })
            }
        }
    }

    /// `bruno-cli`'s early return in `traverse`: a non-root directory whose relative path
    /// contains `node_modules` is loaded with its `folder.yml` and nothing else.
    pub(crate) fn skips_children_of(&self, dir_rel: &str) -> bool {
        self.policy == IgnorePolicy::BrunoCli
            && !dir_rel.is_empty()
            && dir_rel.contains("node_modules")
    }

    /// Whether core never loads `rel`: it matches, or its directory skips its children (a
    /// `folder.yml` is still read unless that directory was itself skipped by its parent).
    pub(crate) fn hides(&self, rel: &str) -> bool {
        if self.matches(rel) {
            return true;
        }
        let (parent, name) = rel.rsplit_once('/').unwrap_or(("", rel));
        if !self.skips_children_of(parent) {
            return false;
        }
        let grandparent = parent.rsplit_once('/').map_or("", |(g, _)| g);
        !(name == super::FOLDER_FILE && !self.skips_children_of(grandparent))
    }
}
