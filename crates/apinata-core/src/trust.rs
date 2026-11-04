//! Host normalisation, loopback detection and the per-user trust file that records which
//! hosts each collection may send to without approval, and which ones the user refused.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::io::Write as _;
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use url::{Host, Url};

/// Why the trust file could not be read or written, or a host could not be recorded.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum TrustError {
    /// The platform has no configuration directory to hold the trust file.
    #[error("no configuration directory for the trusted-hosts file")]
    NoDirectory,
    /// The trust file exists but is not a version 1 trusted-hosts or script-trust document.
    #[error("{}: {message}", path.display())]
    Malformed {
        /// The trust file.
        path: PathBuf,
        /// Why it was rejected.
        message: String,
    },
    /// The trust file could not be read or written.
    #[error("{}: {message}", path.display())]
    Io {
        /// The path that failed.
        path: PathBuf,
        /// The underlying error, as text.
        message: String,
    },
    /// The text names no host.
    #[error("not a host: {host}")]
    InvalidHost {
        /// The text as given.
        host: String,
    },
}

/// Whether `ip` is a loopback address: `127.0.0.0/8`, `::1`, or an IPv4-mapped `127.0.0.0/8`.
pub fn is_loopback(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => v4.is_loopback(),
        IpAddr::V6(v6) => {
            v6.is_loopback() || v6.to_ipv4_mapped().is_some_and(|v4| v4.is_loopback())
        }
    }
}

/// The host in `input` as the URL parser reads it: lowercase, IDNA-encoded, percent-decoded,
/// IPv4 number forms expanded, IPv6 bracketed, without a trailing dot. `input` may be a bare
/// host, an authority or a whole URL. `None` when it names no host.
pub fn normalize_host(input: &str) -> Option<String> {
    let input = input.trim();
    let parsed = if input.contains("://") {
        Url::parse(input).ok()
    } else {
        Url::parse(&format!("http://{input}")).ok()
    };
    let host = match parsed.as_ref().and_then(Url::host) {
        Some(host) => host.to_owned(),
        // A bare IPv6 address has colons a URL authority cannot hold unbracketed.
        None => Host::Ipv6(input.parse().ok()?),
    };
    match host {
        Host::Domain(name) => {
            let name = name.strip_suffix('.').unwrap_or(&name);
            (!name.is_empty()).then(|| name.to_owned())
        }
        // url's Display, not Rust's, so an IPv4-mapped address reads as the hop host does.
        ip => Some(ip.to_string()),
    }
}

/// `host` as a person should read it: a bracketed IPv4-mapped IPv6 address shows its IPv4
/// part dotted (`[::ffff:10.0.0.1]`, not `[::ffff:a00:1]`); anything else is unchanged. For
/// display only, never a trust key.
pub fn display_host(host: &str) -> String {
    host.strip_prefix('[')
        .and_then(|h| h.strip_suffix(']'))
        .and_then(|h| h.parse::<std::net::Ipv6Addr>().ok())
        .and_then(|ip| ip.to_ipv4_mapped())
        .map_or_else(|| host.to_owned(), |v4| format!("[::ffff:{v4}]"))
}

/// [`normalize_host`] of `input`, only when `input` is a bare host: the result must read the
/// same, ignoring case and one trailing dot, so a URL or authority never yields a host other
/// than the one a person read. Wildcards and empty labels are refused. `None` otherwise.
pub fn normalize_bare_host(input: &str) -> Option<String> {
    let input = input.trim();
    if label_fault(input).is_some() {
        return None;
    }
    let host = normalize_host(input)?;
    let bare = input.strip_suffix('.').unwrap_or(input);
    host.eq_ignore_ascii_case(bare).then_some(host)
}

/// Why `input` is not a bare host, as a message to show the person who typed it.
pub fn bare_host_refusal(input: &str) -> String {
    let input = input.trim();
    // The host after an `@` may be the one an attacker wants trusted; never suggest it.
    if input.contains('@') {
        return "not a bare host: enter a bare host name, without a user name or @".to_owned();
    }
    if let Some(fault) = label_fault(input) {
        return format!("not a bare host: {fault}");
    }
    match normalize_host(input) {
        Some(host) => format!("not a bare host: give it as {host}"),
        None => format!("not a host: {input}"),
    }
}

