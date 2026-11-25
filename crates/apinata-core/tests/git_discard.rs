//! `discard` and `undo_discard` against real repositories in a sandbox, with a fake trash that
//! never touches the platform's.

use crate::git_support;

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use apinata_core::{
    DiscardError, FileStatus, GitError, GitState, Trash, TrashError, TrashHandle,
    collection_status_with, discard, repo_lock, undo_discard,
};
use git_support::{Sandbox, write};
use tempfile::TempDir;

/// A trash that moves items into its own temporary directory, or refuses them all.
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
    fn none() -> FakeBin {
        FakeBin {
            available: false,
            ..FakeBin::new()
        }
    }
    /// The bytes of the `n`th trashed item, from 1.
    fn held(&self, n: u32) -> Vec<u8> {
        fs::read(self.dir.path().join(format!("item{n}"))).expect("in the trash")
    }
    fn len(&self) -> usize {
        fs::read_dir(self.dir.path()).expect("read").count()
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
        fs::rename(path, self.dir.path().join(&id))
            .map_err(|e| TrashError::Failed(e.to_string()))?;
        Ok(TrashHandle::new(path.to_path_buf(), id))
    }
    fn restore(&self, handle: &TrashHandle) -> Result<(), TrashError> {
        fs::rename(self.dir.path().join(handle.id()), handle.original())
            .map_err(|e| TrashError::Failed(e.to_string()))
    }
}

/// Repository `r` with `col/{a,b,c}.yml` and `other/o.txt` committed; returns (repo, col).
fn committed(sb: &Sandbox) -> (PathBuf, PathBuf) {
    let repo = sb.repo("r");
    for name in ["a", "b", "c"] {
        write(&repo, &format!("col/{name}.yml"), &format!("{name}\n"));
    }
    write(&repo, "other/o.txt", "o\n");
    sb.commit_all(&repo, "base");
    (repo.clone(), repo.join("col"))
}

fn changes(sb: &Sandbox, col: &Path) -> BTreeMap<String, FileStatus> {
    match collection_status_with(col, &sb.options()).expect("status") {
        GitState::Repo(s) => s
            .files
            .into_iter()
            .map(|(p, f)| (p.to_string_lossy().into_owned(), f))
            .collect(),
        other => panic!("{other:?}"),
    }
}

fn read(col: &Path, rel: &str) -> Vec<u8> {
    fs::read(col.join(rel)).expect("on disk")
}

#[test]
fn a_modified_file_goes_to_the_trash_and_head_comes_back_staged_and_on_disk() {
    let sb = Sandbox::new();
    let (repo, col) = committed(&sb);
    write(&col, "a.yml", "staged\n");
    sb.git(&repo, &["add", "col/a.yml"]);
    write(&col, "a.yml", "mine\n");
    let bin = FakeBin::new();
    let done = discard(&col, &["a.yml"], &bin, &sb.options()).expect("discards");
    assert_eq!(bin.held(1), b"mine\n");
    assert_eq!(read(&col, "a.yml"), b"a\n");
    assert!(
        changes(&sb, &col).is_empty(),
        "index and worktree match HEAD"
    );
    assert!(done.undoable());
}

#[test]
fn a_deleted_file_comes_back_from_head_whether_or_not_the_deletion_was_staged() {
    let sb = Sandbox::new();
    let (repo, col) = committed(&sb);
    fs::remove_file(col.join("a.yml")).unwrap();
    sb.git(&repo, &["rm", "-q", "col/b.yml"]);
    let bin = FakeBin::new();
    discard(&col, &["a.yml"], &bin, &sb.options()).expect("a");
    discard(&col, &["b.yml"], &bin, &sb.options()).expect("b");
    assert_eq!(read(&col, "a.yml"), b"a\n");
    assert_eq!(read(&col, "b.yml"), b"b\n");
    assert_eq!(bin.len(), 0, "nothing was on disk to keep");
    assert!(changes(&sb, &col).is_empty());
}

#[test]
fn an_untracked_file_is_moved_to_the_trash_never_unlinked() {
    let sb = Sandbox::new();
    let (_repo, col) = committed(&sb);
    write(&col, "new.yml", "new\n");
    let bin = FakeBin::new();
    discard(&col, &["new.yml"], &bin, &sb.options()).expect("discards");
    assert!(!col.join("new.yml").exists());
    assert_eq!(bin.held(1), b"new\n");
    assert!(changes(&sb, &col).is_empty());
}

#[test]
fn an_added_file_is_trashed_then_unstaged() {
    let sb = Sandbox::new();
    let (repo, col) = committed(&sb);
    write(&col, "new.yml", "new\n");
    sb.git(&repo, &["add", "col/new.yml"]);
    let bin = FakeBin::new();
    discard(&col, &["new.yml"], &bin, &sb.options()).expect("discards");
    assert!(!col.join("new.yml").exists());
    assert_eq!(bin.held(1), b"new\n");
    assert_eq!(sb.git(&repo, &["ls-files", "col/new.yml"]), "");
    assert!(changes(&sb, &col).is_empty());
}

