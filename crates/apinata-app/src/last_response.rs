//! The last response per request, kept in the app's data dir so a reopened request shows it.
//!
//! One directory holds `index.json`, mapping (collection root, item path) to an entry id and its
//! kept sends, plus `<id>.<seq>.json` per send and `<id>.<seq>.body` when the raw bytes differ
//! from the decoded text and hold no secret. The files are a cache: anything unreadable counts
//! as missing.

use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use apinata_core::{
    Assertion, AssertionOutcome, DigestKey, ItemPath, Outcome, RequestResult, Response,
    ScriptPhase, TestOutcome, Verdict,
};
use chrono::{DateTime, Datelike as _, FixedOffset, Local};
use gpui_kit::{App, AppContext, Global};
use serde::{Deserialize, Serialize};

use crate::app::AppState;
use crate::persist;
use crate::scrub::{Forms, scrub_url};
use crate::stale::{is_within, rebase};

/// The on-disk format version of the index.
pub const FORMAT_VERSION: u32 = 2;
/// The on-disk format version of each send's json file.
pub const ENTRY_VERSION: u32 = 1;

/// The secret-store name of the per-install key that digests kept header values.
pub const DIGEST_KEY_NAME: &str = "kept-send-digest-key";

/// Keyed digests of a kept send's masked header values, by header index, under key `key`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeptDigests {
    /// The id of the key that made the digests.
    pub key: String,
    /// Each masked header's index and the digest of its true value.
    pub values: Vec<(usize, String)>,
}
/// The most bytes, summed over every kept send, the store holds.
pub const STORE_LIMIT: u64 = 200 * 1024 * 1024;
/// How many sends per request are kept unless the settings say otherwise.
pub const DEFAULT_HISTORY: usize = 20;
/// The origin a runner send is kept with; a tab's send has none.
pub const RUN_ORIGIN: &str = "run";
/// The largest body, in bytes, that is kept; a larger one is dropped.
pub const BODY_LIMIT: usize = 5 * 1024 * 1024;
/// The most bytes of a stream response's body kept; the rest is cut, not dropped.
pub const STREAM_BODY_LIMIT: usize = 1024 * 1024;

/// The version of the at-rest masks the startup sweep last applied; a new mask rule bumps it so
/// kept sends are swept again.
const SWEEP_VERSION: u32 = 3;
/// The file in the store dir recording that the sweep finished at [`SWEEP_VERSION`].
const SWEEP_MARKER: &str = "remask.done";
/// How old an unlisted send file must be before the sweep removes it; a younger one may belong
/// to a save another process has not indexed yet.
const ORPHAN_GRACE: Duration = Duration::from_secs(10 * 60);
/// How far before its start a sweep sets its watermark, so a file written meanwhile is read next time.
const WATERMARK_MARGIN: Duration = Duration::from_secs(5);

#[derive(Serialize, Deserialize)]
struct SweepMarker {
    version: u32,
    /// Send files older than this (ms since the epoch) were checked; a version 1 marker had none.
    #[serde(default)]
    checked_ms: u64,
}

/// The response store over one directory; clones share its lock, and other stores on the same
/// directory, in this process or another, contend on a lock file beside it.
#[derive(Clone)]
pub struct ResponseStore {
    dir: PathBuf,
    lock: Arc<Mutex<()>>,
    // Filled once the key loads off the UI thread; until then sends are kept without digests.
    digest_key: Arc<Mutex<Option<DigestKey>>>,
}

impl Global for ResponseStore {}

/// Both store locks, released on drop: the file's first, as fields drop in order.
struct StoreGuard<'a> {
    _file: Option<std::fs::File>,
    _mutex: MutexGuard<'a, ()>,
}

/// A kept response, rebuilt.
pub struct Stored {
    /// The result as kept: no error, warnings or timeline.
    pub result: RequestResult,
    /// When the response was saved.
    pub saved_at: SystemTime,
    /// Whether the body was over [`BODY_LIMIT`] and not kept.
    pub body_dropped: bool,
    /// Whether a secret was scrubbed from the kept body text, so it cannot be revealed.
    pub body_masked: bool,
    /// The places, among the kept headers, of the values a secret or the header mask changed.
    pub masked_headers: Vec<usize>,
    /// Keyed digests of the masked header values' true text, when the send was kept with a key.
    pub header_digests: Option<KeptDigests>,
}

/// How much a save may keep.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// Sends kept per request; 0 counts as 1, the latest send alone.
    pub per_request: usize,
    /// The most bytes all kept sends may take together.
    pub total_bytes: u64,
}

impl Default for Limits {
    fn default() -> Limits {
        Limits {
            per_request: DEFAULT_HISTORY,
            total_bytes: STORE_LIMIT,
        }
    }
}

/// The summary of one kept send, as listed by [`ResponseStore::history`].
#[derive(Debug, Clone, PartialEq)]
pub struct KeptSend {
    /// The send's number within its request, counting up from 0.
    pub seq: u64,
    /// When the send was saved.
    pub saved_at: SystemTime,
    /// The response status.
    pub status: u16,
    /// The response time.
    pub time: Duration,
    /// The response size in bytes.
    pub size: u64,
    /// The method sent, "" when unknown.
    pub method: String,
    /// The URL sent to, query included, "" when unknown.
    pub url: String,
}

impl KeptSend {
    /// "GET https://h/p?q=1"; the URL alone without a method; "" without a URL.
    pub fn target(&self) -> String {
        match (self.method.is_empty(), self.url.is_empty()) {
            (_, true) => String::new(),
            (true, false) => self.url.clone(),
            (false, false) => format!("{} {}", self.method, self.url),
        }
    }
}

/// A kept send with the request it belongs to, as listed by [`ResponseStore::recent`].
#[derive(Debug, Clone, PartialEq)]
pub struct RecentSend {
    /// The collection root.
    pub root: PathBuf,
    /// The request's item path.
    pub item: ItemPath,
    /// The send.
    pub send: KeptSend,
    /// Who sent it: [`RUN_ORIGIN`] for the runner, `None` for a tab.
    pub origin: Option<String>,
}

/// What a send keeps beside its response.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SendExtra {
    /// The request as sent, as `DraftSnapshot` JSON; drafts only.
    pub request: Option<String>,
    /// Who sent it: [`RUN_ORIGIN`] for the runner, `None` for a tab.
    pub origin: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct Index {
    version: u32,
    entries: Vec<IndexEntry>,
}

#[derive(Clone, Serialize, Deserialize)]
struct IndexEntry {
    root: String,
    item: String,
    id: String,
    /// Oldest first; a version 1 index has none.
    #[serde(default)]
    sends: Vec<SendMeta>,
}

#[derive(Clone, Serialize, Deserialize)]
struct SendMeta {
    seq: u64,
    saved_at_ms: u64,
    status: u16,
    time_ms: f64,
    size: u64,
    /// The on-disk size of the send's json and body files.
    bytes: u64,
    /// Absent from an index written before sends kept their target.
    #[serde(default)]
    method: String,
    #[serde(default)]
    url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    origin: Option<String>,
    /// Save order across the whole index, so sends within one millisecond still sort; 0 in an
    /// index written before it.
    #[serde(default)]
    order: u64,
}

#[derive(Serialize, Deserialize)]
struct Entry {
    version: u32,
    saved_at_ms: u64,
    name: String,
    outcome: OutcomeTag,
    status: u16,
    status_text: String,
    url: String,
    headers: Vec<(String, String)>,
    time_ms: f64,
    size: u64,
    truncated: bool,
    body_dropped: bool,
    text: String,
    has_body_file: bool,
    assertions: Vec<KeptAssertion>,
    // Script tests arrived after the format; a file without them has none. Console output is
    // never kept, since a script may log a secret.
    #[serde(default)]
    tests: Vec<KeptTest>,
    // The request snapshot JSON of a draft send; kept here, never in the index.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    request: Option<String>,
    // A secret was scrubbed from `text`; a file written before this field had none marked.
    #[serde(default)]
    masked: bool,
    // The places of the header values a secret or the mask changed; an older file marked none.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    masked_headers: Vec<usize>,
    // Keyed digests of the masked values' true text; an older or keyless file has none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    digests: Option<KeptDigests>,
}

