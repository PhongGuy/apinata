use crate::support;

use apinata_app::sidebar::marker_id;
use apinata_app::sidebar::menu::{MenuEntry, menu_entries};
use apinata_app::sidebar::model::{Expanded, Node, NodeKind, build, example_id, node_id};
use apinata_app::sidebar::{CloneSelected, DeleteSelected, RenameSelected, SidebarEvent};
use apinata_app::snapshot::SnapshotCache;
use apinata_core::Collection;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, SharedString, TestAppContext};
use std::collections::HashSet;
use support::*;

const USERS: &str = "info:\n  name: Users\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: http://x/users\nexamples:\n  - name: ok\n    response:\n      status: 200\n  - name: missing\n    response:\n      status: 404\n";

fn files() -> Vec<(&'static str, String)> {
    vec![
        ("opencollection.yml", ROOT.to_owned()),
        ("users.yml", USERS.to_owned()),
        ("plain.yml", request("Plain", "http://x/plain")),
    ]
}

fn ex(index: usize) -> SharedString {
    example_id(0, &ip("users.yml"), index)
}

fn find<'a>(nodes: &'a [Node], id: &str) -> &'a Node {
    fn go<'a>(nodes: &'a [Node], id: &str) -> Option<&'a Node> {
        nodes.iter().find_map(|n| {
            if n.id.as_ref() == id {
                Some(n)
            } else {
                go(&n.children, id)
            }
        })
    }
    go(nodes, id).unwrap_or_else(|| panic!("{id}"))
}

#[test]
fn examples_nest_under_their_request_collapsed_by_default() {
    let owned = files();
    let refs: Vec<(&str, &str)> = owned.iter().map(|(p, t)| (*p, t.as_str())).collect();
    let dir = write_tree(&refs);
    let content = SnapshotCache::new().build(&Collection::open(dir.path()).unwrap());
    let collections = [(dir.path(), "c", Some(&content))];
    let nodes = build(&collections, &[], &Expanded::new(), "");
    let users = find(&nodes, &node_id(0, &ip("users.yml")));
    assert_eq!(users.kind, NodeKind::Request);
    assert!(!users.expanded, "collapsed by default");
    let kids: Vec<(String, NodeKind, Option<usize>, String)> = users
        .children
        .iter()
        .map(|n| (n.id.to_string(), n.kind, n.example, n.name.clone()))
        .collect();
    assert_eq!(
        kids,
        [
            (
                ex(0).to_string(),
                NodeKind::Example,
                Some(0),
                "ok".to_owned()
            ),
            (
                ex(1).to_string(),
                NodeKind::Example,
                Some(1),
                "missing".to_owned()
            ),
        ]
    );
    assert!(
        find(&nodes, &node_id(0, &ip("plain.yml")))
            .children
            .is_empty()
    );
    let open = Expanded::from([(dir.path().to_path_buf(), HashSet::from([ip("users.yml")]))]);
    assert!(
        find(
            &build(&collections, &[], &open, ""),
            &node_id(0, &ip("users.yml"))
        )
        .expanded
    );
    assert_eq!(
        menu_entries(NodeKind::Example),
        [MenuEntry::Rename, MenuEntry::Delete]
    );
}

fn expand(cx: &mut TestAppContext, h: &SidebarHarness) {
    let toggle = marker_id("examples-toggle", &row("users.yml"));
    cx.update_window(h.win, |_, w, cx| w.click(toggle, cx))
        .unwrap();
    frame(cx, h.win);
}

fn choose_on(cx: &mut TestAppContext, h: &SidebarHarness, id: SharedString, entry: MenuEntry) {
    cx.update_window(h.win, |_, w, cx| w.right_click(id, cx))
        .unwrap();
    frame(cx, h.win);
    cx.update_window(h.win, |_, w, cx| {
        let ix = menu_index(w, entry.label());
        w.within("popup-menu").click(ix, cx)
    })
    .unwrap();
    cx.run_until_parked();
}

#[gpui_kit::test]
fn a_request_click_opens_without_toggling_its_examples(cx: &mut TestAppContext) {
    let h = open_sidebar_with(cx, files());
    let seen = sidebar_events(cx, &h.view);
    cx.update_window(h.win, |_, w, cx| w.click(row("users.yml"), cx))
        .unwrap();
    frame(cx, h.win);
    assert!(
        !shown(cx, h.win, ex(0)),
        "the click did not expand the examples"
    );
    assert!(matches!(
        seen.borrow().last(),
        Some(SidebarEvent::OpenItem { .. })
    ));
    expand(cx, &h);
    assert!(shown(cx, h.win, ex(0)) && shown(cx, h.win, ex(1)));
    cx.update_window(h.win, |_, w, cx| w.click(ex(1), cx))
        .unwrap();
    frame(cx, h.win);
    assert_eq!(
        seen.borrow().last(),
        Some(&SidebarEvent::OpenExample {
            root: h.root.clone(),
            path: ip("users.yml"),
            index: 1,
            permanent: false
        })
    );
    expand(cx, &h);
    assert!(!shown(cx, h.win, ex(0)), "the toggle collapses again");
}

