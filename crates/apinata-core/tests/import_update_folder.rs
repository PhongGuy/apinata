//! Folder-scoped spec baselines kept beside the root one.

use crate::import_update_support::{FakeBin, bundle, imported, ip, paths, snapshot, spec};
use apinata_core::Collection;
use apinata_core::ItemPath;
use apinata_core::Trash;
use apinata_core::import::{
    ChangedField, FieldHashes, ImportedItem, SpecBaseline, UpdateOptions, add_request,
    apply_update, plan_update, record_spec_baseline,
};
use tempfile::TempDir;

fn one(key: &str, hash: &str) -> SpecBaseline {
    let mut b = SpecBaseline::default();
    b.insert(
        key.to_owned(),
        FieldHashes::new([(ChangedField::Target, hash.to_owned())]),
    );
    b
}

fn root_text(c: &Collection) -> String {
    std::fs::read_to_string(c.root().join("opencollection.yml")).unwrap()
}

#[test]
fn a_folder_baseline_round_trips_through_disk() {
    let dir = TempDir::new().unwrap();
    let mut c = Collection::create(&dir.path().join("c"), "c").unwrap();
    let b = one("GET /a", "aaaaaaaaaaaaaaaa");
    c.file_mut().set_spec_baseline_in(&ip("a/b"), &b);
    c.file_mut().save().unwrap();
    let again = Collection::open(c.root()).unwrap();
    assert_eq!(again.file().spec_baseline_in(&ip("a/b")), Some(b));
    assert!(again.file().spec_baseline_in(&ip("a")).is_none());
}

#[test]
fn root_and_folders_hold_separate_maps() {
    let dir = TempDir::new().unwrap();
    let mut c = Collection::create(&dir.path().join("c"), "c").unwrap();
    let (r, a, b) = (
        one("GET /r", "rrrrrrrrrrrrrrrr"),
        one("GET /a", "aaaaaaaaaaaaaaaa"),
        one("GET /b", "bbbbbbbbbbbbbbbb"),
    );
    c.file_mut().set_spec_baseline_in(&ip("fa"), &a);
    c.file_mut().set_spec_baseline_in(&ip("fb"), &b);
    c.file_mut()
        .set_spec_baseline_in(&apinata_core::ItemPath::root(), &r);
    c.file_mut().save().unwrap();
    let f = Collection::open(c.root()).unwrap();
    assert_eq!(f.file().spec_baseline(), Some(r));
    assert_eq!(f.file().spec_baseline_in(&ip("fa")), Some(a));
    assert_eq!(f.file().spec_baseline_in(&ip("fb")), Some(b));
    // `ops` stays ahead of `folders` however the writes were ordered.
    let text = root_text(&f);
    assert!(
        text.find("    ops:").unwrap() < text.find("    folders:").unwrap(),
        "{text}"
    );
}

#[test]
fn clearing_one_folder_leaves_the_others_and_the_root() {
    let dir = TempDir::new().unwrap();
    let mut c = imported(dir.path(), "update-v1.json");
    let root_before = c.file().spec_baseline();
    c.file_mut()
        .set_spec_baseline_in(&ip("fa"), &one("GET /a", "aaaaaaaaaaaaaaaa"));
    c.file_mut()
        .set_spec_baseline_in(&ip("fb"), &one("GET /b", "bbbbbbbbbbbbbbbb"));
    c.file_mut().clear_spec_baseline_in(&ip("fa"));
    c.file_mut().save().unwrap();
    let f = Collection::open(c.root()).unwrap();
    assert!(f.file().spec_baseline_in(&ip("fa")).is_none());
    assert!(f.file().spec_baseline_in(&ip("fb")).is_some());
    assert_eq!(f.file().spec_baseline(), root_before);
}

#[test]
fn clearing_the_last_folder_removes_the_empty_parents() {
    let dir = TempDir::new().unwrap();
    let mut c = Collection::create(&dir.path().join("c"), "c").unwrap();
    c.file_mut()
        .set_spec_baseline_in(&ip("fa"), &one("GET /a", "aaaaaaaaaaaaaaaa"));
    c.file_mut().clear_spec_baseline_in(&ip("fa"));
    c.file_mut().save().unwrap();
    let text = root_text(&c);
    assert!(
        !text.contains("folders:") && !text.contains("openapi:"),
        "{text}"
    );
}

