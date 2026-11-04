//! A per-collection record of the items the CLI and MCP shells moved to the trash, so a later
//! process can put the newest one back. One JSON file per collection, named by a hash of its
//! canonical root, newest entry last. A lock file beside each record serializes every
//! read-modify-write across processes. The app keeps its own in-session undo history and
//! neither writes nor reads these records, so one shell never undoes another's delete.

use std::ffi::{OsStr, OsString};
use std::fs::{self, File};
use std::io::Write as _;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::collection::Trashed;
use crate::item_path::ItemPath;
use crate::trash::TrashHandle;

/// How many deletes a collection's record keeps; older ones fall off.
const KEEP: usize = 50;

/// The directory of trash records the shells share.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct TrashLog {
    dir: PathBuf,
}

/// Why a record could not be read or written, naming its file.
#[derive(Debug)]
pub(crate) struct LogError {
    pub(crate) file: PathBuf,
    pub(crate) message: String,
}

#[derive(Serialize, Deserialize)]
struct Record {
    root: RawPath,
    entries: Vec<Entry>,
}

#[derive(Serialize, Deserialize, Clone)]
pub(crate) struct Entry {
    item: String,
    original: RawPath,
    id: RawPath,
    renumbered: Vec<Renumbered>,
    /// The changed siblings `renumbered` cannot vouch for; an entry without any reads and writes
    /// as before.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    ahead: Vec<Renumbered>,
    /// The update run that pruned the item; a single delete has none, so its entry reads
    /// and writes as before batches existed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    batch: Option<String>,
}

/// A platform path or name kept without loss: a JSON string when it is UTF-8, as every record
/// before this form was, and otherwise the platform's raw units.
#[derive(Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(untagged)]
enum RawPath {
    Text(String),
    Unix { unix: Vec<u8> },
    Windows { windows: Vec<u16> },
}

impl RawPath {
    fn of(s: &OsStr) -> RawPath {
        if let Some(text) = s.to_str() {
            return RawPath::Text(text.to_owned());
        }
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStrExt;
            RawPath::Unix {
                unix: s.as_bytes().to_vec(),
            }
        }
        #[cfg(windows)]
        {
            use std::os::windows::ffi::OsStrExt;
            RawPath::Windows {
                windows: s.encode_wide().collect(),
            }
        }
        #[cfg(not(any(unix, windows)))]
        RawPath::Text(s.to_string_lossy().into_owned())
    }

    /// The name back, or `None` when it was recorded on another kind of platform.
    fn to_os(&self) -> Option<OsString> {
        match self {
            RawPath::Text(text) => Some(text.into()),
            #[cfg(unix)]
            RawPath::Unix { unix } => {
                use std::os::unix::ffi::OsStringExt;
                Some(OsString::from_vec(unix.clone()))
            }
            #[cfg(windows)]
            RawPath::Windows { windows } => {
                use std::os::windows::ffi::OsStringExt;
                Some(OsString::from_wide(windows))
            }
            _ => None,
        }
    }
}

#[derive(Serialize, Deserialize, Clone)]
struct Renumbered {
    path: String,
    old: u64,
    new: u64,
}

impl Entry {
    /// The collection-relative path as recorded, for messages.
    pub(crate) fn item_text(&self) -> &str {
        &self.item
    }

    /// The import update run that pruned this item, when one did.
    pub(crate) fn batch(&self) -> Option<&str> {
        self.batch.as_deref()
    }

    /// The trashed item this entry describes, or `None` when a field does not parse.
    pub(crate) fn to_trashed(&self) -> Option<Trashed> {
        let item = ItemPath::new(&self.item)?;
        let read = |records: &[Renumbered]| {
            records
                .iter()
                .map(|r| Some((ItemPath::new(&r.path)?, r.old, r.new)))
                .collect::<Option<Vec<_>>>()
        };
        let original = PathBuf::from(self.original.to_os()?);
        let handle = TrashHandle::new(original, self.id.to_os()?);
        Some(Trashed::from_parts(
            item,
            handle,
            read(&self.renumbered)?,
            read(&self.ahead)?,
        ))
    }

    fn of(trashed: &Trashed, batch: Option<&str>) -> Entry {
        let (handle, renumbered) = trashed.parts();
        let write = |records: &[(ItemPath, u64, u64)]| {
            records
                .iter()
                .map(|(path, old, new)| Renumbered {
                    path: path.to_string(),
                    old: *old,
                    new: *new,
                })
                .collect()
        };
        Entry {
            item: trashed.item().to_string(),
            original: RawPath::of(handle.original().as_os_str()),
            id: RawPath::of(handle.id()),
            renumbered: write(renumbered),
            ahead: write(trashed.ahead()),
            batch: batch.map(str::to_owned),
        }
    }
}

/// A batch id unique to this run: the time in nanoseconds and the process id, in hex.
pub(crate) fn new_batch() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    format!("{nanos:x}-{:x}", std::process::id())
}