#[gpui_kit::test]
fn the_example_menu_renames_inline_and_deletes_after_asking(cx: &mut TestAppContext) {
    let h = open_sidebar_with(cx, files());
    expand(cx, &h);
    choose_on(cx, &h, ex(1), MenuEntry::Rename);
    cx.update_window(h.win, |_, w, cx| {
        w.input("not found", cx);
        w.press("enter", cx);
    })
    .unwrap();
    wait_for(cx, "the rename", |_| {
        std::fs::read_to_string(h.root.join("users.yml"))
            .unwrap()
            .contains("- name: not found\n")
    });
    choose_on(cx, &h, ex(0), MenuEntry::Delete);
    answer_confirm(cx, "confirm-cancel");
    cx.run_until_parked();
    assert!(
        std::fs::read_to_string(h.root.join("users.yml"))
            .unwrap()
            .contains("- name: ok\n")
    );
    choose_on(cx, &h, ex(0), MenuEntry::Delete);
    answer_confirm(cx, "confirm-delete");
    wait_for(cx, "the delete", |_| {
        !std::fs::read_to_string(h.root.join("users.yml"))
            .unwrap()
            .contains("- name: ok\n")
    });
    frame(cx, h.win);
    assert!(
        shown(cx, h.win, ex(0)),
        "the renamed example moved up to index 0"
    );
    assert!(!shown(cx, h.win, ex(1)));
}

fn select(cx: &mut TestAppContext, h: &SidebarHarness, id: SharedString) {
    cx.update_window(h.win, |_, w, cx| w.click(id, cx)).unwrap();
    cx.run_until_parked();
}

fn disk(h: &SidebarHarness) -> String {
    std::fs::read_to_string(h.root.join("users.yml")).unwrap()
}

