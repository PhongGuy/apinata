//! Pruning: only unedited, generated orphans, and never without the flag.

use crate::import_update_support::{FakeBin, bundle, imported, ip, paths, snapshot, spec};
use apinata_core::import::{
    ImportError, ImportedRequest, UpdateOptions, add_request, apply_update,
};
use apinata_core::{
    Assertion, Collection, ExampleResponse, ItemPath, KeyValue, PostResponseVar, RequestKind,
    ScriptPhase, Trash,
};
use std::fs;
use tempfile::TempDir;

fn prune() -> UpdateOptions {
    let mut o = UpdateOptions::default();
    o.prune = true;
    o
}

fn v2() -> apinata_core::import::ImportBundle {
    bundle(&spec("update-v2.json"))
}

#[test]
fn prune_moves_the_unedited_orphan_to_the_bin_and_drops_its_entry() {
    let dir = TempDir::new().unwrap();
    let mut c = imported(dir.path(), "update-v1.json");
    let bin = FakeBin::new(true);
    let s = apply_update(&mut c, &v2(), &prune(), Some(&bin as &dyn Trash)).unwrap();
    assert_eq!(paths(&s.pruned), ["legacy/removeLegacy.yml"]);
    assert!(!c.root().join("legacy/removeLegacy.yml").exists());
    assert_eq!(bin.taken(), 1);
    assert!(
        c.file()
            .spec_baseline()
            .unwrap()
            .get("GET /legacy")
            .is_none()
    );
    assert!(
        c.file().spec_baseline().is_some(),
        "the baseline stays, even if emptied"
    );
}

#[test]
fn without_a_bin_a_prune_is_refused_and_nothing_changes() {
    let dir = TempDir::new().unwrap();
    let mut c = imported(dir.path(), "update-v1.json");
    let before = snapshot(c.root());
    let err = apply_update(&mut c, &v2(), &prune(), None).unwrap_err();
    assert!(matches!(err, ImportError::TrashUnavailable), "{err}");
    assert!(err.to_string().contains("trash"), "{err}");
    assert_eq!(snapshot(c.root()), before);
}

#[test]
fn an_unavailable_bin_refuses_the_prune_and_nothing_changes() {
    let dir = TempDir::new().unwrap();
    let mut c = imported(dir.path(), "update-v1.json");
    let before = snapshot(c.root());
    let bin = FakeBin::new(false);
    let err = apply_update(&mut c, &v2(), &prune(), Some(&bin as &dyn Trash)).unwrap_err();
    assert!(matches!(err, ImportError::TrashUnavailable), "{err}");
    assert_eq!(snapshot(c.root()), before);
    assert_eq!(bin.taken(), 0);
}

#[test]
fn a_prune_without_a_baseline_returns_the_note_and_never_asks_for_a_bin() {
    let dir = TempDir::new().unwrap();
    let c0 = imported(dir.path(), "update-v1.json");
    let mut c = Collection::open(c0.root()).unwrap();
    c.file_mut().clear_spec_baseline();
    c.file_mut().save().unwrap();
    let before = snapshot(c.root());
    let mut again = Collection::open(c.root()).unwrap();
    let s = apply_update(&mut again, &v2(), &prune(), None).unwrap();
    assert!(s.pruned.is_empty() && s.added.is_empty());
    assert!(
        s.notes
            .iter()
            .any(|n| n.detail.starts_with("no update baseline"))
    );
    assert_eq!(snapshot(c.root()), before);
}

#[test]
fn without_a_bin_a_run_with_nothing_to_prune_still_works() {
    let dir = TempDir::new().unwrap();
    let mut c = imported(dir.path(), "update-v1.json");
    let f = c.request_mut(&ip("legacy/removeLegacy.yml")).unwrap();
    f.set_script(ScriptPhase::Tests, "test('x', function () {});");
    f.save().unwrap();
    let s = apply_update(&mut c, &v2(), &prune(), None).unwrap();
    assert!(s.pruned.is_empty());
    assert!(!s.added.is_empty());
}

#[test]
fn without_the_flag_nothing_is_deleted() {
    let dir = TempDir::new().unwrap();
    let mut c = imported(dir.path(), "update-v1.json");
    let s = apply_update(&mut c, &v2(), &UpdateOptions::default(), None).unwrap();
    assert_eq!(paths(&s.orphaned), ["legacy/removeLegacy.yml"]);
    assert!(s.pruned.is_empty());
    assert!(c.root().join("legacy/removeLegacy.yml").exists());
}

