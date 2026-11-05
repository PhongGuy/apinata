//! Planning and applying a spec update through the actor, with a fake trash.

use crate::support;

use apinata_app::actor::Faults;
use apinata_app::command::{Command, NoticeKind, TreeOp};
use apinata_app::model::CollectionModel;
use apinata_core::MemoryStore;
use apinata_core::import::{
    CollectionFormat, CollectionOptions, ImportBundle, UpdateSummary, parse_collections,
    write_collection,
};
use gpui_kit::{Entity, TestAppContext};
use std::fs;
use std::path::Path;
use std::sync::Arc;
use support::fake_trash::FakeTrash;
use support::*;

const V1: &[u8] = include_bytes!("../../apinata-core/tests/fixtures/openapi/update-v1.json");
const V2: &[u8] = include_bytes!("../../apinata-core/tests/fixtures/openapi/update-v2.json");

fn bundle(spec: &[u8]) -> Arc<ImportBundle> {
    let mut all = parse_collections(
        Some(CollectionFormat::OpenApi),
        spec,
        &CollectionOptions::default(),
    )
    .expect("parses");
    Arc::new(all.remove(0))
}

/// A collection imported from the v1 spec, opened with a fake trash.
fn open_v1(
    cx: &mut TestAppContext,
    parent: &Path,
    bin: Arc<FakeTrash>,
) -> (Entity<CollectionModel>, std::path::PathBuf) {
    open_spec(cx, parent, bin, V1)
}

/// A collection imported from `spec`, opened with a fake trash.
fn open_spec(
    cx: &mut TestAppContext,
    parent: &Path,
    bin: Arc<FakeTrash>,
    spec: &[u8],
) -> (Entity<CollectionModel>, std::path::PathBuf) {
    let b = bundle(spec);
    let (c, _) = write_collection(parent, &b.collection, &b.environments, &MemoryStore::new())
        .expect("writes");
    let root = c.root().to_path_buf();
    let faults = Faults {
        trash: Some(bin),
        ..Faults::default()
    };
    let model = cx.update(|cx| {
        CollectionModel::open_with_faults(root.clone(), Arc::new(MemoryStore::new()), faults, cx)
    });
    cx.run_until_parked();
    (model, root)
}

fn plan(
    cx: &mut TestAppContext,
    model: &Entity<CollectionModel>,
    spec: &[u8],
    prune: bool,
) -> Result<UpdateSummary, String> {
    let (reply, answer) = async_channel::bounded(1);
    let bundle = bundle(spec);
    send(
        cx,
        model,
        Command::PlanUpdate {
            folder: None,
            bundle,
            prune,
            reply,
        },
    );
    cx.run_until_parked();
    answer.try_recv().expect("the actor replied")
}

fn apply(
    cx: &mut TestAppContext,
    model: &Entity<CollectionModel>,
    spec: &[u8],
    prune: bool,
) -> Result<UpdateSummary, String> {
    let (reply, answer) = async_channel::bounded(1);
    let bundle = bundle(spec);
    send(
        cx,
        model,
        Command::ApplyUpdate {
            folder: None,
            bundle,
            prune,
            reply,
        },
    );
    cx.run_until_parked();
    answer.try_recv().expect("the actor replied")
}

fn info(cx: &mut TestAppContext, model: &Entity<CollectionModel>) -> bool {
    let (reply, answer) = async_channel::bounded(1);
    send(cx, model, Command::UpdateInfo { reply });
    cx.run_until_parked();
    answer.try_recv().expect("the actor replied")
}

#[gpui_kit::test]
fn the_plan_equals_what_apply_then_does(cx: &mut TestAppContext) {
    let parent = tempfile::tempdir().unwrap();
    let (model, root) = open_v1(cx, parent.path(), Arc::new(FakeTrash::new()));
    assert!(info(cx, &model));
    let planned = plan(cx, &model, V2, false).expect("planned");
    assert!(!planned.added.is_empty() && !planned.changed.is_empty());
    let before = fs::read_to_string(root.join("people/getUser.yml")).unwrap();
    let applied = apply(cx, &model, V2, false).expect("applied");
    assert_eq!(format!("{planned:?}"), format!("{applied:?}"));
    assert_ne!(
        fs::read_to_string(root.join("people/getUser.yml")).unwrap(),
        before
    );
}

#[gpui_kit::test]
fn the_model_shows_the_updated_content_after_apply(cx: &mut TestAppContext) {
    let parent = tempfile::tempdir().unwrap();
    let (model, _) = open_v1(cx, parent.path(), Arc::new(FakeTrash::new()));
    let before = url(&snap(cx, &model), "people/getUser.yml");
    apply(cx, &model, V2, false).expect("applied");
    let s = snap(cx, &model);
    assert_ne!(url(&s, "people/getUser.yml"), before);
    assert!(s.content.requests.contains_key(&ip("people/listUsers.yml")));
}