// Just enough of an entry file to read its snapshot without rebuilding the body.
#[derive(Deserialize)]
struct EntryRequest {
    version: u32,
    #[serde(default)]
    request: Option<String>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum OutcomeTag {
    Passed,
    Failed,
    Errored,
    Skipped,
    Cancelled,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum VerdictTag {
    Pass,
    Fail,
    Skipped,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum PhaseTag {
    PreRequest,
    PostResponse,
    Tests,
}

#[derive(Serialize, Deserialize)]
struct KeptTest {
    phase: PhaseTag,
    name: String,
    script_error: bool,
    verdict: VerdictTag,
    actual: Option<String>,
    detail: Option<String>,
    reason: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct KeptAssertion {
    expression: String,
    operator: String,
    value: Option<String>,
    verdict: VerdictTag,
    actual: Option<String>,
    detail: Option<String>,
    reason: Option<String>,
}

impl ResponseStore {
    /// A store over `dir`, which is created on the first save.
    pub fn new(dir: PathBuf) -> ResponseStore {
        ResponseStore {
            dir,
            lock: Arc::new(Mutex::new(())),
            digest_key: Arc::new(Mutex::new(None)),
        }
    }

    /// Digests later sends' masked header values under `key`, in this store and its clones.
    pub fn set_digest_key(&self, key: DigestKey) {
        *self.digest_key.lock().unwrap_or_else(|e| e.into_inner()) = Some(key);
    }

    /// The key kept sends are digested under, once loaded.
    pub fn digest_key(&self) -> Option<DigestKey> {
        (self.digest_key.lock().unwrap_or_else(|e| e.into_inner())).clone()
    }

    /// The directory the store writes to.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// The writers' guard: the in-process lock, then the lock file `<dir>.lock`, made with its
    /// parent as needed. It sits beside the store, so `clear` never deletes a held one and a
    /// writer never depends on the store existing. A lock file that cannot be opened or locked
    /// leaves the Mutex alone in charge.
    fn guard(&self) -> StoreGuard<'_> {
        self.locked(true)
    }

    /// The guard for calls that never write: as [`Self::guard`], but a missing store has nothing
    /// to read, so no parent or lock file is made for it.
    fn read_guard(&self) -> StoreGuard<'_> {
        self.locked(false)
    }

    fn locked(&self, write: bool) -> StoreGuard<'_> {
        let mutex = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        let file = self
            .dir
            .file_name()
            .filter(|_| write || self.dir.is_dir())
            .and_then(|name| {
                let mut name = name.to_os_string();
                name.push(".lock");
                let path = self.dir.with_file_name(name);
                if write {
                    std::fs::create_dir_all(path.parent()?).ok()?;
                }
                let file = std::fs::File::options()
                    .read(true)
                    .write(true)
                    .create(true)
                    .truncate(false)
                    .open(path)
                    .ok()?;
                file.lock().ok()?;
                Some(file)
            });
        StoreGuard {
            _file: file,
            _mutex: mutex,
        }
    }

    /// Keeps `result`'s response for (root, item) under [`Limits::default`].
    pub fn save(
        &self,
        root: &Path,
        item: &ItemPath,
        result: &RequestResult,
        saved_at: SystemTime,
    ) -> io::Result<()> {
        self.save_with(root, item, result, saved_at, Limits::default())
    }

    /// Adds `result`'s response as the newest send of (root, item), dropping the oldest sends
    /// past `limits.per_request`. A result without a response writes nothing and returns Ok.
    pub fn save_with(
        &self,
        root: &Path,
        item: &ItemPath,
        result: &RequestResult,
        saved_at: SystemTime,
        limits: Limits,
    ) -> io::Result<()> {
        self.save_redacted(root, item, result, &[], saved_at, limits)
    }

    /// As `save_with`, scrubbing `secrets` and `result.script_sensitive_values()` from every
    /// kept field, and always dropping a URL userinfo password. The raw body file is kept only
    /// when neither the text nor the bytes held a secret.
    pub fn save_redacted(
        &self,
        root: &Path,
        item: &ItemPath,
        result: &RequestResult,
        secrets: &[String],
        saved_at: SystemTime,
        limits: Limits,
    ) -> io::Result<()> {
        let extra = SendExtra::default();
        self.save_send(root, item, result, secrets, saved_at, limits, &extra)
            .map(|_| ())
    }

    /// As `save_redacted`, also keeping `extra`: the snapshot in the send's file and the origin
    /// in the index.
    #[allow(clippy::too_many_arguments)]
    pub fn save_send(
        &self,
        root: &Path,
        item: &ItemPath,
        result: &RequestResult,
        secrets: &[String],
        saved_at: SystemTime,
        limits: Limits,
        extra: &SendExtra,
    ) -> io::Result<Option<u64>> {
        let Some(response) = &result.response else {
            return Ok(None);
        };
        let secrets: Vec<String> = secrets
            .iter()
            .chain(result.script_sensitive_values())
            .chain(result.derived_credentials())
            .cloned()
            .collect();
        let forms = Forms::new(&secrets);
        let _guard = self.guard();
        let root = root.to_string_lossy().into_owned();
        let mut index = self.read_index()?;
        let at = match index.position(&root, item.as_str()) {
            Some(at) => at,
            None => {
                let id = index.free_id(&root, item.as_str());
                index.entries.push(IndexEntry {
                    root: root.clone(),
                    item: item.as_str().to_owned(),
                    id,
                    sends: Vec::new(),
                });
                index.entries.len() - 1
            }
        };
        let id = index.entries[at].id.clone();
        let seq = index.entries[at].sends.last().map_or(0, |s| s.seq + 1);
        // A stream can be endless, so its body is kept as a head instead of being dropped.
        let capped;
        let response =
            if response.stream_end().is_some() && response.body().len() > STREAM_BODY_LIMIT {
                capped = head_of(response);
                &capped
            } else {
                response
            };
        let body_dropped = response.body().len() > BODY_LIMIT;
        let key = self.digest_key();
        let mut entry = Entry::of(
            result,
            response,
            &secrets,
            &forms,
            saved_at,
            body_dropped,
            key.as_ref(),
        );
        entry.request = extra.request.clone();
        let has_body_file = entry.has_body_file;
        std::fs::create_dir_all(&self.dir)?;

        // Body, then send file, then index: a crash never indexes a missing send. A stray body
        // left at this seq by an earlier crash is removed so it cannot pair with this send.
        let body_path = self.body_path(&id, seq);
        if has_body_file {
            let mut file = tempfile::NamedTempFile::new_in(&self.dir)?;
            file.write_all(response.body())?;
            file.as_file().sync_all()?;
            file.persist(&body_path).map_err(|e| e.error)?;
        } else {
            remove_if_present(&body_path)?;
        }
        let json_path = self.json_path(&id, seq);
        persist::save_json(&json_path, &entry)?;
        let mut bytes = std::fs::metadata(&json_path)?.len();
        if has_body_file {
            bytes += std::fs::metadata(&body_path)?.len();
        }
        // The first hop is the request as sent; a result built without one names the response URL.
        let (method, url) = match response.timeline().first() {
            Some(hop) => (hop.method.clone(), hop.url.clone()),
            None => (String::new(), response.url().to_owned()),
        };
        let url = kept_url(&url, &secrets);
        let order = index.sends().map(|s| s.order).max().unwrap_or(0) + 1;
        let sends = &mut index.entries[at].sends;
        sends.push(SendMeta {
            seq,
            saved_at_ms: entry.saved_at_ms,
            status: entry.status,
            time_ms: entry.time_ms,
            size: entry.size,
            bytes,
            method,
            url,
            origin: extra.origin.clone(),
            order,
        });
        let excess = sends.len().saturating_sub(limits.per_request.max(1));
        let mut gone: Vec<(String, u64)> =
            sends.drain(..excess).map(|s| (id.clone(), s.seq)).collect();
        gone.extend(index.evict(limits.total_bytes, &id, seq));
        persist::save_json(&self.index_path(), &index)?;
        for (id, seq) in gone {
            self.remove_send(&id, seq)?;
        }
        Ok(Some(seq))
    }

    /// Sends under any of `roots`, newest first by (saved_at, seq), at most `limit`.
    pub fn recent(&self, roots: &[PathBuf], limit: usize) -> Vec<RecentSend> {
        let _guard = self.read_guard();
        let Ok(index) = self.peek_index() else {
            return Vec::new();
        };
        let roots: Vec<String> = roots
            .iter()
            .map(|r| r.to_string_lossy().into_owned())
            .collect();
        let mut sends: Vec<(&SendMeta, PathBuf, ItemPath)> = index
            .entries
            .iter()
            .filter(|e| roots.contains(&e.root))
            .filter_map(|e| Some((e, ItemPath::new(&e.item)?)))
            .flat_map(|(e, item)| {
                e.sends
                    .iter()
                    .map(move |s| (s, PathBuf::from(&e.root), item.clone()))
            })
            .collect();
        sends.sort_by_key(|(s, ..)| std::cmp::Reverse((s.saved_at_ms, s.order, s.seq)));
        sends
            .into_iter()
            .take(limit)
            .map(|(s, root, item)| RecentSend {
                root,
                item,
                send: s.kept(),
                origin: s.origin.clone(),
            })
            .collect()
    }

    /// The newest kept send, or None when there is none or it cannot be read (the bad send is dropped).
    pub fn load(&self, root: &Path, item: &ItemPath) -> Option<Stored> {
        self.load_at(root, item, None)
    }

    /// The newest kept send as [`Self::load`] returns it, but never writes: a version 1 store is
    /// read in place and an unreadable send is a miss that stays on disk.
    pub fn peek(&self, root: &Path, item: &ItemPath) -> Option<Stored> {
        self.peek_at(root, item, None)
    }

    /// The kept send numbered `seq` as [`Self::load_send`] returns it, but never writes, as
    /// [`Self::peek`] does.
    pub fn peek_send(&self, root: &Path, item: &ItemPath, seq: u64) -> Option<Stored> {
        self.peek_at(root, item, Some(seq))
    }

    // `seq` None means the newest send.
    fn peek_at(&self, root: &Path, item: &ItemPath, seq: Option<u64>) -> Option<Stored> {
        let _guard = self.read_guard();
        let root = root.to_string_lossy().into_owned();
        let index = self.peek_index().ok()?;
        let entry = index.find(&root, item.as_str())?;
        let seq = match seq {
            Some(seq) => entry.sends.iter().find(|s| s.seq == seq)?.seq,
            None => entry.sends.last()?.seq,
        };
        // Send 0 of an unmigrated version 1 key still sits under its old file names.
        let either = |old: PathBuf, new: PathBuf| if seq == 0 && old.exists() { old } else { new };
        let id = &entry.id;
        let json = either(self.dir.join(format!("{id}.json")), self.json_path(id, seq));
        let body = either(self.dir.join(format!("{id}.body")), self.body_path(id, seq));
        Self::read_entry(&json, &body, item, false).map(|(stored, _)| stored)
    }

    /// Send `seq`'s snapshot JSON; None when it has none or cannot be read. Never writes.
    pub fn request_json(&self, root: &Path, item: &ItemPath, seq: u64) -> Option<String> {
        let _guard = self.read_guard();
        let root = root.to_string_lossy().into_owned();
        let index = self.peek_index().ok()?;
        let entry = index.find(&root, item.as_str())?;
        if !entry.sends.iter().any(|s| s.seq == seq) {
            return None;
        }
        let text = std::fs::read_to_string(self.json_path(&entry.id, seq)).ok()?;
        let entry: EntryRequest = serde_json::from_str(&text).ok()?;
        let request = (entry.version == ENTRY_VERSION).then_some(entry.request)??;
        Some(remasked_snapshot(&request).unwrap_or(request))
    }

    /// The kept send numbered `seq`, or None when there is none or it cannot be read (the bad
    /// send is dropped).
    pub fn load_send(&self, root: &Path, item: &ItemPath, seq: u64) -> Option<Stored> {
        self.load_at(root, item, Some(seq))
    }

    /// The kept sends of (root, item), newest first; empty when there are none.
    pub fn history(&self, root: &Path, item: &ItemPath) -> Vec<KeptSend> {
        let _guard = self.read_guard();
        let root = root.to_string_lossy().into_owned();
        let Ok(index) = self.peek_index() else {
            return Vec::new();
        };
        let Some(entry) = index.find(&root, item.as_str()) else {
            return Vec::new();
        };
        entry.sends.iter().rev().map(SendMeta::kept).collect()
    }

    // `seq` None means the newest send.
    fn load_at(&self, root: &Path, item: &ItemPath, seq: Option<u64>) -> Option<Stored> {
        // A load may rewrite the index or a send, so it writes under the writers' guard; a
        // missing store holds nothing to load and is left uncreated.
        if !self.dir.is_dir() {
            return None;
        }
        let _guard = self.guard();
        let root = root.to_string_lossy().into_owned();
        let mut index = self.read_index().ok()?;
        let at = index.position(&root, item.as_str())?;
        let entry = &index.entries[at];
        let seq = match seq {
            Some(seq) => entry.sends.iter().find(|s| s.seq == seq)?.seq,
            None => entry.sends.last()?.seq,
        };
        let id = entry.id.clone();
        let (json, body) = (self.json_path(&id, seq), self.body_path(&id, seq));
        match Self::read_entry(&json, &body, item, true) {
            None => {
                // A bad send is a cache miss: forget it alone, and the key once it has no sends.
                index.entries[at].sends.retain(|s| s.seq != seq);
                if index.entries[at].sends.is_empty() {
                    index.entries.remove(at);
                }
                let _ = persist::save_json(&self.index_path(), &index);
                let _ = self.remove_send(&id, seq);
                None
            }
            Some((stored, rewritten)) => {
                // A re-masked file changed size, so the index's count for it follows.
                if rewritten
                    && let Ok(Some(bytes)) = disk_bytes(&json, &body)
                    && let Some(send) = index.entries[at].sends.iter_mut().find(|s| s.seq == seq)
                    && send.bytes != bytes
                {
                    send.bytes = bytes;
                    let _ = persist::save_json(&self.index_path(), &index);
                }
                Some(stored)
            }
        }
    }

    /// Re-keys the entry of `from`, and every entry under it when it is a folder, to `to`.
    pub fn rename(&self, root: &Path, from: &ItemPath, to: &ItemPath) -> io::Result<()> {
        let _guard = self.guard();
        let root = root.to_string_lossy().into_owned();
        let mut index = self.read_index()?;
        let moves: Vec<(usize, String)> = index
            .entries
            .iter()
            .enumerate()
            .filter(|(_, e)| e.root == root)
            .filter_map(|(i, e)| {
                let to = rebase(&ItemPath::new(&e.item)?, from, to)?;
                Some((i, to.as_str().to_owned()))
            })
            .collect();
        if moves.is_empty() {
            return Ok(());
        }
        // The moved ids stay; an entry not moving that already sits at a target key is replaced.
        for (i, item) in &moves {
            index.entries[*i].item.clone_from(item);
        }
        let moved: Vec<String> = moves
            .iter()
            .map(|(i, _)| index.entries[*i].id.clone())
            .collect();
        let replaced = index.take(|e| {
            e.root == root
                && !moved.contains(&e.id)
                && moves.iter().any(|(_, item)| *item == e.item)
        });
        persist::save_json(&self.index_path(), &index)?;
        self.remove_files(&replaced)
    }

    /// Drops the entry of `item` and every entry under it, files included.
    pub fn remove(&self, root: &Path, item: &ItemPath) -> io::Result<()> {
        let _guard = self.guard();
        let root = root.to_string_lossy().into_owned();
        let mut index = self.read_index()?;
        let gone = index.take(|e| {
            e.root == root && ItemPath::new(&e.item).is_some_and(|p| is_within(&p, item))
        });
        if gone.is_empty() {
            return Ok(());
        }
        persist::save_json(&self.index_path(), &index)?;
        self.remove_files(&gone)
    }

    /// Deletes the whole store directory, orphan files included.
    pub fn clear(&self) -> io::Result<()> {
        let _guard = self.guard();
        match std::fs::remove_dir_all(&self.dir) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        }
    }