#[test]
fn an_orphan_with_a_test_is_kept_and_listed() {
    let dir = TempDir::new().unwrap();
    let mut c = imported(dir.path(), "update-v1.json");
    let f = c.request_mut(&ip("legacy/removeLegacy.yml")).unwrap();
    f.set_script(ScriptPhase::Tests, "test('x', function () {});");
    f.save().unwrap();
    let s = apply_update(&mut c, &v2(), &prune(), None).unwrap();
    assert_eq!(paths(&s.orphaned), ["legacy/removeLegacy.yml"]);
    assert!(s.pruned.is_empty());
    assert!(c.root().join("legacy/removeLegacy.yml").exists());
}

#[test]
fn an_orphan_whose_spec_field_was_edited_is_kept() {
    let dir = TempDir::new().unwrap();
    let mut c = imported(dir.path(), "update-v1.json");
    let f = c.request_mut(&ip("legacy/removeLegacy.yml")).unwrap();
    f.set_docs("my notes");
    f.save().unwrap();
    let s = apply_update(&mut c, &v2(), &prune(), None).unwrap();
    assert!(s.pruned.is_empty());
    assert!(c.root().join("legacy/removeLegacy.yml").exists());
}

#[test]
fn hand_made_requests_and_other_specs_requests_are_never_pruned() {
    let dir = TempDir::new().unwrap();
    let mut c = imported(dir.path(), "update-v1.json");
    add_request(
        &mut c,
        &ItemPath::root(),
        "mine",
        &ImportedRequest::new("GET", "{{baseUrl}}/mine"),
    )
    .unwrap();
    let bin = FakeBin::new(true);
    let s = apply_update(&mut c, &v2(), &prune(), Some(&bin as &dyn Trash)).unwrap();
    assert!(!paths(&s.pruned).contains(&"mine.yml"));
    assert!(c.root().join("mine.yml").exists());
}

#[test]
fn pruning_twice_is_a_no_op() {
    let dir = TempDir::new().unwrap();
    let mut c = imported(dir.path(), "update-v1.json");
    let bin = FakeBin::new(true);
    apply_update(&mut c, &v2(), &prune(), Some(&bin as &dyn Trash)).unwrap();
    let after = snapshot(c.root());
    let mut again = Collection::open(c.root()).unwrap();
    let s = apply_update(&mut again, &v2(), &prune(), None).unwrap();
    assert!(s.pruned.is_empty() && s.orphaned.is_empty());
    assert_eq!(snapshot(c.root()), after);
}

/// Open the saved collection again, as a later run would.
fn reopen(c: &Collection) -> Collection {
    Collection::open(c.root()).unwrap()
}

fn kept_with_note(c: &mut Collection, rel: &str) {
    let s = apply_update(c, &v2(), &prune(), None).unwrap();
    assert!(s.pruned.is_empty(), "{rel} must not be pruned");
    assert_eq!(paths(&s.orphaned), [rel]);
    assert!(
        s.notes
            .iter()
            .any(|n| n.detail.starts_with("not pruned") && n.file.as_deref() == Some(rel))
    );
    assert!(c.root().join(rel).exists());
    assert!(
        c.file()
            .spec_baseline()
            .unwrap()
            .get("GET /legacy")
            .is_some()
    );
}

#[test]
fn an_orphan_with_assertions_is_kept_with_a_note() {
    let dir = TempDir::new().unwrap();
    let mut c = imported(dir.path(), "update-v1.json");
    let f = c.request_mut(&ip("legacy/removeLegacy.yml")).unwrap();
    f.set_assertions(vec![Assertion::new("res.status", "eq", Some("200".into()))]);
    f.save().unwrap();
    kept_with_note(&mut c, "legacy/removeLegacy.yml");
}

#[test]
fn an_orphan_with_only_a_pre_request_script_is_kept_with_a_note() {
    let dir = TempDir::new().unwrap();
    let mut c = imported(dir.path(), "update-v1.json");
    let f = c.request_mut(&ip("legacy/removeLegacy.yml")).unwrap();
    f.set_script(ScriptPhase::PreRequest, "req.setHeader('X', '1');");
    f.save().unwrap();
    kept_with_note(&mut c, "legacy/removeLegacy.yml");
}

