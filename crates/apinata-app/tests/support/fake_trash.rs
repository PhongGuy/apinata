//! A trash for tests: items move into its own temporary directory, so the platform's trash is
//! never touched.
#![allow(dead_code)]

use apinata_core::{Trash, TrashError, TrashHandle};
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use tempfile::TempDir;

#[derive(Debug)]
pub struct FakeTrash {
    dir: TempDir,
    count: Mutex<u32>,
    usable: bool,
}

impl FakeTrash {
    pub fn new() -> FakeTrash {
        FakeTrash {
            dir: tempfile::tempdir().expect("tempdir"),
            count: Mutex::new(0),
            usable: true,
        }
    }

    /// A trash that reports itself unavailable and refuses every item.
    pub fn broken() -> FakeTrash {
        FakeTrash {
            usable: false,
            ..FakeTrash::new()
        }
    }

    /// How many items the trash holds.
    pub fn len(&self) -> usize {
        fs::read_dir(self.dir.path()).expect("read").count()
    }

    /// Where the `n`th trashed item (from 1) sits.
    pub fn stash(&self, n: u32) -> PathBuf {
        self.dir.path().join(format!("item{n}"))
    }
}

impl Trash for FakeTrash {
    fn available(&self) -> bool {
        self.usable
    }

    fn trash(&self, path: &Path) -> Result<TrashHandle, TrashError> {
        if !self.usable {
            return Err(TrashError::Unavailable("no trash here".into()));
        }
        let mut count = self.count.lock().expect("lock");
        *count += 1;
        let id = OsString::from(format!("item{count}"));
        fs::rename(path, self.dir.path().join(&id))
            .map_err(|e| TrashError::Failed(e.to_string()))?;
        Ok(TrashHandle::new(path.to_path_buf(), id))
    }

    fn restore(&self, handle: &TrashHandle) -> Result<(), TrashError> {
        let stash = self.dir.path().join(handle.id());
        if !stash.exists() {
            return Err(TrashError::Missing);
        }
        fs::rename(stash, handle.original()).map_err(|e| TrashError::Failed(e.to_string()))
    }
}