    // The index is written before this runs, so a crash leaves only orphan files.
    fn remove_files(&self, entries: &[IndexEntry]) -> io::Result<()> {
        for entry in entries {
            for send in &entry.sends {
                self.remove_send(&entry.id, send.seq)?;
            }
        }
        Ok(())
    }

    fn remove_send(&self, id: &str, seq: u64) -> io::Result<()> {
        remove_if_present(&self.json_path(id, seq))?;
        remove_if_present(&self.body_path(id, seq))
    }

    /// Re-masks the send files the index lists, then writes the marker. With a marker at
    /// [`SWEEP_VERSION`] only files changed since its watermark are read; without one, every
    /// listed file is. The lock is taken per file, so loads and saves interleave. Any I/O error
    /// leaves the old marker, so the next start covers the same files again; a file the mask
    /// leaves alone is never written.
    fn sweep(&self) {
        let since = std::fs::read(self.dir.join(SWEEP_MARKER))
            .ok()
            .and_then(|bytes| serde_json::from_slice::<SweepMarker>(&bytes).ok())
            .filter(|m| m.version == SWEEP_VERSION)
            .map(|m| m.checked_ms);
        let started = SystemTime::now();
        // A missing dir holds no old send, and the marker would only create it.
        let Ok(listing) = std::fs::read_dir(&self.dir) else {
            return;
        };
        let files = listing.filter_map(|file| match file {
            Err(e) => Some(Err(e)),
            Ok(file) => {
                let name = file.file_name();
                let send = name.to_str().and_then(send_file_name).is_some();
                (send && file.file_type().is_ok_and(|t| t.is_file())).then(|| Ok(file.path()))
            }
        });
        self.sweep_files(files, since, started);
    }

