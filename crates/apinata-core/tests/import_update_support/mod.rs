//! Shared helpers for the spec-update tests.
#![allow(dead_code)] // each test module uses a different subset

use apinata_core::import::{
    CollectionFormat, CollectionOptions, ImportBundle, parse_collections, write_collection,
};
use apinata_core::{Collection, ItemPath, MemoryStore, Trash, TrashError, TrashHandle};
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use tempfile::TempDir;

pub fn spec(name: &str) -> Vec<u8> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/openapi")
        .join(name);
    fs::read(path).expect("fixture")
}

pub fn bundle(bytes: &[u8]) -> ImportBundle {
    parse_collections(
        Some(CollectionFormat::OpenApi),
        bytes,
        &CollectionOptions::default(),
    )
    .expect("parses")
    .remove(0)
}

/// Import a fixture spec into `parent/Shop` and return the opened collection.
pub fn imported(parent: &Path, fixture: &str) -> Collection {
    let b = bundle(&spec(fixture));
    write_collection(parent, &b.collection, &b.environments, &MemoryStore::new())
        .expect("writes")
        .0
}

pub fn ip(rel: &str) -> ItemPath {
    ItemPath::new(rel).expect("item path")
}

pub fn paths(items: &[ItemPath]) -> Vec<&str> {
    items.iter().map(ItemPath::as_str).collect()
}

/// Every file under `root`: relative path to (bytes, modified time).
pub fn snapshot(root: &Path) -> BTreeMap<String, (Vec<u8>, std::time::SystemTime)> {
    let mut out = BTreeMap::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir).expect("dir") {
            let path = entry.expect("entry").path();
            if path.is_dir() {
                stack.push(path);
            } else {
                let rel = path
                    .strip_prefix(root)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/");
                let modified = fs::metadata(&path).unwrap().modified().unwrap();
                out.insert(rel, (fs::read(&path).unwrap(), modified));
            }
        }
    }
    out
}

/// A trash that moves items into its own temporary directory.
#[derive(Debug)]
pub struct FakeBin {
    dir: TempDir,
    next: Mutex<u32>,
    pub usable: bool,
}

impl FakeBin {
    pub fn new(usable: bool) -> FakeBin {
        FakeBin {
            dir: TempDir::new().unwrap(),
            next: Mutex::new(0),
            usable,
        }
    }
    pub fn taken(&self) -> usize {
        fs::read_dir(self.dir.path()).unwrap().count()
    }
}

impl Trash for FakeBin {
    fn available(&self) -> bool {
        self.usable
    }
    fn trash(&self, path: &Path) -> Result<TrashHandle, TrashError> {
        if !self.usable {
            return Err(TrashError::Unavailable("no trash".into()));
        }
        let mut n = self.next.lock().unwrap();
        *n += 1;
        let id = OsString::from(format!("item{n}"));
        fs::rename(path, self.dir.path().join(&id))
            .map_err(|e| TrashError::Failed(e.to_string()))?;
        Ok(TrashHandle::new(PathBuf::from(path), id))
    }
    fn restore(&self, handle: &TrashHandle) -> Result<(), TrashError> {
        fs::rename(self.dir.path().join(handle.id()), handle.original())
            .map_err(|e| TrashError::Failed(e.to_string()))
    }
}