#[test]
fn a_staged_rename_restores_both_paths() {
    let sb = Sandbox::new();
    let (repo, col) = committed(&sb);
    sb.git(&repo, &["mv", "col/c.yml", "col/c2.yml"]);
    write(&col, "c2.yml", "c edited\n");
    assert_eq!(
        changes(&sb, &col).get("c2.yml"),
        Some(&FileStatus::Renamed {
            from: PathBuf::from("c.yml")
        })
    );
    let bin = FakeBin::new();
    let done = discard(&col, &["c2.yml"], &bin, &sb.options()).expect("discards");
    assert_eq!(read(&col, "c.yml"), b"c\n");
    assert!(!col.join("c2.yml").exists());
    assert_eq!(bin.held(1), b"c edited\n");
    assert!(changes(&sb, &col).is_empty());
    undo_discard(&col, &done, &bin, &sb.options()).expect("undoes");
    assert!(
        !col.join("c.yml").exists(),
        "git's restored copy is removed"
    );
    assert_eq!(read(&col, "c2.yml"), b"c edited\n");
}

// What `git restore --source=HEAD --staged --worktree` does with paths HEAD lacks: an added
// path leaves the index and the disk, and a rename's new path is restored alone.
#[test]
fn git_restore_drops_an_added_path_and_restores_only_the_named_side_of_a_rename() {
    let sb = Sandbox::new();
    let (repo, _col) = committed(&sb);
    write(&repo, "col/new.yml", "new\n");
    sb.git(&repo, &["add", "col/new.yml"]);
    sb.git(&repo, &["mv", "col/c.yml", "col/c2.yml"]);
    let restore = ["restore", "--source=HEAD", "--staged", "--worktree", "--"];
    sb.git(&repo, &[&restore[..], &["col/new.yml"]].concat());
    assert!(!repo.join("col/new.yml").exists());
    assert_eq!(sb.git(&repo, &["ls-files", "col/new.yml"]), "");
    sb.git(&repo, &[&restore[..], &["col/c2.yml"]].concat());
    assert!(!repo.join("col/c2.yml").exists());
    assert!(!repo.join("col/c.yml").exists(), "the old side stays gone");
    assert_eq!(
        sb.git(&repo, &["status", "--porcelain", "--", "col"])
            .trim(),
        "D  col/c.yml"
    );
}

#[test]
fn a_conflicted_file_is_refused_and_left_alone() {
    let sb = Sandbox::new();
    let (repo, col) = committed(&sb);
    sb.git(&repo, &["checkout", "-q", "-b", "side"]);
    write(&col, "a.yml", "side\n");
    sb.commit_all(&repo, "side");
    sb.git(&repo, &["checkout", "-q", "main"]);
    write(&col, "a.yml", "main\n");
    sb.commit_all(&repo, "main");
    sb.git_fails(&repo, &["merge", "-q", "side"]);
    let before = read(&col, "a.yml");
    let bin = FakeBin::new();
    let got = discard(&col, &["a.yml"], &bin, &sb.options());
    assert!(matches!(got, Err(DiscardError::Conflicted(_))), "{got:?}");
    assert_eq!(read(&col, "a.yml"), before);
    assert_eq!(bin.len(), 0);
    assert_eq!(changes(&sb, &col)["a.yml"], FileStatus::Conflicted);
}

#[test]
fn without_a_trash_a_modified_file_is_restored_for_good() {
    let sb = Sandbox::new();
    let (_repo, col) = committed(&sb);
    write(&col, "a.yml", "mine\n");
    let done = discard(&col, &["a.yml"], &FakeBin::none(), &sb.options()).expect("discards");
    assert_eq!(read(&col, "a.yml"), b"a\n");
    assert!(!done.undoable());
}

#[test]
fn without_a_trash_an_untracked_or_added_file_is_refused() {
    let sb = Sandbox::new();
    let (repo, col) = committed(&sb);
    write(&col, "new.yml", "new\n");
    write(&col, "added.yml", "added\n");
    sb.git(&repo, &["add", "col/added.yml"]);
    for rel in ["new.yml", "added.yml"] {
        let got = discard(&col, &[rel], &FakeBin::none(), &sb.options());
        assert!(matches!(got, Err(DiscardError::NeedsTrash(_))), "{got:?}");
    }
    assert_eq!(read(&col, "new.yml"), b"new\n");
    assert_eq!(read(&col, "added.yml"), b"added\n");
    assert_eq!(changes(&sb, &col)["added.yml"], FileStatus::Added);
}