#[test]
fn an_orphan_with_only_a_post_response_variable_is_kept_with_a_note() {
    let dir = TempDir::new().unwrap();
    let mut c = imported(dir.path(), "update-v1.json");
    let f = c.request_mut(&ip("legacy/removeLegacy.yml")).unwrap();
    f.set_post_response_vars(vec![PostResponseVar::new("token", "res.body.token")]);
    f.save().unwrap();
    kept_with_note(&mut c, "legacy/removeLegacy.yml");
}

#[test]
fn an_orphan_with_examples_is_kept_with_a_note() {
    let dir = TempDir::new().unwrap();
    let mut c = imported(dir.path(), "update-v1.json");
    c.add_example(
        &ip("legacy/removeLegacy.yml"),
        "ok",
        "",
        &ExampleResponse::new(200),
    )
    .unwrap();
    kept_with_note(&mut c, "legacy/removeLegacy.yml");
}

#[test]
fn an_orphan_with_an_edited_generated_field_is_kept_with_a_note() {
    let dir = TempDir::new().unwrap();
    let mut c = imported(dir.path(), "update-v1.json");
    let f = c.request_mut(&ip("legacy/removeLegacy.yml")).unwrap();
    f.set_headers(vec![KeyValue::new("X-Mine", "1")]).unwrap();
    f.save().unwrap();
    kept_with_note(&mut c, "legacy/removeLegacy.yml");
}

#[test]
fn an_orphan_with_unsaved_edits_is_kept_with_a_note() {
    let dir = TempDir::new().unwrap();
    let mut c = imported(dir.path(), "update-v1.json");
    c.request_mut(&ip("legacy/removeLegacy.yml"))
        .unwrap()
        .set_url("{{baseUrl}}/legacy");
    c.request_mut(&ip("legacy/removeLegacy.yml"))
        .unwrap()
        .set_docs("unsaved");
    kept_with_note(&mut c, "legacy/removeLegacy.yml");
}

#[test]
fn a_non_http_file_in_place_of_the_orphan_is_never_removed() {
    let dir = TempDir::new().unwrap();
    let mut c = imported(dir.path(), "update-v1.json");
    fs::remove_file(c.root().join("legacy/removeLegacy.yml")).unwrap();
    let made = c
        .create_request_of_kind(&ip("legacy"), "removeLegacy", RequestKind::Graphql)
        .unwrap();
    let mut c = reopen(&c);
    apply_update(&mut c, &v2(), &prune(), None).unwrap();
    assert!(c.root().join(made.as_str()).exists());
}

#[test]
fn a_baseline_key_missing_from_spec_and_tree_is_dropped_only_with_prune() {
    let dir = TempDir::new().unwrap();
    let c = imported(dir.path(), "update-v1.json");
    fs::remove_file(c.root().join("legacy/removeLegacy.yml")).unwrap();

    let mut without = reopen(&c);
    let s = apply_update(&mut without, &v2(), &UpdateOptions::default(), None).unwrap();
    assert!(s.pruned.is_empty() && s.orphaned.is_empty());
    assert!(
        without
            .file()
            .spec_baseline()
            .unwrap()
            .get("GET /legacy")
            .is_some()
    );

    let mut with = reopen(&c);
    apply_update(&mut with, &v2(), &prune(), None).unwrap();
    let after = reopen(&c);
    assert!(
        after
            .file()
            .spec_baseline()
            .unwrap()
            .get("GET /legacy")
            .is_none()
    );
}

#[test]
fn a_matched_request_without_a_baseline_entry_is_kept_with_a_note() {
    let dir = TempDir::new().unwrap();
    let mut c = imported(dir.path(), "update-v1.json");
    // v2 lists GET /users, which v1 did not, so the baseline has no entry for this request.
    add_request(
        &mut c,
        &ItemPath::root(),
        "mine",
        &ImportedRequest::new("GET", "{{baseUrl}}/users"),
    )
    .unwrap();
    let bin = FakeBin::new(true);
    let s = apply_update(&mut c, &v2(), &prune(), Some(&bin as &dyn Trash)).unwrap();
    assert!(paths(&s.kept).contains(&"mine.yml"));
    assert!(!paths(&s.added).contains(&"people/listUsers.yml"));
    assert!(
        s.notes
            .iter()
            .any(|n| n.file.as_deref() == Some("mine.yml"))
    );
    assert!(
        c.file()
            .spec_baseline()
            .unwrap()
            .get("GET /users")
            .is_none()
    );
}

