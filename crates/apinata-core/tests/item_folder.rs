//! `item_ops::update_folder` and `read_folder`: the folder and collection edits every shell
//! shares. Path and folder refusals, root versus folder.yml, skip-if-same, merge by name and
//! the docs guard.

use crate::tree_support;

use apinata_core::item_ops::{self, FolderPatch};
use apinata_core::{AuthSpec, KeyValue, VarValue, Variable};
use std::fs;
use tree_support::{BROKEN, ROOT, folder, open, request};

const USERS: &str = "info:\n  name: users\n  type: folder\n  seq: 1\nrequest:\n  headers:\n    - name: X-A\n      value: \"1\"\n      description: kept\n  variables:\n    - name: n\n      value:\n        type: number\n        data: \"1\"\n  auth:\n    type: bearer\n    token: t\ndocs: notes\n";

fn fixture() -> (tempfile::TempDir, apinata_core::Collection) {
    open(&[
        ("opencollection.yml", ROOT),
        ("users/folder.yml", USERS),
        ("users/get.yml", &request("get", 1)),
        ("plain/a.yml", &request("a", 1)),
    ])
}

fn header(name: &str, value: &str) -> KeyValue {
    KeyValue::new(name, value)
}

#[test]
fn root_headers_are_written_to_the_collection_file() {
    let (dir, mut c) = fixture();
    let patch = FolderPatch::new().headers(vec![header("X-Root", "r")]);
    let done = item_ops::update_folder(&mut c, "", &patch).expect("updates");
    assert!(done.changed());
    assert_eq!(done.file(), dir.path().join("opencollection.yml"));
    let text = fs::read_to_string(dir.path().join("opencollection.yml")).expect("read");
    assert!(
        text.contains("name: X-Root") && text.contains("value: r"),
        "{text}"
    );
}

#[test]
fn a_folder_without_folder_yml_gets_one_on_first_save() {
    let (dir, mut c) = fixture();
    let patch = FolderPatch::new()
        .auth(AuthSpec::Inherit)
        .docs("Plain notes");
    let done = item_ops::update_folder(&mut c, "plain", &patch).expect("updates");
    assert!(done.changed());
    assert_eq!(done.file(), dir.path().join("plain/folder.yml"));
    let text = fs::read_to_string(dir.path().join("plain/folder.yml")).expect("read");
    assert!(
        text.contains("name: plain") && text.contains("Plain notes"),
        "{text}"
    );
}

#[test]
fn a_patch_equal_to_the_file_writes_nothing() {
    let (dir, mut c) = fixture();
    let current = item_ops::read_folder(&c, "users").expect("reads");
    let patch = FolderPatch::new()
        .headers(current.headers.clone())
        .vars(current.vars.clone())
        .auth(current.auth.clone())
        .docs(current.docs.clone());
    let done = item_ops::update_folder(&mut c, "users", &patch).expect("updates");
    assert!(!done.changed());
    assert_eq!(
        fs::read_to_string(dir.path().join("users/folder.yml")).expect("read"),
        USERS
    );
    let done = item_ops::update_folder(&mut c, "plain", &FolderPatch::new().docs("")).expect("ok");
    assert!(!done.changed());
    assert!(!dir.path().join("plain/folder.yml").exists());
}

#[test]
fn a_header_named_again_keeps_what_the_file_remembers_about_it() {
    let (dir, mut c) = fixture();
    let mut off = header("X-A", "2");
    off.enabled = false;
    let patch = FolderPatch::new().headers(vec![off, header("X-B", "b")]);
    item_ops::update_folder(&mut c, "users", &patch).expect("updates");
    let read = item_ops::read_folder(&c, "users").expect("reads");
    let summary: Vec<_> = read
        .headers
        .iter()
        .map(|h| {
            (
                h.name.as_str(),
                h.value.as_str(),
                h.enabled,
                h.description.as_deref(),
            )
        })
        .collect();
    assert_eq!(
        summary,
        [("X-A", "2", false, Some("kept")), ("X-B", "b", true, None)]
    );
    let text = fs::read_to_string(dir.path().join("users/folder.yml")).expect("read");
    assert!(text.contains("description: kept"), "{text}");
}