#[gpui_kit::test]
fn selected_example_actions_act_on_the_example_not_the_request(cx: &mut TestAppContext) {
    let h = open_sidebar_with(cx, files());
    expand(cx, &h);
    select(cx, &h, ex(1));
    let yml = |h: &SidebarHarness| {
        std::fs::read_dir(&h.root)
            .unwrap()
            .filter(|e| {
                e.as_ref()
                    .unwrap()
                    .path()
                    .extension()
                    .is_some_and(|x| x == "yml")
            })
            .count()
    };
    let before = yml(&h);
    cx.update_window(h.win, |_, w, cx| {
        h.view
            .update(cx, |s, cx| s.clone_selected(&CloneSelected, w, cx))
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(yml(&h), before, "an example is not duplicated as a request");

    cx.update_window(h.win, |_, w, cx| {
        h.view
            .update(cx, |s, cx| s.rename_selected(&RenameSelected, w, cx))
    })
    .unwrap();
    frame(cx, h.win);
    assert!(
        shown(cx, h.win, marker_id("rename", &ex(1))),
        "the input is on the example row"
    );
    assert!(!shown(cx, h.win, marker_id("rename", &row("users.yml"))));
    cx.update_window(h.win, |_, w, cx| {
        w.input("gone", cx);
        w.press("enter", cx);
    })
    .unwrap();
    wait_for(cx, "the example rename", |_| {
        disk(&h).contains("- name: gone\n")
    });
    assert!(
        disk(&h).contains("  name: Users\n"),
        "the request keeps its name"
    );
    assert!(
        h.root.join("users.yml").exists(),
        "the request file was not renamed"
    );

    select(cx, &h, ex(0));
    cx.update_window(h.win, |_, w, cx| {
        h.view
            .update(cx, |s, cx| s.delete_selected(&DeleteSelected, w, cx))
    })
    .unwrap();
    cx.run_until_parked();
    assert!(confirm_open(cx));
    answer_confirm(cx, "confirm-delete");
    wait_for(cx, "the example delete", |_| {
        !disk(&h).contains("- name: ok\n")
    });
    assert!(h.root.join("users.yml").exists(), "the request survives");
}

#[gpui_kit::test]
fn a_request_with_examples_renames_on_its_own_row(cx: &mut TestAppContext) {
    let h = open_sidebar_with(cx, files());
    expand(cx, &h);
    cx.update_window(h.win, |_, w, cx| w.right_click(row("users.yml"), cx))
        .unwrap();
    frame(cx, h.win);
    cx.update_window(h.win, |_, w, cx| {
        let ix = menu_index(w, MenuEntry::Rename.label());
        w.within("popup-menu").click(ix, cx)
    })
    .unwrap();
    frame(cx, h.win);
    assert_eq!(
        h.view.read_with(cx, |s, _| s.renaming()),
        Some((h.root.clone(), ip("users.yml")))
    );
    assert!(shown(cx, h.win, marker_id("rename", &row("users.yml"))));
    assert!(!shown(cx, h.win, marker_id("rename", &ex(0))));
    assert!(!shown(cx, h.win, marker_id("rename", &ex(1))));
}

fn selected_id(cx: &mut TestAppContext, h: &SidebarHarness) -> Option<SharedString> {
    h.view.read_with(cx, |s, cx| {
        s.tree().read(cx).selected_item().map(|i| i.id.clone())
    })
}

#[gpui_kit::test]
fn a_selected_example_stays_selected_across_rebuilds(cx: &mut TestAppContext) {
    let h = open_sidebar_with(cx, files());
    expand(cx, &h);
    select(cx, &h, ex(1));
    assert_eq!(selected_id(cx, &h), Some(ex(1)));
    let op = apinata_app::command::TreeOp::RenameExample {
        item: ip("users.yml"),
        index: 1,
        name: "gone".into(),
    };
    h.ws.update(cx, |w, cx| {
        w.send(&h.root, apinata_app::command::Command::Tree(op), cx)
    });
    wait_for(cx, "the example rename", |_| {
        disk(&h).contains("- name: gone\n")
    });
    frame(cx, h.win);
    assert!(shown(cx, h.win, ex(1)));
    assert_eq!(
        selected_id(cx, &h),
        Some(ex(1)),
        "the renamed example keeps the selection"
    );
}

const THREE: &str = "info:\n  name: Users\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: http://x/users\nexamples:\n  - name: a\n    response:\n      status: 200\n  - name: b\n    response:\n      status: 201\n  - name: c\n    response:\n      status: 202\n";

// Opens a request with examples a, b and c, expanded, with `selected` chosen.
fn three(cx: &mut TestAppContext, selected: usize) -> SidebarHarness {
    let h = open_sidebar_with(
        cx,
        vec![
            ("opencollection.yml", ROOT.to_owned()),
            ("users.yml", THREE.to_owned()),
        ],
    );
    expand(cx, &h);
    select(cx, &h, ex(selected));
    h
}

fn delete_example(cx: &mut TestAppContext, h: &SidebarHarness, index: usize, gone: &str) {
    let op = apinata_app::command::TreeOp::DeleteExample {
        item: ip("users.yml"),
        index,
    };
    h.ws.update(cx, |w, cx| {
        w.send(&h.root, apinata_app::command::Command::Tree(op), cx)
    });
    let line = format!("- name: {gone}\n");
    wait_for(cx, "the example delete", |_| !disk(h).contains(&line));
    frame(cx, h.win);
}

#[gpui_kit::test]
fn deleting_an_earlier_example_keeps_the_same_example_selected(cx: &mut TestAppContext) {
    let h = three(cx, 1);
    delete_example(cx, &h, 0, "a");
    assert_eq!(selected_id(cx, &h), Some(ex(0)), "b moved up to index 0");
}

#[gpui_kit::test]
fn deleting_the_selected_example_selects_its_request(cx: &mut TestAppContext) {
    let h = three(cx, 1);
    delete_example(cx, &h, 1, "b");
    assert_eq!(selected_id(cx, &h), Some(row("users.yml")));
}

#[gpui_kit::test]
fn deleting_a_later_example_leaves_the_selection(cx: &mut TestAppContext) {
    let h = three(cx, 0);
    delete_example(cx, &h, 1, "b");
    assert_eq!(selected_id(cx, &h), Some(ex(0)));
}

#[gpui_kit::test]
fn deleting_an_example_cancels_a_rename_of_a_later_one(cx: &mut TestAppContext) {
    let h = three(cx, 2);
    cx.update_window(h.win, |_, w, cx| {
        h.view
            .update(cx, |s, cx| s.rename_selected(&RenameSelected, w, cx))
    })
    .unwrap();
    frame(cx, h.win);
    assert!(shown(cx, h.win, marker_id("rename", &ex(2))));
    delete_example(cx, &h, 0, "a");
    assert_eq!(h.view.read_with(cx, |s, _| s.renaming()), None);
}

#[gpui_kit::test]
fn deleting_a_later_example_keeps_a_rename_of_an_earlier_one(cx: &mut TestAppContext) {
    let h = three(cx, 0);
    cx.update_window(h.win, |_, w, cx| {
        h.view
            .update(cx, |s, cx| s.rename_selected(&RenameSelected, w, cx))
    })
    .unwrap();
    frame(cx, h.win);
    delete_example(cx, &h, 2, "c");
    assert!(shown(cx, h.win, marker_id("rename", &ex(0))));
}

// Shift+Tab from any control inside a row returns to the tree, not past it to History; the
// examples chevron is a row control that no collection-level handle tracks.
#[gpui_kit::test]
fn shift_tab_from_the_examples_chevron_lands_on_the_tree(cx: &mut TestAppContext) {
    let h = open_sidebar_with(cx, files());
    cx.update_window(h.win, |_, w, _| w.activate_window())
        .unwrap();
    focus_tree(cx, &h);
    frame(cx, h.win);
    let tree_handle = cx.update_window(h.win, |_, w, cx| w.focused(cx)).unwrap();
    tab_to(cx, h.win, marker_id("examples-toggle", &row("users.yml")));
    cx.simulate_keystrokes(h.win, "shift-tab");
    frame(cx, h.win);
    let now = cx.update_window(h.win, |_, w, cx| w.focused(cx)).unwrap();
    assert_eq!(now, tree_handle, "shift+tab returns to the tree");
}