#[gpui_kit::test]
fn without_prune_nothing_is_removed(cx: &mut TestAppContext) {
    let parent = tempfile::tempdir().unwrap();
    let bin = Arc::new(FakeTrash::new());
    let (model, root) = open_v1(cx, parent.path(), bin.clone());
    let s = apply(cx, &model, V2, false).expect("applied");
    assert!(s.pruned.is_empty() && !s.orphaned.is_empty());
    assert!(root.join("legacy/removeLegacy.yml").exists());
    assert_eq!(bin.len(), 0);
}

#[gpui_kit::test]
fn prune_trashes_the_orphan_and_it_can_be_restored(cx: &mut TestAppContext) {
    let parent = tempfile::tempdir().unwrap();
    let bin = Arc::new(FakeTrash::new());
    let (model, root) = open_v1(cx, parent.path(), bin.clone());
    let planned = plan(cx, &model, V2, true).expect("planned");
    assert!(
        root.join("legacy/removeLegacy.yml").exists(),
        "a plan writes nothing"
    );
    let s = apply(cx, &model, V2, true).expect("applied");
    // The removal is undone in the app, so the applied run says nothing the plan did not.
    assert!(s.notes.iter().all(|n| !n.detail.contains("trash")), "{s:?}");
    assert_eq!(format!("{planned:?}"), format!("{s:?}"));
    assert_eq!(s.pruned.len(), 1);
    let file = root.join("legacy/removeLegacy.yml");
    let item = ip("legacy/removeLegacy.yml");
    assert!(!file.exists());
    assert_eq!(bin.len(), 1);
    let got = notices(cx, &model);
    let [
        NoticeKind::Trashed {
            item: trashed,
            ticket,
        },
    ] = got.as_slice()
    else {
        panic!("{got:?}")
    };
    assert_eq!(*trashed, item);
    send(
        cx,
        &model,
        Command::Tree(TreeOp::Restore { ticket: *ticket }),
    );
    cx.run_until_parked();
    assert_eq!(notices(cx, &model), [NoticeKind::Restored(item)]);
    assert!(file.exists());
    assert_eq!(bin.len(), 0);
}

#[gpui_kit::test]
fn undo_on_a_platform_without_restore_points_to_the_system_trash(cx: &mut TestAppContext) {
    let parent = tempfile::tempdir().unwrap();
    let bin = Arc::new(FakeTrash::no_restore());
    let (model, root) = open_v1(cx, parent.path(), bin.clone());
    apply(cx, &model, V2, true).expect("applied");
    let got = notices(cx, &model);
    let [NoticeKind::Trashed { ticket, .. }] = got.as_slice() else {
        panic!("{got:?}")
    };
    send(
        cx,
        &model,
        Command::Tree(TreeOp::Restore { ticket: *ticket }),
    );
    cx.run_until_parked();
    let got = notices(cx, &model);
    let [NoticeKind::Failed { message, .. }] = got.as_slice() else {
        panic!("{got:?}")
    };
    assert!(
        message.contains("cannot be restored from here") && message.contains("system trash"),
        "{message}"
    );
    assert!(!root.join("legacy/removeLegacy.yml").exists());
    assert_eq!(bin.len(), 1, "the request stays in the platform trash");
}

#[gpui_kit::test]
fn a_collection_without_a_baseline_says_so(cx: &mut TestAppContext) {
    let dir = collection();
    let model = open(cx, &dir);
    assert!(!info(cx, &model));
    for result in [plan(cx, &model, V2, false), apply(cx, &model, V2, false)] {
        let message = result.expect_err("refused");
        assert!(message.contains("Not imported from a spec"), "{message}");
    }
}

#[gpui_kit::test]
fn unsaved_edits_block_apply_and_change_nothing(cx: &mut TestAppContext) {
    let parent = tempfile::tempdir().unwrap();
    let (model, root) = open_v1(cx, parent.path(), Arc::new(FakeTrash::new()));
    send(cx, &model, edit_url("people/getUser.yml", "http://edited/"));
    cx.run_until_parked();
    let before = fs::read_to_string(root.join("people/getUser.yml")).unwrap();
    let message = apply(cx, &model, V2, false).expect_err("blocked");
    assert!(message.contains("unsaved edits"), "{message}");
    assert_eq!(
        fs::read_to_string(root.join("people/getUser.yml")).unwrap(),
        before
    );
    assert!(
        plan(cx, &model, V2, false).is_ok(),
        "a plan is still allowed"
    );
}

#[gpui_kit::test]
fn a_broken_trash_refuses_a_prune_before_writing(cx: &mut TestAppContext) {
    let parent = tempfile::tempdir().unwrap();
    let (model, root) = open_v1(cx, parent.path(), Arc::new(FakeTrash::broken()));
    let before = fs::read_to_string(root.join("people/getUser.yml")).unwrap();
    for result in [plan(cx, &model, V2, true), apply(cx, &model, V2, true)] {
        let message = result.expect_err("refused");
        assert!(message.contains("The trash is not available"), "{message}");
    }
    assert!(root.join("legacy/removeLegacy.yml").exists());
    assert_eq!(
        fs::read_to_string(root.join("people/getUser.yml")).unwrap(),
        before,
        "nothing was written"
    );
    assert!(
        apply(cx, &model, V2, false).is_ok(),
        "no prune needs no trash"
    );
}

