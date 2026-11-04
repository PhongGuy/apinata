use crate::support;

use apinata_app::command::{Command, Edit, NoticeKind, Target, TreeOp};
use apinata_app::model::CollectionModel;
use apinata_core::ItemPath;
use gpui_kit::{Entity, TestAppContext};
use support::*;

fn tree() -> tempfile::TempDir {
    write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", "http://x/0")),
        ("users/folder.yml", &folder("users")),
        ("users/u.yml", &request("u", "http://x/0")),
    ])
}

fn create(cx: &mut TestAppContext, model: &Entity<CollectionModel>, parent: ItemPath) -> ItemPath {
    send(
        cx,
        model,
        Command::Tree(TreeOp::CreateDraft {
            parent,
            kind: apinata_core::RequestKind::Http,
        }),
    );
    cx.run_until_parked();
    let got = notices(cx, model);
    let [NoticeKind::Created(path)] = got.as_slice() else {
        panic!("{got:?}")
    };
    path.clone()
}

fn set_url(cx: &mut TestAppContext, model: &Entity<CollectionModel>, d: &ItemPath, url: &str) {
    let edit = Command::Edit {
        target: Target::Request(d.clone()),
        edit: Edit::Url(url.into()),
    };
    send(cx, model, edit);
    cx.run_until_parked();
    notices(cx, model);
}

fn on_disk(dir: &tempfile::TempDir) -> Vec<String> {
    let mut out: Vec<String> = walk(dir.path());
    out.sort();
    out
}

fn walk(dir: &std::path::Path) -> Vec<String> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            out.extend(walk(&path));
        } else {
            out.push(path.to_string_lossy().into_owned());
        }
    }
    out
}

#[gpui_kit::test]
fn a_draft_is_a_request_but_not_a_row(cx: &mut TestAppContext) {
    let dir = tree();
    let model = open(cx, &dir);
    let d = create(cx, &model, ip("users"));
    assert!(d.is_draft(), "{d:?}");
    let s = snap(cx, &model);
    let view = &s.content.requests[&d];
    assert!(view.draft);
    assert!(!view.dirty);
    assert_eq!(view.name, "Untitled request");
    assert!(s.content.rows.iter().all(|r| r.path != d));
    assert!(!s.content.requests[&ip("a.yml")].draft);
}

#[gpui_kit::test]
fn save_refuses_a_draft_and_save_as_moves_it_to_a_file(cx: &mut TestAppContext) {
    let dir = tree();
    let model = open(cx, &dir);
    let before = on_disk(&dir);
    let d = create(cx, &model, ItemPath::root());
    set_url(cx, &model, &d, "http://x/1");
    assert!(snap(cx, &model).content.requests[&d].dirty);
    assert_eq!(
        apinata_app::closing::dirty_targets(&snap(cx, &model).content),
        vec![Target::Request(d.clone())]
    );
    send(cx, &model, Command::Save(Target::Request(d.clone())));
    cx.run_until_parked();
    let got = notices(cx, &model);
    assert!(
        matches!(got.as_slice(), [NoticeKind::Failed { .. }]),
        "{got:?}"
    );
    assert_eq!(on_disk(&dir), before, "a plain save writes nothing");
    let op = TreeOp::SaveDraft {
        draft: d.clone(),
        parent: ItemPath::root(),
        name: "List".into(),
    };
    send(cx, &model, Command::Tree(op));
    cx.run_until_parked();
    let got = notices(cx, &model);
    let [NoticeKind::Moved { from, to }] = got.as_slice() else {
        panic!("{got:?}")
    };
    assert_eq!((from, to), (&d, &ip("List.yml")));
    let s = snap(cx, &model);
    assert!(!s.content.requests.contains_key(&d));
    let saved = &s.content.requests[&ip("List.yml")];
    assert!(!saved.dirty && !saved.draft);
    assert!(s.content.rows.iter().any(|r| r.path == ip("List.yml")));
    let disk = std::fs::read_to_string(dir.path().join("List.yml")).unwrap();
    assert!(disk.contains("http://x/1"), "{disk}");
}

#[gpui_kit::test]
fn discard_leaves_nothing_behind(cx: &mut TestAppContext) {
    let dir = tree();
    let model = open(cx, &dir);
    let before = on_disk(&dir);
    let d = create(cx, &model, ip("users"));
    set_url(cx, &model, &d, "http://x/1");
    send(cx, &model, Command::Revert(Target::Request(d.clone())));
    cx.run_until_parked();
    let got = notices(cx, &model);
    assert!(
        !got.iter().any(|n| matches!(n, NoticeKind::Failed { .. })),
        "{got:?}"
    );
    assert!(
        !snap(cx, &model).content.requests[&d].dirty,
        "reverted in memory"
    );
    send(
        cx,
        &model,
        Command::Tree(TreeOp::DiscardDraft { draft: d.clone() }),
    );
    cx.run_until_parked();
    assert!(notices(cx, &model).is_empty());
    assert!(!snap(cx, &model).content.requests.contains_key(&d));
    assert_eq!(on_disk(&dir), before);
}

#[gpui_kit::test]
fn save_as_into_an_unknown_folder_fails_and_keeps_the_draft(cx: &mut TestAppContext) {
    let dir = tree();
    let model = open(cx, &dir);
    let before = on_disk(&dir);
    let d = create(cx, &model, ItemPath::root());
    let op = TreeOp::SaveDraft {
        draft: d.clone(),
        parent: ip("missing"),
        name: "List".into(),
    };
    send(cx, &model, Command::Tree(op));
    cx.run_until_parked();
    let got = notices(cx, &model);
    assert!(
        matches!(got.as_slice(), [NoticeKind::Failed { .. }]),
        "{got:?}"
    );
    assert!(snap(cx, &model).content.requests[&d].draft);
    assert_eq!(on_disk(&dir), before);
}
