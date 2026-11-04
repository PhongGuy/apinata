use crate::support;

use apinata_app::command::{Command, TreeOp};
use apinata_app::session::Session;
use apinata_app::tabs::TabKind;
use gpui_kit::TestAppContext;
use support::*;

fn kinds(cx: &mut TestAppContext, a: &Area) -> Vec<TabKind> {
    a.ws.read_with(cx, |w, _| {
        w.tabs().entries().iter().map(|e| e.kind.clone()).collect()
    })
}

fn open_items(
    cx: &mut TestAppContext,
    a: &Area,
    items: &[&str],
) -> Option<apinata_app::tabs::TabId> {
    let items = items.iter().map(|p| ip(p)).collect();
    a.ws.update(cx, |w, cx| w.open_runner_items(&a.root, items, cx))
}

#[gpui_kit::test]
fn the_same_selection_reuses_one_tab_whatever_its_order(cx: &mut TestAppContext) {
    let a = open_area(cx);
    let first = open_items(cx, &a, &["b.yml", "a.yml"]).expect("opens");
    let again = open_items(cx, &a, &["a.yml", "b.yml", "a.yml"]).expect("activates");
    assert_eq!(first, again);
    assert_eq!(
        kinds(cx, &a),
        [TabKind::RunSelection(vec![ip("a.yml"), ip("b.yml")])]
    );
    let other = open_items(cx, &a, &["a.yml"]).expect("a different selection");
    assert_ne!(first, other);
    assert_eq!(kinds(cx, &a).len(), 2);
}

#[gpui_kit::test]
fn nothing_opens_for_an_empty_selection_or_items_that_do_not_exist(cx: &mut TestAppContext) {
    let a = open_area(cx);
    assert_eq!(open_items(cx, &a, &[]), None);
    assert_eq!(open_items(cx, &a, &["gone.yml", "nope/x.yml"]), None);
    assert!(kinds(cx, &a).is_empty());
}

#[gpui_kit::test]
fn the_tab_follows_a_rename_and_closes_when_every_member_is_gone(cx: &mut TestAppContext) {
    let a = open_area(cx);
    open_items(cx, &a, &["a.yml", "b.yml"]).expect("opens");
    a.ws.update(cx, |w, cx| {
        w.send(
            &a.root,
            Command::Tree(TreeOp::Rename {
                item: ip("a.yml"),
                name: "Z".into(),
                rename_file: true,
            }),
            cx,
        )
    });
    cx.run_until_parked();
    let items = match kinds(cx, &a).as_slice() {
        [TabKind::RunSelection(items)] => items.clone(),
        other => panic!("{other:?}"),
    };
    assert_eq!(items.len(), 2);
    assert!(!items.contains(&ip("a.yml")));
    assert!(
        items.iter().all(|i| a.root.join(i.as_str()).exists()),
        "{items:?}"
    );
    assert!(
        items.windows(2).all(|w| w[0] < w[1]),
        "kept sorted: {items:?}"
    );
    for i in &items {
        a.ws.update(cx, |w, cx| {
            w.send(
                &a.root,
                Command::Tree(TreeOp::Delete { item: i.clone() }),
                cx,
            )
        });
    }
    cx.run_until_parked();
    assert!(kinds(cx, &a).is_empty());
}

#[gpui_kit::test]
fn a_closed_selection_tab_is_not_remembered_for_reopen(cx: &mut TestAppContext) {
    let a = open_area(cx);
    let id = open_items(cx, &a, &["a.yml", "b.yml"]).expect("opens");
    a.ws.update(cx, |w, cx| w.update_tabs(cx, |t| t.close(id)));
    assert_eq!(a.ws.update(cx, |w, cx| w.reopen_tab(cx)), None);
}

#[gpui_kit::test]
fn the_session_never_records_a_selection_tab(cx: &mut TestAppContext) {
    let col = two_requests();
    let state = tempfile::tempdir().unwrap();
    let session = session_at(cx, &state, Session::default());
    let ws = workspace(cx, &session);
    let root = col.path().to_path_buf();
    ws.update(cx, |w, cx| w.open(root.clone(), cx));
    cx.run_until_parked();
    ws.update(cx, |w, cx| {
        w.open_runner_items(&root, vec![ip("a.yml"), ip("b.yml")], cx)
    })
    .expect("opens");
    cx.run_until_parked();
    ws.update(cx, |w, cx| w.open_item(&root, ip("a.yml"), false, cx))
        .expect("a request tab");
    cx.run_until_parked();
    let saved = session_value(cx, &session);
    let tabs = saved
        .collections
        .get(&root)
        .map(|c| c.tabs.clone())
        .unwrap_or_default();
    assert_eq!(tabs, ["a.yml"], "only the request tab is saved");
}