/// Three requests in one folder; `ONE` drops the first and last, `THREE_DOCS` changes the
/// first one's description.
const THREE: &[u8] = br#"{"openapi":"3.0.3","info":{"title":"Shop","version":"1"},"paths":{
 "/legacy/a":{"get":{"tags":["legacy"],"operationId":"legacyA","responses":{"200":{"description":"ok"}}}},
 "/legacy/b":{"get":{"tags":["legacy"],"operationId":"legacyB","responses":{"200":{"description":"ok"}}}},
 "/legacy/c":{"get":{"tags":["legacy"],"operationId":"legacyC","responses":{"200":{"description":"ok"}}}}}}"#;
const ONE: &[u8] = br#"{"openapi":"3.0.3","info":{"title":"Shop","version":"2"},"paths":{
 "/legacy/b":{"get":{"tags":["legacy"],"operationId":"legacyB","responses":{"200":{"description":"ok"}}}}}}"#;
const THREE_DOCS: &[u8] = br#"{"openapi":"3.0.3","info":{"title":"Shop","version":"3"},"paths":{
 "/legacy/a":{"get":{"tags":["legacy"],"operationId":"legacyA","description":"Now documented.","responses":{"200":{"description":"ok"}}}},
 "/legacy/b":{"get":{"tags":["legacy"],"operationId":"legacyB","responses":{"200":{"description":"ok"}}}},
 "/legacy/c":{"get":{"tags":["legacy"],"operationId":"legacyC","responses":{"200":{"description":"ok"}}}}}}"#;

/// Applies `ONE` with removal to the open `THREE` collection; the tickets, newest removal first,
/// the order an undo restores them in.
fn prune_two(cx: &mut TestAppContext, model: &Entity<CollectionModel>) -> Vec<u64> {
    apply(cx, model, ONE, true).expect("applied");
    let mut tickets: Vec<u64> = notices(cx, model)
        .into_iter()
        .filter_map(|n| match n {
            NoticeKind::Trashed { ticket, .. } => Some(ticket),
            _ => None,
        })
        .collect();
    assert_eq!(tickets.len(), 2);
    tickets.reverse();
    tickets
}

fn restore(cx: &mut TestAppContext, model: &Entity<CollectionModel>, ticket: u64) {
    send(cx, model, Command::Tree(TreeOp::Restore { ticket }));
    cx.run_until_parked();
}

#[gpui_kit::test]
fn an_undone_removal_is_refreshed_by_a_later_update_as_generated(cx: &mut TestAppContext) {
    let parent = tempfile::tempdir().unwrap();
    let (model, _) = open_spec(cx, parent.path(), Arc::new(FakeTrash::new()), THREE);
    for ticket in prune_two(cx, &model) {
        restore(cx, &model, ticket);
    }
    let got = notices(cx, &model);
    assert!(
        got.iter().all(|n| matches!(n, NoticeKind::Restored(_))),
        "{got:?}"
    );
    let s = apply(cx, &model, THREE_DOCS, false).expect("applied");
    let changed: Vec<&str> = s.changed.iter().map(|c| c.path.as_str()).collect();
    assert_eq!(changed, ["legacy/legacyA.yml"], "{s:?}");
    assert!(
        s.notes.iter().all(|n| !n.detail.contains("not generated")),
        "{:?}",
        s.notes
    );
}

#[gpui_kit::test]
fn undo_after_a_later_update_re_added_the_request_never_overwrites_it(cx: &mut TestAppContext) {
    let parent = tempfile::tempdir().unwrap();
    let bin = Arc::new(FakeTrash::new());
    let (model, root) = open_spec(cx, parent.path(), bin.clone(), THREE);
    let tickets = prune_two(cx, &model);
    let s = apply(cx, &model, THREE_DOCS, false).expect("applied");
    assert_eq!(s.added.len(), 2, "{s:?}");
    let files = ["legacy/legacyA.yml", "legacy/legacyC.yml"];
    let re_added: Vec<Vec<u8>> = files
        .iter()
        .map(|f| fs::read(root.join(f)).unwrap())
        .collect();
    for ticket in tickets {
        restore(cx, &model, ticket);
        let got = notices(cx, &model);
        assert!(
            matches!(got.as_slice(), [NoticeKind::Failed { .. }]),
            "the restore is refused: {got:?}"
        );
    }
    let now: Vec<Vec<u8>> = files
        .iter()
        .map(|f| fs::read(root.join(f)).unwrap())
        .collect();
    assert_eq!(now, re_added, "the re-added files are untouched");
    assert_eq!(bin.len(), 2, "the removed ones stay in the trash");
}