// The trust file matches hosts exactly, so a pattern or a malformed name could never match
// the host a request actually sends to.
fn label_fault(input: &str) -> Option<&'static str> {
    let name = input.strip_suffix('.').unwrap_or(input);
    if name.contains('*') {
        Some("wildcards are not supported; trust each host by name")
    } else if !name.is_empty() && !name.starts_with('[') && name.split('.').any(str::is_empty) {
        Some("a host name cannot have an empty label, such as a leading dot or two dots in a row")
    } else {
        None
    }
}

/// The per-user list of trusted and denied hosts, keyed by canonical collection root.
/// Changes stay in memory until [`TrustStore::save`]; saving replaces the file, not merges.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct TrustStore {
    path: PathBuf,
    collections: BTreeMap<String, Lists>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
struct Lists {
    #[serde(default)]
    trusted: Vec<String>,
    #[serde(default)]
    denied: Vec<String>,
}

#[derive(Serialize, Deserialize)]
struct TrustFile {
    version: u32,
    #[serde(default)]
    collections: BTreeMap<String, Lists>,
}

impl TrustStore {
    /// `<config dir>/apinata/trusted-hosts.json`, or `None` when the platform has no config dir.
    pub fn default_path() -> Option<PathBuf> {
        dirs::config_dir().map(|dir| dir.join("apinata").join("trusted-hosts.json"))
    }

    /// Load the store at [`TrustStore::default_path`].
    pub fn load_default() -> Result<TrustStore, TrustError> {
        Self::load(Self::default_path().ok_or(TrustError::NoDirectory)?)
    }

    /// Load the store at `path`; a missing file is an empty store. Hand-written entries are
    /// normalised and entries naming no host are dropped. Fails on unreadable or bad JSON.
    pub fn load(path: impl Into<PathBuf>) -> Result<TrustStore, TrustError> {
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
        let clean = |hosts: Vec<String>| {
            let mut out: Vec<String> = Vec::new();
            for host in hosts.iter().filter_map(|h| normalize_host(h)) {
                if !out.contains(&host) {
                    out.push(host);
                }
            }
            out
        };
        let collections = file
            .collections
            .into_iter()
            .map(|(root, lists)| {
                let lists = Lists {
                    trusted: clean(lists.trusted),
                    denied: clean(lists.denied),
                };
                (root, lists)
            })
            .collect();
        Ok(TrustStore { path, collections })
    }

    /// An empty store that [`TrustStore::save`] writes to `path`.
    pub fn empty(path: impl Into<PathBuf>) -> TrustStore {
        TrustStore {
            path: path.into(),
            collections: BTreeMap::new(),
        }
    }

    /// The file this store loads from and saves to.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Whether `host` is on `collection`'s trusted list. Exact match, no wildcards.
    pub fn is_trusted(&self, collection: &Path, host: &str) -> bool {
        self.contains(collection, host, |l| &l.trusted)
    }

    /// Whether `host` is on `collection`'s denied list.
    pub fn is_denied(&self, collection: &Path, host: &str) -> bool {
        self.contains(collection, host, |l| &l.denied)
    }

    /// `collection`'s trusted hosts, normalised, in the order they were added.
    pub fn trusted(&self, collection: &Path) -> Vec<String> {
        self.lists(collection)
            .map(|l| l.trusted.clone())
            .unwrap_or_default()
    }

    /// `collection`'s denied hosts, normalised, in the order they were added.
    pub fn denied(&self, collection: &Path) -> Vec<String> {
        self.lists(collection)
            .map(|l| l.denied.clone())
            .unwrap_or_default()
    }

    /// Trust `host` for `collection`, taking it off the denied list.
    pub fn trust(&mut self, collection: &Path, host: &str) -> Result<(), TrustError> {
        let host = valid_host(host)?;
        let lists = self.collections.entry(key(collection)).or_default();
        lists.denied.retain(|h| *h != host);
        if !lists.trusted.contains(&host) {
            lists.trusted.push(host);
        }
        Ok(())
    }

