//! `Collection::clone_folder` and `item_ops::clone_item`: a folder copied beside itself with
//! everything in it, under a new name.

use crate::tree_support;

use apinata_core::{ItemPath, TreeError, item_ops};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use tree_support::{ROOT, folder, ip, item, open, read, request, yaml};

const USERS: &str = "info:\n  name: users\n  type: folder\n  seq: 1\nrequest:\n  headers:\n    - name: X-A\n      value: \"1\"\n  auth:\n    type: bearer\n    token: t\ndocs: notes\n";

const WITH_EXAMPLE: &str = "info:\n  name: post\n  type: http\n  seq: 2\nhttp:\n  method: POST\n  url: https://x.test/post\nexamples:\n  - name: created\n    response:\n      status: 201\n";

/// The source files below `users`, relative to the collection root.
const SOURCE: &[&str] = &[
    "users/get.yml",
    "users/post.yml",
    "users/app.yml",
    "users/deep/folder.yml",
    "users/deep/x.yml",
    "users/bare/y.yml",
];

fn fixture() -> (tempfile::TempDir, apinata_core::Collection) {
    open(&[
        ("opencollection.yml", ROOT),
        ("users/folder.yml", USERS),
        ("users/get.yml", &request("get", 1)),
        ("users/post.yml", WITH_EXAMPLE),
        ("users/app.yml", &item("app", "app", 3)),
        ("users/deep/folder.yml", &folder("deep", 4)),
        ("users/deep/x.yml", &request("x", 1)),
        ("users/bare/y.yml", &request("y", 1)),
        ("other.yml", &request("other", 2)),
    ])
}

