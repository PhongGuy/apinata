//! `Collection::create` and `Collection::unique_dir_name`, Bruno's import naming and root file.

use apinata_core::{Collection, CreateError};
use std::fs;
use tempfile::TempDir;

/// Bruno's import root file for a collection with no docs, auth or variables; the bytes of
/// the import-parity corpus's `postman-with-scripts/opencollection.yml` with the name changed.
const BARE: &str = "opencollection: 1.0.0\n\ninfo:\n  name: \"Edge: cases/1\"\nbundled: false\nextensions:\n  bruno:\n    ignore:\n      - node_modules\n      - .git\n";

#[test]
fn create_writes_brunos_import_root_file_and_opens() {
    let dir = TempDir::new().expect("tempdir");
    let root = dir.path().join("Edge- cases-1");
    let collection = Collection::create(&root, "Edge: cases/1").expect("creates");
    assert_eq!(
        fs::read_to_string(root.join("opencollection.yml")).expect("root file"),
        BARE
    );
    assert_eq!(collection.name(), "Edge: cases/1");
    assert_eq!(collection.root(), root);
    assert!(collection.tree().requests().is_empty());
}

#[test]
fn create_refuses_an_existing_directory_or_an_unusable_name() {
    let dir = TempDir::new().expect("tempdir");
    let root = dir.path().join("taken");
    fs::create_dir(&root).expect("mkdir");
    fs::write(root.join("keep.txt"), "x").expect("write");
    assert!(matches!(
        Collection::create(&root, "taken"),
        Err(CreateError::NotEmpty { .. })
    ));
    assert_eq!(
        fs::read_to_string(root.join("keep.txt")).expect("kept"),
        "x"
    );
    assert!(!root.join("opencollection.yml").exists());
    // 100 characters but 300 UTF-8 bytes, over the file system's 255-byte limit.
    let long = "名".repeat(100);
    for name in ["", "  ", "CON", "...", long.as_str()] {
        let result = Collection::create(&dir.path().join("x"), name);
        assert!(
            matches!(result, Err(CreateError::InvalidName { .. })),
            "{name:?}"
        );
    }
    assert!(!dir.path().join("x").exists());
}

#[test]
fn unique_dir_name_follows_find_unique_folder_name() {
    let dir = TempDir::new().expect("tempdir");
    assert_eq!(Collection::unique_dir_name(dir.path(), "API"), "API");
    fs::create_dir(dir.path().join("API")).expect("mkdir");
    assert_eq!(Collection::unique_dir_name(dir.path(), "API"), "API - 1");
    fs::create_dir(dir.path().join("API - 1")).expect("mkdir");
    assert_eq!(Collection::unique_dir_name(dir.path(), "API"), "API - 2");
    // The check is on the sanitised name; the result keeps the original characters.
    fs::create_dir(dir.path().join("a-b")).expect("mkdir");
    assert_eq!(Collection::unique_dir_name(dir.path(), "a/b"), "a/b - 1");
    fs::write(dir.path().join("f"), "").expect("write");
    assert_eq!(
        Collection::unique_dir_name(dir.path(), "f"),
        "f - 1",
        "a file takes the name too"
    );
}

#[test]
fn create_fills_an_existing_empty_directory() {
    let dir = TempDir::new().expect("tempdir");
    let root = dir.path().join("Empty");
    fs::create_dir(&root).expect("mkdir");
    let collection = Collection::create(&root, "Empty").expect("creates in place");
    assert_eq!(collection.root(), root);
    assert_eq!(collection.name(), "Empty");
    assert!(root.join("opencollection.yml").is_file());
}

#[test]
fn create_takes_over_a_directory_holding_only_ignored_dotfiles() {
    let dir = TempDir::new().expect("tempdir");
    let root = dir.path().join("repo");
    fs::create_dir_all(root.join(".git")).expect("mkdir");
    fs::write(root.join(".gitignore"), "target\n").expect("write");
    Collection::create(&root, "repo").expect("a fresh repository is empty enough");
    assert!(root.join("opencollection.yml").is_file());
    assert!(root.join(".git").is_dir());
    assert_eq!(
        fs::read_to_string(root.join(".gitignore")).unwrap(),
        "target\n"
    );
}

#[test]
fn create_refuses_a_directory_with_another_hidden_entry_or_a_file() {
    let dir = TempDir::new().expect("tempdir");
    let root = dir.path().join("repo");
    fs::create_dir_all(root.join(".git")).expect("mkdir");
    fs::write(root.join(".secret"), "x").expect("write");
    // The folder existing is fine; the refusal names what is wrong, that it is not empty.
    let err = Collection::create(&root, "repo").expect_err("refused");
    assert!(matches!(err, CreateError::NotEmpty { .. }), "{err:?}");
    assert_eq!(err.to_string(), format!("{} is not empty", root.display()));
    assert!(root.join(".git").is_dir());
    assert!(!root.join("opencollection.yml").exists());
    let file = dir.path().join("plain");
    fs::write(&file, "x").expect("write");
    assert!(matches!(
        Collection::create(&file, "plain"),
        Err(CreateError::Exists { .. })
    ));
    assert_eq!(fs::read_to_string(&file).expect("kept"), "x");
}

// A read-only empty folder makes writing the root file fail; the folder was not ours to remove.
#[cfg(unix)]
#[test]
fn a_failed_create_keeps_an_existing_folder() {
    use std::os::unix::fs::PermissionsExt;
    let dir = TempDir::new().expect("tempdir");
    let root = dir.path().join("locked");
    fs::create_dir(&root).expect("mkdir");
    let _g = crate::common::ModeGuard::set(&root, 0o555);
    let result = Collection::create(&root, "locked");
    fs::set_permissions(&root, fs::Permissions::from_mode(0o755)).expect("chmod back");
    assert!(matches!(result, Err(CreateError::Io { .. })), "{result:?}");
    assert!(root.is_dir(), "the user's folder stays");
    assert_eq!(fs::read_dir(&root).expect("read").count(), 0);
}

#[test]
fn collection_name_reads_info_name_without_a_full_open() {
    let dir = TempDir::new().expect("tempdir");
    fs::write(
        dir.path().join("opencollection.yml"),
        "opencollection: 1.0.0\ninfo:\n  name: Pretty Name\n",
    )
    .expect("write");
    assert_eq!(
        apinata_core::collection_name(dir.path()).as_deref(),
        Some("Pretty Name")
    );
}

#[test]
fn collection_name_is_none_when_missing_invalid_or_unnamed() {
    let dir = TempDir::new().expect("tempdir");
    assert_eq!(apinata_core::collection_name(dir.path()), None);
    fs::write(dir.path().join("opencollection.yml"), "a: [unclosed\n").expect("write");
    assert_eq!(apinata_core::collection_name(dir.path()), None);
    fs::write(
        dir.path().join("opencollection.yml"),
        "opencollection: 1.0.0\n",
    )
    .expect("write");
    assert_eq!(apinata_core::collection_name(dir.path()), None);
}

#[test]
fn collection_name_is_none_for_an_empty_or_blank_name() {
    let dir = TempDir::new().expect("tempdir");
    for name in ["\"\"", "\"   \""] {
        fs::write(
            dir.path().join("opencollection.yml"),
            format!("opencollection: 1.0.0\ninfo:\n  name: {name}\n"),
        )
        .expect("write");
        assert_eq!(apinata_core::collection_name(dir.path()), None, "{name}");
    }
}
