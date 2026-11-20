//! Taking the spec's values for chosen requests, or chosen fields, during an update.

use crate::import_update_support::{bundle, imported, ip, paths, snapshot, spec};
use apinata_core::Collection;
use apinata_core::import::{
    ChangedField, ImportError, TakeSpec, UpdateOptions, apply_update, file_field_hashes,
    plan_update,
};
use std::fs;
use std::path::Path;
use tempfile::TempDir;

fn v2() -> apinata_core::import::ImportBundle {
    bundle(&spec("update-v2.json"))
}

fn taking(specs: Vec<TakeSpec>) -> UpdateOptions {
    let mut o = UpdateOptions::default();
    o.take_spec = specs;
    o
}

/// getUser with a hand-edited URL and docs, which v2 changes both of.
fn edited_get_user(parent: &Path) -> Collection {
    let mut c = imported(parent, "update-v1.json");
    let f = c.request_mut(&ip("people/getUser.yml")).unwrap();
    f.set_url("{{baseUrl}}/users/:id?mine=1");
    f.set_docs("my notes");
    f.save().unwrap();
    c
}

#[test]
fn a_named_edited_request_takes_the_spec_and_the_next_update_is_clean() {
    let dir = TempDir::new().unwrap();
    let mut c = edited_get_user(dir.path());
    let o = taking(vec![TakeSpec::request(ip("people/getUser.yml"))]);
    let planned = plan_update(&c, &v2(), &o).unwrap();
    let s = apply_update(&mut c, &v2(), &o, None).unwrap();
    assert_eq!(format!("{planned:?}"), format!("{s:?}"));
    assert!(s.edited.is_empty(), "{s:?}");
    assert_eq!(s.changed.len(), 1, "{s:?}");
    assert_eq!(s.changed[0].path.as_str(), "people/getUser.yml");
    assert_eq!(
        s.changed[0].fields,
        [ChangedField::Target, ChangedField::Docs]
    );

    let mut reopened = Collection::open(c.root()).unwrap();
    let get = reopened.request(&ip("people/getUser.yml")).unwrap();
    assert!(get.url().contains("include") && !get.url().contains("mine"));
    assert_eq!(get.docs(), "Fetch a user");
    let recorded = reopened.file().spec_baseline().unwrap();
    let recorded = recorded.get("GET /users/{}").unwrap().clone();
    assert_eq!(
        recorded,
        file_field_hashes(get),
        "the hashes match the spec"
    );

    let next = apply_update(&mut reopened, &v2(), &UpdateOptions::default(), None).unwrap();
    assert!(
        next.edited.is_empty() && next.changed.is_empty(),
        "{next:?}"
    );
}

#[test]
fn a_named_field_takes_the_spec_and_the_other_edited_field_stays() {
    let dir = TempDir::new().unwrap();
    let mut c = edited_get_user(dir.path());
    let o = taking(vec![TakeSpec::fields(
        ip("people/getUser.yml"),
        vec![ChangedField::Target],
    )]);
    let s = apply_update(&mut c, &v2(), &o, None).unwrap();
    assert_eq!(s.changed[0].fields, [ChangedField::Target]);
    assert_eq!(s.edited_fields[0].fields, [ChangedField::Docs]);

    let mut reopened = Collection::open(c.root()).unwrap();
    let get = reopened.request(&ip("people/getUser.yml")).unwrap();
    assert!(get.url().contains("include"));
    assert_eq!(get.docs(), "my notes");
    let next = apply_update(&mut reopened, &v2(), &UpdateOptions::default(), None).unwrap();
    assert!(next.changed.is_empty(), "{next:?}");
    assert_eq!(next.edited_fields[0].fields, [ChangedField::Docs]);
}

#[test]
fn an_unnamed_edited_request_stays_as_it_is() {
    let dir = TempDir::new().unwrap();
    let mut c = edited_get_user(dir.path());
    let f = c.request_mut(&ip("people/createUser.yml")).unwrap();
    f.set_docs("kept");
    f.save().unwrap();
    let before = fs::read(c.root().join("people/createUser.yml")).unwrap();
    let o = taking(vec![TakeSpec::request(ip("people/getUser.yml"))]);
    let s = apply_update(&mut c, &v2(), &o, None).unwrap();
    assert!(paths(&s.kept).contains(&"people/createUser.yml"), "{s:?}");
    assert_eq!(
        fs::read(c.root().join("people/createUser.yml")).unwrap(),
        before
    );
}

#[test]
fn a_named_request_the_spec_did_not_change_is_put_back_to_the_spec() {
    let dir = TempDir::new().unwrap();
    let mut c = imported(dir.path(), "update-v1.json");
    let f = c.request_mut(&ip("people/createUser.yml")).unwrap();
    f.set_docs("mine");
    f.save().unwrap();
    let o = taking(vec![TakeSpec::request(ip("people/createUser.yml"))]);
    let s = apply_update(&mut c, &v2(), &o, None).unwrap();
    let change = s
        .changed
        .iter()
        .find(|ch| ch.path.as_str() == "people/createUser.yml")
        .expect("changed");
    assert_eq!(change.fields, [ChangedField::Docs]);
    let reopened = Collection::open(c.root()).unwrap();
    assert_eq!(
        reopened
            .request(&ip("people/createUser.yml"))
            .unwrap()
            .docs(),
        ""
    );
}

#[test]
fn an_unknown_request_is_an_error_and_nothing_is_written() {
    let dir = TempDir::new().unwrap();
    let mut c = edited_get_user(dir.path());
    let before = snapshot(c.root());
    let o = taking(vec![
        TakeSpec::request(ip("people/getUser.yml")),
        TakeSpec::request(ip("people/nope.yml")),
    ]);
    let planned = plan_update(&c, &v2(), &o);
    assert!(
        matches!(planned, Err(ImportError::NotMatched { ref path }) if path.as_str() == "people/nope.yml"),
        "{planned:?}"
    );
    let applied = apply_update(&mut c, &v2(), &o, None);
    assert!(
        matches!(applied, Err(ImportError::NotMatched { .. })),
        "{applied:?}"
    );
    assert_eq!(snapshot(c.root()), before);
}

#[test]
fn a_dry_run_reports_the_taken_request_as_changed_and_writes_nothing() {
    let dir = TempDir::new().unwrap();
    let c = edited_get_user(dir.path());
    let before = snapshot(c.root());
    let o = taking(vec![TakeSpec::request(ip("people/getUser.yml"))]);
    let s = plan_update(&c, &v2(), &o).unwrap();
    assert_eq!(s.changed[0].path.as_str(), "people/getUser.yml");
    assert_eq!(snapshot(c.root()), before);
}

#[test]
fn take_spec_parses_a_path_with_or_without_fields() {
    let whole = TakeSpec::parse("people/getUser.yml").unwrap();
    assert_eq!(whole.path.as_str(), "people/getUser.yml");
    assert!(whole.fields.is_empty());
    let some = TakeSpec::parse("people/getUser.yml:target,docs").unwrap();
    assert_eq!(some.path.as_str(), "people/getUser.yml");
    assert_eq!(some.fields, [ChangedField::Target, ChangedField::Docs]);
    let err = TakeSpec::parse("people/getUser.yml:target,colour").unwrap_err();
    assert!(err.contains("colour"), "{err}");
    assert!(TakeSpec::parse("").is_err());
}
