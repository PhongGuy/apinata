//! A request's identity survives edits to its path or method, through the file the baseline
//! recorded for it.

use crate::import_update_support::{FakeBin, bundle, imported, ip, paths, snapshot, spec};
use apinata_core::import::{SpecBaseline, UpdateOptions, apply_update, plan_update};
use apinata_core::{Collection, Trash};
use std::fs;
use std::time::Duration;
use tempfile::TempDir;

const V9: &str = "{{baseUrl}}/v9/users/:id";

fn v2() -> apinata_core::import::ImportBundle {
    bundle(&spec("update-v2.json"))
}

fn no_dupes(c: &Collection) {
    let people = c.root().join("people");
    let names: Vec<String> = fs::read_dir(people)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert!(
        !names.iter().any(|n| n.contains(" 1.yml")),
        "duplicate written: {names:?}"
    );
}

fn edit_url(c: &mut Collection, rel: &str, url: &str) {
    let f = c.request_mut(&ip(rel)).unwrap();
    f.set_url(url);
    f.save().unwrap();
}

#[test]
fn an_edited_path_stays_the_same_request_and_is_listed_edited() {
    let dir = TempDir::new().unwrap();
    let mut c = imported(dir.path(), "update-v1.json");
    edit_url(&mut c, "people/getUser.yml", V9);
    let planned = plan_update(&c, &v2(), &UpdateOptions::default()).unwrap();
    let s = apply_update(&mut c, &v2(), &UpdateOptions::default(), None).unwrap();
    assert_eq!(format!("{planned:?}"), format!("{s:?}"));
    assert_eq!(paths(&s.edited), ["people/getUser.yml"]);
    assert_eq!(paths(&s.added), ["people/listUsers.yml"]);
    no_dupes(&c);
    let reopened = Collection::open(c.root()).unwrap();
    let get = reopened.request(&ip("people/getUser.yml")).unwrap();
    assert_eq!(get.url(), V9);
    assert_eq!(get.docs(), "Fetch a user");

    let after_first = snapshot(c.root());
    std::thread::sleep(Duration::from_millis(30));
    let mut again = Collection::open(c.root()).unwrap();
    let second = apply_update(&mut again, &v2(), &UpdateOptions::default(), None).unwrap();
    assert!(second.added.is_empty());
    assert_eq!(paths(&second.edited), ["people/getUser.yml"]);
    assert_eq!(snapshot(c.root()), after_first);
}

#[test]
fn an_edited_method_stays_the_same_request() {
    let dir = TempDir::new().unwrap();
    let mut c = imported(dir.path(), "update-v1.json");
    let f = c.request_mut(&ip("people/getUser.yml")).unwrap();
    f.set_method("POST");
    f.save().unwrap();
    let s = apply_update(&mut c, &v2(), &UpdateOptions::default(), None).unwrap();
    assert_eq!(paths(&s.added), ["people/listUsers.yml"]);
    no_dupes(&c);
    let reopened = Collection::open(c.root()).unwrap();
    assert_eq!(
        reopened
            .request(&ip("people/getUser.yml"))
            .unwrap()
            .method(),
        "POST"
    );
}

fn relocated(to: &str) {
    let dir = TempDir::new().unwrap();
    let c0 = imported(dir.path(), "update-v1.json");
    fs::rename(c0.root().join("people/getUser.yml"), c0.root().join(to)).unwrap();
    let mut c = Collection::open(c0.root()).unwrap();
    let s = apply_update(&mut c, &v2(), &UpdateOptions::default(), None).unwrap();
    assert_eq!(paths(&s.added), ["people/listUsers.yml"], "{s:?}");
    assert_eq!(s.changed.len(), 1);
    assert_eq!(s.changed[0].path.as_str(), to);
    no_dupes(&c);
    let mut c = Collection::open(c0.root()).unwrap();
    assert_eq!(
        c.file().spec_baseline().unwrap().file("GET /users/{}"),
        Some(to)
    );
    // The relink is what lets a later path edit on the moved file still match.
    edit_url(&mut c, to, V9);
    let s = apply_update(&mut c, &v2(), &UpdateOptions::default(), None).unwrap();
    assert!(s.added.is_empty(), "{s:?}");
    assert_eq!(c.request(&ip(to)).unwrap().url(), V9);
}

#[test]
fn a_renamed_file_is_found_by_key_and_relinked() {
    relocated("people/fetchUser.yml");
}

#[test]
fn a_moved_file_is_found_by_key_and_relinked() {
    relocated("legacy/getUser.yml");
}

