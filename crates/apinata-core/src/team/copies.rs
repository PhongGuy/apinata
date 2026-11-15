//! The folder outside the repository that holds the local side of each rebase conflict.

use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

/// One folder of conflict copies, each kept at its path relative to the collection root.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct ConflictCopies {
    dir: PathBuf,
}

impl ConflictCopies {
    /// The copies kept under `dir`, which need not exist yet.
    pub fn new(dir: PathBuf) -> ConflictCopies {
        ConflictCopies { dir }
    }

    /// The folder the copies live in.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Every copy's `/`-separated relative path, sorted; empty when the folder is missing.
    pub fn list(&self) -> io::Result<Vec<String>> {
        let mut out = Vec::new();
        match collect(&self.dir, "", &mut out) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            other => other?,
        }
        out.sort();
        Ok(out)
    }

    /// The bytes of the copy at `rel`.
    pub fn read(&self, rel: &str) -> io::Result<Vec<u8>> {
        fs::read(self.path_of(rel)?)
    }

    /// Delete one copy and any folders it leaves empty; a missing copy is fine.
    pub fn remove(&self, rel: &str) -> io::Result<()> {
        let path = self.path_of(rel)?;
        match fs::remove_file(&path) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e),
            _ => {}
        }
        // Climb toward the copy folder, stopping at the first folder that still holds something.
        let mut dir = path.parent();
        while let Some(d) = dir.filter(|d| *d != self.dir && d.starts_with(&self.dir)) {
            if fs::remove_dir(d).is_err() {
                break;
            }
            dir = d.parent();
        }
        Ok(())
    }

    // A relative path that stays inside the folder: plain names only, so no copy escapes it.
    fn path_of(&self, rel: &str) -> io::Result<PathBuf> {
        let rel = Path::new(rel);
        let plain = rel.components().all(|c| matches!(c, Component::Normal(_)));
        if !plain || rel.as_os_str().is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "a copy's path must be relative and plain",
            ));
        }
        Ok(self.dir.join(rel))
    }
}

/// Every regular file under `dir`, named `prefix` plus its `/`-joined path below `dir`.
fn collect(dir: &Path, prefix: &str, out: &mut Vec<String>) -> io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let rel = format!("{prefix}{name}");
        let kind = entry.file_type()?;
        if kind.is_dir() {
            collect(&entry.path(), &format!("{rel}/"), out)?;
        } else if kind.is_file() {
            out.push(rel);
        }
    }
    Ok(())
}
