//! The approval inbox: one JSON file per host an agent asked to reach (version 1) or per set
//! of script texts it asked to run (version 2), which the app watches and answers.

use crate::script_trust::{ScriptDecision, ScriptTrust, is_script_hash};
use crate::trust::{TrustError, TrustStore, normalize_host, write_atomic};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Longest display field, in chars, an approval entry keeps; longer text is cut and ends in
/// an ellipsis.
pub const DISPLAY_MAX_CHARS: usize = 120;

/// `text` as stored for display: past the limit, its first chars and an ellipsis, cut by char
/// so no code point splits.
fn cap_display(text: &str) -> String {
    if text.chars().nth(DISPLAY_MAX_CHARS).is_none() {
        return text.to_owned();
    }
    let mut cut: String = text.chars().take(DISPLAY_MAX_CHARS - 1).collect();
    cut.push('\u{2026}');
    cut
}

/// A directory of pending approval requests.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ApprovalInbox {
    dir: PathBuf,
}

/// One pending request, as read from its file.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Approval {
    /// The collection's canonical root.
    pub collection: PathBuf,
    /// The host, normalised.
    pub host: String,
    /// The item path of the refused request, cut to [`DISPLAY_MAX_CHARS`]; untrusted agent text.
    pub request: String,
    /// When it was filed, RFC 3339 in UTC to the second.
    pub requested_at: String,
    /// The proxy the refused hop went through, as `scheme://host:port`, when the host is one;
    /// cut to [`DISPLAY_MAX_CHARS`].
    pub proxy: Option<String>,
    /// The host name a host override maps to `host`, when it does, cut to
    /// [`DISPLAY_MAX_CHARS`]; untrusted agent text.
    pub mapped_from: Option<String>,
    /// The file holding it.
    pub file: PathBuf,
}

/// One pending request to approve script texts, as read from its file.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ScriptApproval {
    /// The collection's canonical root.
    pub collection: PathBuf,
    /// The item path sent or run, `""` for the whole collection, cut to [`DISPLAY_MAX_CHARS`];
    /// untrusted agent text.
    pub request: String,
    /// The [`crate::script_hash`] of each text awaiting approval.
    pub hashes: Vec<String>,
    /// When it was filed, RFC 3339 in UTC to the second.
    pub requested_at: String,
    /// The file holding it.
    pub file: PathBuf,
}

/// What [`ApprovalInbox::request`] or [`ApprovalInbox::request_scripts`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Filing {
    /// A new request was written to this file.
    Written(PathBuf),
    /// One is already pending for the collection and host, or one covers every script
    /// hash; nothing was written.
    Pending,
    /// The host is denied, or scripts are blocked, for the collection; nothing was written.
    Denied,
}

#[derive(Serialize, Deserialize)]
struct ApprovalFile {
    version: u32,
    collection: String,
    host: String,
    request: String,
    requested_at: String,
    // Optional so files without it, from either direction, stay valid version 1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    proxy: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    mapped_from: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct ScriptApprovalFile {
    version: u32,
    kind: String,
    collection: String,
    request: String,
    hashes: Vec<String>,
    requested_at: String,
}

/// Either kind of pending entry, so pruning and removal walk both.
enum Entry {
    Host(Approval),
    Scripts(ScriptApproval),
}

impl Entry {
    fn file(&self) -> &Path {
        match self {
            Entry::Host(a) => &a.file,
            Entry::Scripts(a) => &a.file,
        }
    }
}

impl ApprovalInbox {
    /// `<local data dir>/apinata/approvals`, or `None` on a platform without one.
    pub fn default_dir() -> Option<PathBuf> {
        dirs::data_local_dir().map(|dir| dir.join("apinata").join("approvals"))
    }

    /// The inbox at [`ApprovalInbox::default_dir`].
    pub fn open_default() -> Result<ApprovalInbox, TrustError> {
        Ok(Self::new(
            Self::default_dir().ok_or(TrustError::NoDirectory)?,
        ))
    }

    /// The inbox kept in `dir`, which the first request creates.
    pub fn new(dir: impl Into<PathBuf>) -> ApprovalInbox {
        ApprovalInbox { dir: dir.into() }
    }

    /// The inbox directory.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// File a request to trust `host` for `collection` as `<pid>-<unix-nanos>.json`, unless
    /// `store` denies the host or a request for the pair is already pending. Fails with
    /// `InvalidHost` when `host` names no host, or `Io` when the file cannot be written.
    pub fn request(
        &self,
        store: &TrustStore,
        collection: &Path,
        host: &str,
        request: &str,
    ) -> Result<Filing, TrustError> {
        self.request_via(store, collection, host, request, None)
    }

    /// Like [`ApprovalInbox::request`], recording `proxy` when `host` is the proxy a hop went
    /// through. A pending request for the pair is not refiled, whatever proxy it named.
    pub fn request_via(
        &self,
        store: &TrustStore,
        collection: &Path,
        host: &str,
        request: &str,
        proxy: Option<&str>,
    ) -> Result<Filing, TrustError> {
        self.file_host(store, collection, host, request, proxy, None)
    }