#[test]
fn an_orphan_with_an_edited_path_is_listed_but_not_pruned() {
    let dir = TempDir::new().unwrap();
    let mut c = imported(dir.path(), "update-v1.json");
    edit_url(&mut c, "legacy/removeLegacy.yml", "{{baseUrl}}/v9/legacy");
    let mut o = UpdateOptions::default();
    o.prune = true;
    let bin = FakeBin::new(true);
    let s = apply_update(&mut c, &v2(), &o, Some(&bin as &dyn Trash)).unwrap();
    assert_eq!(paths(&s.orphaned), ["legacy/removeLegacy.yml"]);
    assert!(s.pruned.is_empty());
    assert!(c.root().join("legacy/removeLegacy.yml").exists());
    assert_eq!(bin.taken(), 0);
    assert!(
        s.notes
            .iter()
            .any(|n| n.detail.contains("generated field was edited")),
        "{:?}",
        s.notes
    );
}

// Left deliberately: a file renamed AND path-edited in one step has neither its recorded
// name nor its key to be found by, so the operation is added anew and the old file is
// listed nowhere. Detecting it would need content similarity, which is out of scope.
#[test]
fn pins_a_renamed_and_path_edited_file_is_added_anew() {
    let dir = TempDir::new().unwrap();
    let c0 = imported(dir.path(), "update-v1.json");
    fs::rename(
        c0.root().join("people/getUser.yml"),
        c0.root().join("people/fetchUser.yml"),
    )
    .unwrap();
    let mut c = Collection::open(c0.root()).unwrap();
    edit_url(&mut c, "people/fetchUser.yml", V9);
    let s = apply_update(&mut c, &v2(), &UpdateOptions::default(), None).unwrap();
    assert!(paths(&s.added).contains(&"people/getUser.yml"), "{s:?}");
    let old = "people/fetchUser.yml";
    assert!(!paths(&s.orphaned).contains(&old));
    assert!(!paths(&s.edited).contains(&old));
    assert!(!paths(&s.kept).contains(&old));
    assert!(c.root().join(old).exists());
}

// Left deliberately: a baseline written before files were recorded cannot say where an
// operation lives, so a path edited before the first update still reads as a new operation.
#[test]
fn pins_a_legacy_baseline_with_an_edited_path_adds_a_duplicate() {
    let dir = TempDir::new().unwrap();
    let mut c = imported(dir.path(), "update-v1.json");
    let recorded = c.file().spec_baseline().unwrap();
    let mut baseline = SpecBaseline::default();
    for key in recorded.keys() {
        baseline.insert(key.to_owned(), recorded.get(key).unwrap().clone());
    }
    c.file_mut().set_spec_baseline(&baseline);
    c.file_mut().save().unwrap();
    edit_url(&mut c, "people/getUser.yml", V9);
    let s = apply_update(&mut c, &v2(), &UpdateOptions::default(), None).unwrap();
    assert!(paths(&s.added).contains(&"people/getUser 1.yml"), "{s:?}");
}

fn record(c: &mut Collection, key: &str, file: &str) {
    let mut b = c.file().spec_baseline().unwrap();
    b.set_file(key.to_owned(), file.to_owned());
    c.file_mut().set_spec_baseline(&b);
    c.file_mut().save().unwrap();
}

#[test]
fn two_keys_recording_one_file_fall_back_to_their_keys() {
    let dir = TempDir::new().unwrap();
    let mut c = imported(dir.path(), "update-v1.json");
    record(&mut c, "GET /legacy", "people/getUser.yml");
    let mut o = UpdateOptions::default();
    o.prune = true;
    let bin = FakeBin::new(true);
    let s = apply_update(&mut c, &v2(), &o, Some(&bin as &dyn Trash)).unwrap();
    assert_eq!(paths(&s.orphaned), ["legacy/removeLegacy.yml"], "{s:?}");
    assert_eq!(paths(&s.pruned), ["legacy/removeLegacy.yml"]);
    assert_eq!(bin.taken(), 1);
    assert!(c.root().join("people/getUser.yml").exists());
    assert_eq!(paths(&s.added), ["people/listUsers.yml"]);
    no_dupes(&c);
}

#[test]
fn a_recorded_file_that_is_missing_falls_back_to_the_key() {
    let dir = TempDir::new().unwrap();
    let mut c = imported(dir.path(), "update-v1.json");
    record(&mut c, "GET /users/{}", "people/gone.yml");
    let s = apply_update(&mut c, &v2(), &UpdateOptions::default(), None).unwrap();
    assert_eq!(paths(&s.added), ["people/listUsers.yml"]);
    assert_eq!(s.changed[0].path.as_str(), "people/getUser.yml");
    assert_eq!(
        c.file().spec_baseline().unwrap().file("GET /users/{}"),
        Some("people/getUser.yml")
    );
}
