//! The platform trash on Linux, run in child processes with a scratch data directory so the
//! real trash is never touched.

#![cfg(target_os = "linux")]

use apinata_core::{SystemTrash, Trash, TrashError};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Command;

fn mode(path: &Path) -> u32 {
    fs::metadata(path).expect("metadata").permissions().mode() & 0o777
}

/// Runs one ignored child test of this module with the given data directory and no HOME.
fn run_child(name: &str, data_home: &Path, victim: &Path) {
    let out = Command::new(std::env::current_exe().expect("exe"))
        .args([&format!("system_trash::{name}"), "--exact", "--ignored"])
        .env("XDG_DATA_HOME", data_home)
        .env("VICTIM", victim)
        .env_remove("HOME")
        .output()
        .expect("child");
    assert!(
        out.status.success(),
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
#[ignore = "run by the parent test with a scratch XDG_DATA_HOME"]
fn child_first_use_creates_the_home_trash() {
    let data = std::path::PathBuf::from(std::env::var_os("XDG_DATA_HOME").expect("data home"));
    let work = data.parent().expect("parent").join("work");
    fs::create_dir_all(&work).unwrap();
    let file = work.join("gone.txt");
    fs::write(&file, "x").unwrap();
    assert!(!data.join("Trash").exists());

    let trash = SystemTrash::new();
    assert!(trash.available());
    let handle = trash.trash(&file).expect("trashed");

    assert!(!file.exists());
    let home = data.join("Trash");
    for dir in [&home, &home.join("files"), &home.join("info")] {
        assert_eq!(mode(dir), 0o700, "{}", dir.display());
    }
    let info = fs::read_to_string(home.join("info/gone.txt.trashinfo")).expect("trashinfo");
    let expected = work.canonicalize().unwrap().join("gone.txt");
    assert!(info.starts_with("[Trash Info]\n"), "{info}");
    assert!(
        info.contains(&format!("Path={}", expected.display())),
        "{info}"
    );
    assert!(info.contains("\nDeletionDate="), "{info}");
    assert!(home.join("files/gone.txt").exists());
    assert_eq!(handle.original(), expected.as_path());
}

#[test]
fn a_missing_home_trash_is_created_on_first_use() {
    let scratch = tempfile::tempdir().unwrap();
    run_child(
        "child_first_use_creates_the_home_trash",
        &scratch.path().join("share"),
        scratch.path(),
    );
}

#[test]
#[ignore = "run by the parent test with an uncreatable XDG_DATA_HOME"]
fn child_uncreatable_home_trash_refuses() {
    let file = std::path::PathBuf::from(std::env::var_os("VICTIM").expect("victim"));
    fs::write(&file, "x").unwrap();

    let trash = SystemTrash::new();
    assert!(!trash.available());
    assert!(matches!(
        trash.trash(&file),
        Err(TrashError::Unavailable(_))
    ));
    assert!(file.exists());
}

#[test]
fn a_home_trash_that_cannot_be_created_still_refuses() {
    // A regular file stands where the data directory's parent would be, so no permission
    // trick is needed and the test also holds for root.
    let scratch = tempfile::tempdir().unwrap();
    let blocker = scratch.path().join("blocker");
    fs::write(&blocker, "").unwrap();
    let work = scratch.path().join("work");
    fs::create_dir(&work).unwrap();
    run_child(
        "child_uncreatable_home_trash_refuses",
        &blocker.join("share"),
        &work.join("victim.txt"),
    );
}