    /// Like [`ApprovalInbox::request`], recording `mapped_from` as the host name a host
    /// override maps to `host`. The name is context for the person deciding; trust, denial and
    /// the pending check still key on `host` alone.
    pub fn request_mapped(
        &self,
        store: &TrustStore,
        collection: &Path,
        host: &str,
        request: &str,
        mapped_from: &str,
    ) -> Result<Filing, TrustError> {
        self.file_host(store, collection, host, request, None, Some(mapped_from))
    }

    fn file_host(
        &self,
        store: &TrustStore,
        collection: &Path,
        host: &str,
        request: &str,
        proxy: Option<&str>,
        mapped_from: Option<&str>,
    ) -> Result<Filing, TrustError> {
        let host = normalize_host(host).ok_or_else(|| TrustError::InvalidHost {
            host: host.to_owned(),
        })?;
        if store.is_denied(collection, &host) {
            return Ok(Filing::Denied);
        }
        let collection = canonical(collection);
        if self
            .pending()?
            .iter()
            .any(|a| a.collection == collection && a.host == host)
        {
            return Ok(Filing::Pending);
        }
        self.write_entry(|requested_at| ApprovalFile {
            version: 1,
            collection: collection.to_string_lossy().into_owned(),
            host,
            request: cap_display(request),
            requested_at,
            proxy: proxy.map(cap_display),
            mapped_from: mapped_from.map(cap_display),
        })
    }

    /// File a request to approve the script texts `hashes` (each a [`crate::script_hash`])
    /// for `collection`, as a version 2 entry. Hashes of the wrong shape are dropped. Nothing
    /// is written when `trust` blocks the collection's scripts (`Denied`), or when no
    /// well-formed hash is left or one pending entry already covers every hash (`Pending`, as
    /// nothing new awaits approval).
    pub fn request_scripts(
        &self,
        trust: &ScriptTrust,
        collection: &Path,
        request: &str,
        hashes: &[String],
    ) -> Result<Filing, TrustError> {
        if trust.decision(collection) == Some(ScriptDecision::Blocked) {
            return Ok(Filing::Denied);
        }
        // The reader rejects an entry holding a malformed hash, so such a file could never be
        // shown or pruned; keep only well-formed hashes and file nothing when none remain.
        let hashes: Vec<String> = hashes
            .iter()
            .filter(|h| is_script_hash(h))
            .cloned()
            .collect();
        if hashes.is_empty() {
            return Ok(Filing::Pending);
        }
        let collection = canonical(collection);
        if self
            .pending_scripts()?
            .iter()
            .any(|a| a.collection == collection && hashes.iter().all(|h| a.hashes.contains(h)))
        {
            return Ok(Filing::Pending);
        }
        self.write_entry(|requested_at| ScriptApprovalFile {
            version: 2,
            kind: "scripts".to_owned(),
            collection: collection.to_string_lossy().into_owned(),
            request: cap_display(request),
            hashes,
            requested_at,
        })
    }