#[test]
fn clearing_the_root_keeps_the_folder_entries() {
    let dir = TempDir::new().unwrap();
    let mut c = imported(dir.path(), "update-v1.json");
    c.file_mut()
        .set_spec_baseline_in(&ip("fa"), &one("GET /a", "aaaaaaaaaaaaaaaa"));
    c.file_mut().clear_spec_baseline();
    c.file_mut().save().unwrap();
    let f = Collection::open(c.root()).unwrap();
    assert!(f.file().spec_baseline().is_none());
    assert!(f.file().spec_baseline_in(&ip("fa")).is_some());
}

#[test]
fn adding_a_folder_entry_leaves_the_root_ops_bytes_alone() {
    let dir = TempDir::new().unwrap();
    let mut c = imported(dir.path(), "update-v1.json");
    let before = root_text(&c);
    c.file_mut()
        .set_spec_baseline_in(&ip("fa"), &one("GET /a", "aaaaaaaaaaaaaaaa"));
    c.file_mut().save().unwrap();
    let after = root_text(&c);
    // The root block is the tail of the file, so it must reappear unchanged.
    assert!(
        after.contains(&before[before.find("    ops:").unwrap()..]),
        "{after}"
    );
    assert!(after.contains("folders:") && after.contains("fa:"));
}

#[test]
fn only_folder_entries_means_no_root_baseline() {
    let dir = TempDir::new().unwrap();
    let mut c = Collection::create(&dir.path().join("c"), "c").unwrap();
    c.file_mut()
        .set_spec_baseline_in(&ip("fa"), &one("GET /a", "aaaaaaaaaaaaaaaa"));
    c.file_mut().save().unwrap();
    assert!(
        Collection::open(c.root())
            .unwrap()
            .file()
            .spec_baseline()
            .is_none()
    );
}

#[test]
fn record_spec_baseline_records_the_given_requests_under_the_folder() {
    let dir = TempDir::new().unwrap();
    let mut c = imported(dir.path(), "update-v1.json");
    record_spec_baseline(&mut c, &ip("people"), &[ip("people/getUser.yml")]).unwrap();
    let f = Collection::open(c.root()).unwrap();
    let b = f.file().spec_baseline_in(&ip("people")).expect("folder");
    assert_eq!(b.len(), 1);
    assert_eq!(b.file("GET /users/{}"), Some("people/getUser.yml"));
    assert_eq!(f.file().spec_baseline().unwrap().len(), 3);
}

fn v2() -> apinata_core::import::ImportBundle {
    bundle(&spec("update-v2.json"))
}

fn opts(folder: &str) -> UpdateOptions {
    let mut o = UpdateOptions::default();
    o.folder = Some(ip(folder));
    o
}

/// Place the spec's items under a new folder `name` and record that folder's baseline.
fn place(c: &mut Collection, name: &str, fixture: &str) {
    place_in(c, &ItemPath::root(), name, fixture);
}

fn place_in(c: &mut Collection, parent: &ItemPath, name: &str, fixture: &str) {
    fn walk(
        c: &mut Collection,
        parent: &ItemPath,
        items: &[ImportedItem],
        out: &mut Vec<ItemPath>,
    ) {
        for item in items {
            match item {
                ImportedItem::Request(r) => {
                    out.push(add_request(c, parent, &r.name, &r.request).unwrap());
                }
                ImportedItem::Folder(f) => {
                    let sub = c.create_folder(parent, &f.name).unwrap();
                    walk(c, &sub, &f.items, out);
                }
                _ => {}
            }
        }
    }
    let folder = c.create_folder(parent, name).unwrap();
    let mut placed = Vec::new();
    walk(
        c,
        &folder,
        &bundle(&spec(fixture)).collection.items,
        &mut placed,
    );
    record_spec_baseline(c, &folder, &placed).unwrap();
}

fn two_folders(dir: &std::path::Path) -> Collection {
    let mut c = Collection::create(&dir.join("c"), "c").unwrap();
    place(&mut c, "a", "update-v1.json");
    place(&mut c, "b", "update-v1.json");
    c
}

#[test]
fn a_folder_update_refreshes_keeps_edits_and_adds_inside_the_folder() {
    let dir = TempDir::new().unwrap();
    let mut c = two_folders(dir.path());
    let f = c.request_mut(&ip("a/people/getUser.yml")).unwrap();
    f.set_url("{{baseUrl}}/users/:id?mine=1");
    f.save().unwrap();
    let s = apply_update(&mut c, &v2(), &opts("a"), None).unwrap();
    assert_eq!(paths(&s.edited), ["a/people/getUser.yml"]);
    assert_eq!(paths(&s.added), ["a/people/listUsers.yml"]);
    let r = Collection::open(c.root()).unwrap();
    let get = r.request(&ip("a/people/getUser.yml")).unwrap();
    assert!(get.url().contains("mine=1") && !get.url().contains("include"));
    assert_eq!(get.docs(), "Fetch a user");
    assert!(r.request(&ip("a/people/listUsers.yml")).is_some());
}