    // The sweep over the send files `files`, read when newer than the watermark `since`. Only
    // files the index snapshot lists are re-masked, so one aged out meanwhile is not recreated;
    // without a readable index every file counts as listed. The marker is written only when
    // every read, removal and save succeeded.
    fn sweep_files(
        &self,
        files: impl IntoIterator<Item = io::Result<PathBuf>>,
        since: Option<u64>,
        started: SystemTime,
    ) {
        let index = {
            let _guard = self.read_guard();
            self.index_snapshot()
        };
        let current = index.as_ref().is_some_and(|i| i.version == FORMAT_VERSION);
        let mut clean = true;
        let mut orphans = Vec::new();
        // A young unlisted json the mask would change may be indexed later with its current
        // mtime, so the watermark stays below it and the next sweep still reads it.
        let mut watermark = started;
        for file in files {
            let Ok(path) = file else {
                clean = false;
                continue;
            };
            let Some((id, seq, is_json)) = path
                .file_name()
                .and_then(|n| n.to_str())
                .and_then(send_file_name)
            else {
                continue;
            };
            let modified = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
            if !index.as_ref().is_none_or(|i| i.lists(id, seq)) {
                match modified {
                    Some(m) if m + ORPHAN_GRACE < started => {
                        if current {
                            orphans.push(path.clone());
                        }
                    }
                    Some(m) if is_json && needs_remask(&path) => watermark = watermark.min(m),
                    _ => {}
                }
                continue;
            }
            let seen = since.is_some_and(|t| modified.is_some_and(|m| epoch_ms(m) < t));
            if !is_json || seen {
                continue;
            }
            let _guard = self.guard();
            clean &= match remasked_entry(&path) {
                Ok(Some((entry, true))) => persist::save_json(&path, &entry).is_ok(),
                Ok(_) => true,
                Err(_) => false,
            };
        }
        let _guard = self.guard();
        if current {
            clean &= self.tidy_index(&orphans, started);
        }
        // A store cleared meanwhile stays gone; the marker would only recreate its dir.
        if clean && self.dir.is_dir() {
            let marker = SweepMarker {
                version: SWEEP_VERSION,
                checked_ms: epoch_ms(
                    (watermark.checked_sub(WATERMARK_MARGIN)).unwrap_or(UNIX_EPOCH),
                ),
            };
            let _ = persist::save_json(&self.dir.join(SWEEP_MARKER), &marker);
        }
    }

    // Under the lock, against a fresh read of the index: removes the `orphans` it still does not
    // list and that are still older than the grace before `started`, then sets each listed
    // send's `bytes` to its files' size and drops its url's userinfo, saving only on a change.
    // Nothing happens unless the index is at FORMAT_VERSION. False on any I/O error.
    fn tidy_index(&self, orphans: &[PathBuf], started: SystemTime) -> bool {
        let Some(mut index) = self.index_snapshot() else {
            // An index that exists but cannot be read may still hold a username.
            return std::fs::read(self.index_path())
                .map_or_else(|e| e.kind() == io::ErrorKind::NotFound, |_| true);
        };
        if index.version != FORMAT_VERSION {
            return true;
        }
        let mut clean = true;
        for orphan in orphans {
            let name = orphan.file_name().and_then(|n| n.to_str());
            let Some((id, seq, _)) = name.and_then(send_file_name) else {
                continue;
            };
            if index.lists(id, seq) {
                continue;
            }
            // Re-stat: a save may have rewritten it since the listing; one gone counts as removed.
            match std::fs::metadata(orphan).and_then(|m| m.modified()) {
                Ok(m) if m + ORPHAN_GRACE < started => {
                    clean &= remove_if_present(orphan).is_ok();
                }
                Ok(_) => {}
                Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                Err(_) => clean = false,
            }
        }
        let mut changed = false;
        for entry in &mut index.entries {
            for send in &mut entry.sends {
                let url = apinata_core::redact_url_userinfo(&send.url);
                if url != send.url {
                    send.url = url;
                    changed = true;
                }
                let (json, body) = self.send_files(&entry.id, send.seq);
                match disk_bytes(&json, &body) {
                    Ok(Some(bytes)) if bytes != send.bytes => {
                        send.bytes = bytes;
                        changed = true;
                    }
                    Ok(_) => {}
                    Err(_) => clean = false,
                }
            }
        }
        if changed {
            clean &= persist::save_json(&self.index_path(), &index).is_ok();
        }
        clean
    }

    // Send `seq`'s json and body paths; send 0 of an unmigrated version 1 key keeps its old names.
    fn send_files(&self, id: &str, seq: u64) -> (PathBuf, PathBuf) {
        let old = self.dir.join(format!("{id}.json"));
        if seq == 0 && old.exists() {
            (old, self.dir.join(format!("{id}.body")))
        } else {
            (self.json_path(id, seq), self.body_path(id, seq))
        }
    }

    /// The index file as stored when it parses at version 1 or [`FORMAT_VERSION`]; None when it
    /// is missing, unreadable, unparsable or of another version. Never migrates.
    fn index_snapshot(&self) -> Option<Index> {
        let text = std::fs::read_to_string(self.index_path()).ok()?;
        let index = serde_json::from_str::<Index>(&text).ok()?;
        (index.version == FORMAT_VERSION || index.version == 1).then_some(index)
    }

    fn json_path(&self, id: &str, seq: u64) -> PathBuf {
        self.dir.join(format!("{id}.{seq}.json"))
    }

    fn body_path(&self, id: &str, seq: u64) -> PathBuf {
        self.dir.join(format!("{id}.{seq}.body"))
    }

    fn index_path(&self) -> PathBuf {
        self.dir.join("index.json")
    }

    /// The index, migrated from version 1 when needed; empty when it is missing, unparsable or of
    /// another version; Err when it cannot be read or migrated.
    fn read_index(&self) -> io::Result<Index> {
        let index = self.stored_index()?;
        match index.version {
            1 => self.migrate_v1(index),
            _ => Ok(index),
        }
    }

    /// The index as [`Self::read_index`] sees it, but a version 1 store is viewed in memory and
    /// left as is, so listing sends never writes.
    fn peek_index(&self) -> io::Result<Index> {
        let mut index = self.stored_index()?;
        if index.version == 1 {
            index.version = FORMAT_VERSION;
            index
                .entries
                .retain_mut(|entry| match self.v1_meta(&entry.id) {
                    Ok(Some(meta)) => {
                        entry.sends = vec![meta];
                        true
                    }
                    _ => false,
                });
        }
        Ok(index)
    }

    /// The index file as stored, version 1 or current; empty when it is missing, unparsable or
    /// of another version; Err when it cannot be read.
    fn stored_index(&self) -> io::Result<Index> {
        let empty = Index {
            version: FORMAT_VERSION,
            entries: Vec::new(),
        };
        // A missing index is an empty store, but an unreadable one must not be overwritten.
        let text = match std::fs::read_to_string(self.index_path()) {
            Ok(text) => text,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(empty),
            Err(e) => return Err(e),
        };
        match serde_json::from_str::<Index>(&text) {
            Ok(index) if index.version == FORMAT_VERSION || index.version == 1 => Ok(index),
            _ => Ok(empty),
        }
    }

    // A version 1 key's send 0, from wherever a part-done migration left each of its files.
    fn v1_meta(&self, id: &str) -> io::Result<Option<SendMeta>> {
        let either = |old: PathBuf, new: PathBuf| if old.exists() { old } else { new };
        let json = either(self.dir.join(format!("{id}.json")), self.json_path(id, 0));
        let body = either(self.dir.join(format!("{id}.body")), self.body_path(id, 0));
        meta_at(&json, &body, 0)
    }

