//! The method is a spec-owned field: a local change to it is reported as edited.

use crate::import_update_support::{bundle, imported, ip, paths, spec};
use apinata_core::Collection;
use apinata_core::import::{
    ChangedField, FieldHashes, UpdateOptions, UpdateSummary, apply_update, plan_update,
};
use tempfile::TempDir;

fn v2() -> apinata_core::import::ImportBundle {
    bundle(&spec("update-v2.json"))
}

fn set_method(c: &mut Collection, path: &str, method: &str) {
    let f = c.request_mut(&ip(path)).unwrap();
    f.set_method(method);
    f.save().unwrap();
}

fn edited_fields(s: &UpdateSummary, path: &str) -> Vec<ChangedField> {
    let hits: Vec<_> = s
        .edited_fields
        .iter()
        .filter(|e| e.path.as_str() == path)
        .collect();
    assert_eq!(hits.len(), 1, "exactly one detail entry: {s:?}");
    hits[0].fields.clone()
}

// Strip the method hash from every recorded entry, as a baseline from before the field was.
fn make_legacy(c: &mut Collection) {
    let old = c.file().spec_baseline().unwrap();
    let mut legacy = apinata_core::import::SpecBaseline::default();
    for key in old.keys() {
        let hashes = old.get(key).unwrap();
        let kept = hashes
            .iter()
            .filter(|(f, _)| *f != ChangedField::Method)
            .map(|(f, h)| (f, h.to_owned()));
        legacy.insert(key.to_owned(), FieldHashes::new(kept));
        legacy.set_file(key.to_owned(), old.file(key).unwrap().to_owned());
    }
    c.file_mut().set_spec_baseline(&legacy);
    c.file_mut().save().unwrap();
}

#[test]
fn a_changed_method_is_listed_as_edited_and_other_fields_refresh() {
    let dir = TempDir::new().unwrap();
    let mut c = imported(dir.path(), "update-v1.json");
    set_method(&mut c, "people/getUser.yml", "POST");
    let s = apply_update(&mut c, &v2(), &UpdateOptions::default(), None).unwrap();
    assert_eq!(paths(&s.edited), ["people/getUser.yml"]);
    assert_eq!(
        edited_fields(&s, "people/getUser.yml"),
        [ChangedField::Method]
    );
    assert_eq!(s.changed.len(), 1);
    assert_eq!(
        s.changed[0].fields,
        [ChangedField::Target, ChangedField::Docs]
    );
    assert!(!paths(&s.added).contains(&"people/getUser.yml"));
    assert_eq!(
        c.request(&ip("people/getUser.yml")).unwrap().method(),
        "POST"
    );
    // Still edited on the next run, and nothing is duplicated.
    let again = apply_update(&mut c, &v2(), &UpdateOptions::default(), None).unwrap();
    assert_eq!(paths(&again.edited), ["people/getUser.yml"]);
    assert!(again.changed.is_empty(), "{again:?}");
    assert_eq!(paths(&again.added), Vec::<&str>::new());
}

#[test]
fn a_plan_names_the_method_without_writing() {
    let dir = TempDir::new().unwrap();
    let mut c = imported(dir.path(), "update-v1.json");
    set_method(&mut c, "people/getUser.yml", "PUT");
    let s = plan_update(&c, &v2(), &UpdateOptions::default()).unwrap();
    assert_eq!(
        edited_fields(&s, "people/getUser.yml"),
        [ChangedField::Method]
    );
}

#[test]
fn a_method_and_a_url_edit_list_one_request_naming_both() {
    let dir = TempDir::new().unwrap();
    let mut c = imported(dir.path(), "update-v1.json");
    let f = c.request_mut(&ip("people/getUser.yml")).unwrap();
    f.set_method("POST");
    f.set_url("{{baseUrl}}/users/:id?mine=1");
    f.save().unwrap();
    let s = plan_update(&c, &v2(), &UpdateOptions::default()).unwrap();
    assert_eq!(paths(&s.edited), ["people/getUser.yml"]);
    assert_eq!(
        edited_fields(&s, "people/getUser.yml"),
        [ChangedField::Target, ChangedField::Method]
    );
    assert_eq!(s.changed[0].fields, [ChangedField::Docs]);
}

