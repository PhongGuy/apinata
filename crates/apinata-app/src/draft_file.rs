//! The on-disk form of an untitled draft kept across restarts: `<data dir>/drafts/<stem>.json`.

use std::fs::{File, OpenOptions, TryLockError};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use apinata_core::{DraftSnapshot, ItemPath};
use serde::{Deserialize, Serialize};

use crate::scrub::Forms;

/// The largest draft file kept, measured as `save_json` writes it.
pub const MAX_DRAFT_BYTES: usize = 1 << 20;
/// The most draft files kept and opened.
pub const MAX_DRAFTS: usize = 50;
/// Secrets shorter than this are not scrubbed, as they would redact ordinary text.
pub const MIN_SECRET_CHARS: usize = 4;

const VERSION: u32 = 1;

/// One kept draft: where it lives, its history key and its blanked, scrubbed content.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DraftFile {
    /// The schema version, 1.
    pub version: u32,
    /// The draft's history key, `untitled-<ms><6 digits>.draft`.
    pub key: String,
    /// The collection root the draft belongs to.
    pub root: PathBuf,
    /// The folder the draft was made in, as an item path string; `""` for the root.
    pub parent: String,
    /// When the history key was made, in milliseconds since the epoch.
    pub created_ms: u64,
    /// Whether credentials were blanked out of the content.
    pub credentials_blanked: bool,
    /// The request, as `DraftSnapshot::to_json`.
    pub request: String,
    /// Which of the tabs sharing this history key the file belongs to; 0 for the first, and
    /// absent from files written before tabs could share a key.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub slot: u32,
}

fn is_zero(n: &u32) -> bool {
    *n == 0
}

/// Why a draft could not be built into a file.
#[derive(Debug, Clone, PartialEq)]
pub enum BuildError {
    /// The file would be over `MAX_DRAFT_BYTES`.
    TooLarge,
    /// The request cannot be written as JSON, so the file would hold `null`.
    Unserializable(String),
}

/// `<stem>.json` for key `<stem>.draft`.
pub fn file_name(key: &ItemPath) -> String {
    let name = key.file_name();
    format!("{}.json", name.strip_suffix(".draft").unwrap_or(name))
}

/// The file name of tab `slot` of key `<stem>.draft`: `<stem>.json` for slot 0, else
/// `<stem>.<slot>.json`.
pub fn file_name_slot(key: &ItemPath, slot: u32) -> String {
    if slot == 0 {
        return file_name(key);
    }
    let name = key.file_name();
    format!(
        "{}.{slot}.json",
        name.strip_suffix(".draft").unwrap_or(name)
    )
}

/// The milliseconds a history key was made at; None for another name.
pub fn created_ms(key: &ItemPath) -> Option<u64> {
    if !key.is_draft() {
        return None;
    }
    let digits = key
        .file_name()
        .strip_prefix("untitled-")?
        .strip_suffix(".draft")?;
    // The key appends six random digits to the milliseconds.
    let ms = digits.get(..digits.len().checked_sub(6)?)?;
    ms.parse().ok()
}

/// The version 1 file for a draft: `snapshot` (already blanked) with every string scrubbed of
/// `secrets` of 4 or more characters. `TooLarge` when the pretty JSON plus newline is over MAX_DRAFT_BYTES, `Unserializable` when the
/// request has no JSON form.
pub fn build(
    key: &ItemPath,
    root: &Path,
    parent: &ItemPath,
    snapshot: &DraftSnapshot,
    credentials_blanked: bool,
    secrets: &[String],
) -> Result<DraftFile, BuildError> {
    build_slot(key, 0, root, parent, snapshot, credentials_blanked, secrets)
}

/// `build` for tab `slot` of the history key.
pub fn build_slot(
    key: &ItemPath,
    slot: u32,
    root: &Path,
    parent: &ItemPath,
    snapshot: &DraftSnapshot,
    credentials_blanked: bool,
    secrets: &[String],
) -> Result<DraftFile, BuildError> {
    let kept: Vec<String> = secrets
        .iter()
        .filter(|s| s.chars().count() >= MIN_SECRET_CHARS)
        .cloned()
        .collect();
    let forms = Forms::new(&kept);
    let mut snapshot = snapshot.clone();
    snapshot.map_strings(|s| forms.scrub(s));
    // `to_json` yields "null" for a mapping JSON cannot hold, which load would refuse silently.
    let request = snapshot.to_json();
    if DraftSnapshot::from_json(&request).is_err() {
        return Err(BuildError::Unserializable(
            "the request cannot be written as JSON".into(),
        ));
    }
    let file = DraftFile {
        version: VERSION,
        key: key.as_str().to_owned(),
        root: root.to_path_buf(),
        parent: parent.as_str().to_owned(),
        created_ms: created_ms(key).unwrap_or(0),
        credentials_blanked,
        request,
        slot,
    };
    let size = serde_json::to_vec_pretty(&file).map_or(usize::MAX, |b| b.len() + 1);
    if size > MAX_DRAFT_BYTES {
        return Err(BuildError::TooLarge);
    }
    Ok(file)
}

