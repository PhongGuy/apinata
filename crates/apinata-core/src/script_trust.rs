//! The per-user script trust file: whether each collection may run scripts, and the exact
//! script texts the user approved for agent-driven sends, keyed by canonical collection root.

use crate::trust::{TrustError, key, symlink_target, write_atomic};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

/// Lowercase hex SHA-256 of the text's UTF-8 bytes.
pub fn script_hash(text: &str) -> String {
    Sha256::digest(text.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Whether `hash` has the shape [`script_hash`] produces: 64 lowercase hex digits.
pub(crate) fn is_script_hash(hash: &str) -> bool {
    hash.len() == 64 && hash.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// Whether a collection's scripts may run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ScriptDecision {
    /// Scripts run.
    Allowed,
    /// Scripts never run.
    Blocked,
}

impl ScriptDecision {
    fn name(self) -> &'static str {
        match self {
            ScriptDecision::Allowed => "allowed",
            ScriptDecision::Blocked => "blocked",
        }
    }
}

/// The per-user script decisions and approved script hashes, keyed by canonical collection
/// root. Changes stay in memory until [`ScriptTrust::save`]; saving replaces the file.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct ScriptTrust {
    path: PathBuf,
    collections: BTreeMap<String, Entry>,
}

#[derive(Debug, Clone, Default, PartialEq)]
struct Entry {
    decision: Option<ScriptDecision>,
    approved: BTreeSet<String>,
}

// Values are read loosely so a hand-edited entry degrades to "absent" rather than failing
// the whole file: an unknown decision or a malformed hash can only lose trust.
#[derive(Serialize, Deserialize)]
struct TrustFile {
    version: u32,
    #[serde(default)]
    collections: BTreeMap<String, EntryFile>,
}

#[derive(Serialize, Deserialize)]
struct EntryFile {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    scripts: Option<serde_json::Value>,
    #[serde(default)]
    approved: Vec<serde_json::Value>,
}

impl ScriptTrust {
    /// `<config dir>/apinata/script-trust.json`, or `None` when the platform has no config dir.
    pub fn default_path() -> Option<PathBuf> {
        dirs::config_dir().map(|dir| dir.join("apinata").join("script-trust.json"))
    }

    /// Load the trust file at [`ScriptTrust::default_path`].
    pub fn load_default() -> Result<ScriptTrust, TrustError> {
        Self::load(Self::default_path().ok_or(TrustError::NoDirectory)?)
    }