#[test]
fn two_folders_with_the_same_operation_key_do_not_collide() {
    let dir = TempDir::new().unwrap();
    let mut c = two_folders(dir.path());
    let before_b = snapshot(&c.root().join("b"));
    apply_update(&mut c, &v2(), &opts("a"), None).unwrap();
    let after_b = snapshot(&c.root().join("b"));
    assert_eq!(before_b, after_b, "b is untouched by an update of a");
    let b = Collection::open(c.root()).unwrap();
    assert!(b.request(&ip("b/people/listUsers.yml")).is_none());
    // The other way round: updating b leaves a's refreshed files alone.
    let before_a = snapshot(&c.root().join("a"));
    let s = apply_update(&mut c, &v2(), &opts("b"), None).unwrap();
    assert_eq!(paths(&s.added), ["b/people/listUsers.yml"]);
    assert_eq!(before_a, snapshot(&c.root().join("a")));
}

/// A root import of v1 plus folders `a` and `b`, each holding its own v1 import.
fn root_and_folders(dir: &std::path::Path) -> Collection {
    let mut c = imported(dir, "update-v1.json");
    place(&mut c, "a", "update-v1.json");
    place(&mut c, "b", "update-v1.json");
    c
}

#[test]
fn a_root_update_ignores_folder_subtrees() {
    let dir = TempDir::new().unwrap();
    let mut c = root_and_folders(dir.path());
    let (a, b) = (snapshot(&c.root().join("a")), snapshot(&c.root().join("b")));
    let s = apply_update(&mut c, &v2(), &UpdateOptions::default(), None).unwrap();
    assert_eq!(paths(&s.orphaned), ["legacy/removeLegacy.yml"]);
    assert_eq!(paths(&s.added), ["people/listUsers.yml"]);
    assert_eq!(a, snapshot(&c.root().join("a")));
    assert_eq!(b, snapshot(&c.root().join("b")));
    let again = plan_update(&c, &v2(), &UpdateOptions::default()).unwrap();
    for p in again.changed.iter().map(|c| &c.path).chain(&again.kept) {
        assert!(
            !p.as_str().starts_with("a/") && !p.as_str().starts_with("b/"),
            "{p:?}"
        );
    }
}

#[test]
fn a_folder_prune_trashes_only_that_folders_orphan() {
    let dir = TempDir::new().unwrap();
    let mut c = root_and_folders(dir.path());
    let before = (
        snapshot(&c.root().join("b")),
        c.root().join("legacy/removeLegacy.yml"),
    );
    let bin = FakeBin::new(true);
    let mut o = opts("a");
    o.prune = true;
    let s = apply_update(&mut c, &v2(), &o, Some(&bin as &dyn Trash)).unwrap();
    assert_eq!(paths(&s.pruned), ["a/legacy/removeLegacy.yml"]);
    assert_eq!(bin.taken(), 1);
    assert!(!c.root().join("a/legacy/removeLegacy.yml").exists());
    assert!(before.1.exists());
    assert_eq!(before.0, snapshot(&c.root().join("b")));
}

#[test]
fn a_root_prune_leaves_the_folders_orphans_and_files_alone() {
    let dir = TempDir::new().unwrap();
    let mut c = root_and_folders(dir.path());
    let (a, b) = (snapshot(&c.root().join("a")), snapshot(&c.root().join("b")));
    let bin = FakeBin::new(true);
    let mut o = UpdateOptions::default();
    o.prune = true;
    let s = apply_update(&mut c, &v2(), &o, Some(&bin as &dyn Trash)).unwrap();
    assert_eq!(paths(&s.pruned), ["legacy/removeLegacy.yml"]);
    assert_eq!(bin.taken(), 1);
    assert_eq!(a, snapshot(&c.root().join("a")));
    assert_eq!(b, snapshot(&c.root().join("b")));
    assert!(c.root().join("a/legacy/removeLegacy.yml").exists());
}