#[test]
fn a_held_repository_lock_leaves_everything_as_it_was() {
    let sb = Sandbox::new();
    let (repo, col) = committed(&sb);
    write(&col, "a.yml", "mine\n");
    let lock = repo_lock(&repo, Duration::ZERO, &sb.options()).expect("free");
    assert!(lock.is_held());
    let bin = FakeBin::new();
    let got = discard(&col, &["a.yml"], &bin, &sb.options());
    assert!(matches!(got, Err(DiscardError::Busy(_))), "{got:?}");
    assert_eq!(read(&col, "a.yml"), b"mine\n");
    assert_eq!(bin.len(), 0);
}

#[test]
fn undo_puts_back_the_exact_bytes() {
    let sb = Sandbox::new();
    let (_repo, col) = committed(&sb);
    let mine: &[u8] = b"line one\r\nline two\r\n\xff\xfe no newline";
    fs::write(col.join("a.yml"), mine).unwrap();
    write(&col, "new.yml", "new\n");
    let bin = FakeBin::new();
    let a = discard(&col, &["a.yml"], &bin, &sb.options()).expect("a");
    let new = discard(&col, &["new.yml"], &bin, &sb.options()).expect("new");
    undo_discard(&col, &a, &bin, &sb.options()).expect("undo a");
    undo_discard(&col, &new, &bin, &sb.options()).expect("undo new");
    assert_eq!(read(&col, "a.yml"), mine);
    assert_eq!(read(&col, "new.yml"), b"new\n");
}

#[test]
fn undo_leaves_a_restored_copy_edited_since() {
    let sb = Sandbox::new();
    let (_repo, col) = committed(&sb);
    write(&col, "a.yml", "mine\n");
    let bin = FakeBin::new();
    let done = discard(&col, &["a.yml"], &bin, &sb.options()).expect("discards");
    write(&col, "a.yml", "edited after\n");
    let got = undo_discard(&col, &done, &bin, &sb.options());
    assert!(matches!(got, Err(DiscardError::Changed(_))), "{got:?}");
    assert_eq!(read(&col, "a.yml"), b"edited after\n");
    assert_eq!(
        bin.held(1),
        b"mine\n",
        "the trashed copy stays in the trash"
    );
}

#[test]
fn undo_waits_for_the_repository_lock() {
    let sb = Sandbox::new();
    let (repo, col) = committed(&sb);
    write(&col, "a.yml", "mine\n");
    let bin = FakeBin::new();
    let done = discard(&col, &["a.yml"], &bin, &sb.options()).expect("discards");
    let lock = repo_lock(&repo, Duration::ZERO, &sb.options()).expect("free");
    let got = undo_discard(&col, &done, &bin, &sb.options());
    assert!(matches!(got, Err(DiscardError::Busy(_))), "{got:?}");
    assert_eq!(read(&col, "a.yml"), b"a\n");
    drop(lock);
}

#[test]
fn a_path_leaving_the_collection_is_refused_and_nothing_is_touched() {
    let sb = Sandbox::new();
    let (repo, col) = committed(&sb);
    write(&repo, "other/o.txt", "changed\n");
    let bin = FakeBin::new();
    let outside = repo.join("other/o.txt");
    for rel in [
        Path::new("../other/o.txt"),
        outside.as_path(),
        Path::new(""),
    ] {
        let got = discard(&col, &[rel], &bin, &sb.options());
        assert!(
            matches!(got, Err(DiscardError::Git(GitError::Malformed(_)))),
            "{rel:?}: {got:?}"
        );
    }
    // A path inside the collection that git lists no change for is refused too.
    let got = discard(&col, &["b.yml"], &bin, &sb.options());
    assert!(matches!(got, Err(DiscardError::Unchanged(_))), "{got:?}");
    assert_eq!(fs::read(&outside).unwrap(), b"changed\n");
    assert_eq!(bin.len(), 0);
}

#[cfg(unix)]
#[test]
fn a_symlink_is_never_followed_out_of_the_collection() {
    let sb = Sandbox::new();
    let (repo, col) = committed(&sb);
    write(&repo, "outside/x.yml", "outside\n");
    std::os::unix::fs::symlink(repo.join("outside"), col.join("link")).unwrap();
    let bin = FakeBin::new();
    // Git sees the link, not what lies behind it, so a path through it is not a change here.
    let got = discard(&col, &["link/x.yml"], &bin, &sb.options());
    assert!(got.is_err(), "{got:?}");
    assert_eq!(bin.len(), 0);
    // Discarding the untracked link trashes the link itself.
    discard(&col, &["link"], &bin, &sb.options()).expect("discards the link");
    assert!(col.join("link").symlink_metadata().is_err());
    assert_eq!(
        fs::read(repo.join("outside/x.yml")).unwrap(),
        b"outside\n",
        "the target is untouched"
    );
}