#[test]
fn an_edit_to_a_field_the_spec_did_not_change_is_not_reported_as_edited() {
    let dir = TempDir::new().unwrap();
    let mut c = imported(dir.path(), "update-v1.json");
    let f = c.request_mut(&ip("people/getUser.yml")).unwrap();
    f.set_headers(vec![KeyValue::new("X-Mine", "1")]).unwrap();
    f.save().unwrap();
    let s = apply_update(&mut c, &v2(), &UpdateOptions::default(), None).unwrap();
    assert!(s.edited.is_empty());
    assert_eq!(s.changed.len(), 1);
    let headers = c.request(&ip("people/getUser.yml")).unwrap().headers();
    assert_eq!(headers.len(), 1);
    assert_eq!(headers[0].name, "X-Mine");
}

#[cfg(unix)]
#[test]
fn a_renumber_failure_after_the_removal_is_a_note_not_a_failed_prune() {
    let dir = TempDir::new().unwrap();
    let mut c = imported(dir.path(), "update-v1.json");
    // A sibling folder numbered after the orphan must move up, and its folder.yml cannot be
    // written once the directory is read-only.
    c.create_folder(&ip("legacy"), "sub").unwrap();
    let locked = c.root().join("legacy/sub");
    let _g = crate::common::ModeGuard::set(&locked, 0o555);
    if fs::write(locked.join("probe"), "").is_ok() {
        // Running as root: the failure cannot be staged.
        fs::remove_file(locked.join("probe")).unwrap();
        return;
    }
    let mut c = reopen(&c);
    let bin = FakeBin::new(true);
    let s = apply_update(&mut c, &v2(), &prune(), Some(&bin as &dyn Trash)).unwrap();
    assert_eq!(paths(&s.pruned), ["legacy/removeLegacy.yml"]);
    assert_eq!(bin.taken(), 1);
    assert!(s.notes.iter().any(|n| {
        n.detail
            .starts_with("removed, but siblings were not renumbered")
    }));
    assert!(
        c.file()
            .spec_baseline()
            .unwrap()
            .get("GET /legacy")
            .is_none()
    );
}

#[test]
fn a_pruned_request_is_recorded_and_restores_from_the_trash_log() {
    let dir = TempDir::new().unwrap();
    let logs = TempDir::new().unwrap();
    let mut c = imported(dir.path(), "update-v1.json");
    let bin = FakeBin::new(true);
    let log = apinata_core::TrashLog::new(logs.path());
    let mut o = prune();
    o.trash_log = Some(log.clone());
    let s = apply_update(&mut c, &v2(), &o, Some(&bin as &dyn Trash)).unwrap();
    assert_eq!(paths(&s.pruned), ["legacy/removeLegacy.yml"]);
    assert!(
        s.notes.iter().all(|n| !n.detail.contains("restored")),
        "{:?}",
        s.notes
    );
    let restored = apinata_core::item_ops::restore_last_trashed(&mut c, &bin, &log).unwrap();
    assert_eq!(restored.item().to_string(), "legacy/removeLegacy.yml");
    assert!(c.root().join("legacy/removeLegacy.yml").is_file());
}

#[test]
fn a_prune_whose_record_cannot_be_written_is_a_note_not_a_failure() {
    let dir = TempDir::new().unwrap();
    let logs = TempDir::new().unwrap();
    let blocked = logs.path().join("not-a-dir");
    fs::write(&blocked, "a file where the log directory goes").unwrap();
    let mut c = imported(dir.path(), "update-v1.json");
    let bin = FakeBin::new(true);
    let mut o = prune();
    o.trash_log = Some(apinata_core::TrashLog::new(&blocked));
    let s = apply_update(&mut c, &v2(), &o, Some(&bin as &dyn Trash)).unwrap();
    assert_eq!(paths(&s.pruned), ["legacy/removeLegacy.yml"]);
    assert!(
        s.notes.iter().any(|n| n
            .detail
            .starts_with("moved to the trash, but it cannot be restored from here: ")
            && n.file.as_deref() == Some("legacy/removeLegacy.yml")),
        "{:?}",
        s.notes
    );
}
