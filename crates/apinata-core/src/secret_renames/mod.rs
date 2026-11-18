//! Stored secrets that a pulled environment or variable rename left under their old name.

use std::collections::BTreeSet;
use std::fmt;
use std::path::Path;

use crate::collection::{Collection, Environment};
use crate::document::Document;
use crate::env_file::EnvVariable;
use crate::env_ops::secret_names;
use crate::error::EnvError;
use crate::git::{EnvChanges, GitError, GitOptions, blob_at, env_changes, resolve_commit};
use crate::secrets::{SecretKey, SecretStore};
use crate::team::ConflictCopies;

mod moves;

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

    /// From the commit `rev` names to `HEAD`, both resolved in the repository holding `root`;
    /// `None` when either names no commit.
    pub fn to_head(
        root: &Path,
        rev: &str,
        opts: &GitOptions,
    ) -> Result<Option<PullRange>, GitError> {
        let Some(from) = resolve_commit(root, rev, opts)? else {
            return Ok(None);
        };
        Ok(resolve_commit(root, "HEAD", opts)?.map(|to| PullRange::new(from, to)))
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

impl fmt::Display for SecretName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.environment, self.variable)
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

/// Whether a rename can be moved without a person choosing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum RenameStatus {
    /// Sure, with one new name that is free or already holds the same value.
    Ready,
    /// Not sure, or several new names: a person picks one.
    Ambiguous,
    /// The one new name holds a different value, which is never overwritten.
    Taken,
}

impl SecretRename {
    /// A row moving `from` to one of `to`.
    pub fn new(from: SecretName, to: Vec<(SecretName, TargetState)>, sure: bool) -> SecretRename {
        SecretRename { from, to, sure }
    }

    /// Whether the row can move as it stands.
    pub fn status(&self) -> RenameStatus {
        match self.to.as_slice() {
            [(_, TargetState::Taken)] => RenameStatus::Taken,
            [(_, TargetState::Free | TargetState::Same)] if self.sure => RenameStatus::Ready,
            _ => RenameStatus::Ambiguous,
        }
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
        copies: Option<&ConflictCopies>,
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
            let Some(was) = self.env_at(&range.from, &pair.old, opts)? else {
                continue;
            };
            let was_secret: BTreeSet<String> = secret_names(&was).into_iter().collect();
            let mut nows = Vec::new();
            for stem in &pair.new {
                if let Some(now) = self.env_at(&range.to, stem, opts)? {
                    nows.push((stem, now.variables()));
                }
            }
            let stored = orphans
                .orphans
                .iter()
                .filter(|o| o.environment == pair.old && was_secret.contains(&o.variable));
            for orphan in stored {
                let from = SecretName::new(&pair.old, &orphan.variable);
                // Each candidate stem crossed with the names the variable may have taken there.
                let mut wanted = Vec::new();
                let mut kept = true;
                for (stem, now) in &nows {
                    let (vars, same) = var_targets(&orphan.variable, &was_secret, now);
                    kept &= same;
                    wanted.extend(
                        vars.into_iter()
                            .map(|v| SecretName::new(*stem, v))
                            .filter(|to| *to != from),
                    );
                }
                let to = self.targets(&from, &wanted, store)?;
                if !to.is_empty() {
                    renames.push(SecretRename::new(from, to, pair.sure && kept));
                }
            }
        }
        renames.sort_by(|a, b| a.from.cmp(&b.from));
        Ok(hold_back(renames, copies))
    }

    /// Environment `stem`'s file at commit `rev`; `None` when that commit has no such file or
    /// it does not parse, which gives the pair no rows.
    fn env_at(
        &self,
        rev: &str,
        stem: &str,
        opts: &GitOptions,
    ) -> Result<Option<Environment>, EnvError> {
        let rel = format!("environments/{stem}.yml");
        let bytes = blob_at(self.root(), rev, &rel, opts).map_err(scan_failed)?;
        let doc = bytes.and_then(|b| Document::parse(&self.root().join(&rel), b).ok());
        Ok(doc.map(|doc| Environment::from_parts(stem.to_owned(), doc)))
    }

    /// The names environment `stem`'s file on disk marks secret; empty when it is missing,
    /// does not parse or `stem` is not a plain file stem. Read from disk rather than the loaded
    /// environments, since a pull lands before the collection reloads.
    pub(crate) fn declared_on_disk(&self, stem: &str) -> Vec<String> {
        if stem.is_empty() || stem == ".." || stem.contains(['/', '\\']) {
            return Vec::new();
        }
        let path = self.root().join(format!("environments/{stem}.yml"));
        Document::load(&path)
            .map(|doc| secret_names(&Environment::from_parts(stem.to_owned(), doc)))
            .unwrap_or_default()
    }

    /// Each of `wanted` that its environment's file declares secret now, with the store's
    /// state there compared against the value under `from`.
    fn targets(
        &self,
        from: &SecretName,
        wanted: &[SecretName],
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
        for to in wanted {
            let declared = self.declared_on_disk(&to.environment);
            if !declared.contains(&to.variable) {
                continue;
            }
            let state = match read(&self.secret_key(&to.environment, &to.variable))? {
                None => TargetState::Free,
                Some(v) if Some(&v) == old.as_ref() => TargetState::Same,
                Some(_) => TargetState::Taken,
            };
            out.push((to.clone(), state));
        }
        Ok(out)
    }
}

/// The names secret `var` of an old file may have in `now`, and whether it kept its name.
/// A secret gone from `now` may have become any secret `now` added; one kept but made plain
/// is not gone, so it is never paired.
fn var_targets(
    var: &str,
    was_secret: &BTreeSet<String>,
    now: &[EnvVariable],
) -> (Vec<String>, bool) {
    if now.iter().any(|v| v.name == var) {
        return (vec![var.to_owned()], true);
    }
    let added: BTreeSet<&String> = now
        .iter()
        .filter(|v| v.secret && !was_secret.contains(&v.name))
        .map(|v| &v.name)
        .collect();
    (added.into_iter().cloned().collect(), false)
}

/// Split `rows` into those to offer and a count of those touching a file with an unsettled
/// copy or local-delete mark. Both lists are read afresh, and a list that cannot be read holds
/// every row back, since a move made mid-clash could follow the wrong side.
fn hold_back(rows: Vec<SecretRename>, copies: Option<&ConflictCopies>) -> SecretRenames {
    let Some(copies) = copies else {
        return SecretRenames::new(rows, 0);
    };
    let clashing: Option<BTreeSet<String>> = match (copies.list(), copies.deleted()) {
        (Ok(list), Ok(deleted)) => Some(list.into_iter().chain(deleted).collect()),
        _ => None,
    };
    let (held, offered): (Vec<_>, Vec<_>) = rows.into_iter().partition(|row| {
        clashing.as_ref().is_none_or(|paths| {
            std::iter::once(&row.from)
                .chain(row.to.iter().map(|(to, _)| to))
                .any(|name| paths.contains(&format!("environments/{}.yml", name.environment)))
        })
    });
    SecretRenames::new(offered, held.len())
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

/// Every removed environment paired with every added one, and each modified one with itself. A
/// rename pairing is sure only when the range removed one environment, added one, and git matched
/// the two as a rename; a modified environment is always sure.
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
        .chain(changes.modified.iter().map(|stem| EnvPair {
            old: stem.clone(),
            new: vec![stem.clone()],
            sure: true,
        }))
        .collect()
}