    /// Write the entry `build` makes from the filing time to a new `<pid>-<unix-nanos>.json`.
    fn write_entry<T: Serialize>(
        &self,
        build: impl FnOnce(String) -> T,
    ) -> Result<Filing, TrustError> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        // Two filings in one clock tick bump the nanoseconds rather than share a name.
        let name = |nanos: u128| {
            self.dir
                .join(format!("{}-{nanos}.json", std::process::id()))
        };
        let mut nanos = now.as_nanos();
        while name(nanos).exists() {
            nanos += 1;
        }
        let path = name(nanos);
        let file = build(rfc3339(now.as_secs()));
        let mut bytes = serde_json::to_vec_pretty(&file).map_err(|e| io_error(&path, e))?;
        bytes.push(b'\n');
        write_atomic(&path, &bytes)?;
        Ok(Filing::Written(path))
    }

    /// Every pending request, sorted by file name. A missing directory is empty; dot-files,
    /// non-`.json` files, unparsable files and versions other than 1 are skipped.
    pub fn pending(&self) -> Result<Vec<Approval>, TrustError> {
        let mut found: Vec<Approval> = self
            .entry_files()?
            .into_iter()
            .filter_map(|path| {
                let file: ApprovalFile =
                    serde_json::from_str(&fs::read_to_string(&path).ok()?).ok()?;
                (file.version == 1).then(|| Approval {
                    collection: PathBuf::from(file.collection),
                    host: file.host,
                    request: file.request,
                    requested_at: file.requested_at,
                    proxy: file.proxy,
                    mapped_from: file.mapped_from,
                    file: path,
                })
            })
            .collect();
        found.sort_by(|a, b| a.file.cmp(&b.file));
        Ok(found)
    }

    /// Every pending script request, sorted by file name. Entries other than version 2
    /// `kind: scripts` with a non-empty list of well-formed hashes are skipped.
    pub fn pending_scripts(&self) -> Result<Vec<ScriptApproval>, TrustError> {
        let mut found: Vec<ScriptApproval> = self
            .entry_files()?
            .into_iter()
            .filter_map(|path| {
                let file: ScriptApprovalFile =
                    serde_json::from_str(&fs::read_to_string(&path).ok()?).ok()?;
                let valid = file.version == 2
                    && file.kind == "scripts"
                    && !file.hashes.is_empty()
                    && file.hashes.iter().all(|h| is_script_hash(h));
                valid.then(|| ScriptApproval {
                    collection: PathBuf::from(file.collection),
                    request: file.request,
                    hashes: file.hashes,
                    requested_at: file.requested_at,
                    file: path,
                })
            })
            .collect();
        found.sort_by(|a, b| a.file.cmp(&b.file));
        Ok(found)
    }

    /// Delete `approval`'s file; `false` when it was already gone.
    pub fn remove_scripts(&self, approval: &ScriptApproval) -> Result<bool, TrustError> {
        match fs::remove_file(&approval.file) {
            Ok(()) => Ok(true),
            Err(e) if e.kind() == ErrorKind::NotFound => Ok(false),
            Err(e) => Err(io_error(&approval.file, e)),
        }
    }

    /// The candidate entry files: `.json` files in the inbox that are not dot-files. A
    /// missing directory has none.
    fn entry_files(&self) -> Result<Vec<PathBuf>, TrustError> {
        let entries = match fs::read_dir(&self.dir) {
            Ok(entries) => entries,
            Err(e) if e.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(io_error(&self.dir, e)),
        };
        Ok(entries
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                let name = path.file_name().and_then(|n| n.to_str()).unwrap_or(".");
                !name.starts_with('.') && name.ends_with(".json")
            })
            .collect())
    }

    /// Delete every pending request for `collection` and `host`, returning how many.
    pub fn resolve(&self, collection: &Path, host: &str) -> Result<usize, TrustError> {
        let collection = canonical(collection);
        let Some(host) = normalize_host(host) else {
            return Ok(0);
        };
        self.remove_where(
            |e| matches!(e, Entry::Host(a) if a.collection == collection && a.host == host),
        )
    }

    /// Delete pending host and script requests whose file was last modified more than
    /// `older_than` ago, returning how many. Age is the mtime, so a hand-edited stamp cannot
    /// keep a file alive.
    pub fn prune(&self, older_than: Duration) -> Result<usize, TrustError> {
        let now = SystemTime::now();
        self.remove_where(|e| {
            fs::metadata(e.file())
                .and_then(|m| m.modified())
                .ok()
                .and_then(|at| now.duration_since(at).ok())
                .is_some_and(|age| age > older_than)
        })
    }

    /// Delete the pending host and script requests `doomed` picks. A file already gone was
    /// removed by another process and is not counted.
    fn remove_where(&self, doomed: impl Fn(&Entry) -> bool) -> Result<usize, TrustError> {
        let mut entries: Vec<Entry> = self.pending()?.into_iter().map(Entry::Host).collect();
        entries.extend(self.pending_scripts()?.into_iter().map(Entry::Scripts));
        let mut removed = 0;
        for entry in entries.iter().filter(|e| doomed(e)) {
            match fs::remove_file(entry.file()) {
                Ok(()) => removed += 1,
                Err(e) if e.kind() == ErrorKind::NotFound => {}
                Err(e) => return Err(io_error(entry.file(), e)),
            }
        }
        Ok(removed)
    }
}

/// The canonical collection root, falling back to the path as given, as the trust store keys it.
fn canonical(path: &Path) -> PathBuf {
    fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

fn io_error(path: &Path, e: impl std::fmt::Display) -> TrustError {
    TrustError::Io {
        path: path.to_path_buf(),
        message: e.to_string(),
    }
}

/// `secs` since the Unix epoch as `YYYY-MM-DDTHH:MM:SSZ`, through Howard Hinnant's
/// days-to-civil conversion so no date crate is needed.
fn rfc3339(secs: u64) -> String {
    let (days, rem) = ((secs / 86_400) as i64, secs % 86_400);
    let z = days + 719_468;
    let (era, doe) = (z.div_euclid(146_097), z.rem_euclid(146_097));
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rem / 3_600,
        rem % 3_600 / 60,
        rem % 60
    )
}

#[cfg(test)]
mod tests {
    use super::rfc3339;

    #[test]
    fn timestamps_are_rfc3339_utc() {
        assert_eq!(rfc3339(0), "1970-01-01T00:00:00Z");
        assert_eq!(rfc3339(951_782_400), "2000-02-29T00:00:00Z");
        assert_eq!(rfc3339(1_791_580_443), "2026-10-09T21:14:03Z");
        assert_eq!(rfc3339(4_102_444_799), "2099-12-31T23:59:59Z");
    }
}