/// Every file under `root` as relative path to bytes.
fn files(root: &Path) -> BTreeMap<String, Vec<u8>> {
    fn walk(base: &Path, dir: &Path, out: &mut BTreeMap<String, Vec<u8>>) {
        for entry in fs::read_dir(dir).expect("read dir") {
            let path = entry.expect("entry").path();
            if path.is_dir() {
                walk(base, &path, out);
            } else {
                let rel = path.strip_prefix(base).expect("below base");
                out.insert(
                    rel.to_string_lossy().into_owned(),
                    fs::read(&path).expect("read"),
                );
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(root, root, &mut out);
    out
}

#[test]
fn the_copy_holds_every_child_unchanged_and_renames_only_its_top_folder() {
    let (dir, mut c) = fixture();
    let before = files(dir.path());
    let made = c.clone_folder(&ip("users"), "  users 2 ").expect("copies");
    assert_eq!(made, ip("users 2"));
    for rel in SOURCE {
        let copy = rel.replacen("users/", "users 2/", 1);
        assert_eq!(yaml(&dir, &copy), yaml(&dir, rel), "{copy}");
    }
    // A subfolder that had no folder.yml gets none.
    assert!(!dir.path().join("users 2/bare/folder.yml").exists());
    let mut top = yaml(&dir, "users/folder.yml");
    top["info"]["name"] = "users 2".into();
    top["info"]["seq"] = 3.into();
    assert_eq!(yaml(&dir, "users 2/folder.yml"), top);
    // The source is byte for byte as it was.
    let after = files(dir.path());
    for (rel, bytes) in &before {
        assert_eq!(after.get(rel), Some(bytes), "{rel}");
    }
    let copy = c.folder(&made).expect("in the tree");
    assert_eq!(copy.name(), "users 2");
    assert!(c.request(&ip("users 2/deep/x.yml")).is_some());
    assert!(c.opaque_item(&ip("users 2/app.yml")).is_some());
}

#[test]
fn a_folder_cloned_twice_to_one_name_numbers_the_second_copys_name() {
    let (dir, mut c) = fixture();
    let first = c.clone_folder(&ip("users"), "users copy").expect("copies");
    let second = c.clone_folder(&ip("users"), "users copy").expect("copies");
    assert_eq!(
        (first.clone(), second.clone()),
        (ip("users copy"), ip("users copy 1"))
    );
    assert_eq!(
        yaml(&dir, "users copy/folder.yml")["info"]["name"],
        "users copy"
    );
    assert_eq!(
        yaml(&dir, "users copy 1/folder.yml")["info"]["name"],
        "users copy 1"
    );
    assert_eq!(c.folder(&first).expect("there").name(), "users copy");
    assert_eq!(c.folder(&second).expect("there").name(), "users copy 1");
}

#[test]
fn a_folder_without_a_folder_file_cloned_twice_numbers_the_second_copys_name() {
    let (dir, mut c) = fixture();
    c.clone_folder(&ip("users/bare"), "bare copy")
        .expect("copies");
    let second = c
        .clone_folder(&ip("users/bare"), "bare copy")
        .expect("copies");
    assert_eq!(second, ip("users/bare copy 1"));
    assert_eq!(
        yaml(&dir, "users/bare copy/folder.yml")["info"]["name"],
        "bare copy"
    );
    assert_eq!(
        yaml(&dir, "users/bare copy 1/folder.yml")["info"]["name"],
        "bare copy 1"
    );
    assert_eq!(c.folder(&second).expect("there").name(), "bare copy 1");
}

#[test]
fn a_request_cloned_twice_to_one_name_numbers_the_second_copys_name() {
    let (dir, mut c) = fixture();
    c.clone_request(&ip("users/get.yml"), "get copy")
        .expect("copies");
    let second = c
        .clone_request(&ip("users/get.yml"), "get copy")
        .expect("copies");
    assert_eq!(second, ip("users/get copy 1.yml"));
    assert_eq!(yaml(&dir, "users/get copy.yml")["info"]["name"], "get copy");
    assert_eq!(
        yaml(&dir, "users/get copy 1.yml")["info"]["name"],
        "get copy 1"
    );
    assert_eq!(c.request(&second).expect("there").name(), "get copy 1");
}

#[test]
fn a_folder_without_a_folder_file_gets_one_in_the_copy() {
    let (dir, mut c) = fixture();
    let made = c.clone_folder(&ip("users/bare"), "bare 2").expect("copies");
    assert_eq!(made, ip("users/bare 2"));
    let top = yaml(&dir, "users/bare 2/folder.yml");
    assert_eq!(top["info"]["name"], "bare 2");
    assert_eq!(top["info"]["type"], "folder");
    assert_eq!(
        yaml(&dir, "users/bare 2/y.yml"),
        yaml(&dir, "users/bare/y.yml")
    );
}

#[test]
fn the_root_an_unknown_path_and_a_blank_name_are_refused() {
    let (dir, mut c) = fixture();
    let before = files(dir.path());
    let root = c.clone_folder(&ItemPath::root(), "x").unwrap_err();
    assert!(matches!(root, TreeError::InvalidMove { .. }), "{root:?}");
    let missing = c.clone_folder(&ip("nope"), "x").unwrap_err();
    assert!(matches!(missing, TreeError::NotFound { .. }), "{missing:?}");
    let request = c.clone_folder(&ip("other.yml"), "x").unwrap_err();
    assert!(matches!(request, TreeError::NotFound { .. }), "{request:?}");
    let blank = c.clone_folder(&ip("users"), "  ").unwrap_err();
    assert!(matches!(blank, TreeError::InvalidName { .. }), "{blank:?}");
    let reserved = c.clone_folder(&ip("users"), "environments").unwrap_err();
    assert!(
        matches!(reserved, TreeError::InvalidName { .. }),
        "{reserved:?}"
    );
    assert_eq!(files(dir.path()), before);
}

#[test]
fn unsaved_edits_are_copied_and_stay_unsaved_in_the_source() {
    let (dir, mut c) = fixture();
    let source_bytes = read(&dir, "users/get.yml");
    c.request_mut(&ip("users/get.yml"))
        .expect("there")
        .set_url("https://x.test/edited");
    c.folder_file_mut(&ip("users"))
        .expect("folder file")
        .set_docs("edited notes");
    c.folder_file_mut(&ip("users/bare"))
        .expect("folder file")
        .set_docs("bare notes");
    c.clone_folder(&ip("users"), "users 2").expect("copies");
    assert_eq!(
        yaml(&dir, "users 2/get.yml")["http"]["url"],
        "https://x.test/edited"
    );
    assert_eq!(
        yaml(&dir, "users 2/folder.yml")["docs"]["content"],
        "edited notes"
    );
    assert_eq!(
        yaml(&dir, "users 2/bare/folder.yml")["docs"]["content"],
        "bare notes"
    );
    assert_eq!(read(&dir, "users/get.yml"), source_bytes);
    assert!(c.request(&ip("users/get.yml")).expect("there").is_dirty());
}

#[test]
fn a_draft_is_neither_copied_nor_copyable() {
    let (dir, mut c) = fixture();
    let draft = c.create_draft(&ip("users")).expect("a draft");
    let made = c.clone_folder(&ip("users"), "users 2").expect("copies");
    let copied: Vec<String> = files(&dir.path().join("users 2")).into_keys().collect();
    let source: Vec<String> = files(&dir.path().join("users")).into_keys().collect();
    assert_eq!(copied, source);
    assert!(c.folder(&made).is_some());
    let err = c.clone_folder(&draft, "x").unwrap_err();
    assert!(matches!(err, TreeError::NotFound { .. }), "{err:?}");
}

/// Nested directories whose deepest file sits just under the platform's path limit, so the
/// same file in a copy with a longer top directory name cannot be written.
fn deep_rel(root_len: usize) -> String {
    const LIMIT: usize = 4095;
    let fixed = root_len + "/users".len() + "/x.yml".len();
    let mut rel = String::from("users");
    let mut left = LIMIT - fixed;
    while left > 0 {
        let take = left.min(201) - 1;
        if take == 0 {
            break;
        }
        rel.push('/');
        rel.push_str(&"d".repeat(take));
        left -= take + 1;
    }
    rel
}

#[test]
fn a_failed_write_leaves_neither_a_half_folder_nor_a_tree_entry() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let deep = deep_rel(dir.path().as_os_str().len());
    let deep_file = format!("{deep}/x.yml");
    assert!(dir.path().join(&deep_file).as_os_str().len() <= 4095);
    for (rel, text) in [
        ("opencollection.yml", ROOT),
        ("users/folder.yml", USERS),
        ("users/get.yml", &request("get", 1)),
        (deep_file.as_str(), &request("x", 1)),
    ] {
        let path = dir.path().join(rel);
        fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        fs::write(path, text).expect("write");
    }
    let mut c = apinata_core::Collection::open(dir.path()).expect("opens");
    let before = files(dir.path());
    let err = c
        .clone_folder(&ip("users"), "users with a longer name")
        .unwrap_err();
    assert!(matches!(err, TreeError::Io { .. }), "{err:?}");
    assert_eq!(files(dir.path()), before);
    assert!(!dir.path().join("users with a longer name").exists());
    assert!(c.folder(&ip("users with a longer name")).is_none());
}

#[test]
fn clone_item_picks_folder_request_or_opaque_item_by_what_is_at_the_path() {
    let (dir, mut c) = fixture();
    let folder = item_ops::clone_item(&mut c, "users", "users 2").expect("folder copied");
    assert_eq!(folder, ip("users 2"));
    assert!(dir.path().join("users 2/deep/x.yml").exists());
    let request = item_ops::clone_item(&mut c, "other.yml", "other 2").expect("request copied");
    assert_eq!(request, ip("other 2.yml"));
    let opaque = item_ops::clone_item(&mut c, "users/app.yml", "app 2").expect("opaque copied");
    assert_eq!(opaque, ip("users/app 2.yml"));
    assert!(c.opaque_item(&opaque).is_some());
    let by_request = item_ops::clone_request(&mut c, "users/app.yml", "app 3").expect("copied");
    assert_eq!(by_request, ip("users/app 3.yml"));
    let folder_only = item_ops::clone_folder(&mut c, "other.yml", "x").unwrap_err();
    assert!(
        matches!(
            folder_only,
            item_ops::ItemError::Tree(TreeError::NotFound { .. })
        ),
        "{folder_only:?}"
    );
}

#[test]
fn an_opaque_item_is_copied_with_only_its_name_and_seq_changed() {
    let (dir, mut c) = fixture();
    let before = files(dir.path());
    let made = c
        .clone_opaque(&ip("users/app.yml"), "  app 2 ")
        .expect("copies");
    assert_eq!(made, ip("users/app 2.yml"));
    let mut expected = yaml(&dir, "users/app.yml");
    expected["info"]["name"] = "app 2".into();
    // Past the four counted siblings and the highest seq, 4.
    expected["info"]["seq"] = 5.into();
    assert_eq!(yaml(&dir, "users/app 2.yml"), expected);
    let copy = c.opaque_item(&made).expect("in the tree");
    assert_eq!(
        (copy.name(), copy.kind(), copy.seq()),
        ("app 2", "app", Some(5))
    );
    let after = files(dir.path());
    for (rel, bytes) in &before {
        assert_eq!(after.get(rel), Some(bytes), "{rel}");
    }
    // A taken file name gets the numeric suffix, and the display name gets it too.
    let again = c
        .clone_opaque(&ip("users/app.yml"), "app 2")
        .expect("copies");
    assert_eq!(again, ip("users/app 2 1.yml"));
    assert!(read(&dir, "users/app 2 1.yml").contains("name: app 2 1\n"));
    assert_eq!(c.opaque_item(&again).expect("there").name(), "app 2 1");
}

#[test]
fn clone_opaque_refuses_anything_but_an_opaque_item() {
    let (_dir, mut c) = fixture();
    for path in ["users/get.yml", "users", "nope.yml"] {
        let err = c.clone_opaque(&ip(path), "x").unwrap_err();
        assert!(matches!(err, TreeError::NotFound { .. }), "{path}: {err:?}");
    }
}
