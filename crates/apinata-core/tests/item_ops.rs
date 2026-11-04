//! Item operations over raw collection-relative paths, shared by the CLI and the MCP server:
//! path parsing, method checks before a file exists, opaque-item refusals, drop targets and
//! the renumber warnings of delete and trash.

use crate::tree_support;

use apinata_core::{ItemError, NewRequest, Trash, TrashError, TrashHandle, item_ops};
use std::ffi::OsString;
use std::fs;
use std::path::Path;
use std::sync::Mutex;
use tempfile::TempDir;
use tree_support::{ROOT, exists, folder, item, open, request, write};

/// A trash that moves items into its own temporary directory, so tests never touch the
/// platform's.
#[derive(Debug)]
struct FakeBin {
    dir: TempDir,
    next: Mutex<u32>,
}

impl FakeBin {
    fn new() -> FakeBin {
        FakeBin {
            dir: TempDir::new().expect("tempdir"),
            next: Mutex::new(0),
        }
    }
}

impl Trash for FakeBin {
    fn available(&self) -> bool {
        true
    }
    fn trash(&self, path: &Path) -> Result<TrashHandle, TrashError> {
        let mut n = self.next.lock().expect("lock");
        *n += 1;
        let id = OsString::from(format!("item{n}"));
        fs::rename(path, self.dir.path().join(&id))
            .map_err(|e| TrashError::Failed(e.to_string()))?;
        Ok(TrashHandle::new(path.to_path_buf(), id))
    }
    fn restore(&self, _: &TrashHandle) -> Result<(), TrashError> {
        Err(TrashError::Failed("not used".into()))
    }
}

#[test]
fn a_bad_path_is_named_before_anything_is_touched() {
    let (_dir, mut c) = open(&[("opencollection.yml", ROOT)]);
    let err = item_ops::create_folder(&mut c, "../out", "x").unwrap_err();
    assert_eq!(err.to_string(), "invalid item path: ../out");
    let err = item_ops::rename_item(&mut c, "/abs.yml", "x").unwrap_err();
    assert_eq!(err.to_string(), "invalid item path: /abs.yml");
}

#[test]
fn create_request_sets_the_method_and_url_through_the_setters() {
    let (dir, mut c) = open(&[
        ("opencollection.yml", ROOT),
        ("f/folder.yml", &folder("f", 1)),
    ]);
    let new = NewRequest::new()
        .method("m-search")
        .url("https://x.test/find?q=1");
    let path = item_ops::create_request(&mut c, "f", "Find", &new).expect("created");
    assert_eq!(path.to_string(), "f/Find.yml");
    let r = c.request(&path).expect("loaded");
    assert_eq!((r.method(), r.is_dirty()), ("M-SEARCH", false));
    let text = fs::read_to_string(dir.path().join("f/Find.yml")).expect("read");
    assert!(text.contains("https://x.test/find?q=1"), "{text}");
    assert!(text.contains("name: q"), "the query became a param: {text}");
}

#[test]
fn create_request_refuses_a_bad_method_before_creating_a_file() {
    let (dir, mut c) = open(&[("opencollection.yml", ROOT)]);
    let err =
        item_ops::create_request(&mut c, "", "Bad", &NewRequest::new().method("GE T")).unwrap_err();
    assert!(matches!(err, ItemError::Method(_)), "{err:?}");
    assert!(err.to_string().contains("method"), "{err}");
    assert!(!exists(&dir, "Bad.yml"));
}

#[test]
fn create_request_without_method_or_url_keeps_the_preset_bytes() {
    let (dir, mut c) = open(&[("opencollection.yml", ROOT)]);
    let a = item_ops::create_request(&mut c, "", "A", &NewRequest::new()).expect("created");
    let direct = c
        .create_request_from_presets(&apinata_core::ItemPath::root(), "B", None)
        .expect("created");
    let read =
        |p: &apinata_core::ItemPath| fs::read_to_string(p.to_path(dir.path())).expect("read");
    assert_eq!(
        read(&a)
            .replace("name: A", "name: B")
            .replace("seq: 1", "seq: 2"),
        read(&direct)
    );
}

#[test]
fn an_opaque_item_is_refused_a_rename_by_kind() {
    let (dir, mut c) = open(&[
        ("opencollection.yml", ROOT),
        ("q.yml", &item("grpc", "q", 1)),
    ]);
    let err = item_ops::rename_item(&mut c, "q.yml", "Q2").unwrap_err();
    assert_eq!(
        err.to_string(),
        "`q.yml` is a grpc item; it cannot be renamed yet, so move or delete it instead"
    );
    assert!(exists(&dir, "q.yml"));
}

