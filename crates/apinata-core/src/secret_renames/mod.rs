//! Stored secrets that a pulled environment rename left under their old name.

use std::collections::BTreeSet;

use crate::collection::{Collection, Environment};
use crate::document::Document;
use crate::error::EnvError;
use crate::git::{EnvChanges, GitError, GitOptions, blob_at, env_changes};
use crate::secrets::{SecretKey, SecretStore};
use crate::team::ConflictCopies;

/// The commits a pull moved the collection between.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct PullRange {
    /// The commit before the pull.
    pub from: String,
    /// The commit the pull landed on.
    pub to: String,
}

impl PullRange {
    /// A range from commit `from` to commit `to`.
    pub fn new(from: impl Into<String>, to: impl Into<String>) -> PullRange {
        PullRange {
            from: from.into(),
            to: to.into(),
        }
    }
}

/// A secret's place in the store: an environment's file stem and a variable.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub struct SecretName {
    /// The environment's file stem.
    pub environment: String,
    /// The variable name.
    pub variable: String,
}

impl SecretName {
    /// The name of `variable` in `environment`.
    pub fn new(environment: impl Into<String>, variable: impl Into<String>) -> SecretName {
        SecretName {
            environment: environment.into(),
            variable: variable.into(),
        }
    }
}

/// What the store holds under a rename's new name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum TargetState {
    /// Nothing.
    Free,
    /// The same value as the old name.
    Same,
    /// A different value.
    Taken,
}

/// One stored secret left under an old name, with the names it may have moved to.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct SecretRename {
    /// The name the value is stored under.
    pub from: SecretName,
    /// Each candidate new name, most likely first, with what the store holds there.
    pub to: Vec<(SecretName, TargetState)>,
    /// Whether the pull leaves exactly one reading of the rename.
    pub sure: bool,
}

impl SecretRename {
    /// A row moving `from` to one of `to`.
    pub fn new(from: SecretName, to: Vec<(SecretName, TargetState)>, sure: bool) -> SecretRename {
        SecretRename { from, to, sure }
    }
}

/// What a scan of a pull range found.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct SecretRenames {
    /// The rows, sorted by old name.
    pub renames: Vec<SecretRename>,
    /// Renames not listed because they touch a clash that is not settled yet.
    pub held_back: usize,
}

impl SecretRenames {
    /// A scan result of `renames` with `held_back` more waiting.
    pub fn new(renames: Vec<SecretRename>, held_back: usize) -> SecretRenames {
        SecretRenames { renames, held_back }
    }
}

impl Collection {
    /// Renames in `range` with a value stored under the old name, sorted by old name; renames
    /// touching an unsettled clash in `copies` are counted in `held_back` instead of listed.
    /// [`EnvError::RenameScan`] when git cannot say what the range changed; a range commit not
    /// in the repository finds nothing.
    pub fn secret_renames(
        &self,
        range: &PullRange,
        _copies: Option<&ConflictCopies>,
        store: &dyn SecretStore,
        opts: &GitOptions,
    ) -> Result<SecretRenames, EnvError> {
        let Some(changes) =
            env_changes(self.root(), &range.from, &range.to, opts).map_err(scan_failed)?
        else {
            return Ok(SecretRenames::default());
        };
        let pairs = env_pairs(&changes);
        if pairs.is_empty() {
            return Ok(SecretRenames::default());
        }
        // Only a value orphaned now is offered, so nothing still in use is ever moved.
        let orphans = self.all_orphaned_secrets(store)?;
        let mut renames = Vec::new();
        for pair in pairs {
            let Some(was_secret) = self.secrets_at(&range.from, &pair.old, opts)? else {
                continue;
            };
            let stored = orphans
                .orphans
                .iter()
                .filter(|o| o.environment == pair.old && was_secret.contains(&o.variable));
            for orphan in stored {
                let from = SecretName::new(&pair.old, &orphan.variable);
                let to = self.targets(&from, &pair.new, store)?;
                if !to.is_empty() {
                    renames.push(SecretRename::new(from, to, pair.sure));
                }
            }
        }
        renames.sort_by(|a, b| a.from.cmp(&b.from));
        Ok(SecretRenames::new(renames, 0))
    }

    /// The names marked secret in environment `stem`'s file at commit `rev`; `None` when that
    /// commit has no such file or it does not parse, which gives the pair no rows.
    fn secrets_at(
        &self,
        rev: &str,
        stem: &str,
        opts: &GitOptions,
    ) -> Result<Option<BTreeSet<String>>, EnvError> {
        let rel = format!("environments/{stem}.yml");
        let bytes = blob_at(self.root(), rev, &rel, opts).map_err(scan_failed)?;
        let Some(doc) = bytes.and_then(|b| Document::parse(&self.root().join(&rel), b).ok()) else {
            return Ok(None);
        };
        let env = Environment::from_parts(stem.to_owned(), doc);
        let names = env.variables().into_iter().filter(|v| v.secret);
        Ok(Some(names.map(|v| v.name).collect()))
    }

    /// Each of `stems` whose file declares `from`'s variable secret now, with the store's state
    /// there compared against the value under `from`.
    fn targets(
        &self,
        from: &SecretName,
        stems: &[String],
        store: &dyn SecretStore,
    ) -> Result<Vec<(SecretName, TargetState)>, EnvError> {
        let read = |key: &SecretKey| {
            store.get(key).map_err(|error| EnvError::SecretStore {
                environment: key.environment.clone(),
                variable: key.variable.clone(),
                error,
            })
        };
        let old = read(&self.secret_key(&from.environment, &from.variable))?;
        let mut out = Vec::new();
        for stem in stems {
            let declared = self.try_saved_secret_names(stem).unwrap_or_default();
            if !declared.contains(&from.variable) {
                continue;
            }
            let new = read(&self.secret_key(stem, &from.variable))?;
            let state = match new {
                None => TargetState::Free,
                Some(v) if Some(&v) == old.as_ref() => TargetState::Same,
                Some(_) => TargetState::Taken,
            };
            out.push((SecretName::new(stem, &from.variable), state));
        }
        Ok(out)
    }
}

/// A git failure while reading the range, which must not read as "nothing to move".
fn scan_failed(e: GitError) -> EnvError {
    EnvError::RenameScan {
        message: e.to_string(),
    }
}

/// One environment the range removed, with the stems it may have become.
struct EnvPair {
    old: String,
    /// Git's rename partner first, then the other added stems sorted.
    new: Vec<String>,
    sure: bool,
}

/// Every removed environment paired with every added one. A pairing is sure only when the range
/// removed one environment, added one, and git matched the two as a rename.
fn env_pairs(changes: &EnvChanges) -> Vec<EnvPair> {
    let deleted: BTreeSet<&String> = changes
        .deleted
        .iter()
        .chain(changes.renamed.iter().map(|(old, _)| old))
        .collect();
    let added: BTreeSet<&String> = changes
        .added
        .iter()
        .chain(changes.renamed.iter().map(|(_, new)| new))
        .collect();
    let sure = deleted.len() == 1 && added.len() == 1 && changes.renamed.len() == 1;
    deleted
        .into_iter()
        .map(|old| {
            let partner = changes
                .renamed
                .iter()
                .find(|(o, _)| o == old)
                .map(|(_, n)| n);
            let rest = added.iter().filter(|n| Some(**n) != partner).copied();
            EnvPair {
                old: old.clone(),
                new: partner.into_iter().chain(rest).cloned().collect(),
                sure,
            }
        })
        .filter(|pair| !pair.new.is_empty())
        .collect()
}