#[test]
fn a_method_set_to_the_spec_value_again_is_not_edited() {
    let dir = TempDir::new().unwrap();
    let mut c = imported(dir.path(), "update-v1.json");
    set_method(&mut c, "people/getUser.yml", "POST");
    set_method(&mut c, "people/getUser.yml", "GET");
    let s = plan_update(&c, &v2(), &UpdateOptions::default()).unwrap();
    assert!(s.edited.is_empty() && s.edited_fields.is_empty(), "{s:?}");
}

#[test]
fn an_unrelated_edit_does_not_report_the_method() {
    let dir = TempDir::new().unwrap();
    let mut c = imported(dir.path(), "update-v1.json");
    let f = c.request_mut(&ip("people/getUser.yml")).unwrap();
    f.set_url("{{baseUrl}}/users/:id?mine=1");
    f.save().unwrap();
    let s = plan_update(&c, &v2(), &UpdateOptions::default()).unwrap();
    assert_eq!(
        edited_fields(&s, "people/getUser.yml"),
        [ChangedField::Target]
    );
}

#[test]
fn a_legacy_baseline_adopts_a_matching_method_silently() {
    let dir = TempDir::new().unwrap();
    let mut c = imported(dir.path(), "update-v1.json");
    make_legacy(&mut c);
    let s = apply_update(&mut c, &v2(), &UpdateOptions::default(), None).unwrap();
    assert!(s.edited.is_empty() && s.edited_fields.is_empty(), "{s:?}");
    assert_eq!(s.changed.len(), 1);
    assert_eq!(paths(&s.kept), ["people/createUser.yml"]);
    let b = c.file().spec_baseline().unwrap();
    for key in ["GET /users/{}", "POST /users"] {
        assert!(
            b.get(key).unwrap().get(ChangedField::Method).is_some(),
            "{key}"
        );
    }
    let second = apply_update(&mut c, &v2(), &UpdateOptions::default(), None).unwrap();
    assert!(
        second.changed.is_empty() && second.edited.is_empty(),
        "{second:?}"
    );
}

#[test]
fn a_legacy_baseline_leaves_a_different_method_alone() {
    let dir = TempDir::new().unwrap();
    let mut c = imported(dir.path(), "update-v1.json");
    make_legacy(&mut c);
    set_method(&mut c, "people/getUser.yml", "POST");
    let s = apply_update(&mut c, &v2(), &UpdateOptions::default(), None).unwrap();
    assert!(s.edited.is_empty() && s.edited_fields.is_empty(), "{s:?}");
    assert!(s.added.iter().all(|p| p.as_str() != "people/getUser.yml"));
    assert_eq!(
        c.request(&ip("people/getUser.yml")).unwrap().method(),
        "POST"
    );
    let b = c.file().spec_baseline().unwrap();
    assert!(
        b.get("GET /users/{}")
            .unwrap()
            .get(ChangedField::Method)
            .is_none()
    );
    let second = apply_update(&mut c, &v2(), &UpdateOptions::default(), None).unwrap();
    assert!(
        second.edited.is_empty() && second.changed.is_empty(),
        "{second:?}"
    );
}

#[test]
fn an_orphan_whose_method_was_edited_is_kept_and_reported() {
    let dir = TempDir::new().unwrap();
    let mut c = imported(dir.path(), "update-v1.json");
    set_method(&mut c, "legacy/removeLegacy.yml", "DELETE");
    let mut o = UpdateOptions::default();
    o.prune = true;
    let s = apply_update(&mut c, &v2(), &o, None).unwrap();
    assert!(s.pruned.is_empty(), "{s:?}");
    assert!(c.root().join("legacy/removeLegacy.yml").exists());
    assert!(
        s.notes.iter().any(|n| n
            .detail
            .contains("not pruned: a generated field was edited")),
        "{s:?}"
    );
}