/// The draft files found in a directory.
pub struct Loaded {
    /// The valid files to open, oldest first.
    pub drafts: Vec<DraftFile>,
    /// A message naming each file that could not be used.
    pub skipped: Vec<String>,
    /// How many valid files were left closed beyond `MAX_DRAFTS`.
    pub unopened: usize,
}

/// Every valid `*.json` in `dir`; the newest MAX_DRAFTS by created_ms, returned oldest first.
pub fn load_dir(dir: &Path) -> Loaded {
    let mut drafts = Vec::new();
    let mut skipped = Vec::new();
    let entries = std::fs::read_dir(dir).into_iter().flatten().flatten();
    for path in entries.map(|e| e.path()) {
        if path.extension().is_none_or(|e| e != "json") {
            continue;
        }
        match load_one(&path) {
            Ok(file) => drafts.push(file),
            Err(why) => skipped.push(format!("{}: {why}", path.display())),
        }
    }
    drafts.sort_by(|a: &DraftFile, b| {
        (a.created_ms, &a.key, a.slot).cmp(&(b.created_ms, &b.key, b.slot))
    });
    let unopened = drafts.len().saturating_sub(MAX_DRAFTS);
    drafts.drain(..unopened);
    Loaded {
        drafts,
        skipped,
        unopened,
    }
}

/// One file, checked for size before it is read and for a key matching its own name.
fn load_one(path: &Path) -> Result<DraftFile, String> {
    let len = std::fs::metadata(path).map_err(|e| e.to_string())?.len();
    if len > MAX_DRAFT_BYTES as u64 {
        return Err("over 1 MiB".into());
    }
    let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let file: DraftFile = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    if file.version != VERSION {
        return Err(format!("unknown version {}", file.version));
    }
    file.snapshot().ok_or("unreadable request")?;
    let key = file
        .item_key()
        .filter(|k| created_ms(k).is_some())
        .ok_or("not a draft key")?;
    if path.file_name().and_then(|n| n.to_str()) != Some(file_name_slot(&key, file.slot).as_str()) {
        return Err("key does not match the file name".into());
    }
    Ok(file)
}

/// An exclusive lock on `dir/.lock`, creating both. Waits up to half a second for another
/// holder to let go; None when it still holds it then, or when opening or locking fails.
pub fn lock(dir: &Path) -> Option<File> {
    let opened = std::fs::create_dir_all(dir).and_then(|()| {
        OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(dir.join(".lock"))
    });
    let file = match opened {
        Ok(file) => file,
        Err(e) => {
            eprintln!("apinata: drafts: could not open the lock: {e}");
            return None;
        }
    };
    // A child forked before its holder let go keeps the lock until it execs, so poll briefly.
    let deadline = Instant::now() + Duration::from_millis(500);
    loop {
        match file.try_lock() {
            Ok(()) => return Some(file),
            Err(TryLockError::WouldBlock) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(TryLockError::WouldBlock) => return None,
            Err(TryLockError::Error(e)) => {
                eprintln!("apinata: drafts: could not lock: {e}");
                return None;
            }
        }
    }
}

impl DraftFile {
    /// The stored request; None when it does not parse.
    pub fn snapshot(&self) -> Option<DraftSnapshot> {
        DraftSnapshot::from_json(&self.request).ok()
    }

    /// The draft's request name; "Untitled request" when the stored request names none.
    pub fn name(&self) -> String {
        let request: Option<serde_json::Value> = serde_json::from_str(&self.request).ok();
        (request.as_ref())
            .and_then(|r| r.pointer("/info/name")?.as_str())
            .unwrap_or("Untitled request")
            .to_owned()
    }

    /// The history key as an item path; None unless it names a draft.
    pub fn item_key(&self) -> Option<ItemPath> {
        ItemPath::new(&self.key).filter(ItemPath::is_draft)
    }

    /// The folder the draft was made in; the root when the stored string is not a valid path.
    pub fn parent_path(&self) -> ItemPath {
        ItemPath::new(&self.parent).unwrap_or_else(ItemPath::root)
    }
}