    // Version 1 kept one `<id>.json` per key; it becomes send 0. Files move before the index is
    // rewritten and a file already at its new name counts as moved, so a crash or an I/O error
    // part way leaves a version 1 index that the next read migrates again.
    fn migrate_v1(&self, old: Index) -> io::Result<Index> {
        let mut index = Index {
            version: FORMAT_VERSION,
            entries: Vec::new(),
        };
        for mut entry in old.entries {
            let id = entry.id.clone();
            rename_if_present(
                &self.dir.join(format!("{id}.body")),
                &self.body_path(&id, 0),
            )?;
            rename_if_present(
                &self.dir.join(format!("{id}.json")),
                &self.json_path(&id, 0),
            )?;
            // Missing, unparsable or foreign: the entry was already a cache miss under version 1.
            match self.send_meta(&id, 0)? {
                Some(meta) => {
                    entry.sends = vec![meta];
                    index.entries.push(entry);
                }
                None => self.remove_send(&id, 0)?,
            }
        }
        persist::save_json(&self.index_path(), &index)?;
        Ok(index)
    }

    /// The summary of send `seq`, or None when its files are missing, unparsable or of a foreign
    /// entry version; Err on any other I/O error.
    fn send_meta(&self, id: &str, seq: u64) -> io::Result<Option<SendMeta>> {
        meta_at(&self.json_path(id, seq), &self.body_path(id, seq), seq)
    }

    /// The send in `json` (and `body`), re-masked, and whether it was rewritten: with
    /// `rewrite`, a file the mask changed is saved masked, and a failed save still returns the
    /// masked send.
    fn read_entry(
        json: &Path,
        body: &Path,
        item: &ItemPath,
        rewrite: bool,
    ) -> Option<(Stored, bool)> {
        let (entry, changed) = remasked_entry(json).ok()??;
        let rewritten = changed && rewrite && persist::save_json(json, &entry).is_ok();
        let body = if entry.has_body_file {
            Some(std::fs::read(body).ok()?)
        } else {
            None
        };
        Some((entry.rebuild(item, body), rewritten))
    }
}

impl Index {
    /// Whether a send file of `id` numbered `seq` (None for a version 1 name) is kept. A version 1
    /// index lists send 0 of each key under either name.
    fn lists(&self, id: &str, seq: Option<u64>) -> bool {
        (self.entries.iter().filter(|e| e.id == id)).any(|e| match seq {
            None => true,
            Some(seq) if self.version == 1 => seq == 0,
            Some(seq) => e.sends.iter().any(|s| s.seq == seq),
        })
    }

    /// Every kept send of every entry.
    fn sends(&self) -> impl Iterator<Item = &SendMeta> {
        self.entries.iter().flat_map(|e| &e.sends)
    }

    /// Removes the entries matching `drop` and returns them.
    fn take(&mut self, drop: impl Fn(&IndexEntry) -> bool) -> Vec<IndexEntry> {
        let (gone, kept) = std::mem::take(&mut self.entries)
            .into_iter()
            .partition(drop);
        self.entries = kept;
        gone
    }

    /// Drops the globally oldest sends, by (saved_at, seq), until the kept bytes fit in `limit`,
    /// never send `seq` of `id`. Keys left empty leave the index. Returns the dropped (id, seq).
    fn evict(&mut self, limit: u64, id: &str, seq: u64) -> Vec<(String, u64)> {
        let mut total: u64 = self
            .entries
            .iter()
            .flat_map(|e| &e.sends)
            .map(|s| s.bytes)
            .sum();
        let mut gone = Vec::new();
        while total > limit {
            let oldest = self
                .entries
                .iter()
                .enumerate()
                .flat_map(|(i, e)| e.sends.iter().enumerate().map(move |(j, s)| (i, j, e, s)))
                .filter(|(_, _, e, s)| !(e.id == id && s.seq == seq))
                .min_by_key(|(.., s)| (s.saved_at_ms, s.seq))
                .map(|(i, j, ..)| (i, j));
            let Some((i, j)) = oldest else { break };
            let send = self.entries[i].sends.remove(j);
            total -= send.bytes;
            gone.push((self.entries[i].id.clone(), send.seq));
        }
        self.entries.retain(|e| !e.sends.is_empty());
        gone
    }

    fn position(&self, root: &str, item: &str) -> Option<usize> {
        self.entries
            .iter()
            .position(|e| e.root == root && e.item == item)
    }

    fn find(&self, root: &str, item: &str) -> Option<&IndexEntry> {
        self.entries
            .iter()
            .find(|e| e.root == root && e.item == item)
    }

    /// The key's hash id, suffixed `-1`, `-2` and so on while another key holds it.
    fn free_id(&self, root: &str, item: &str) -> String {
        let base = format!("{:016x}", fnv1a(root, item));
        let taken = |id: &str| self.entries.iter().any(|e| e.id == id);
        if !taken(&base) {
            return base;
        }
        let mut n = 1u64;
        while taken(&format!("{base}-{n}")) {
            n += 1;
        }
        format!("{base}-{n}")
    }
}

/// FNV-1a 64 over `root`, a zero byte and `item`.
fn fnv1a(root: &str, item: &str) -> u64 {
    root.bytes()
        .chain([0u8])
        .chain(item.bytes())
        .fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
            (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3)
        })
}

// Send `seq`'s summary from its entry file `json` and body file `body`; see `send_meta`.
fn meta_at(json: &Path, body: &Path, seq: u64) -> io::Result<Option<SendMeta>> {
    let text = match std::fs::read_to_string(json) {
        Ok(text) => text,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    };
    let Ok(entry) = serde_json::from_str::<Entry>(&text) else {
        return Ok(None);
    };
    if entry.version != ENTRY_VERSION {
        return Ok(None);
    }
    let mut bytes = text.len() as u64;
    if entry.has_body_file {
        match std::fs::metadata(body) {
            Ok(meta) => bytes += meta.len(),
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e),
        }
    }
    Ok(Some(SendMeta {
        seq,
        saved_at_ms: entry.saved_at_ms,
        status: entry.status,
        time_ms: entry.time_ms,
        size: entry.size,
        bytes,
        method: String::new(),
        // The send file may predate urls losing their username; the index never keeps one.
        url: apinata_core::redact_url_userinfo(&entry.url),
        origin: None,
        order: 0,
    }))
}

/// The id, seq (None for a version 1 name) and whether it is the json of a send file named
/// exactly `<id>.<seq>.json|body` or `<id>.json|body`; None for any other name.
fn send_file_name(name: &str) -> Option<(&str, Option<u64>, bool)> {
    let (stem, is_json) = match name.rsplit_once('.')? {
        (stem, "json") => (stem, true),
        (stem, "body") => (stem, false),
        _ => return None,
    };
    let (id, seq) = match stem.split_once('.') {
        Some((id, seq)) if !seq.is_empty() && seq.bytes().all(|b| b.is_ascii_digit()) => {
            (id, Some(seq.parse().ok()?))
        }
        Some(_) => return None,
        None => (stem, None),
    };
    let id_ok = !id.is_empty() && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-');
    (id_ok && id != "index").then_some((id, seq, is_json))
}

/// The on-disk size of a send's `json` and, when present, `body`; None when `json` is missing.
fn disk_bytes(json: &Path, body: &Path) -> io::Result<Option<u64>> {
    let json = match std::fs::metadata(json) {
        Ok(meta) => meta.len(),
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    };
    let body = match std::fs::metadata(body) {
        Ok(meta) => meta.len(),
        Err(e) if e.kind() == io::ErrorKind::NotFound => 0,
        Err(e) => return Err(e),
    };
    Ok(Some(json + body))
}

