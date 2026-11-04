//! Deleting into a trash and restoring from it, with a fake trash that never touches the
//! platform's.

use crate::tree_support;

use apinata_core::{Trash, TrashError, TrashHandle, TreeError};
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use tempfile::TempDir;
use tree_support::{BROKEN, ROOT, disk_seq, exists, folder, ip, open, request};

/// A trash that moves items into its own temporary directory.
#[derive(Debug)]
struct FakeBin {
    dir: TempDir,
    next: Mutex<u32>,
    available: bool,
}

impl FakeBin {
    fn new() -> FakeBin {
        FakeBin {
            dir: TempDir::new().expect("tempdir"),
            next: Mutex::new(0),
            available: true,
        }
    }
    fn stash(&self, id: &OsString) -> PathBuf {
        self.dir.path().join(id)
    }
}

impl Trash for FakeBin {
    fn available(&self) -> bool {
        self.available
    }
    fn trash(&self, path: &Path) -> Result<TrashHandle, TrashError> {
        if !self.available {
            return Err(TrashError::Unavailable("no trash".into()));
        }
        let mut n = self.next.lock().expect("lock");
        *n += 1;
        let id = OsString::from(format!("item{n}"));
        fs::rename(path, self.stash(&id)).map_err(|e| TrashError::Failed(e.to_string()))?;
        Ok(TrashHandle::new(path.to_path_buf(), id))
    }
    fn restore(&self, handle: &TrashHandle) -> Result<(), TrashError> {
        fs::rename(self.stash(handle.id()), handle.original())
            .map_err(|e| TrashError::Failed(e.to_string()))
    }
}

#[test]
fn a_trashed_request_leaves_the_tree_and_the_disk_and_renumbers_siblings() {
    let (dir, mut c) = open(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", 1)),
        ("b.yml", &request("b", 2)),
        ("c.yml", &request("c", 3)),
    ]);
    let bin = FakeBin::new();
    let trashed = c.trash(&ip("b.yml"), &bin).expect("trashes");
    assert_eq!(trashed.item(), &ip("b.yml"));
    assert!(!exists(&dir, "b.yml") && c.request(&ip("b.yml")).is_none());
    assert_eq!(disk_seq(&dir, "c.yml"), Some(2));
    assert!(bin.stash(&OsString::from("item1")).exists());
}

#[test]
fn restoring_puts_the_files_and_the_sibling_numbers_back() {
    let (dir, mut c) = open(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", 1)),
        ("f/folder.yml", &folder("f", 2)),
        ("f/x.yml", &request("x", 1)),
        ("c.yml", &request("c", 3)),
    ]);
    let bin = FakeBin::new();
    let trashed = c.trash(&ip("f"), &bin).expect("trashes");
    assert!(!exists(&dir, "f") && c.folder(&ip("f")).is_none());
    assert_eq!(disk_seq(&dir, "c.yml"), Some(2));
    c.restore_trashed(&trashed, &bin).expect("restores");
    assert!(exists(&dir, "f/x.yml"));
    assert!(c.request(&ip("f/x.yml")).is_some() && c.folder(&ip("f")).is_some());
    assert_eq!(disk_seq(&dir, "c.yml"), Some(3));
    assert_eq!(c.request(&ip("c.yml")).expect("c").seq(), Some(3));
}

#[test]
fn a_restore_onto_a_taken_name_refuses_and_leaves_both_files() {
    let (dir, mut c) = open(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", 1)),
        ("b.yml", &request("b", 2)),
    ]);
    let bin = FakeBin::new();
    let trashed = c.trash(&ip("b.yml"), &bin).expect("trashes");
    fs::write(dir.path().join("b.yml"), request("newer", 2)).expect("write");
    c.reload();
    let err = c.restore_trashed(&trashed, &bin).expect_err("clashes");
    assert!(matches!(err, TreeError::RestoreClash { .. }), "{err:?}");
    assert_eq!(
        fs::read_to_string(dir.path().join("b.yml")).expect("read"),
        request("newer", 2),
        "the file on disk is never overwritten"
    );
    assert!(
        bin.stash(&OsString::from("item1")).exists(),
        "the trashed copy stays in the trash"
    );
}

#[test]
fn an_unavailable_trash_deletes_nothing() {
    let (dir, mut c) = open(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", 1)),
        ("bad.yml", BROKEN),
    ]);
    let mut bin = FakeBin::new();
    bin.available = false;
    let err = c.trash(&ip("a.yml"), &bin).expect_err("refused");
    assert!(matches!(err, TreeError::Trash { .. }), "{err:?}");
    assert!(exists(&dir, "a.yml") && c.request(&ip("a.yml")).is_some());
}