#[test]
fn a_nested_folder_with_its_own_entry_is_left_to_that_entry() {
    let dir = TempDir::new().unwrap();
    let mut c = Collection::create(&dir.path().join("c"), "c").unwrap();
    place(&mut c, "a", "update-v1.json");
    place_in(&mut c, &ip("a"), "b", "update-v1.json");
    // With a's own copy gone, a lookup by operation key would reach the nested folder's file.
    std::fs::remove_file(c.root().join("a/people/getUser.yml")).unwrap();
    let mut c = Collection::open(c.root()).unwrap();
    let inner = snapshot(&c.root().join("a/b"));
    let s = apply_update(&mut c, &v2(), &opts("a"), None).unwrap();
    for p in s
        .changed
        .iter()
        .map(|c| &c.path)
        .chain(&s.kept)
        .chain(&s.edited)
    {
        assert!(!p.as_str().starts_with("a/b/"), "{p:?}");
    }
    assert_eq!(paths(&s.orphaned), ["a/legacy/removeLegacy.yml"]);
    let a_ops = Collection::open(c.root())
        .unwrap()
        .file()
        .spec_baseline_in(&ip("a"))
        .unwrap();
    assert_eq!(a_ops.file("GET /users/{}"), Some("a/people/getUser.yml"));
    assert_eq!(inner, snapshot(&c.root().join("a/b")));
    let s = apply_update(&mut c, &v2(), &opts("a/b"), None).unwrap();
    assert_eq!(paths(&s.added), ["a/b/people/listUsers.yml"]);
}

#[test]
fn a_second_folder_update_writes_nothing() {
    let dir = TempDir::new().unwrap();
    let mut c = two_folders(dir.path());
    apply_update(&mut c, &v2(), &opts("a"), None).unwrap();
    let first = snapshot(c.root());
    std::thread::sleep(std::time::Duration::from_millis(30));
    let s = apply_update(&mut c, &v2(), &opts("a"), None).unwrap();
    assert!(s.changed.is_empty() && s.added.is_empty(), "{s:?}");
    assert_eq!(first, snapshot(c.root()));
}

#[test]
fn a_folder_dry_run_equals_the_applied_summary() {
    let dir = TempDir::new().unwrap();
    let mut c = two_folders(dir.path());
    let planned = plan_update(&c, &v2(), &opts("a")).unwrap();
    let applied = apply_update(&mut c, &v2(), &opts("a"), None).unwrap();
    assert_eq!(format!("{planned:?}"), format!("{applied:?}"));
}

#[test]
fn a_missing_folder_gives_the_no_baseline_note_naming_it() {
    let dir = TempDir::new().unwrap();
    let mut c = two_folders(dir.path());
    let before = snapshot(c.root());
    let s = apply_update(&mut c, &v2(), &opts("nope"), None).unwrap();
    let n = &s.notes[0];
    assert!(
        n.detail.starts_with("no update baseline") && n.detail.contains("nope"),
        "{n:?}"
    );
    assert_eq!(before, snapshot(c.root()));
}

#[test]
fn a_folder_baseline_records_the_method_and_a_method_edit_is_reported() {
    let dir = TempDir::new().unwrap();
    let mut c = two_folders(dir.path());
    let b = c.file().spec_baseline_in(&ip("a")).expect("folder");
    assert!(
        b.get("GET /users/{}")
            .unwrap()
            .get(ChangedField::Method)
            .is_some()
    );
    let f = c.request_mut(&ip("a/people/getUser.yml")).unwrap();
    f.set_method("POST");
    f.save().unwrap();
    let s = apply_update(&mut c, &v2(), &opts("a"), None).unwrap();
    assert_eq!(paths(&s.edited), ["a/people/getUser.yml"]);
    assert_eq!(s.edited_fields[0].fields, [ChangedField::Method]);
    let r = Collection::open(c.root()).unwrap();
    assert_eq!(
        r.request(&ip("a/people/getUser.yml")).unwrap().method(),
        "POST"
    );
}

#[test]
fn baseline_folders_come_back_in_tree_order() {
    let dir = TempDir::new().unwrap();
    let root = dir.path().join("c");
    let mut c = Collection::create(&root, "c").unwrap();
    for name in ["a", "b", "gone"] {
        std::fs::create_dir_all(root.join(name)).unwrap();
        std::fs::write(
            root.join(name).join("folder.yml"),
            format!("info:\n  name: {name}\n  type: folder\n"),
        )
        .unwrap();
    }
    // Imported into b first, a second, and one entry whose folder is then removed.
    for name in ["b", "a", "gone"] {
        c.file_mut()
            .set_spec_baseline_in(&ip(name), &one("GET /x", "xxxxxxxxxxxxxxxx"));
    }
    c.file_mut().save().unwrap();
    std::fs::remove_dir_all(root.join("gone")).unwrap();
    let c = Collection::open(&root).unwrap();
    // The file keeps import order, the collection answers in the sidebar's order.
    assert_eq!(paths(&c.file().spec_baseline_folders()), ["b", "a", "gone"]);
    assert_eq!(paths(&c.baseline_folders()), ["a", "b"]);
}