fn epoch_ms(at: SystemTime) -> u64 {
    at.duration_since(UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

/// The entry in `json`, re-masked, and whether the mask changed it; None when it is unparsable
/// (invalid UTF-8 included) or of a foreign entry version; Err when it cannot be read, a missing
/// file included, since the sweep must not count a send renamed away as done.
fn remasked_entry(json: &Path) -> io::Result<Option<(Entry, bool)>> {
    let bytes = std::fs::read(json)?;
    let Ok(mut entry) = serde_json::from_slice::<Entry>(&bytes) else {
        return Ok(None);
    };
    if entry.version != ENTRY_VERSION {
        return Ok(None);
    }
    let changed = entry.remask();
    Ok(Some((entry, changed)))
}

/// Whether the send file `json` would be rewritten by the mask, or cannot be read to tell.
fn needs_remask(json: &Path) -> bool {
    remasked_entry(json).map_or(true, |e| e.is_some_and(|(_, changed)| changed))
}

/// Snapshot `json` with its url passwords dropped, or None when that changes nothing or it does
/// not parse (such a snapshot cannot be shown either).
fn remasked_snapshot(json: &str) -> Option<String> {
    let mut snap = apinata_core::DraftSnapshot::from_json(json).ok()?;
    snap.drop_url_passwords().then(|| snap.to_json())
}

/// A sent url as a kept response records it: no userinfo at all, since a username alone can be
/// a token, and every secret scrubbed.
fn kept_url(url: &str, secrets: &[String]) -> String {
    scrub_url(&apinata_core::redact_url_userinfo(url), secrets)
}

/// Renames `from` to `to`; a `from` that is already gone counts as done.
fn rename_if_present(from: &Path, to: &Path) -> io::Result<()> {
    match std::fs::rename(from, to) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

fn remove_if_present(path: &Path) -> io::Result<()> {
    match std::fs::remove_file(path) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

impl Entry {
    /// The entry to keep, every free-text field scrubbed with `forms`; it has a body file only
    /// when the raw bytes differ from the text and neither carries a secret. With `key`, each
    /// masked header value is digested as received.
    fn of(
        result: &RequestResult,
        response: &Response,
        secrets: &[String],
        forms: &Forms,
        saved_at: SystemTime,
        body_dropped: bool,
        key: Option<&DigestKey>,
    ) -> Entry {
        let text = if body_dropped {
            String::new()
        } else {
            forms.scrub(response.text())
        };
        let has_body_file = !body_dropped
            && response.body() != response.text().as_bytes()
            && text == response.text()
            && !forms.in_bytes(response.body());
        let masked = !body_dropped && text != response.text();
        // A header can carry a value no secret list knows (a url password, a cookie), so the
        // kept copy also takes core's response mask, the one rule examples and reports share.
        let headers: Vec<(String, String)> = (response.headers().iter())
            .map(|(name, value)| {
                let kept = apinata_core::mask_response_header(name, &forms.scrub(value));
                (name.clone(), kept)
            })
            .collect();
        let masked_headers: Vec<usize> = (response.headers().iter().zip(&headers).enumerate())
            .filter(|(_, ((_, raw), (_, kept)))| raw != kept)
            .map(|(ix, _)| ix)
            .collect();
        let digests = key.map(|key| KeptDigests {
            key: key.id().to_owned(),
            values: (masked_headers.iter())
                .map(|&ix| {
                    let (name, raw) = &response.headers()[ix];
                    (ix, key.digest(name, raw))
                })
                .collect(),
        });
        // A core variant added later is kept as the nearest neutral tag.
        let outcome = match result.outcome {
            Outcome::Passed => OutcomeTag::Passed,
            Outcome::Failed => OutcomeTag::Failed,
            Outcome::Skipped => OutcomeTag::Skipped,
            Outcome::Cancelled => OutcomeTag::Cancelled,
            _ => OutcomeTag::Errored,
        };
        Entry {
            version: ENTRY_VERSION,
            saved_at_ms: saved_at
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64,
            name: result.name.clone(),
            outcome,
            status: response.status(),
            status_text: forms.scrub(response.status_text()),
            url: kept_url(response.url(), secrets),
            headers,
            time_ms: response.response_time().as_secs_f64() * 1000.0,
            size: response.size(),
            truncated: response.truncated(),
            body_dropped,
            text,
            has_body_file,
            masked,
            masked_headers,
            digests,
            assertions: result
                .assertions
                .iter()
                .map(|a| KeptAssertion::of(a, forms))
                .collect(),
            tests: kept_tests(result, forms),
            request: None,
        }
    }

    /// Applies the at-rest masks to a file kept before them, returning whether anything changed.
    /// The masks are idempotent, so a file written since re-masks to itself.
    fn remask(&mut self) -> bool {
        let mut changed = false;
        for (ix, (name, value)) in self.headers.iter_mut().enumerate() {
            let kept = apinata_core::mask_response_header(name, value);
            if kept != *value {
                *value = kept;
                self.masked_headers.push(ix);
                changed = true;
            }
        }
        self.masked_headers.sort_unstable();
        self.masked_headers.dedup();
        if let Some(request) = self.request.as_deref().and_then(remasked_snapshot) {
            self.request = Some(request);
            changed = true;
        }
        // A send kept before urls lost their username still holds one; the snapshot keeps its own.
        let url = apinata_core::redact_url_userinfo(&self.url);
        if url != self.url {
            self.url = url;
            changed = true;
        }
        changed
    }

    /// The kept result; `body` is the body file's bytes when the entry has one.
    fn rebuild(self, item: &ItemPath, body: Option<Vec<u8>>) -> Stored {
        let mut response = Response::new(self.status)
            .with_status_text(self.status_text)
            .with_url(self.url);
        for (name, value) in self.headers {
            response = response.with_header(name, value);
        }
        // The size goes on after the body, since setting a body resets it.
        let bytes = match body {
            Some(bytes) => bytes,
            None => self.text.as_bytes().to_vec(),
        };
        response = response.with_body(bytes, self.text);
        response = if self.truncated {
            response.with_truncated(self.size)
        } else {
            response.with_size(self.size)
        };
        // A hand-edited or odd time reads as zero rather than panicking in from_secs_f64.
        let secs = self.time_ms / 1000.0;
        let time = Duration::try_from_secs_f64(secs).unwrap_or_default();
        let response = response.with_response_time(time);
        let outcome = match self.outcome {
            OutcomeTag::Passed => Outcome::Passed,
            OutcomeTag::Failed => Outcome::Failed,
            OutcomeTag::Errored => Outcome::Errored,
            OutcomeTag::Skipped => Outcome::Skipped,
            OutcomeTag::Cancelled => Outcome::Cancelled,
        };
        let assertions = self
            .assertions
            .into_iter()
            .map(KeptAssertion::rebuild)
            .collect();
        let mut result =
            RequestResult::from_parts(item.clone(), self.name, outcome, Some(response), assertions);
        result.tests = self.tests.into_iter().map(KeptTest::rebuild).collect();
        Stored {
            result,
            saved_at: UNIX_EPOCH + Duration::from_millis(self.saved_at_ms),
            body_dropped: self.body_dropped,
            body_masked: self.masked,
            header_digests: self.digests.map(|mut d| {
                d.values.retain(|(ix, _)| self.masked_headers.contains(ix));
                d
            }),
            masked_headers: self.masked_headers,
        }
    }
}

impl SendMeta {
    fn kept(&self) -> KeptSend {
        // Rounded to whole nanoseconds, so a time saved from a Duration reads back equal; an odd
        // value saturates or reads as zero.
        let time = Duration::from_nanos((self.time_ms * 1_000_000.0).round() as u64);
        KeptSend {
            seq: self.seq,
            saved_at: UNIX_EPOCH + Duration::from_millis(self.saved_at_ms),
            status: self.status,
            time,
            size: self.size,
            method: self.method.clone(),
            url: self.url.clone(),
        }
    }
}

/// A verdict's tag with its failed actual, detail and skip reason.
type VerdictParts = (VerdictTag, Option<String>, Option<String>, Option<String>);

fn verdict_parts(verdict: &Verdict) -> VerdictParts {
    match verdict {
        Verdict::Pass => (VerdictTag::Pass, None, None, None),
        Verdict::Fail { actual, detail, .. } => {
            (VerdictTag::Fail, Some(actual.clone()), detail.clone(), None)
        }
        Verdict::Skipped { reason, .. } => (VerdictTag::Skipped, None, None, Some(reason.clone())),
        _ => (VerdictTag::Skipped, None, None, None),
    }
}

fn verdict_of(
    tag: VerdictTag,
    actual: Option<String>,
    detail: Option<String>,
    reason: Option<String>,
) -> Verdict {
    match tag {
        VerdictTag::Pass => Verdict::Pass,
        VerdictTag::Fail => Verdict::fail(actual.unwrap_or_default(), detail),
        VerdictTag::Skipped => Verdict::skipped(reason.unwrap_or_default()),
    }
}

/// The result's script tests, with every secret form scrubbed from their text: a test
/// name or message may quote a secret the request itself never carried.
fn kept_tests(result: &RequestResult, forms: &Forms) -> Vec<KeptTest> {
    let scrub = |text: String| forms.scrub(&text);
    result
        .tests
        .iter()
        .map(|test| {
            let mut kept = KeptTest::of(test);
            kept.name = scrub(kept.name);
            kept.actual = kept.actual.map(scrub);
            kept.detail = kept.detail.map(scrub);
            kept.reason = kept.reason.map(scrub);
            kept
        })
        .collect()
}

impl KeptTest {
    fn of(test: &TestOutcome) -> KeptTest {
        // A core phase added later is kept as the tests phase.
        let phase = match test.phase {
            ScriptPhase::PreRequest => PhaseTag::PreRequest,
            ScriptPhase::PostResponse => PhaseTag::PostResponse,
            _ => PhaseTag::Tests,
        };
        let (verdict, actual, detail, reason) = verdict_parts(&test.verdict);
        KeptTest {
            phase,
            name: test.name.clone(),
            script_error: test.script_error,
            verdict,
            actual,
            detail,
            reason,
        }
    }

    fn rebuild(self) -> TestOutcome {
        let phase = match self.phase {
            PhaseTag::PreRequest => ScriptPhase::PreRequest,
            PhaseTag::PostResponse => ScriptPhase::PostResponse,
            PhaseTag::Tests => ScriptPhase::Tests,
        };
        let verdict = verdict_of(self.verdict, self.actual, self.detail, self.reason);
        let mut test = TestOutcome::new(phase, self.name, verdict);
        test.script_error = self.script_error;
        test
    }
}

impl KeptAssertion {
    /// The assertion with its value, actual, detail and reason scrubbed; a server echo of a
    /// secret can land in any of them.
    fn of(outcome: &AssertionOutcome, forms: &Forms) -> KeptAssertion {
        let a = outcome.assertion();
        let (verdict, actual, detail, reason) = verdict_parts(outcome.verdict());
        let scrub = |text: String| forms.scrub(&text);
        KeptAssertion {
            expression: a.expression().to_owned(),
            operator: a.operator().to_owned(),
            value: a.value().map(|v| forms.scrub(v)),
            verdict,
            actual: actual.map(scrub),
            detail: detail.map(scrub),
            reason: reason.map(scrub),
        }
    }

    fn rebuild(self) -> AssertionOutcome {
        let verdict = verdict_of(self.verdict, self.actual, self.detail, self.reason);
        AssertionOutcome::new(
            Assertion::new(self.expression, self.operator, self.value),
            verdict,
        )
    }
}

/// The wall clock for age labels: the system time, moved by however far the executor's
/// clock runs ahead of the real one, which is nothing outside tests' simulated time.
pub fn wall_now(cx: &App) -> SystemTime {
    let (simulated, real) = (cx.background_executor().now(), Instant::now());
    let now = SystemTime::now();
    match simulated.checked_duration_since(real) {
        Some(ahead) => now + ahead,
        None => now - real.duration_since(simulated),
    }
}

/// How long after an age of `elapsed` its label (see `age_of`) next reads differently: at
/// the next whole minute, hour or day, whichever unit the label counts in.
pub fn next_age_change(elapsed: Duration) -> Duration {
    let unit = match elapsed.as_secs() {
        s if s < 3_600 => 60,
        s if s < 86_400 => 3_600,
        _ => 86_400,
    };
    let next = (elapsed.as_secs() / unit + 1) * unit;
    Duration::from_secs(next) - elapsed
}

/// Whether (root, item) may keep a response: not a draft, and the request file exists.
pub fn keeps(root: &Path, item: &ItemPath) -> bool {
    !item.is_draft() && root.join(item.as_str()).is_file()
}

/// "10 Oct, 20:41 (2 hours ago)", with the year after the month when it differs from `now`'s.
pub fn saved_label(saved: DateTime<FixedOffset>, now: DateTime<FixedOffset>) -> String {
    let seconds = (now - saved).num_seconds().max(0);
    let date = if saved.year() == now.year() {
        saved_date(saved)
    } else {
        saved.format("%-d %b %Y, %H:%M").to_string()
    };
    format!("{date} ({})", age(seconds))
}

/// "10 Oct, 20:41", never with the year, so the status row never grows as the year turns.
pub fn saved_date(saved: DateTime<FixedOffset>) -> String {
    saved.format("%-d %b, %H:%M").to_string()
}

/// The exact time for the tooltip: "2026-10-10 20:41:07 +02:00".
pub fn saved_tooltip(saved: DateTime<FixedOffset>) -> String {
    saved.format("%Y-%m-%d %H:%M:%S %:z").to_string()
}

/// Both, for a `SystemTime`, in the local zone.
pub fn local_labels(saved: SystemTime, now: SystemTime) -> (String, String) {
    let saved = DateTime::<Local>::from(saved).fixed_offset();
    let now = DateTime::<Local>::from(now).fixed_offset();
    (saved_label(saved, now), saved_tooltip(saved))
}

/// [`saved_date`] for a `SystemTime`, in the local zone.
pub fn local_date(saved: SystemTime) -> String {
    saved_date(DateTime::<Local>::from(saved).fixed_offset())
}

/// How long before `now` the send was saved: "just now", "2 minutes ago" and so on.
pub fn age_of(saved: SystemTime, now: SystemTime) -> String {
    let seconds = now.duration_since(saved).map_or(0, |d| d.as_secs());
    age(i64::try_from(seconds).unwrap_or(i64::MAX))
}

/// The age in the status row, short so it fits beside the date: "just now", "1m ago",
/// "23h ago", "29d ago", "12mo ago", "5y ago". It changes when `age_of` does.
pub fn short_age(saved: SystemTime, now: SystemTime) -> String {
    let seconds = now.duration_since(saved).map_or(0, |d| d.as_secs());
    match age_unit(i64::try_from(seconds).unwrap_or(i64::MAX)) {
        None => "just now".to_owned(),
        Some((n, _, unit)) => format!("{n}{unit} ago"),
    }
}

// The long age: "just now", "2 minutes ago".
fn age(seconds: i64) -> String {
    let Some((n, unit, _)) = age_unit(seconds) else {
        return "just now".to_owned();
    };
    let plural = if n == 1 { "" } else { "s" };
    format!("{n} {unit}{plural} ago")
}

// The age in the coarsest whole unit that fits, with the unit's long and short names, or
// `None` under a minute; a month counts as 30 days and a year as 365.
fn age_unit(seconds: i64) -> Option<(i64, &'static str, &'static str)> {
    let days = seconds / 86_400;
    Some(match seconds {
        s if s < 60 => return None,
        s if s < 3_600 => (s / 60, "minute", "m"),
        s if s < 86_400 => (s / 3_600, "hour", "h"),
        _ if days < 30 => (days, "day", "d"),
        _ if days < 365 => (days / 30, "month", "mo"),
        _ => (days / 365, "year", "y"),
    })
}

/// One store mutation or read, run in order by the queue.
pub enum StoreOp {
    /// Keep `result`'s response for (root, item).
    Save {
        /// The collection root.
        root: PathBuf,
        /// The request's item path.
        item: ItemPath,
        /// The finished send.
        result: Arc<RequestResult>,
        /// The secret values the request substituted, scrubbed from what is kept.
        secrets: Vec<String>,
        /// When it finished.
        saved_at: SystemTime,
        /// How much the store may keep.
        limits: Limits,
        /// The snapshot and origin kept beside the response.
        extra: SendExtra,
        /// Told the seq the send was kept as, when it was kept.
        kept: Option<async_channel::Sender<KeptAs>>,
    },
    /// Re-key (root, from), and everything under it, to `to`.
    Rename {
        /// The collection root.
        root: PathBuf,
        /// The old item path.
        from: ItemPath,
        /// The new item path.
        to: ItemPath,
    },
    /// Drop (root, item) and everything under it.
    Remove {
        /// The collection root.
        root: PathBuf,
        /// The deleted item path.
        item: ItemPath,
    },
    /// Delete every kept response.
    Clear,
    /// Reply with the kept sends of (root, item), newest first.
    History {
        /// The collection root.
        root: PathBuf,
        /// The request's item path.
        item: ItemPath,
        /// Where the list goes.
        reply: async_channel::Sender<Vec<KeptSend>>,
    },
    /// Reply with up to `limit` sends under `roots`, newest first.
    Recent {
        /// The collection roots to list.
        roots: Vec<PathBuf>,
        /// The most sends to list.
        limit: usize,
        /// Where the list goes.
        reply: async_channel::Sender<Vec<RecentSend>>,
    },
    /// Reply with send `seq` of (root, item), or None when it is gone or unreadable.
    LoadSend {
        /// The collection root.
        root: PathBuf,
        /// The request's item path.
        item: ItemPath,
        /// The send's number.
        seq: u64,
        /// Where the send goes.
        reply: async_channel::Sender<Option<Stored>>,
    },
    /// Reply with send `seq` of (root, item) as `LoadSend` does, without writing the store.
    PeekSend {
        /// The collection root.
        root: PathBuf,
        /// The request's item path.
        item: ItemPath,
        /// The send's number.
        seq: u64,
        /// Where the send goes.
        reply: async_channel::Sender<Option<Stored>>,
    },
    /// Reply with send `seq`'s snapshot JSON, or None; never writes the store.
    PeekRequest {
        /// The collection root.
        root: PathBuf,
        /// The request's item path.
        item: ItemPath,
        /// The send's number.
        seq: u64,
        /// Where the snapshot goes.
        reply: async_channel::Sender<Option<String>>,
    },
}

impl StoreOp {
    fn run(self, store: &ResponseStore) {
        match self {
            StoreOp::Save {
                root,
                item,
                result,
                secrets,
                saved_at,
                limits,
                extra,
                kept,
            } => {
                let saved =
                    store.save_send(&root, &item, &result, &secrets, saved_at, limits, &extra);
                if let (Ok(Some(seq)), Some(kept)) = (saved, kept) {
                    let _ = kept.try_send(KeptAs::new(seq, saved_at));
                }
            }
            StoreOp::Rename { root, from, to } => {
                let _ = store.rename(&root, &from, &to);
            }
            StoreOp::Remove { root, item } => {
                let _ = store.remove(&root, &item);
            }
            StoreOp::Clear => {
                let _ = store.clear();
            }
            // A caller that stopped listening is not an error.
            StoreOp::History { root, item, reply } => {
                let _ = reply.try_send(store.history(&root, &item));
            }
            StoreOp::Recent {
                roots,
                limit,
                reply,
            } => {
                let _ = reply.try_send(store.recent(&roots, limit));
            }
            StoreOp::LoadSend {
                root,
                item,
                seq,
                reply,
            } => {
                let _ = reply.try_send(store.load_send(&root, &item, seq));
            }
            StoreOp::PeekSend {
                root,
                item,
                seq,
                reply,
            } => {
                let _ = reply.try_send(store.peek_send(&root, &item, seq));
            }
            StoreOp::PeekRequest {
                root,
                item,
                seq,
                reply,
            } => {
                let _ = reply.try_send(store.request_json(&root, &item, seq));
            }
        }
    }
}

struct ResponseQueue(async_channel::Sender<StoreOp>);

impl Global for ResponseQueue {}

/// Sets the `ResponseStore` and `ResponseQueue` globals, starts the one worker that runs ops in
/// order, and starts the startup re-mask sweep of kept sends.
pub fn install(store: ResponseStore, cx: &mut App) {
    let (sender, receiver) = async_channel::unbounded::<StoreOp>();
    let worker = store.clone();
    let sweeper = store.clone();
    cx.set_global(store);
    cx.set_global(ResponseQueue(sender));
    cx.background_spawn(async move {
        while let Ok(op) = receiver.recv().await {
            op.run(&worker);
        }
    })
    .detach();
    // The re-mask sweep runs beside the worker, so neither startup nor queued ops wait on it.
    cx.background_spawn(async move { sweeper.sweep() }).detach();
}

/// Queues `op`; does nothing when no queue is installed. Errors are ignored.
pub fn enqueue(op: StoreOp, cx: &App) {
    if let Some(queue) = cx.try_global::<ResponseQueue>() {
        let _ = queue.0.try_send(op);
    }
}

/// Which kept send a save became: its seq, and its save time as the history list reads it back.
/// A seq is reused once the newest send is gone, so the pair tells the two apart.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeptAs {
    /// The send's number within its request.
    pub seq: u64,
    /// The save time, cut to the whole milliseconds the index keeps.
    pub saved_at: SystemTime,
}

impl KeptAs {
    fn new(seq: u64, saved_at: SystemTime) -> KeptAs {
        let ms = saved_at
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        KeptAs {
            seq,
            saved_at: UNIX_EPOCH + Duration::from_millis(ms),
        }
    }

    /// Whether `send` is this kept send.
    pub fn is(&self, send: &KeptSend) -> bool {
        send.seq == self.seq && send.saved_at == self.saved_at
    }
}

/// Queues a Save of (root, item) when remembering is on (on without `AppState`) and a store is
/// installed; the history limit comes from the settings. The answer, when one is queued, brings
/// the seq the send was kept as.
pub fn record(
    root: PathBuf,
    item: ItemPath,
    result: Arc<RequestResult>,
    secrets: Vec<String>,
    extra: SendExtra,
    cx: &App,
) -> Option<async_channel::Receiver<KeptAs>> {
    // Remembering counts as on without `AppState`, so a model built alone still keeps.
    let app = cx.try_global::<AppState>();
    let remember = app.is_none_or(|app| app.settings.read(cx).get().remember_responses);
    if !remember || !cx.has_global::<ResponseStore>() {
        return None;
    }
    let limits = app
        .map(|app| Limits {
            per_request: app.settings.read(cx).get().history_limit as usize,
            total_bytes: STORE_LIMIT,
        })
        .unwrap_or_default();
    let (reply, answer) = async_channel::bounded(1);
    let op = StoreOp::Save {
        root,
        item,
        result,
        secrets,
        saved_at: SystemTime::now(),
        limits,
        extra,
        kept: Some(reply),
    };
    enqueue(op, cx);
    Some(answer)
}

/// Clears every kept send, takes shown kept results off screen and re-reads every live
/// response pane's history list.
pub fn clear_all(cx: &mut App) {
    enqueue(StoreOp::Clear, cx);
    crate::send_model::forget_all_restored(cx);
    // Queued after the Clear, so each re-read sees the emptied store.
    crate::response_pane::refresh_all_history(cx);
}

/// The first [`STREAM_BODY_LIMIT`] bytes of a response's body, cut back to a UTF-8 boundary,
/// marked truncated with the original size kept.
fn head_of(response: &Response) -> Response {
    let size = response.size();
    let bytes = response.body();
    let cut = match std::str::from_utf8(&bytes[..STREAM_BODY_LIMIT]) {
        Ok(_) => STREAM_BODY_LIMIT,
        Err(e) => e.valid_up_to(),
    };
    let head = &bytes[..cut];
    let text = String::from_utf8_lossy(head).into_owned();
    response.clone().with_body(head, text).with_truncated(size)
}

#[cfg(test)]
mod sweep_tests {
    use super::*;

    // A send renamed away (a version 1 migration) between the listing and the read.
    #[test]
    fn a_listed_send_gone_before_its_read_writes_no_marker() {
        let dir = tempfile::tempdir().unwrap();
        let store = ResponseStore::new(dir.path().to_path_buf());
        store.sweep_files([Ok(dir.path().join("v.json"))], None, SystemTime::now());
        assert!(!dir.path().join(SWEEP_MARKER).exists());
    }

    // A v2 index listing nothing, so every send file beside it is an orphan.
    fn empty_store() -> (tempfile::TempDir, ResponseStore) {
        let dir = tempfile::tempdir().unwrap();
        let store = ResponseStore::new(dir.path().to_path_buf());
        let index = Index {
            version: FORMAT_VERSION,
            entries: Vec::new(),
        };
        persist::save_json(&store.index_path(), &index).unwrap();
        (dir, store)
    }

    // The orphan was old when listed but was rewritten before the removal.
    #[test]
    fn an_orphan_touched_after_the_listing_is_kept() {
        let (dir, store) = empty_store();
        let orphan = dir.path().join("o.0.json");
        std::fs::write(&orphan, b"{}").unwrap();
        assert!(store.tidy_index(std::slice::from_ref(&orphan), SystemTime::now()));
        assert!(orphan.exists());
    }

    #[test]
    fn an_orphan_gone_before_the_removal_counts_as_removed() {
        let (dir, store) = empty_store();
        let orphan = dir.path().join("o.0.json");
        assert!(store.tidy_index(&[orphan], SystemTime::now()));
    }

    #[test]
    fn a_store_cleared_mid_sweep_is_not_recreated_for_the_marker() {
        let dir = tempfile::tempdir().unwrap();
        let store = ResponseStore::new(dir.path().join("responses"));
        store.sweep_files([], None, SystemTime::now());
        assert!(!store.dir().exists());
    }
}
