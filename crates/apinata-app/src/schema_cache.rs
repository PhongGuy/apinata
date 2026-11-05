//! GraphQL schemas kept in the app's data dir between runs, one file per request URL.
//!
//! A file is named by a SHA-256 of the URL, so no URL or secret appears in a name, and holds the
//! schema text scrubbed of the load's secrets. The files are a cache: an unreadable, oversized or
//! unparsable one counts as missing and is removed.

use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use apinata_core::Schema;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::persist;
use crate::scrub::scrub_text;

/// The largest cache file kept: the introspection answer limit, so any loadable answer fits.
pub const FILE_LIMIT: u64 = apinata_core::SCHEMA_BODY_LIMIT;
/// The most bytes all cache files may hold together; the oldest go first past it.
pub const TOTAL_LIMIT: u64 = 128 * 1024 * 1024;

const FORMAT_VERSION: u32 = 1;

#[derive(Serialize, Deserialize)]
struct CachedSchema {
    version: u32,
    text: String,
}

/// The schema cache directory and its size limits.
#[derive(Debug, Clone, PartialEq)]
pub struct SchemaCache {
    dir: PathBuf,
    file_limit: u64,
    total_limit: u64,
}

impl SchemaCache {
    /// A cache in `dir` with the default limits.
    pub fn new(dir: PathBuf) -> SchemaCache {
        SchemaCache::with_limits(dir, FILE_LIMIT, TOTAL_LIMIT)
    }

    /// A cache in `dir` keeping files of at most `file_limit` bytes, `total_limit` in all.
    pub fn with_limits(dir: PathBuf, file_limit: u64, total_limit: u64) -> SchemaCache {
        SchemaCache {
            dir,
            file_limit,
            total_limit,
        }
    }

    /// The directory the files live in.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// The file name kept for `url`: the hex SHA-256 of the URL, then `.json`.
    pub fn file_name(url: &str) -> String {
        let digest = Sha256::new()
            .chain_update(b"apinata graphql schema\0")
            .chain_update(url.as_bytes())
            .finalize();
        let mut name: String = digest.iter().map(|b| format!("{b:02x}")).collect();
        name.push_str(".json");
        name
    }

    /// The schema kept for `url`; `None` when there is none or its file cannot be used, in
    /// which case the file is removed.
    pub fn load(&self, url: &str) -> Option<Schema> {
        let path = self.dir.join(SchemaCache::file_name(url));
        let size = std::fs::metadata(&path).ok()?.len();
        let schema = (size <= self.file_limit)
            .then(|| std::fs::read(&path).ok())
            .flatten()
            .and_then(|bytes| serde_json::from_slice::<CachedSchema>(&bytes).ok())
            .filter(|kept| kept.version == FORMAT_VERSION)
            .and_then(|kept| Schema::from_text(&kept.text).ok());
        if schema.is_none() {
            let _ = std::fs::remove_file(&path);
        }
        schema
    }

    /// Keeps `text`, a schema answer or file, for `url` with every one of `secrets` scrubbed,
    /// then evicts the oldest files past the total limit. True when kept; a text that no longer
    /// parses once scrubbed, or is over the file limit, is not kept and drops the older copy.
    pub fn store(&self, url: &str, text: &str, secrets: &[String]) -> io::Result<bool> {
        let path = self.dir.join(SchemaCache::file_name(url));
        let text = scrub_text(text, secrets);
        let bytes = match Schema::from_text(&text) {
            Ok(_) => serde_json::to_vec(&CachedSchema {
                version: FORMAT_VERSION,
                text,
            })?,
            Err(_) => Vec::new(),
        };
        if bytes.is_empty() || bytes.len() as u64 > self.file_limit {
            return match std::fs::remove_file(&path) {
                Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
                _ => Ok(false),
            };
        }
        persist::save_bytes(&path, &bytes)?;
        self.evict(&path);
        Ok(true)
    }

    // Removes the oldest cache files, never `kept`, until the rest fit the total limit.
    fn evict(&self, kept: &Path) {
        let Ok(read) = std::fs::read_dir(&self.dir) else {
            return;
        };
        let mut files: Vec<(SystemTime, u64, PathBuf)> = read
            .filter_map(|entry| {
                let entry = entry.ok()?;
                let name = entry.file_name();
                let stem = name.to_str()?.strip_suffix(".json")?;
                if stem.len() != 64 || !stem.bytes().all(|b| b.is_ascii_hexdigit()) {
                    return None;
                }
                let meta = entry.metadata().ok()?;
                let modified = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
                Some((modified, meta.len(), entry.path()))
            })
            .collect();
        let mut total: u64 = files.iter().map(|(_, len, _)| len).sum();
        files.sort();
        for (_, len, path) in files {
            if total <= self.total_limit {
                break;
            }
            if path != kept && std::fs::remove_file(&path).is_ok() {
                total -= len;
            }
        }
    }
}