    /// Deny `host` for `collection`, taking it off the trusted list.
    pub fn deny(&mut self, collection: &Path, host: &str) -> Result<(), TrustError> {
        let host = valid_host(host)?;
        let lists = self.collections.entry(key(collection)).or_default();
        lists.trusted.retain(|h| *h != host);
        if !lists.denied.contains(&host) {
            lists.denied.push(host);
        }
        Ok(())
    }

    /// Take `host` off both of `collection`'s lists; `true` when it was on either.
    pub fn remove(&mut self, collection: &Path, host: &str) -> bool {
        let Some(host) = normalize_host(host) else {
            return false;
        };
        let Some(lists) = self.collections.get_mut(&key(collection)) else {
            return false;
        };
        let before = lists.trusted.len() + lists.denied.len();
        lists.trusted.retain(|h| *h != host);
        lists.denied.retain(|h| *h != host);
        before != lists.trusted.len() + lists.denied.len()
    }

    /// Write the store to its path atomically as pretty JSON, creating the directory and
    /// dropping collections whose lists are both empty.
    pub fn save(&self) -> Result<(), TrustError> {
        let collections = self
            .collections
            .iter()
            .filter(|(_, l)| !l.trusted.is_empty() || !l.denied.is_empty())
            .map(|(root, l)| (root.clone(), l.clone()))
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

    fn lists(&self, collection: &Path) -> Option<&Lists> {
        self.collections.get(&key(collection))
    }

    fn contains(&self, collection: &Path, host: &str, list: fn(&Lists) -> &Vec<String>) -> bool {
        match (normalize_host(host), self.lists(collection)) {
            (Some(host), Some(lists)) => list(lists).contains(&host),
            _ => false,
        }
    }
}

/// The canonical collection root as text, falling back to the path as given, as
/// `Collection::secret_key` does.
pub(crate) fn key(collection: &Path) -> String {
    let root = fs::canonicalize(collection).unwrap_or_else(|_| collection.to_path_buf());
    root.to_string_lossy().into_owned()
}

fn valid_host(host: &str) -> Result<String, TrustError> {
    normalize_host(host).ok_or_else(|| TrustError::InvalidHost {
        host: host.to_owned(),
    })
}

/// Runs `body` while holding an exclusive lock on a sidecar lock file beside `path`'s target,
/// so writers in any process take turns. Not reentrant: `body` must not lock the same file.
pub(crate) fn with_lock<R>(
    path: &Path,
    body: impl FnOnce() -> Result<R, TrustError>,
) -> Result<R, TrustError> {
    // Lock beside the file `write_atomic` actually replaces, so a symlinked file shares one
    // lock with writers that name its target directly.
    let target = symlink_target(path);
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
    let out = body();
    drop(guard);
    out
}

/// Replace `path` with `bytes` through a uniquely named hidden sibling and a rename, so a
/// reader never sees a half-written file and concurrent writers never share a temporary.
/// A symlinked `path` is written through to its target, keeping the link. Creates the parent
/// directory; removes the temporary on failure.
pub(crate) fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), TrustError> {
    let io = |at: &Path, e: std::io::Error| TrustError::Io {
        path: at.to_path_buf(),
        message: e.to_string(),
    };
    let path = &symlink_target(path);
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(parent).map_err(|e| io(parent, e))?;
    let name = path
        .file_name()
        .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
    // The NamedTempFile deletes itself if the write or the rename fails.
    let mut temp = tempfile::Builder::new()
        .prefix(&format!(".{name}."))
        .suffix(".apinata-tmp")
        .tempfile_in(parent)
        .map_err(|e| io(parent, e))?;
    temp.write_all(bytes).map_err(|e| io(path, e))?;
    temp.persist(path).map_err(|e| io(path, e.error))?;
    Ok(())
}

/// The file a write to `path` should replace: `path` itself, or the end of its symlink
/// chain (which may not exist yet), so a link a user manages keeps pointing at the data.
pub(crate) fn symlink_target(path: &Path) -> PathBuf {
    let mut current = path.to_path_buf();
    // Bounded like the kernel's own limit, so a symlink loop ends instead of spinning.
    for _ in 0..40 {
        let Ok(next) = fs::read_link(&current) else {
            break;
        };
        current = match current.parent() {
            Some(dir) if next.is_relative() => dir.join(next),
            _ => next,
        };
    }
    current
}
