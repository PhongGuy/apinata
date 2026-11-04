//! apply_update: refresh, add, idempotency, and what it must not touch.

use crate::import_update_support::{bundle, imported, ip, paths, snapshot, spec};
use apinata_core::import::{UpdateOptions, apply_update, plan_update};
use apinata_core::{Assertion, AuthSpec, Collection, ExampleResponse, ScriptPhase};
use std::fs;
use std::time::Duration;
use tempfile::TempDir;

fn v2() -> apinata_core::import::ImportBundle {
    bundle(&spec("update-v2.json"))
}

#[test]
fn apply_refreshes_the_changed_request_adds_the_new_one_and_matches_the_plan() {
    let dir = TempDir::new().unwrap();
    let mut c = imported(dir.path(), "update-v1.json");
    let planned = plan_update(&c, &v2(), &UpdateOptions::default()).unwrap();
    let applied = apply_update(&mut c, &v2(), &UpdateOptions::default(), None).unwrap();
    assert_eq!(format!("{planned:?}"), format!("{applied:?}"));
    let reopened = Collection::open(c.root()).unwrap();
    let get = reopened.request(&ip("people/getUser.yml")).unwrap();
    assert!(get.url().contains("include"), "{}", get.url());
    assert_eq!(get.docs(), "Fetch a user");
    assert!(reopened.request(&ip("people/listUsers.yml")).is_some());
    let baseline = reopened.file().spec_baseline().unwrap();
    assert!(baseline.get("GET /users").is_some());
    assert!(
        baseline.get("GET /legacy").is_some(),
        "no prune: the orphan keeps its entry"
    );
    assert!(
        reopened.request(&ip("legacy/removeLegacy.yml")).is_some(),
        "nothing deleted"
    );
}

#[test]
fn a_test_added_to_a_request_survives_the_update() {
    let dir = TempDir::new().unwrap();
    let mut c = imported(dir.path(), "update-v1.json");
    let f = c.request_mut(&ip("people/getUser.yml")).unwrap();
    f.set_script(ScriptPhase::Tests, "test('ok', function () {});");
    f.set_name("Get one user");
    f.save().unwrap();
    apply_update(&mut c, &v2(), &UpdateOptions::default(), None).unwrap();
    let reopened = Collection::open(c.root()).unwrap();
    let get = reopened.request(&ip("people/getUser.yml")).unwrap();
    assert!(get.script(ScriptPhase::Tests).contains("test('ok'"));
    assert_eq!(get.name(), "Get one user");
    assert!(get.url().contains("include"));
}

#[test]
fn updating_twice_changes_no_file_bytes_or_mtimes() {
    let dir = TempDir::new().unwrap();
    let mut c = imported(dir.path(), "update-v1.json");
    apply_update(&mut c, &v2(), &UpdateOptions::default(), None).unwrap();
    let after_first = snapshot(c.root());
    std::thread::sleep(Duration::from_millis(30));
    let mut again = Collection::open(c.root()).unwrap();
    let second = apply_update(&mut again, &v2(), &UpdateOptions::default(), None).unwrap();
    assert!(second.added.is_empty() && second.changed.is_empty());
    assert_eq!(snapshot(c.root()), after_first);
}

#[test]
fn an_edited_url_is_not_overwritten_and_stays_edited_on_the_next_run() {
    let dir = TempDir::new().unwrap();
    let mut c = imported(dir.path(), "update-v1.json");
    let f = c.request_mut(&ip("people/getUser.yml")).unwrap();
    f.set_url("{{baseUrl}}/users/:id?mine=1");
    f.save().unwrap();
    for _ in 0..2 {
        let s = apply_update(&mut c, &v2(), &UpdateOptions::default(), None).unwrap();
        assert_eq!(paths(&s.edited), ["people/getUser.yml"]);
    }
    let reopened = Collection::open(c.root()).unwrap();
    let get = reopened.request(&ip("people/getUser.yml")).unwrap();
    assert!(get.url().contains("mine=1") && !get.url().contains("include"));
    assert_eq!(
        get.docs(),
        "Fetch a user",
        "the unedited field was still refreshed"
    );
}

#[test]
fn unsaved_in_memory_edits_make_the_request_skipped_and_the_file_untouched() {
    let dir = TempDir::new().unwrap();
    let mut c = imported(dir.path(), "update-v1.json");
    let before = fs::read(c.root().join("people/getUser.yml")).unwrap();
    c.request_mut(&ip("people/getUser.yml"))
        .unwrap()
        .set_docs("draft");
    let s = apply_update(&mut c, &v2(), &UpdateOptions::default(), None).unwrap();
    assert_eq!(paths(&s.edited), ["people/getUser.yml"]);
    assert_eq!(
        fs::read(c.root().join("people/getUser.yml")).unwrap(),
        before
    );
}