    /// Load the trust file at `path`; a missing file is empty. Unknown decisions read as
    /// undecided and malformed hashes are dropped. Fails on unreadable, non-JSON or
    /// non-version-1 files.
    pub fn load(path: impl Into<PathBuf>) -> Result<ScriptTrust, TrustError> {
        let path = path.into();
        let text = match fs::read_to_string(&path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Self::empty(path)),
            Err(e) => {
                return Err(TrustError::Io {
                    path,
                    message: e.to_string(),
                });
            }
        };
        let malformed = |message: String| TrustError::Malformed {
            path: path.clone(),
            message,
        };
        let file: TrustFile = serde_json::from_str(&text).map_err(|e| malformed(e.to_string()))?;
        if file.version != 1 {
            return Err(malformed(format!("unsupported version {}", file.version)));
        }
        let collections = file
            .collections
            .into_iter()
            .map(|(root, entry)| {
                let decision = match entry.scripts.as_ref().and_then(|v| v.as_str()) {
                    Some("allowed") => Some(ScriptDecision::Allowed),
                    Some("blocked") => Some(ScriptDecision::Blocked),
                    _ => None,
                };
                let approved = entry
                    .approved
                    .iter()
                    .filter_map(|v| v.as_str())
                    .filter(|h| is_script_hash(h))
                    .map(str::to_owned)
                    .collect();
                (root, Entry { decision, approved })
            })
            .collect();
        Ok(ScriptTrust { path, collections })
    }

    /// Loads the trust file at `path`, applies `change`, and saves it, all while holding an
    /// exclusive lock on a sidecar lock file, so concurrent writers in any process never lose
    /// one another's changes. A file that fails to load is neither changed nor written. The
    /// lock is not reentrant: `change` must not call `update` on the same file, or it deadlocks.
    pub fn update<R>(
        path: impl Into<PathBuf>,
        change: impl FnOnce(&mut ScriptTrust) -> R,
    ) -> Result<R, TrustError> {
        let path = path.into();
        // Lock beside the file `save` actually replaces, so a symlinked trust file shares
        // one lock with writers that name its target directly.
        let target = symlink_target(&path);
        let mut lock_name = target.file_name().unwrap_or_default().to_os_string();
        lock_name.push(".lock");
        let lock = target.with_file_name(lock_name);
        let io = |at: &Path, e: std::io::Error| TrustError::Io {
            path: at.to_path_buf(),
            message: e.to_string(),
        };
        if let Some(dir) = lock.parent().filter(|p| !p.as_os_str().is_empty()) {
            fs::create_dir_all(dir).map_err(|e| io(&lock, e))?;
        }
        let guard = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&lock)
            .map_err(|e| io(&lock, e))?;
        guard.lock().map_err(|e| io(&lock, e))?;
        let mut trust = Self::load(path)?;
        let out = change(&mut trust);
        trust.save()?;
        drop(guard);
        Ok(out)
    }

    /// An empty trust file that [`ScriptTrust::save`] writes to `path`.
    pub fn empty(path: impl Into<PathBuf>) -> ScriptTrust {
        ScriptTrust {
            path: path.into(),
            collections: BTreeMap::new(),
        }
    }

    /// The file this loads from and saves to.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// `collection`'s decision, or `None` when the user has not decided.
    pub fn decision(&self, collection: &Path) -> Option<ScriptDecision> {
        self.collections
            .get(&key(collection))
            .and_then(|e| e.decision)
    }

    /// Record `collection`'s decision; `None` makes it undecided. Approvals are kept.
    pub fn set_decision(&mut self, collection: &Path, decision: Option<ScriptDecision>) {
        self.collections
            .entry(key(collection))
            .or_default()
            .decision = decision;
    }

    /// Whether `hash` is approved for `collection`.
    pub fn is_approved(&self, collection: &Path, hash: &str) -> bool {
        self.collections
            .get(&key(collection))
            .is_some_and(|e| e.approved.contains(hash))
    }

    /// `collection`'s approved hashes.
    pub fn approved(&self, collection: &Path) -> BTreeSet<String> {
        self.collections
            .get(&key(collection))
            .map(|e| e.approved.clone())
            .unwrap_or_default()
    }

    /// Approve `hashes` for `collection`, ignoring any that are not [`script_hash`] values.
    pub fn approve<S: AsRef<str>>(
        &mut self,
        collection: &Path,
        hashes: impl IntoIterator<Item = S>,
    ) {
        let entry = self.collections.entry(key(collection)).or_default();
        entry.approved.extend(
            hashes
                .into_iter()
                .map(|h| h.as_ref().to_owned())
                .filter(|h| is_script_hash(h)),
        );
    }

    /// Write the file atomically as pretty JSON, creating the directory and dropping
    /// collections with no decision and no approvals.
    pub fn save(&self) -> Result<(), TrustError> {
        let collections = self
            .collections
            .iter()
            .filter(|(_, e)| e.decision.is_some() || !e.approved.is_empty())
            .map(|(root, e)| {
                let entry = EntryFile {
                    scripts: e.decision.map(|d| d.name().into()),
                    approved: e.approved.iter().map(|h| h.as_str().into()).collect(),
                };
                (root.clone(), entry)
            })
            .collect();
        let file = TrustFile {
            version: 1,
            collections,
        };
        let mut bytes = serde_json::to_vec_pretty(&file).map_err(|e| TrustError::Io {
            path: self.path.clone(),
            message: e.to_string(),
        })?;
        bytes.push(b'\n');
        write_atomic(&self.path, &bytes)
    }
}