#[test]
fn rename_and_clone_return_the_new_paths() {
    let (dir, mut c) = open(&[("opencollection.yml", ROOT), ("a.yml", &request("a", 1))]);
    let copy = item_ops::clone_request(&mut c, "a.yml", "A copy").expect("cloned");
    assert_eq!(copy.to_string(), "A copy.yml");
    let renamed = item_ops::rename_item(&mut c, "a.yml", "Renamed").expect("renamed");
    assert_eq!(renamed.to_string(), "Renamed.yml");
    assert!(exists(&dir, "Renamed.yml") && !exists(&dir, "a.yml"));
}

#[test]
fn a_move_takes_exactly_one_target() {
    let (dir, mut c) = open(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", 1)),
        ("f/folder.yml", &folder("f", 2)),
    ]);
    let both = item_ops::move_item(&mut c, "a.yml", Some("f"), None, Some("f")).unwrap_err();
    assert_eq!(
        both.to_string(),
        "give exactly one of `inside`, `before` and `after`"
    );
    let none = item_ops::move_item(&mut c, "a.yml", None, None, None).unwrap_err();
    assert_eq!(none.to_string(), both.to_string());
    let moved = item_ops::move_item(&mut c, "a.yml", Some("f"), None, None).expect("moved");
    assert_eq!(moved.to_string(), "f/a.yml");
    assert!(exists(&dir, "f/a.yml"));
    let back = item_ops::move_item(&mut c, "f/a.yml", None, Some("f"), None).expect("moved");
    assert_eq!(back.to_string(), "a.yml");
}

#[test]
fn a_conflict_names_its_file_so_a_caller_can_retry() {
    let (dir, mut c) = open(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", 1)),
        ("b.yml", &request("b", 2)),
    ]);
    write(
        &dir,
        "b.yml",
        &request("b", 2).replace("x.test/b", "x.test/other"),
    );
    let err = item_ops::delete_item(&mut c, "a.yml").unwrap_err();
    assert_eq!(
        err.conflict_path(),
        Some(dir.path().join("b.yml").as_path())
    );
    let other = item_ops::delete_item(&mut c, "nope.yml").unwrap_err();
    assert_eq!(other.conflict_path(), None);
}

#[test]
fn trash_item_moves_the_item_into_the_bin_and_renumbers() {
    let (dir, mut c) = open(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", 1)),
        ("b.yml", &request("b", 2)),
    ]);
    let bin = FakeBin::new();
    let warnings = item_ops::trash_item(&mut c, "a.yml", &bin).expect("trashed");
    assert!(warnings.is_empty(), "{warnings:?}");
    assert!(!exists(&dir, "a.yml"));
    assert!(bin.dir.path().join("item1").is_file());
    let b = fs::read_to_string(dir.path().join("b.yml")).expect("read");
    assert!(b.contains("seq: 1"), "{b}");
}

/// Stage a sibling whose folder.yml cannot be written, run `remove`, and return its warnings;
/// `None` when permissions are not enforced (running as root).
#[cfg(unix)]
fn with_locked_sibling(
    remove: impl FnOnce(&mut apinata_core::Collection) -> Vec<String>,
) -> Option<(TempDir, Vec<String>)> {
    use std::os::unix::fs::PermissionsExt;
    let (dir, mut c) = open(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", 1)),
        ("f/folder.yml", &folder("f", 2)),
    ]);
    let locked = dir.path().join("f");
    let _g = crate::common::ModeGuard::set(&locked, 0o555);
    if fs::write(locked.join("probe"), "").is_ok() {
        fs::remove_file(locked.join("probe")).expect("cleanup");
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).expect("chmod");
        return None;
    }
    let warnings = remove(&mut c);
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).expect("chmod");
    Some((dir, warnings))
}

#[cfg(unix)]
fn assert_one_renumber_warning(dir: &TempDir, warnings: &[String]) {
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    let text = &warnings[0];
    assert!(
        text.starts_with("deleted, but could not renumber the siblings: "),
        "{text}"
    );
    assert!(text.contains("f/folder.yml"), "{text}");
    assert!(!text.contains(&dir.path().display().to_string()), "{text}");
    assert!(!exists(dir, "a.yml"));
}

#[cfg(unix)]
#[test]
fn delete_item_reports_a_failed_renumber_as_a_warning_without_the_root() {
    let Some((dir, warnings)) =
        with_locked_sibling(|c| item_ops::delete_item(c, "a.yml").expect("deleted"))
    else {
        return;
    };
    assert_one_renumber_warning(&dir, &warnings);
}

#[cfg(unix)]
#[test]
fn trash_item_reports_a_failed_renumber_as_the_same_warning() {
    let bin = FakeBin::new();
    let Some((dir, warnings)) =
        with_locked_sibling(|c| item_ops::trash_item(c, "a.yml", &bin).expect("trashed"))
    else {
        return;
    };
    assert_one_renumber_warning(&dir, &warnings);
}