#[test]
fn a_collection_without_a_baseline_is_byte_for_byte_untouched() {
    let dir = TempDir::new().unwrap();
    let c0 = imported(dir.path(), "update-v1.json");
    let mut c = Collection::open(c0.root()).unwrap();
    c.file_mut().clear_spec_baseline();
    c.file_mut().save().unwrap();
    let before = snapshot(c.root());
    std::thread::sleep(Duration::from_millis(30));
    let mut again = Collection::open(c.root()).unwrap();
    let s = apply_update(&mut again, &v2(), &UpdateOptions::default(), None).unwrap();
    assert!(s.added.is_empty() && s.changed.is_empty());
    assert_eq!(snapshot(c.root()), before);
}

#[test]
fn a_conflict_fails_that_request_only_and_the_run_continues() {
    let dir = TempDir::new().unwrap();
    let mut c = imported(dir.path(), "update-v1.json");
    let recorded = c
        .file()
        .spec_baseline()
        .unwrap()
        .get("GET /users/{}")
        .cloned()
        .unwrap();
    // Another program rewrites the file after the collection loaded it.
    let path = c.root().join("people/getUser.yml");
    let mut text = fs::read_to_string(&path).unwrap();
    text.push('\n');
    fs::write(&path, text).unwrap();
    let s = apply_update(&mut c, &v2(), &UpdateOptions::default(), None).unwrap();
    assert!(s.changed.is_empty());
    assert!(
        s.notes
            .iter()
            .any(|n| format!("{n:?}").contains("people/getUser.yml"))
    );
    assert_eq!(
        paths(&s.added),
        ["people/listUsers.yml"],
        "the run continued"
    );
    let baseline = c.file().spec_baseline().unwrap();
    assert_eq!(
        baseline.get("GET /users/{}"),
        Some(&recorded),
        "its entry never moved"
    );
    assert!(
        baseline.get("GET /users").is_some(),
        "the other request's entry was written"
    );
    // Same Collection, no reopen: the failed request was rolled back, so it is retried
    // rather than mistaken for a local edit.
    let retry = plan_update(&c, &v2(), &UpdateOptions::default()).unwrap();
    assert!(retry.edited.is_empty(), "{retry:?}");
    assert_eq!(
        paths(
            &retry
                .changed
                .iter()
                .map(|ch| ch.path.clone())
                .collect::<Vec<_>>()
        ),
        ["people/getUser.yml"]
    );
}

#[test]
fn a_hand_set_new_value_is_adopted_into_the_baseline() {
    let dir = TempDir::new().unwrap();
    let mut c = imported(dir.path(), "update-v1.json");
    let f = c.request_mut(&ip("people/getUser.yml")).unwrap();
    f.set_docs("Fetch a user");
    f.save().unwrap();
    let s = apply_update(&mut c, &v2(), &UpdateOptions::default(), None).unwrap();
    assert_eq!(
        s.changed[0].fields,
        [apinata_core::import::ChangedField::Target]
    );
    let reopened = Collection::open(c.root()).unwrap();
    let on_file = apinata_core::import::file_field_hashes(
        reopened.request(&ip("people/getUser.yml")).unwrap(),
    );
    let baseline = reopened.file().spec_baseline().unwrap();
    let recorded = baseline.get("GET /users/{}").unwrap();
    assert_eq!(
        on_file, *recorded,
        "every recorded hash now equals the file's"
    );
}

#[test]
fn a_new_operation_goes_into_the_existing_tag_folder() {
    let dir = TempDir::new().unwrap();
    let mut c = imported(dir.path(), "update-v1.json");
    apply_update(&mut c, &v2(), &UpdateOptions::default(), None).unwrap();
    assert_eq!(
        c.tree()
            .folders()
            .iter()
            .filter(|f| f.name() == "people")
            .count(),
        1
    );
}

#[test]
fn auth_assertions_and_examples_survive_a_refresh() {
    let dir = TempDir::new().unwrap();
    let mut c = imported(dir.path(), "update-v1.json");
    let item = ip("people/getUser.yml");
    c.add_example(&item, "ok", "", &ExampleResponse::new(200))
        .unwrap();
    let f = c.request_mut(&item).unwrap();
    f.set_auth(AuthSpec::Basic {
        username: "u".into(),
        password: "p".into(),
    });
    f.set_assertions(vec![Assertion::new("res.status", "eq", Some("200".into()))]);
    f.save().unwrap();
    let s = apply_update(&mut c, &v2(), &UpdateOptions::default(), None).unwrap();
    assert_eq!(s.changed.len(), 1);
    let reopened = Collection::open(c.root()).unwrap();
    let get = reopened.request(&item).unwrap();
    assert!(matches!(get.auth(), AuthSpec::Basic { .. }));
    assert_eq!(get.assertions().len(), 1);
    assert_eq!(get.examples().len(), 1);
    assert!(get.url().contains("include"));
}
