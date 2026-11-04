//! `workspace.yml` read as a view: the collections and shared environments of a workspace.
//! There is no write API; the file is never changed.

use crate::error::OpenError;
use crate::global_env::GlobalEnvironments;
use std::path::{Path, PathBuf};
use yaml_serde::{Mapping, Value};

const FILE: &str = "workspace.yml";

/// Why a folder could not be read as a workspace.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum WorkspaceError {
    /// The folder has no `workspace.yml`.
    #[error("{} is not a workspace: no workspace.yml", dir.display())]
    NotAWorkspace {
        /// The folder that was tried.
        dir: PathBuf,
    },
    /// `workspace.yml` is not a workspace file.
    #[error("{}: {message}", path.display())]
    Malformed {
        /// The file.
        path: PathBuf,
        /// Why it was rejected.
        message: String,
    },
    /// The file could not be read.
    #[error("{}: {source}", path.display())]
    Io {
        /// The path that failed.
        path: PathBuf,
        /// The underlying error.
        #[source]
        source: std::io::Error,
    },
}

/// One collection listed by a workspace.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct WorkspaceCollection {
    name: String,
    path: String,
    root: PathBuf,
    remote: Option<String>,
    extra: Vec<(String, String)>,
}

impl WorkspaceCollection {
    /// The display name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The path as written, relative to the workspace folder or absolute.
    pub fn path(&self) -> &str {
        &self.path
    }

    /// The collection folder: `path` resolved against the workspace folder.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The `remote` URL, when listed.
    pub fn remote(&self) -> Option<&str> {
        self.remote.as_deref()
    }

    /// Unknown keys of the entry as (key, YAML text of the value).
    pub fn extra(&self) -> &[(String, String)] {
        &self.extra
    }
}

/// One spec listed by a workspace.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct WorkspaceSpec {
    name: String,
    path: String,
}

impl WorkspaceSpec {
    /// The display name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The path as written.
    pub fn path(&self) -> &str {
        &self.path
    }
}

/// A workspace folder's `workspace.yml`, read-only.
#[derive(Debug)]
#[non_exhaustive]
pub struct WorkspaceFile {
    dir: PathBuf,
    name: String,
    collections: Vec<WorkspaceCollection>,
    skipped: usize,
    specs: Vec<WorkspaceSpec>,
    docs: String,
    extra: Vec<(String, String)>,
}

/// Whether `dir/workspace.yml` is a file.
pub fn is_workspace_dir(dir: impl AsRef<Path>) -> bool {
    dir.as_ref().join(FILE).is_file()
}

fn text(value: &Value) -> Option<&str> {
    value.as_str().filter(|s| !s.trim().is_empty())
}

fn extras(map: &Mapping, known: &[&str]) -> Vec<(String, String)> {
    map.iter()
        .filter_map(|(k, v)| {
            let key = k.as_str()?;
            if known.contains(&key) {
                return None;
            }
            let body = yaml_serde::to_string(v).ok()?;
            Some((key.to_string(), body.trim_end().to_string()))
        })
        .collect()
}

impl WorkspaceFile {
    /// Read `dir/workspace.yml`. A file that is not a mapping, or whose `info.type` is not
    /// `workspace` or whose `info.name` is not a string, is [`WorkspaceError::Malformed`].
    pub fn open(dir: impl Into<PathBuf>) -> Result<WorkspaceFile, WorkspaceError> {
        let dir = dir.into();
        let path = dir.join(FILE);
        let raw = match std::fs::read_to_string(&path) {
            Ok(raw) => raw,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(WorkspaceError::NotAWorkspace { dir });
            }
            Err(source) => return Err(WorkspaceError::Io { path, source }),
        };
        let malformed = |message: &str| WorkspaceError::Malformed {
            path: path.clone(),
            message: message.to_string(),
        };
        let value: Value = yaml_serde::from_str(&raw).map_err(|e| malformed(&e.to_string()))?;
        let map = value
            .as_mapping()
            .ok_or_else(|| malformed("not a YAML mapping"))?;
        let info = map.get("info").and_then(Value::as_mapping);
        if info.and_then(|i| i.get("type")).and_then(Value::as_str) != Some("workspace") {
            return Err(malformed("info.type is not workspace"));
        }
        let name = info
            .and_then(|i| i.get("name"))
            .and_then(Value::as_str)
            .ok_or_else(|| malformed("info.name is missing"))?
            .to_string();

        let mut collections = Vec::new();
        let mut skipped = 0;
        let entries = map.get("collections").and_then(Value::as_sequence);
        for entry in entries.into_iter().flatten() {
            let parsed = entry.as_mapping().and_then(|m| {
                let name = text(m.get("name")?)?;
                let path = text(m.get("path")?)?;
                Some(WorkspaceCollection {
                    name: name.to_string(),
                    path: path.to_string(),
                    root: dir.join(path),
                    remote: m.get("remote").and_then(Value::as_str).map(String::from),
                    extra: extras(m, &["name", "path", "remote"]),
                })
            });
            match parsed {
                Some(c) => collections.push(c),
                None => skipped += 1,
            }
        }

        let specs = map
            .get("specs")
            .and_then(Value::as_sequence)
            .into_iter()
            .flatten()
            .filter_map(|s| {
                let m = s.as_mapping()?;
                Some(WorkspaceSpec {
                    name: text(m.get("name")?)?.to_string(),
                    path: text(m.get("path")?)?.to_string(),
                })
            })
            .collect();

        let docs = map
            .get("docs")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let extra = extras(
            map,
            &["opencollection", "info", "collections", "specs", "docs"],
        );
        Ok(WorkspaceFile {
            dir,
            name,
            collections,
            skipped,
            specs,
            docs,
            extra,
        })
    }

    /// `info.name`.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The workspace folder.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// The valid collection entries, in file order.
    pub fn collections(&self) -> &[WorkspaceCollection] {
        &self.collections
    }

    /// How many `collections` entries were invalid and skipped.
    pub fn skipped(&self) -> usize {
        self.skipped
    }

    /// The valid spec entries; empty when `specs` is not a list.
    pub fn specs(&self) -> &[WorkspaceSpec] {
        &self.specs
    }

    /// The `docs` text, empty when absent.
    pub fn docs(&self) -> &str {
        &self.docs
    }

    /// Unknown top-level keys as (key, YAML text of the value).
    pub fn extra(&self) -> &[(String, String)] {
        &self.extra
    }

    /// The workspace's shared environments, from `<dir>/environments`.
    pub fn environments(&self) -> Result<GlobalEnvironments, OpenError> {
        GlobalEnvironments::open(self.dir.join("environments"))
    }
}
