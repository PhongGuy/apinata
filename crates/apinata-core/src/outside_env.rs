//! An environment read from a file outside the collection (`run --env-file`). Its secrets never
//! come from the collection's secret store, since such a file may carry any environment's name.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::Path;

use apinata_format::ReadErrorKind;
use serde_json::Value;

use crate::collection::Environment;
use crate::document::{Document, read_regular};
use crate::edit::VarValue;
use crate::environment::{EnvValue, ResolvedEnv, resolve_environment};
use crate::import::{EnvironmentFormat, ImportedEnvironment, parse_environment};

/// The kind of file an [`EnvironmentFile`] was read from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum EnvFileKind {
    /// An OpenCollection environment, as kept under `environments/`.
    OpenCollection,
    /// A Postman environment export.
    Postman,
}

/// Why [`EnvironmentFile::load`] failed. Messages name the path, never a value.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum EnvFileError {
    /// The file could not be read.
    #[error("could not read {path}: {reason}")]
    Read {
        /// The path as given.
        path: String,
        /// The system's reason.
        reason: String,
    },
    /// The file is not valid JSON or YAML.
    #[error("{path} is not valid JSON or YAML: {message}")]
    Parse {
        /// The path as given.
        path: String,
        /// The parser's message, with its position when known.
        message: String,
    },
    /// The file parses but has neither environment shape.
    #[error(
        "{path} is not an environment file (expected an OpenCollection environment or a Postman environment export)"
    )]
    Unrecognized {
        /// The path as given.
        path: String,
    },
    /// The file is an environment export of a kind that is not read here, such as Hoppscotch.
    #[error(
        "{path} is an environment export whose format is not supported (expected an OpenCollection environment or a Postman environment export)"
    )]
    Unsupported {
        /// The path as given.
        path: String,
    },
    /// An OpenCollection environment names a parent, which only a collection can hold.
    #[error("{path} uses extends, which an environment file outside the collection cannot")]
    Extends {
        /// The path as given.
        path: String,
    },
    /// An export of a recognized kind that its reader refused.
    #[error("{path}: {message}")]
    Invalid {
        /// The path as given.
        path: String,
        /// The reader's message.
        message: String,
    },
}

/// The variables of an environment file outside the collection, ready for
/// [`ContextOptions::env_file`](crate::ContextOptions::env_file).
#[derive(Clone)]
#[non_exhaustive]
pub struct EnvironmentFile {
    name: String,
    kind: EnvFileKind,
    /// The enabled variables; a secret without a value is [`EnvValue::Secret`].
    vars: BTreeMap<String, EnvValue>,
    /// Secrets the file carries a value for, which stay hidden in logs and messages.
    sensitive: BTreeSet<String>,
    /// What the Postman reader changed or dropped in `values`, as `source: detail`.
    notes: Vec<String>,
}

impl EnvironmentFile {
    /// Read the environment at `path`, telling a Postman export from an OpenCollection
    /// environment by its content. An OpenCollection file is named by its stem
    /// and may not use `extends`.
    pub fn load(path: &Path) -> Result<EnvironmentFile, EnvFileError> {
        let shown = path.display().to_string();
        let mut bytes = read_regular(path).map_err(|e| EnvFileError::Read {
            path: shown.clone(),
            reason: e.to_string(),
        })?;
        // Editors on Windows often write a byte order mark, which JSON parsers refuse.
        if bytes.starts_with(b"\xEF\xBB\xBF") {
            bytes.drain(..3);
        }
        // A Postman export is told apart by its `values` list; other importable shapes are not
        // tried, since they overlap an OpenCollection file's `name` and `variables`.
        if EnvironmentFormat::detect(&bytes) == Some(EnvironmentFormat::Postman) {
            // The detector also reads YAML, but Postman only ever writes JSON.
            if serde_json::from_slice::<serde::de::IgnoredAny>(&bytes).is_err() {
                return Err(EnvFileError::Invalid {
                    path: shown,
                    message: "a Postman environment export must be JSON".to_owned(),
                });
            }
            let format = Some(EnvironmentFormat::Postman);
            let imported =
                parse_environment(format, &bytes).map_err(|e| EnvFileError::Invalid {
                    path: shown.clone(),
                    message: e.to_string(),
                })?;
            return Ok(EnvironmentFile::imported(imported));
        }
        EnvironmentFile::open_collection(path, shown, bytes)
    }

    /// The environment's name: the export's own, or an OpenCollection file's stem.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The kind of file it was read from.
    pub fn kind(&self) -> EnvFileKind {
        self.kind
    }

    /// What reading a Postman export renamed or dropped among its values, one `source: detail`
    /// line each, naming variables but never their values.
    pub fn notes(&self) -> &[String] {
        &self.notes
    }

