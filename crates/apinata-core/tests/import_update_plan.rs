//! plan_update against the v1 and v2 fixtures: it reports, and it writes nothing.

use crate::import_update_support::{bundle, imported, ip, paths, snapshot, spec};
use apinata_core::import::{
    ChangedField, ImportedRequest, UpdateOptions, add_request, plan_update,
};
use apinata_core::{Collection, ItemPath, ScriptPhase};
use tempfile::TempDir;

fn v2() -> apinata_core::import::ImportBundle {
    bundle(&spec("update-v2.json"))
}

#[test]
fn the_plan_reports_changed_added_kept_and_orphaned() {
    let dir = TempDir::new().unwrap();
    let c = imported(dir.path(), "update-v1.json");
    let before = snapshot(c.root());
    let s = plan_update(&c, &v2(), &UpdateOptions::default()).unwrap();
    assert_eq!(s.changed.len(), 1);
    assert_eq!(s.changed[0].path.as_str(), "people/getUser.yml");
    assert_eq!(
        s.changed[0].fields,
        [ChangedField::Target, ChangedField::Docs]
    );
    assert_eq!(paths(&s.added), ["people/listUsers.yml"]);
    assert_eq!(paths(&s.kept), ["people/createUser.yml"]);
    assert_eq!(paths(&s.orphaned), ["legacy/removeLegacy.yml"]);
    assert!(s.pruned.is_empty() && s.edited.is_empty());
    assert_eq!(snapshot(c.root()), before, "a plan writes nothing");
}

#[test]
fn prune_lists_the_orphan_as_pruned_but_still_writes_nothing() {
    let dir = TempDir::new().unwrap();
    let c = imported(dir.path(), "update-v1.json");
    let before = snapshot(c.root());
    let mut o = UpdateOptions::default();
    o.prune = true;
    let s = plan_update(&c, &v2(), &o).unwrap();
    assert_eq!(paths(&s.pruned), ["legacy/removeLegacy.yml"]);
    assert_eq!(snapshot(c.root()), before);
}

#[test]
fn a_locally_edited_url_is_left_and_listed_as_edited() {
    let dir = TempDir::new().unwrap();
    let mut c = imported(dir.path(), "update-v1.json");
    let f = c.request_mut(&ip("people/getUser.yml")).unwrap();
    f.set_url("{{baseUrl}}/users/:id?mine=1");
    f.save().unwrap();
    let s = plan_update(&c, &v2(), &UpdateOptions::default()).unwrap();
    assert_eq!(paths(&s.edited), ["people/getUser.yml"]);
    assert_eq!(s.changed[0].fields, [ChangedField::Docs]);
}

#[test]
fn a_request_with_unsaved_edits_is_edited_and_untouched() {
    let dir = TempDir::new().unwrap();
    let mut c = imported(dir.path(), "update-v1.json");
    c.request_mut(&ip("people/getUser.yml"))
        .unwrap()
        .set_docs("draft");
    let s = plan_update(&c, &v2(), &UpdateOptions::default()).unwrap();
    assert_eq!(paths(&s.edited), ["people/getUser.yml"]);
    assert!(s.changed.is_empty());
}

#[test]
fn a_collection_without_a_baseline_changes_nothing_and_says_why() {
    let dir = TempDir::new().unwrap();
    let mut c = Collection::create(&dir.path().join("c"), "c").unwrap();
    add_request(
        &mut c,
        &ItemPath::root(),
        "g",
        &ImportedRequest::new("GET", "{{baseUrl}}/users/:id"),
    )
    .unwrap();
    let s = plan_update(&c, &v2(), &UpdateOptions::default()).unwrap();
    assert!(s.added.is_empty() && s.changed.is_empty() && s.orphaned.is_empty());
    assert!(
        s.notes
            .iter()
            .any(|n| n.detail.starts_with("no update baseline"))
    );
}

#[test]
fn only_the_first_of_two_matching_requests_is_refreshed() {
    let dir = TempDir::new().unwrap();
    let mut c = imported(dir.path(), "update-v1.json");
    let copy = c
        .clone_request(&ip("people/getUser.yml"), "getUser copy")
        .unwrap();
    let s = plan_update(&c, &v2(), &UpdateOptions::default()).unwrap();
    assert_eq!(s.changed.len(), 1);
    assert_eq!(s.changed[0].path.as_str(), "people/getUser.yml");
    assert!(
        s.notes.iter().any(|n| n.detail.contains("duplicate")
            && n.file.as_deref().is_none_or(|f| f == copy.as_str()))
    );
    assert!(!paths(&s.orphaned).contains(&copy.as_str()));
}

#[test]
fn an_orphan_with_a_script_is_listed_but_never_pruned() {
    let dir = TempDir::new().unwrap();
    let mut c = imported(dir.path(), "update-v1.json");
    let f = c.request_mut(&ip("legacy/removeLegacy.yml")).unwrap();
    f.set_script(ScriptPhase::Tests, "test('x', function () {});");
    f.save().unwrap();
    let mut o = UpdateOptions::default();
    o.prune = true;
    let s = plan_update(&c, &v2(), &o).unwrap();
    assert_eq!(paths(&s.orphaned), ["legacy/removeLegacy.yml"]);
    assert!(s.pruned.is_empty());
    assert!(!s.notes.is_empty());
}

#[test]
fn a_field_the_user_already_set_to_the_new_value_is_adopted_not_refreshed() {
    let dir = TempDir::new().unwrap();
    let mut c = imported(dir.path(), "update-v1.json");
    let f = c.request_mut(&ip("people/getUser.yml")).unwrap();
    f.set_docs("Fetch a user"); // v2's docs, typed by hand
    f.save().unwrap();
    let s = plan_update(&c, &v2(), &UpdateOptions::default()).unwrap();
    assert!(s.edited.is_empty());
    assert_eq!(s.changed[0].fields, [ChangedField::Target]);
}

#[test]
fn tree_order_is_requests_first_then_subfolders() {
    // Pins the first-match order the Decisions list assumes.
    let dir = TempDir::new().unwrap();
    let mut c = imported(dir.path(), "update-v1.json");
    // A recorded file outranks tree order, so drop the record to leave tree order the tiebreak.
    let recorded = c.file().spec_baseline().unwrap();
    let mut legacy = apinata_core::import::SpecBaseline::default();
    for key in recorded.keys() {
        legacy.insert(key.to_owned(), recorded.get(key).unwrap().clone());
    }
    c.file_mut().set_spec_baseline(&legacy);
    c.file_mut().save().unwrap();
    let root_copy = c.clone_request(&ip("people/getUser.yml"), "x").unwrap();
    c.move_item(
        &root_copy,
        apinata_core::DropTarget::Inside(ItemPath::root()),
    )
    .unwrap();
    let s = plan_update(&c, &v2(), &UpdateOptions::default()).unwrap();
    assert_eq!(s.changed[0].path.as_str(), "x.yml");
}
