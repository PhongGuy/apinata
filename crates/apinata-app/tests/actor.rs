use crate::support;

use apinata_app::command::{Command, Edit, Field, NoticeKind, Target};
use apinata_app::model::CollectionModel;
use apinata_core::MemoryStore;
use gpui_kit::TestAppContext;
use std::sync::Arc;
use support::*;

fn one() -> tempfile::TempDir {
    write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", "http://x/0")),
    ])
}

#[gpui_kit::test]
fn a_burst_of_edits_costs_one_snapshot(cx: &mut TestAppContext) {
    let dir = one();
    let model = open(cx, &dir);
    assert_eq!(model.read_with(cx, |m, _| m.snapshots_received()), 1);
    for u in ["http://x/1", "http://x/2", "http://x/3"] {
        send(cx, &model, edit_url("a.yml", u));
    }
    assert_eq!(model.read_with(cx, |m, _| m.pending()), 3);
    cx.run_until_parked();
    let s = snap(cx, &model);
    assert_eq!((s.seq, s.acked), (2, 3));
    assert_eq!(url(&s, "a.yml"), "http://x/3");
    assert_eq!(
        model.read_with(cx, |m, _| (m.snapshots_received(), m.pending())),
        (2, 0)
    );
}

#[gpui_kit::test]
fn save_writes_the_draft_in_command_order(cx: &mut TestAppContext) {
    let dir = one();
    let model = open(cx, &dir);
    send(cx, &model, edit_url("a.yml", "http://x/saved"));
    send(cx, &model, Command::Save(Target::Request(ip("a.yml"))));
    send(cx, &model, edit_url("a.yml", "http://x/after"));
    cx.run_until_parked();
    let on_disk = std::fs::read_to_string(dir.path().join("a.yml")).unwrap();
    assert!(on_disk.contains("http://x/saved"));
    let s = snap(cx, &model);
    assert_eq!(url(&s, "a.yml"), "http://x/after");
    assert!(s.content.requests[&ip("a.yml")].dirty);
}

#[gpui_kit::test]
fn url_and_params_are_in_flight_together(cx: &mut TestAppContext) {
    let dir = one();
    let model = open(cx, &dir);
    let a = Target::Request(ip("a.yml"));
    send(cx, &model, edit_url("a.yml", "http://x/1?q=1"));
    let (unit, method) = model.read_with(cx, |m, _| {
        (
            m.in_flight(&a, Field::UrlParams),
            m.in_flight(&a, Field::Method),
        )
    });
    assert!(unit && !method);
    cx.run_until_parked();
    assert!(!model.read_with(cx, |m, _| m.in_flight(&a, Field::UrlParams)));
    assert_eq!(
        snap(cx, &model).content.requests[&ip("a.yml")].params.len(),
        1
    );
}

#[gpui_kit::test]
fn an_unknown_target_is_a_notice_and_the_actor_carries_on(cx: &mut TestAppContext) {
    let dir = one();
    let model = open(cx, &dir);
    send(cx, &model, edit_url("nope.yml", "http://x/1"));
    send(
        cx,
        &model,
        Command::Edit {
            target: Target::Collection,
            edit: Edit::Url("http://x".into()),
        },
    );
    send(cx, &model, edit_url("a.yml", "http://x/2"));
    cx.run_until_parked();
    assert_eq!(
        notices(cx, &model),
        [
            NoticeKind::NotFound(Target::Request(ip("nope.yml"))),
            NoticeKind::Unsupported {
                target: Target::Collection,
                field: Field::UrlParams
            },
        ]
    );
    assert_eq!(url(&snap(cx, &model), "a.yml"), "http://x/2");
}

#[gpui_kit::test]
fn revert_drops_the_draft(cx: &mut TestAppContext) {
    let dir = one();
    let model = open(cx, &dir);
    send(cx, &model, edit_url("a.yml", "http://x/draft"));
    send(cx, &model, Command::Revert(Target::Request(ip("a.yml"))));
    cx.run_until_parked();
    let s = snap(cx, &model);
    assert_eq!(url(&s, "a.yml"), "http://x/0");
    assert!(!s.content.requests[&ip("a.yml")].dirty);
}

#[gpui_kit::test]
fn a_missing_root_reports_open_failure(cx: &mut TestAppContext) {
    let model = cx.update(|cx| {
        CollectionModel::open(
            "/nonexistent/apinata-root".into(),
            Arc::new(MemoryStore::new()),
            cx,
        )
    });
    cx.run_until_parked();
    assert!(model.read_with(cx, |m, _| m.open_error().is_some()
        && m.snapshot().is_none()));
    send(cx, &model, edit_url("a.yml", "http://x/1"));
    assert_eq!(model.read_with(cx, |m, _| m.pending()), 0);
}

#[gpui_kit::test]
fn an_invalid_method_is_refused_and_a_custom_one_is_stored_uppercased(cx: &mut TestAppContext) {
    let dir = one();
    let model = open(cx, &dir);
    let method = |m: &str| Command::Edit {
        target: Target::Request(ip("a.yml")),
        edit: Edit::Method(m.into()),
    };
    send(cx, &model, method("GE T"));
    cx.run_until_parked();
    let got = notices(cx, &model);
    assert!(
        matches!(got.as_slice(), [NoticeKind::Failed { target: Some(Target::Request(p)), message, .. }]
            if *p == ip("a.yml") && message.contains("GE T")),
        "{got:?}"
    );
    let kept = snap(cx, &model).content.requests[&ip("a.yml")]
        .method
        .clone();
    assert_eq!(kept, "GET");
    send(cx, &model, method("propfind"));
    cx.run_until_parked();
    assert_eq!(
        snap(cx, &model).content.requests[&ip("a.yml")].method,
        "PROPFIND"
    );
}