impl TrashLog {
    /// Records kept in `dir`.
    pub fn new(dir: impl Into<PathBuf>) -> TrashLog {
        TrashLog { dir: dir.into() }
    }

    /// The per-user directory, `apinata/trash-log` under the local data directory.
    pub fn default_dir() -> Option<PathBuf> {
        dirs::data_local_dir().map(|d| d.join("apinata").join("trash-log"))
    }

    /// The file that holds the record for the collection at `root`.
    pub fn file_for(&self, root: &Path) -> PathBuf {
        let digest = Sha256::digest(canonical(root).as_os_str().as_encoded_bytes());
        let name: String = digest[..16].iter().map(|b| format!("{b:02x}")).collect();
        self.dir.join(format!("{name}.json"))
    }

    /// The record for `root`, held exclusively against other processes until the guard drops.
    /// A lock that cannot be taken, such as in a directory not yet created, leaves it unlocked
    /// rather than failing. The directory is created first, so the first use is locked too.
    pub(crate) fn locked(&self, root: &Path) -> Locked<'_> {
        let _ = fs::create_dir_all(&self.dir);
        let mut path = self.file_for(root).into_os_string();
        path.push(".lock");
        let lock = File::options()
            .create(true)
            .truncate(false)
            .write(true)
            .open(path)
            .ok()
            .filter(|f| f.lock().is_ok());
        Locked {
            log: self,
            root: root.to_path_buf(),
            _lock: lock,
        }
    }

    /// The entries for `root`, oldest first. A missing file is empty, and so is one recorded
    /// for another root.
    fn entries(&self, root: &Path) -> Result<Vec<Entry>, LogError> {
        let file = self.file_for(root);
        let text = match fs::read_to_string(&file) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(log_error(&file, e)),
        };
        let record: Record = serde_json::from_str(&text).map_err(|e| log_error(&file, e))?;
        Ok(if record.root == RawPath::of(canonical(root).as_os_str()) {
            record.entries
        } else {
            Vec::new()
        })
    }

    /// Replace the entries for `root`, keeping the newest [`KEEP`].
    fn store(&self, root: &Path, mut entries: Vec<Entry>) -> Result<(), LogError> {
        let file = self.file_for(root);
        entries.drain(..entries.len().saturating_sub(KEEP));
        let record = Record {
            root: RawPath::of(canonical(root).as_os_str()),
            entries,
        };
        let text = serde_json::to_string_pretty(&record).map_err(|e| log_error(&file, e))?;
        write_atomic(&file, text.as_bytes()).map_err(|e| log_error(&file, e))
    }
}

/// One collection's record while [`TrashLog::locked`] holds it.
pub(crate) struct Locked<'a> {
    log: &'a TrashLog,
    root: PathBuf,
    _lock: Option<File>,
}

impl Locked<'_> {
    /// The entries, oldest first; see [`TrashLog::entries`].
    pub(crate) fn entries(&self) -> Result<Vec<Entry>, LogError> {
        self.log.entries(&self.root)
    }

    /// Replace the entries, keeping the newest [`KEEP`].
    pub(crate) fn store(&self, entries: Vec<Entry>) -> Result<(), LogError> {
        self.log.store(&self.root, entries)
    }

    /// Append `trashed`, in `batch` when an update pruned it. An unreadable record is
    /// replaced, and the returned warning says so. It works on the held lock, since a second
    /// lock from this process would wait on the first.
    pub(crate) fn record(
        &self,
        trashed: &Trashed,
        batch: Option<&str>,
    ) -> Result<Option<String>, String> {
        let (mut entries, warning) = match self.entries() {
            Ok(entries) => (entries, None),
            Err(e) => (
                Vec::new(),
                Some(format!(
                    "the trash record {} could not be read ({}) and was replaced",
                    e.file.display(),
                    e.message
                )),
            ),
        };
        entries.push(Entry::of(trashed, batch));
        self.store(entries).map_err(|e| e.to_string())?;
        Ok(warning)
    }
}

impl std::fmt::Display for LogError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.file.display(), self.message)
    }
}

fn log_error(file: &Path, e: impl std::fmt::Display) -> LogError {
    LogError {
        file: file.to_path_buf(),
        message: e.to_string(),
    }
}

/// The root as the record names it: canonical when it can be resolved.
fn canonical(root: &Path) -> PathBuf {
    root.canonicalize().unwrap_or_else(|_| root.to_path_buf())
}

/// Write through a temporary file in the same directory, so a reader never sees half a record.
fn write_atomic(file: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let dir = file.parent().unwrap_or(Path::new("."));
    fs::create_dir_all(dir)?;
    let mut tmp = tempfile::NamedTempFile::new_in(dir)?;
    tmp.write_all(bytes)?;
    tmp.as_file().sync_all()?;
    tmp.persist(file).map_err(|e| e.error)?;
    Ok(())
}
