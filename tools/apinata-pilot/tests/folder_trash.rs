//! The pilot's folder-backed trash: deletes go to a folder inside the launch data dir.

use apinata_core::Trash;
use apinata_pilot::folder_trash::FolderTrash;

fn trash_in(dir: &std::path::Path) -> FolderTrash {
    FolderTrash::new(dir.join("pilot-trash"))
}

#[test]
fn a_file_moves_into_the_folder_and_comes_back() {
    let dir = tempfile::tempdir().unwrap();
    let trash = trash_in(dir.path());
    assert!(trash.available());
    let file = dir.path().join("a.yml");
    std::fs::write(&file, "content").unwrap();

    let handle = trash.trash(&file).unwrap();
    assert_eq!(handle.original(), file);
    assert!(!file.exists());
    assert!(dir.path().join("pilot-trash").join(handle.id()).exists());

    trash.restore(&handle).unwrap();
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "content");
}

#[test]
fn a_directory_moves_whole_and_comes_back() {
    let dir = tempfile::tempdir().unwrap();
    let trash = trash_in(dir.path());
    let folder = dir.path().join("users");
    std::fs::create_dir_all(folder.join("deep")).unwrap();
    std::fs::write(folder.join("deep/x.yml"), "x").unwrap();

    let handle = trash.trash(&folder).unwrap();
    assert!(!folder.exists());
    trash.restore(&handle).unwrap();
    assert_eq!(
        std::fs::read_to_string(folder.join("deep/x.yml")).unwrap(),
        "x"
    );
}

#[test]
fn two_items_with_the_same_name_get_distinct_ids() {
    let dir = tempfile::tempdir().unwrap();
    let trash = trash_in(dir.path());
    let file = dir.path().join("a.yml");
    std::fs::write(&file, "one").unwrap();
    let first = trash.trash(&file).unwrap();
    std::fs::write(&file, "two").unwrap();
    let second = trash.trash(&file).unwrap();
    assert_ne!(first.id(), second.id());
    trash.restore(&second).unwrap();
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "two");
}

#[test]
fn restore_refuses_to_overwrite() {
    let dir = tempfile::tempdir().unwrap();
    let trash = trash_in(dir.path());
    let file = dir.path().join("a.yml");
    std::fs::write(&file, "old").unwrap();
    let handle = trash.trash(&file).unwrap();
    std::fs::write(&file, "new").unwrap();

    assert!(trash.restore(&handle).is_err());
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "new");
}

#[test]
fn a_handle_for_an_unknown_id_is_missing() {
    let dir = tempfile::tempdir().unwrap();
    let trash = trash_in(dir.path());
    let handle = apinata_core::TrashHandle::new(dir.path().join("a.yml"), "nope".into());
    let result = trash.restore(&handle);
    assert!(
        matches!(result, Err(apinata_core::TrashError::Missing)),
        "{result:?}"
    );
    assert!(!dir.path().join("a.yml").exists());
}

#[test]
fn a_second_trash_on_the_same_root_never_reuses_an_id() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("a.yml");
    std::fs::write(&file, "first").unwrap();
    let first = trash_in(dir.path()).trash(&file).unwrap();
    // A relaunch on the same data dir builds a fresh trash over the same folder.
    let relaunched = trash_in(dir.path());
    std::fs::write(&file, "second").unwrap();
    let second = relaunched.trash(&file).unwrap();
    assert_ne!(first.id(), second.id());

    relaunched.restore(&second).unwrap();
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "second");
    std::fs::remove_file(&file).unwrap();
    relaunched.restore(&first).unwrap();
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "first");
}

#[test]
fn an_emptied_trash_reports_the_item_missing() {
    let dir = tempfile::tempdir().unwrap();
    let trash = trash_in(dir.path());
    let file = dir.path().join("a.yml");
    std::fs::write(&file, "content").unwrap();
    let handle = trash.trash(&file).unwrap();
    std::fs::remove_dir_all(dir.path().join("pilot-trash")).unwrap();

    let err = trash.restore(&handle).unwrap_err();
    assert!(matches!(err, apinata_core::TrashError::Missing), "{err:?}");
    assert!(!file.exists());
}