    /// The names the file marks secret, with or without a value, sorted.
    pub fn secret_names(&self) -> Vec<String> {
        let unstored = self
            .vars
            .iter()
            .filter(|(_, value)| **value == EnvValue::Secret)
            .map(|(name, _)| name);
        let names: BTreeSet<&String> = unstored.chain(&self.sensitive).collect();
        names.into_iter().cloned().collect()
    }

    /// The layer a context starts from. No secret has an owner, so the store is never asked.
    pub(crate) fn resolved(&self) -> ResolvedEnv {
        ResolvedEnv {
            vars: self.vars.clone(),
            ..ResolvedEnv::default()
        }
    }

    pub(crate) fn sensitive(&self) -> &BTreeSet<String> {
        &self.sensitive
    }

    /// An export's enabled variables; a secret with a value is used as given, but kept hidden.
    fn imported(env: ImportedEnvironment) -> EnvironmentFile {
        let mut vars = BTreeMap::new();
        let mut sensitive = BTreeSet::new();
        for var in env.variables.into_iter().filter(|v| v.enabled) {
            let text = match var.value {
                VarValue::Text(text) => text,
                VarValue::Typed { data, .. } => data,
            };
            sensitive.remove(&var.name);
            let value = if var.secret && text.is_empty() {
                EnvValue::Secret
            } else {
                if var.secret {
                    sensitive.insert(var.name.clone());
                }
                EnvValue::Plain(Value::String(text))
            };
            vars.insert(var.name, value);
        }
        // Notes on the export's other top-level keys, such as `id`, are noise for a run.
        let notes = env
            .notes
            .iter()
            .filter(|n| n.source.starts_with("values"))
            .map(|n| format!("{}: {}", n.source, n.detail))
            .collect();
        EnvironmentFile {
            name: env.name,
            kind: EnvFileKind::Postman,
            vars,
            sensitive,
            notes,
        }
    }

    /// An OpenCollection environment: a mapping holding a `variables` list.
    fn open_collection(
        path: &Path,
        shown: String,
        bytes: Vec<u8>,
    ) -> Result<EnvironmentFile, EnvFileError> {
        let root = apinata_format::read_mapping(&bytes).map_err(|e| match e.kind() {
            ReadErrorKind::NotMapping => EnvFileError::Unrecognized {
                path: shown.clone(),
            },
            _ => EnvFileError::Parse {
                path: shown.clone(),
                message: e.to_string(),
            },
        })?;
        if root.get("extends").is_some_and(|v| !v.is_null()) {
            return Err(EnvFileError::Extends { path: shown });
        }
        let Some(yaml_serde::Value::Sequence(variables)) = root.get("variables") else {
            return Err(EnvFileError::Unrecognized { path: shown });
        };
        // A Hoppscotch export shares `name` and `variables`, but marks itself with `v` and
        // names its variables by `key`.
        let nameless = !variables.is_empty() && variables.iter().all(|v| v.get("name").is_none());
        if root.get("v").is_some() || nameless {
            return Err(EnvFileError::Unsupported { path: shown });
        }
        let name = path
            .file_stem()
            .map_or_else(String::new, |s| s.to_string_lossy().into_owned());
        let env = Environment::from_parts(name.clone(), Document::new(path.to_path_buf(), root));
        let vars = resolve_environment(std::slice::from_ref(&env), &name)
            .map(|r| r.vars)
            .unwrap_or_default();
        Ok(EnvironmentFile {
            name,
            kind: EnvFileKind::OpenCollection,
            vars,
            sensitive: BTreeSet::new(),
            notes: Vec::new(),
        })
    }
}

/// Shows names only, since variables often carry credentials.
impl fmt::Debug for EnvironmentFile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EnvironmentFile")
            .field("name", &self.name)
            .field("kind", &self.kind)
            .field("variables", &self.vars.keys().collect::<Vec<_>>())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Collection, ContextOptions, MemoryStore, Persistence, TokenCache, VarContext};
    use std::sync::Arc;

    /// The persistent token cache keys tokens by environment name, so a file named like a
    /// collection environment must not see that environment's tokens.
    #[test]
    fn a_file_environment_keeps_tokens_in_memory_only() {
        let dir = tempfile::TempDir::new().expect("temp dir");
        std::fs::write(
            dir.path().join("opencollection.yml"),
            "opencollection: 1.0.0\ninfo:\n  name: c\n",
        )
        .expect("write");
        let outside = tempfile::TempDir::new().expect("temp dir");
        let env = outside.path().join("prod.yml");
        std::fs::write(&env, "variables: []\n").expect("write");
        let c = Collection::open(dir.path()).expect("opens");
        let cache = TokenCache::open(dir.path().join("tokens.json"), Arc::new(MemoryStore::new()));
        let options = ContextOptions::new()
            .oauth2_tokens(cache)
            .env_file(EnvironmentFile::load(&env).expect("loads"));
        let ctx = VarContext::new(&c, options).expect("context");
        assert!(matches!(
            ctx.oauth2_tokens().persistence(),
            Persistence::MemoryOnly { .. }
        ));
    }
}
