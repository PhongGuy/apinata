//! Sends that failed on the network (refused, DNS, TLS, timed out, dropped), kept as a short
//! list in the response store's directory so History can show them. They hold no response, so
//! they never count against the kept sends; the list has its own caps and shares the store's
//! lock, clearing, renames and removals.

use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use apinata_core::{ItemPath, RequestResult, RunError};
use serde::{Deserialize, Serialize};

use crate::last_response::{Limits, RecentFailure, ResponseStore, kept_url};
use crate::persist;
use crate::stale::{is_within, rebase};

/// The list's file name inside the store directory.
pub(crate) const FILE: &str = "failures.json";
/// The most failures kept across every request.
const FAILURE_LIMIT: usize = 500;
const VERSION: u32 = 1;

#[derive(Serialize, Deserialize)]
struct FailureFile {
    version: u32,
    /// Oldest first.
    failures: Vec<FailureMeta>,
}

#[derive(Clone, Serialize, Deserialize)]
struct FailureMeta {
    root: String,
    item: String,
    at_ms: u64,
    method: String,
    url: String,
    reason: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    origin: Option<String>,
}

impl ResponseStore {
    /// Adds `result` to the failure list of (root, item) when it failed on the network without
    /// a response, keeping the newest `limits.per_request` per request. The URL is scrubbed of
    /// `secrets`, the values the scripts read and any credential core derived. Returns whether
    /// anything was kept.
    #[allow(clippy::too_many_arguments)]
    pub fn save_failure(
        &self,
        root: &Path,
        item: &ItemPath,
        result: &RequestResult,
        method: &str,
        secrets: &[String],
        at: SystemTime,
        limits: Limits,
        origin: Option<String>,
    ) -> io::Result<bool> {
        let failure = result.error.as_ref().and_then(RunError::send_failure);
        let (Some((url, reason)), None) = (failure, &result.response) else {
            return Ok(false);
        };
        let secrets: Vec<String> = secrets
            .iter()
            .chain(result.script_sensitive_values())
            .chain(result.derived_credentials())
            .cloned()
            .collect();
        let meta = FailureMeta {
            root: root.to_string_lossy().into_owned(),
            item: item.as_str().to_owned(),
            at_ms: at
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64,
            method: method.to_owned(),
            url: kept_url(url, &secrets),
            reason: reason.to_owned(),
            origin,
        };
        let (root, item) = (meta.root.clone(), meta.item.clone());
        let _guard = self.guard();
        let mut list = self.read_failures();
        list.push(meta);
        // Drop this request's oldest past its limit, then the oldest overall past the cap.
        let mine = |f: &FailureMeta| f.root == root && f.item == item;
        let count = list.iter().filter(|f| mine(f)).count();
        let mut excess = count.saturating_sub(limits.per_request.max(1));
        list.retain(|f| {
            let drop = excess > 0 && mine(f);
            excess -= usize::from(drop);
            !drop
        });
        let over = list.len().saturating_sub(FAILURE_LIMIT);
        list.drain(..over);
        std::fs::create_dir_all(self.dir())?;
        self.write_failures(list)?;
        Ok(true)
    }

    /// Failures under any of `roots`, newest first.
    pub fn failures(&self, roots: &[PathBuf]) -> Vec<RecentFailure> {
        let _guard = self.read_guard();
        let roots: Vec<String> = roots
            .iter()
            .map(|r| r.to_string_lossy().into_owned())
            .collect();
        let mut listed: Vec<RecentFailure> = self
            .read_failures()
            .into_iter()
            .rev()
            .filter(|f| roots.contains(&f.root))
            .filter_map(|f| {
                Some(RecentFailure {
                    item: ItemPath::new(&f.item)?,
                    root: PathBuf::from(f.root),
                    at: UNIX_EPOCH + Duration::from_millis(f.at_ms),
                    method: f.method,
                    url: f.url,
                    reason: f.reason,
                    origin: f.origin,
                })
            })
            .collect();
        // Stable, so failures within one millisecond stay newest first.
        listed.sort_by_key(|f| std::cmp::Reverse(f.at));
        listed
    }

    /// Re-keys the failures of `from`, and of everything under it, to `to`; the caller holds the
    /// store's guard.
    pub(crate) fn rebase_failures(
        &self,
        root: &Path,
        from: &ItemPath,
        to: &ItemPath,
    ) -> io::Result<()> {
        let root = root.to_string_lossy();
        let mut list = self.read_failures();
        let mut changed = false;
        for f in list.iter_mut().filter(|f| f.root == root) {
            let moved = ItemPath::new(&f.item).and_then(|p| rebase(&p, from, to));
            if let Some(moved) = moved {
                f.item = moved.as_str().to_owned();
                changed = true;
            }
        }
        if changed {
            self.write_failures(list)
        } else {
            Ok(())
        }
    }

    /// Drops the failures of `item` and of everything under it; the caller holds the guard.
    pub(crate) fn drop_failures(&self, root: &Path, item: &ItemPath) -> io::Result<()> {
        let root = root.to_string_lossy();
        let mut list = self.read_failures();
        let before = list.len();
        list.retain(|f| {
            f.root != root || !ItemPath::new(&f.item).is_some_and(|p| is_within(&p, item))
        });
        if list.len() == before {
            return Ok(());
        }
        self.write_failures(list)
    }

    // The list is a cache like the rest of the store: a missing or unreadable file is empty.
    fn read_failures(&self) -> Vec<FailureMeta> {
        std::fs::read(self.dir().join(FILE))
            .ok()
            .and_then(|bytes| serde_json::from_slice::<FailureFile>(&bytes).ok())
            .filter(|f| f.version == VERSION)
            .map(|f| f.failures)
            .unwrap_or_default()
    }

    fn write_failures(&self, failures: Vec<FailureMeta>) -> io::Result<()> {
        let file = FailureFile {
            version: VERSION,
            failures,
        };
        persist::save_json(&self.dir().join(FILE), &file)
    }
}