#[test]
fn typed_variables_are_written_with_their_type() {
    let (_dir, mut c) = fixture();
    let typed = Variable::new(
        "n",
        VarValue::Typed {
            kind: "number".into(),
            data: "5".into(),
        },
    );
    let patch =
        FolderPatch::new().vars(vec![typed, Variable::new("h", VarValue::Text("x".into()))]);
    item_ops::update_folder(&mut c, "users", &patch).expect("updates");
    let read = item_ops::read_folder(&c, "users").expect("reads");
    assert!(
        read.vars[0].value
            == VarValue::Typed {
                kind: "number".into(),
                data: "5".into()
            }
    );
    assert_eq!(read.vars[1].name, "h");
}

#[test]
fn a_bad_path_a_missing_folder_and_a_broken_folder_yml_are_refused() {
    let (dir, mut c) = open(&[("opencollection.yml", ROOT), ("bad/folder.yml", BROKEN)]);
    let patch = FolderPatch::new().docs("x");
    let err = item_ops::update_folder(&mut c, "../out", &patch).unwrap_err();
    assert_eq!(err.to_string(), "invalid item path: ../out");
    let err = item_ops::update_folder(&mut c, "nope", &patch).unwrap_err();
    assert_eq!(err.to_string(), "no folder at nope");
    let err = item_ops::update_folder(&mut c, "bad", &patch).unwrap_err();
    assert!(err.to_string().contains("folder.yml"), "{err}");
    assert!(item_ops::read_folder(&c, "bad").is_err());
    assert_eq!(
        fs::read_to_string(dir.path().join("bad/folder.yml")).expect("read"),
        BROKEN
    );
}

#[test]
fn a_refused_edit_names_the_folder_and_leaves_no_draft() {
    let (dir, mut c) = fixture();
    let patch = FolderPatch::new()
        .docs("changed")
        .headers(vec![header("Bad\nName", "1")]);
    let err = item_ops::update_folder(&mut c, "users", &patch).unwrap_err();
    assert!(err.to_string().starts_with("users: "), "{err}");
    let err = item_ops::update_folder(&mut c, "", &patch).unwrap_err();
    assert!(!err.to_string().starts_with(": "), "{err}");
    item_ops::update_folder(&mut c, "users", &FolderPatch::new().auth(AuthSpec::None))
        .expect("updates");
    let text = fs::read_to_string(dir.path().join("users/folder.yml")).expect("read");
    assert!(!text.contains("changed"), "{text}");
}

#[test]
fn a_file_changed_on_disk_is_a_conflict_with_its_path() {
    let (dir, mut c) = fixture();
    let file = dir.path().join("users/folder.yml");
    fs::write(&file, folder("users", 3)).expect("other writer");
    let err = item_ops::update_folder(&mut c, "users", &FolderPatch::new().docs("x")).unwrap_err();
    assert_eq!(err.conflict_path(), Some(file.as_path()));
    assert_eq!(fs::read_to_string(&file).expect("read"), folder("users", 3));
}

#[test]
fn read_folder_reads_the_collection_and_an_empty_folder() {
    let (_dir, c) = fixture();
    let root = item_ops::read_folder(&c, "").expect("reads");
    assert!(root.headers.is_empty() && root.docs.is_empty());
    let plain = item_ops::read_folder(&c, "plain").expect("reads");
    assert!(plain.vars.is_empty());
    let users = item_ops::read_folder(&c, "users").expect("reads");
    assert_eq!(users.docs, "notes");
    assert!(users.auth == AuthSpec::Bearer { token: "t".into() });
}

#[test]
fn folder_sections_debug_shows_names_but_hides_the_token() {
    let (_dir, c) = fixture();
    let sections = item_ops::read_folder(&c, "users").expect("reads");
    let text = format!("{sections:?}");
    assert!(text.contains("X-A"), "{text}");
    assert!(!text.contains("token: \"t\""), "{text}");
}
