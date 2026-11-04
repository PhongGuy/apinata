use crate::support;

use apinata_app::command::{Command, Edit, Target, TreeOp};
use apinata_app::tab_area::TabArea;
use apinata_app::tabs::{TabKind, Tabs};
use apinata_app::workspace::{Workspace, WorkspaceEvent};
use gpui_kit::{AppContext as _, Entity, TestAppContext};
use std::path::Path;
use support::*;

#[test]
fn a_move_onto_a_member_dedups_and_a_move_past_one_resorts() {
    let root = Path::new("/c");
    let mut tabs = Tabs::new();
    tabs.open(
        root,
        TabKind::RunSelection(vec![ip("a.yml"), ip("z.yml")]),
        false,
    );
    tabs.follow_move(root, &ip("z.yml"), &ip("a.yml"));
    assert_eq!(
        tabs.entries()[0].kind,
        TabKind::RunSelection(vec![ip("a.yml")])
    );

    let mut tabs = Tabs::new();
    tabs.open(
        root,
        TabKind::RunSelection(vec![ip("b.yml"), ip("z.yml")]),
        false,
    );
    tabs.follow_move(root, &ip("z.yml"), &ip("a.yml"));
    assert_eq!(
        tabs.entries()[0].kind,
        TabKind::RunSelection(vec![ip("a.yml"), ip("b.yml")])
    );
}

#[gpui_kit::test]
fn closing_a_selection_reports_every_member(cx: &mut TestAppContext) {
    let a = open_area(cx);
    let seen = events(cx, &a.ws);
    a.ws.update(cx, |w, cx| {
        w.open_runner_items(&a.root, vec![ip("b.yml"), ip("a.yml")], cx)
    })
    .expect("opens");
    // Removed outside the app: the app's own delete closes its tabs without a report.
    let files = [a.root.join("a.yml"), a.root.join("b.yml")];
    for f in &files {
        std::fs::remove_file(f).unwrap();
    }
    a.ws.update(cx, |w, cx| {
        w.send(&a.root, Command::Reload(files.to_vec()), cx)
    });
    cx.run_until_parked();
    let closed = WorkspaceEvent::TabsClosed {
        root: a.root.clone(),
        items: vec![ip("a.yml"), ip("b.yml")],
        tabs: 1,
    };
    assert!(seen.borrow().contains(&closed), "{:?}", seen.borrow());
}

#[gpui_kit::test]
fn the_selection_title_counts_single_requests(cx: &mut TestAppContext) {
    let a = open_area(cx);
    for (items, label) in [
        (vec![ip("a.yml")], "Selected (1)"),
        (vec![ip("a.yml"), ip("b.yml")], "Selected (2)"),
    ] {
        a.ws.update(cx, |w, cx| w.open_runner_items(&a.root, items, cx))
            .expect("opens");
        cx.run_until_parked();
        let entry =
            a.ws.read_with(cx, |w, _| w.tabs().active().cloned())
                .expect("a tab");
        assert_eq!(a.view.read_with(cx, |v, cx| v.tab_label(&entry, cx)), label);
    }
}

#[gpui_kit::test]
fn the_selection_title_counts_the_requests_it_runs(cx: &mut TestAppContext) {
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("f/folder.yml", &folder("f")),
        ("f/one.yml", &request("one", "http://x/1")),
        ("f/two.yml", &request("two", "http://x/2")),
        ("a.yml", &request("a", "http://x/a")),
    ]);
    let (ws, root, view) = {
        let a = open_area_of(cx, col);
        (a.ws.clone(), a.root.clone(), a.view.clone())
    };
    // A folder counts as the requests inside it, so two items run three requests.
    ws.update(cx, |w, cx| {
        w.open_runner_items(&root, vec![ip("f"), ip("a.yml")], cx)
    })
    .expect("opens");
    cx.run_until_parked();
    let entry = ws
        .read_with(cx, |w, _| w.tabs().active().cloned())
        .expect("a tab");
    assert_eq!(
        view.read_with(cx, |v, cx| v.tab_label(&entry, cx)),
        "Selected (3)"
    );
}

fn delete_in_app(cx: &mut TestAppContext, a: &Area, item: &str) {
    let op = Command::Tree(TreeOp::Delete { item: ip(item) });
    a.ws.update(cx, |w, cx| w.send(&a.root, op, cx));
    cx.run_until_parked();
}

fn remove_on_disk(cx: &mut TestAppContext, a: &Area, item: &str) {
    let file = a.root.join(item);
    std::fs::remove_file(&file).unwrap();
    a.ws.update(cx, |w, cx| w.send(&a.root, Command::Reload(vec![file]), cx));
    cx.run_until_parked();
}

fn closed_events(seen: &[WorkspaceEvent]) -> Vec<&WorkspaceEvent> {
    (seen.iter())
        .filter(|e| matches!(e, WorkspaceEvent::TabsClosed { .. }))
        .collect()
}

fn selection_tabs(cx: &mut TestAppContext, a: &Area) -> usize {
    a.ws.read_with(cx, |w, _| {
        (w.tabs().entries().iter())
            .filter(|e| matches!(e.kind, TabKind::RunSelection(_)))
            .count()
    })
}

// Each delivery covers only one member, yet the app said where both went.
#[gpui_kit::test]
fn members_deleted_one_by_one_in_the_app_close_the_selection_quietly(cx: &mut TestAppContext) {
    let a = open_area(cx);
    let seen = events(cx, &a.ws);
    a.ws.update(cx, |w, cx| {
        w.open_runner_items(&a.root, vec![ip("a.yml"), ip("b.yml")], cx)
    })
    .expect("opens");
    delete_in_app(cx, &a, "a.yml");
    assert_eq!(selection_tabs(cx, &a), 1);
    delete_in_app(cx, &a, "b.yml");
    assert_eq!(selection_tabs(cx, &a), 0);
    let seen = seen.borrow();
    assert!(closed_events(&seen).is_empty(), "{seen:?}");
}

#[gpui_kit::test]
fn a_member_gone_on_disk_before_an_app_delete_stays_quiet(cx: &mut TestAppContext) {
    let a = open_area(cx);
    let seen = events(cx, &a.ws);
    a.ws.update(cx, |w, cx| {
        w.open_runner_items(&a.root, vec![ip("a.yml"), ip("b.yml")], cx)
    })
    .expect("opens");
    remove_on_disk(cx, &a, "a.yml");
    assert_eq!(selection_tabs(cx, &a), 1);
    delete_in_app(cx, &a, "b.yml");
    assert_eq!(selection_tabs(cx, &a), 0);
    let seen = seen.borrow();
    assert!(closed_events(&seen).is_empty(), "{seen:?}");
}

// The last member left without the app knowing, so the toast still explains the close.
#[gpui_kit::test]
fn a_last_member_removed_on_disk_still_reports(cx: &mut TestAppContext) {
    let a = open_area(cx);
    let seen = events(cx, &a.ws);
    a.ws.update(cx, |w, cx| {
        w.open_runner_items(&a.root, vec![ip("a.yml"), ip("b.yml")], cx)
    })
    .expect("opens");
    delete_in_app(cx, &a, "a.yml");
    remove_on_disk(cx, &a, "b.yml");
    assert_eq!(selection_tabs(cx, &a), 0);
    let closed = WorkspaceEvent::TabsClosed {
        root: a.root.clone(),
        items: vec![ip("a.yml"), ip("b.yml")],
        tabs: 1,
    };
    assert_eq!(closed_events(&seen.borrow()), vec![&closed]);
}

// Another collection's delivery must not forget what this one already lost.
#[gpui_kit::test]
fn another_roots_delivery_keeps_what_the_selection_lost(cx: &mut TestAppContext) {
    let a = open_area(cx);
    let other = two_requests();
    let other_root = other.path().to_path_buf();
    a.ws.update(cx, |w, cx| w.open(other_root.clone(), cx));
    cx.run_until_parked();
    let seen = events(cx, &a.ws);
    a.ws.update(cx, |w, cx| {
        w.open_runner_items(&a.root, vec![ip("a.yml"), ip("b.yml")], cx)
    })
    .expect("opens");
    delete_in_app(cx, &a, "a.yml");
    let op = Command::Tree(TreeOp::Delete { item: ip("a.yml") });
    a.ws.update(cx, |w, cx| w.send(&other_root, op, cx))
        .expect("sent");
    cx.run_until_parked();
    assert!(!other_root.join("a.yml").exists());
    delete_in_app(cx, &a, "b.yml");
    assert_eq!(selection_tabs(cx, &a), 0);
    let seen = seen.borrow();
    assert!(closed_events(&seen).is_empty(), "{seen:?}");
    // Only the first delete changed what the selection holds; the last one closed it.
    let changed = (seen.iter())
        .filter(|e| matches!(e, WorkspaceEvent::SelectionChanged(_)))
        .count();
    assert_eq!(changed, 1, "{seen:?}");
}

// Undo brings a member back, so a later close cannot count it as gone earlier.
#[gpui_kit::test]
fn a_member_restored_by_undo_counts_as_present_again(cx: &mut TestAppContext) {
    use std::sync::Arc;
    init(cx);
    let col = two_requests();
    let root = col.path().to_path_buf();
    let state = tempfile::tempdir().unwrap();
    let session = session_at(cx, &state, apinata_app::session::Session::default());
    let ws = workspace(cx, &session);
    ws.update(cx, |w, _| {
        w.set_trash(Arc::new(support::fake_trash::FakeTrash::new()))
    });
    ws.update(cx, |w, cx| w.open(root.clone(), cx));
    cx.run_until_parked();
    let seen = events(cx, &ws);
    ws.update(cx, |w, cx| {
        w.open_runner_items(&root, vec![ip("a.yml"), ip("b.yml")], cx)
    })
    .expect("opens");
    let trash = Command::Tree(TreeOp::Trash { item: ip("a.yml") });
    let ticket = ws
        .update(cx, |w, cx| w.send(&root, trash, cx))
        .expect("sent");
    cx.run_until_parked();
    let restore = Command::Tree(TreeOp::Restore { ticket });
    ws.update(cx, |w, cx| w.send(&root, restore, cx));
    cx.run_until_parked();
    assert!(root.join("a.yml").exists());
    // Both commands queue before the actor runs, so one snapshot carries both removals.
    let file = root.join("a.yml");
    std::fs::remove_file(&file).unwrap();
    ws.update(cx, |w, cx| {
        w.send(&root, Command::Reload(vec![file]), cx);
        w.send(
            &root,
            Command::Tree(TreeOp::Delete { item: ip("b.yml") }),
            cx,
        );
    });
    cx.run_until_parked();
    let selections = ws.read_with(cx, |w, _| {
        (w.tabs().entries().iter())
            .filter(|e| matches!(e.kind, TabKind::RunSelection(_)))
            .count()
    });
    assert_eq!(selections, 0);
    let closed = WorkspaceEvent::TabsClosed {
        root: root.clone(),
        items: vec![ip("a.yml"), ip("b.yml")],
        tabs: 1,
    };
    assert_eq!(closed_events(&seen.borrow()), vec![&closed]);
}

// The plan the selection tab's body lists, by item path, once its runner view exists.
fn body_rows(
    cx: &mut TestAppContext,
    ws: &Entity<Workspace>,
    view: &Entity<TabArea>,
) -> Vec<String> {
    let id = ws
        .read_with(cx, |w, _| w.tabs().active().map(|e| e.id))
        .expect("a tab");
    let runner = view
        .read_with(cx, |v, _| v.runner(id).cloned())
        .expect("a runner view");
    let model = runner.read_with(cx, |r, _| r.model().clone());
    model.read_with(cx, |m, _| {
        (m.preview().iter())
            .map(|r| r.path.as_str().to_owned())
            .collect()
    })
}

#[gpui_kit::test]
fn a_member_deleted_in_the_app_leaves_the_selection_body(cx: &mut TestAppContext) {
    http_ready(cx);
    let a = open_area(cx);
    a.ws.update(cx, |w, cx| {
        w.open_runner_items(&a.root, vec![ip("a.yml"), ip("b.yml")], cx)
    })
    .expect("opens");
    wait_real(cx, |cx| body_rows(cx, &a.ws, &a.view).len() == 2);
    delete_in_app(cx, &a, "a.yml");
    wait_real(cx, |cx| body_rows(cx, &a.ws, &a.view) == ["b.yml"]);
}

#[gpui_kit::test]
fn a_member_restored_by_undo_returns_to_the_selection_body(cx: &mut TestAppContext) {
    use std::sync::Arc;
    http_ready(cx);
    support::init(cx);
    let col = two_requests();
    let root = col.path().to_path_buf();
    let state = tempfile::tempdir().unwrap();
    let session = session_at(cx, &state, apinata_app::session::Session::default());
    let ws = workspace(cx, &session);
    // A collection keeps the trash it opened with, so the fake goes in first.
    ws.update(cx, |w, _| {
        w.set_trash(Arc::new(support::fake_trash::FakeTrash::new()))
    });
    ws.update(cx, |w, cx| w.open(root.clone(), cx));
    cx.run_until_parked();
    let shared = ws.clone();
    let (_win, view) = cx
        .update(|cx| {
            gpui_kit::open_window(gpui_kit::WindowOptions::default(), cx, |w, cx| {
                cx.new(|cx| TabArea::new(shared, w, cx))
            })
        })
        .unwrap();
    ws.update(cx, |w, cx| {
        w.open_runner_items(&root, vec![ip("a.yml"), ip("b.yml")], cx)
    })
    .expect("opens");
    wait_real(cx, |cx| body_rows(cx, &ws, &view).len() == 2);
    let trash = Command::Tree(TreeOp::Trash { item: ip("a.yml") });
    let ticket = ws
        .update(cx, |w, cx| w.send(&root, trash, cx))
        .expect("sent");
    wait_real(cx, |cx| body_rows(cx, &ws, &view) == ["b.yml"]);
    let restore = Command::Tree(TreeOp::Restore { ticket });
    ws.update(cx, |w, cx| w.send(&root, restore, cx));
    wait_real(cx, |cx| body_rows(cx, &ws, &view) == ["a.yml", "b.yml"]);
}

// A collection whose folder `f` holds two requests, open in a tab area. A collection keeps
// the trash it opened with, so the fake goes in before the open.
fn folder_area(cx: &mut TestAppContext) -> Area {
    area_of(
        cx,
        &[
            ("opencollection.yml", ROOT),
            ("f/a.yml", &request("a", "http://x/0")),
            ("f/b.yml", &request("b", "http://x/b")),
        ],
    )
}

// A collection of `files`, open in a tab area.
fn area_of(cx: &mut TestAppContext, files: &[(&str, &str)]) -> Area {
    support::init(cx);
    let col = write_tree(files);
    let root = col.path().to_path_buf();
    let state = tempfile::tempdir().unwrap();
    let session = session_at(cx, &state, apinata_app::session::Session::default());
    let ws = workspace(cx, &session);
    let trash = std::sync::Arc::new(support::fake_trash::FakeTrash::new());
    ws.update(cx, |w, _| w.set_trash(trash));
    ws.update(cx, |w, cx| w.open(root.clone(), cx));
    cx.run_until_parked();
    let shared = ws.clone();
    let (win, view) = cx
        .update(|cx| {
            gpui_kit::open_window(gpui_kit::WindowOptions::default(), cx, |w, cx| {
                cx.new(|cx| TabArea::new(shared, w, cx))
            })
        })
        .unwrap();
    cx.run_until_parked();
    Area {
        col,
        state,
        ws,
        root,
        win,
        view,
    }
}

#[gpui_kit::test]
fn a_request_deleted_in_the_app_leaves_its_folder_runner_body(cx: &mut TestAppContext) {
    http_ready(cx);
    let a = folder_area(cx);
    a.ws.update(cx, |w, cx| w.open_runner(&a.root, ip("f"), cx))
        .expect("opens");
    wait_real(cx, |cx| body_rows(cx, &a.ws, &a.view).len() == 2);
    delete_in_app(cx, &a, "f/a.yml");
    wait_real(cx, |cx| body_rows(cx, &a.ws, &a.view) == ["f/b.yml"]);
}

#[gpui_kit::test]
fn a_request_restored_by_undo_returns_to_its_folder_runner_body(cx: &mut TestAppContext) {
    http_ready(cx);
    let a = folder_area(cx);
    a.ws.update(cx, |w, cx| w.open_runner(&a.root, ip("f"), cx))
        .expect("opens");
    wait_real(cx, |cx| body_rows(cx, &a.ws, &a.view).len() == 2);
    let trash = Command::Tree(TreeOp::Trash {
        item: ip("f/a.yml"),
    });
    let ticket =
        a.ws.update(cx, |w, cx| w.send(&a.root, trash, cx))
            .expect("sent");
    wait_real(cx, |cx| body_rows(cx, &a.ws, &a.view) == ["f/b.yml"]);
    let restore = Command::Tree(TreeOp::Restore { ticket });
    a.ws.update(cx, |w, cx| w.send(&a.root, restore, cx));
    wait_real(cx, |cx| {
        body_rows(cx, &a.ws, &a.view) == ["f/a.yml", "f/b.yml"]
    });
}

// The active runner tab's planned rows as (path, name, method).
fn body_shown(cx: &mut TestAppContext, a: &Area) -> Vec<(String, String, String)> {
    let id = a.ws.read_with(cx, |w, _| w.tabs().active().map(|e| e.id));
    let runner = a
        .view
        .read_with(cx, |v, _| v.runner(id.expect("a tab")).cloned());
    let model = runner
        .expect("a runner view")
        .read_with(cx, |r, _| r.model().clone());
    model.read_with(cx, |m, _| {
        (m.preview().iter())
            .map(|r| (r.path.as_str().to_owned(), r.name.clone(), r.method.clone()))
            .collect()
    })
}

fn open_folder_runner(cx: &mut TestAppContext, a: &Area) -> apinata_app::tabs::TabId {
    let id = a.ws.update(cx, |w, cx| w.open_runner(&a.root, ip("f"), cx));
    wait_real(cx, |cx| body_shown(cx, a).len() == 2);
    id.expect("opens")
}

// Rewrites `item` under `a`'s root on disk and reloads it.
fn rewrite(cx: &mut TestAppContext, a: &Area, item: &str, text: &str) {
    let file = a.root.join(item);
    std::fs::write(&file, text).unwrap();
    a.ws.update(cx, |w, cx| w.send(&a.root, Command::Reload(vec![file]), cx));
    cx.run_until_parked();
}

fn replans(seen: &[WorkspaceEvent], id: apinata_app::tabs::TabId) -> usize {
    (seen.iter())
        .filter(|e| matches!(e, WorkspaceEvent::SelectionChanged(t) if *t == id))
        .count()
}

#[gpui_kit::test]
fn a_request_renamed_under_a_folder_runner_shows_its_new_name(cx: &mut TestAppContext) {
    http_ready(cx);
    let a = folder_area(cx);
    open_folder_runner(cx, &a);
    let op = Command::Tree(TreeOp::Rename {
        item: ip("f/a.yml"),
        name: "renamed".into(),
        rename_file: false,
    });
    a.ws.update(cx, |w, cx| w.send(&a.root, op, cx));
    wait_real(cx, |cx| body_shown(cx, &a)[0].1 == "renamed");
}

#[gpui_kit::test]
fn a_method_change_under_a_folder_runner_shows_the_new_method(cx: &mut TestAppContext) {
    http_ready(cx);
    let a = folder_area(cx);
    open_folder_runner(cx, &a);
    let text = request("a", "http://x/0").replace("method: GET", "method: POST");
    rewrite(cx, &a, "f/a.yml", &text);
    wait_real(cx, |cx| body_shown(cx, &a)[0].2 == "POST");
}

// A folder member stays on disk while a request inside it goes, so the plan must drop it.
#[gpui_kit::test]
fn a_request_deleted_inside_a_selected_folder_leaves_the_selection_body(cx: &mut TestAppContext) {
    http_ready(cx);
    let a = folder_area(cx);
    a.ws.update(cx, |w, cx| w.open_runner_items(&a.root, vec![ip("f")], cx))
        .expect("opens");
    wait_real(cx, |cx| body_shown(cx, &a).len() == 2);
    delete_in_app(cx, &a, "f/a.yml");
    wait_real(cx, |cx| body_rows(cx, &a.ws, &a.view) == ["f/b.yml"]);
}

// A change to a field the runner body does not show leaves its plan alone.
#[gpui_kit::test]
fn a_url_change_under_a_folder_runner_does_not_replan(cx: &mut TestAppContext) {
    http_ready(cx);
    let a = folder_area(cx);
    let id = open_folder_runner(cx, &a);
    rewrite(cx, &a, "f/b.yml", &request("b", "http://x/b2"));
    let seen = events(cx, &a.ws);
    rewrite(cx, &a, "f/a.yml", &request("a", "http://x/1"));
    assert_eq!(replans(&seen.borrow(), id), 0, "{:?}", seen.borrow());
}

// Another collection's delete must neither re-plan this runner nor make it forget what it
// planned, which would re-plan it on this collection's next unrelated delivery.
#[gpui_kit::test]
fn another_roots_delete_does_not_replan_a_folder_runner(cx: &mut TestAppContext) {
    http_ready(cx);
    let a = folder_area(cx);
    let other = two_requests();
    let other_root = other.path().to_path_buf();
    a.ws.update(cx, |w, cx| w.open(other_root.clone(), cx));
    cx.run_until_parked();
    let id = open_folder_runner(cx, &a);
    rewrite(cx, &a, "f/b.yml", &request("b", "http://x/b2"));
    let seen = events(cx, &a.ws);
    let op = Command::Tree(TreeOp::Delete { item: ip("a.yml") });
    a.ws.update(cx, |w, cx| w.send(&other_root, op, cx))
        .expect("sent");
    cx.run_until_parked();
    assert!(!other_root.join("a.yml").exists());
    rewrite(cx, &a, "f/a.yml", &request("a", "http://x/1"));
    assert_eq!(replans(&seen.borrow(), id), 0, "{:?}", seen.borrow());
}

// The runner plans when its view opens, so neither the deliveries around the open nor a
// later unrelated one ask it for another plan.
#[gpui_kit::test]
fn opening_a_folder_runner_does_not_replan_it(cx: &mut TestAppContext) {
    http_ready(cx);
    let a = folder_area(cx);
    let seen = events(cx, &a.ws);
    let id = open_folder_runner(cx, &a);
    rewrite(cx, &a, "f/a.yml", &request("a", "http://x/1"));
    assert_eq!(replans(&seen.borrow(), id), 0, "{:?}", seen.borrow());
}

// Edits `field` of `item` the way the request editor does, then saves it as Ctrl+S does.
fn edit_and_save(cx: &mut TestAppContext, a: &Area, item: &str, edit: Edit) {
    let target = Target::Request(ip(item));
    let command = Command::Edit {
        target: target.clone(),
        edit,
    };
    a.ws.update(cx, |w, cx| w.send(&a.root, command, cx));
    cx.run_until_parked();
    a.ws.update(cx, |w, cx| w.send(&a.root, Command::Save(target), cx));
    cx.run_until_parked();
}

#[gpui_kit::test]
fn a_method_edited_and_saved_under_a_folder_runner_shows_the_new_method(cx: &mut TestAppContext) {
    http_ready(cx);
    let a = folder_area(cx);
    let id = open_folder_runner(cx, &a);
    let seen = events(cx, &a.ws);
    edit_and_save(cx, &a, "f/a.yml", Edit::Method("POST".into()));
    wait_real(cx, |cx| body_shown(cx, &a)[0].2 == "POST");
    assert_eq!(replans(&seen.borrow(), id), 1, "{:?}", seen.borrow());
}

#[gpui_kit::test]
fn a_name_edited_and_saved_under_a_folder_runner_shows_the_new_name(cx: &mut TestAppContext) {
    http_ready(cx);
    let a = folder_area(cx);
    let id = open_folder_runner(cx, &a);
    let seen = events(cx, &a.ws);
    edit_and_save(cx, &a, "f/a.yml", Edit::Name("renamed".into()));
    wait_real(cx, |cx| body_shown(cx, &a)[0].1 == "renamed");
    assert_eq!(replans(&seen.borrow(), id), 1, "{:?}", seen.borrow());
}

// Neither the unsaved edit nor its save of a field the body does not show re-plans.
#[gpui_kit::test]
fn a_url_edited_and_saved_under_a_folder_runner_does_not_replan(cx: &mut TestAppContext) {
    http_ready(cx);
    let a = folder_area(cx);
    let id = open_folder_runner(cx, &a);
    let seen = events(cx, &a.ws);
    edit_and_save(cx, &a, "f/a.yml", Edit::Url("http://x/9".into()));
    assert!(
        std::fs::read_to_string(a.root.join("f/a.yml"))
            .unwrap()
            .contains("x/9")
    );
    assert_eq!(replans(&seen.borrow(), id), 0, "{:?}", seen.borrow());
}

// A method edit that is reverted before any save leaves the saved plan as it was.
#[gpui_kit::test]
fn a_method_edit_reverted_unsaved_does_not_replan(cx: &mut TestAppContext) {
    http_ready(cx);
    let a = folder_area(cx);
    let id = open_folder_runner(cx, &a);
    let seen = events(cx, &a.ws);
    let target = Target::Request(ip("f/a.yml"));
    for method in ["POST", "GET"] {
        let edit = Command::Edit {
            target: target.clone(),
            edit: Edit::Method(method.into()),
        };
        a.ws.update(cx, |w, cx| w.send(&a.root, edit, cx));
        cx.run_until_parked();
    }
    assert_eq!(replans(&seen.borrow(), id), 0, "{:?}", seen.borrow());
}

fn open_folder_selection(cx: &mut TestAppContext, a: &Area) -> apinata_app::tabs::TabId {
    let id =
        a.ws.update(cx, |w, cx| w.open_runner_items(&a.root, vec![ip("f")], cx));
    wait_real(cx, |cx| body_shown(cx, a).len() == 2);
    id.expect("opens")
}

// Asks the active runner tab to keep only requests tagged `tags`.
fn filter_tags(cx: &mut TestAppContext, a: &Area, tags: &str) {
    let id = a.ws.read_with(cx, |w, _| w.tabs().active().map(|e| e.id));
    let runner = a
        .view
        .read_with(cx, |v, _| v.runner(id.expect("a tab")).cloned());
    let model = runner
        .expect("a runner view")
        .read_with(cx, |r, _| r.model().clone());
    model.update(cx, |m, cx| m.set_tags(tags.into(), cx));
}

fn order(cx: &mut TestAppContext, a: &Area) -> Vec<String> {
    body_shown(cx, a).into_iter().map(|r| r.0).collect()
}

fn tagged(name: &str, url: &str) -> String {
    request(name, url).replace("  seq: 1\n", "  seq: 1\n  tags:\n    - smoke\n")
}

#[gpui_kit::test]
fn a_seq_change_on_disk_reorders_a_folder_runner_body(cx: &mut TestAppContext) {
    http_ready(cx);
    let a = folder_area(cx);
    open_folder_runner(cx, &a);
    let text = request("a", "http://x/0").replace("seq: 1", "seq: 5");
    rewrite(cx, &a, "f/a.yml", &text);
    wait_real(cx, |cx| order(cx, &a) == ["f/b.yml", "f/a.yml"]);
}

#[gpui_kit::test]
fn a_seq_change_on_disk_reorders_a_selection_body(cx: &mut TestAppContext) {
    http_ready(cx);
    let a = folder_area(cx);
    open_folder_selection(cx, &a);
    let text = request("a", "http://x/0").replace("seq: 1", "seq: 5");
    rewrite(cx, &a, "f/a.yml", &text);
    wait_real(cx, |cx| order(cx, &a) == ["f/b.yml", "f/a.yml"]);
}

#[gpui_kit::test]
fn a_move_in_the_app_reorders_a_folder_runner_body(cx: &mut TestAppContext) {
    http_ready(cx);
    let a = folder_area(cx);
    open_folder_runner(cx, &a);
    let mv = TreeOp::Move {
        item: ip("f/a.yml"),
        to: apinata_core::DropTarget::After(ip("f/b.yml")),
    };
    a.ws.update(cx, |w, cx| w.send(&a.root, Command::Tree(mv), cx));
    wait_real(cx, |cx| order(cx, &a) == ["f/b.yml", "f/a.yml"]);
}

#[gpui_kit::test]
fn a_move_in_the_app_reorders_a_selection_body(cx: &mut TestAppContext) {
    http_ready(cx);
    let a = folder_area(cx);
    open_folder_selection(cx, &a);
    let mv = TreeOp::Move {
        item: ip("f/a.yml"),
        to: apinata_core::DropTarget::After(ip("f/b.yml")),
    };
    a.ws.update(cx, |w, cx| w.send(&a.root, Command::Tree(mv), cx));
    wait_real(cx, |cx| order(cx, &a) == ["f/b.yml", "f/a.yml"]);
}

#[gpui_kit::test]
fn a_tag_added_on_disk_enters_a_filtered_folder_runner_body(cx: &mut TestAppContext) {
    http_ready(cx);
    let a = folder_area(cx);
    open_folder_runner(cx, &a);
    filter_tags(cx, &a, "smoke");
    wait_real(cx, |cx| body_shown(cx, &a).is_empty());
    rewrite(cx, &a, "f/a.yml", &tagged("a", "http://x/0"));
    wait_real(cx, |cx| order(cx, &a) == ["f/a.yml"]);
}

#[gpui_kit::test]
fn a_tag_added_on_disk_enters_a_filtered_selection_body(cx: &mut TestAppContext) {
    http_ready(cx);
    let a = folder_area(cx);
    open_folder_selection(cx, &a);
    filter_tags(cx, &a, "smoke");
    wait_real(cx, |cx| body_shown(cx, &a).is_empty());
    rewrite(cx, &a, "f/a.yml", &tagged("a", "http://x/0"));
    wait_real(cx, |cx| order(cx, &a) == ["f/a.yml"]);
}

#[gpui_kit::test]
fn a_tag_edited_and_saved_enters_a_filtered_folder_runner_body(cx: &mut TestAppContext) {
    http_ready(cx);
    let a = folder_area(cx);
    let id = open_folder_runner(cx, &a);
    filter_tags(cx, &a, "smoke");
    wait_real(cx, |cx| body_shown(cx, &a).is_empty());
    let seen = events(cx, &a.ws);
    edit_and_save(cx, &a, "f/a.yml", Edit::Tags(vec!["smoke".into()]));
    wait_real(cx, |cx| order(cx, &a) == ["f/a.yml"]);
    assert_eq!(replans(&seen.borrow(), id), 1, "{:?}", seen.borrow());
}

#[gpui_kit::test]
fn a_tag_edited_and_saved_enters_a_filtered_selection_body(cx: &mut TestAppContext) {
    http_ready(cx);
    let a = folder_area(cx);
    let id = open_folder_selection(cx, &a);
    filter_tags(cx, &a, "smoke");
    wait_real(cx, |cx| body_shown(cx, &a).is_empty());
    let seen = events(cx, &a.ws);
    edit_and_save(cx, &a, "f/a.yml", Edit::Tags(vec!["smoke".into()]));
    wait_real(cx, |cx| order(cx, &a) == ["f/a.yml"]);
    assert_eq!(replans(&seen.borrow(), id), 1, "{:?}", seen.borrow());
}

#[gpui_kit::test]
fn a_url_change_under_a_selection_does_not_replan(cx: &mut TestAppContext) {
    http_ready(cx);
    let a = folder_area(cx);
    let id = open_folder_selection(cx, &a);
    rewrite(cx, &a, "f/b.yml", &request("b", "http://x/b2"));
    let seen = events(cx, &a.ws);
    rewrite(cx, &a, "f/a.yml", &request("a", "http://x/1"));
    assert_eq!(replans(&seen.borrow(), id), 0, "{:?}", seen.borrow());
}

// A runner tab the session brings back plans when its view opens, so neither the delivery
// that restores it nor the collection's next unrelated one asks it for another plan.
#[gpui_kit::test]
fn a_restored_folder_runner_does_not_replan(cx: &mut TestAppContext) {
    use apinata_app::session::{CollectionSession, SavedPanel, SavedPanelKind, SavedTab, Session};
    http_ready(cx);
    support::init(cx);
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("f/a.yml", &request("a", "http://x/0")),
        ("f/b.yml", &request("b", "http://x/b")),
    ]);
    let root = col.path().to_path_buf();
    let runner = SavedPanelKind::Runner("f".into());
    let saved = CollectionSession {
        panels: vec![SavedPanel {
            kind: runner.clone(),
            position: 0,
        }],
        active: Some(SavedTab::Panel(runner)),
        ..CollectionSession::default()
    };
    let mut start = Session {
        open: vec![root.clone()],
        ..Session::default()
    };
    start.collections.insert(root.clone(), saved);
    let state = tempfile::tempdir().unwrap();
    let session = session_at(cx, &state, start);
    let ws = workspace(cx, &session);
    let seen = events(cx, &ws);
    ws.update(cx, |w, cx| w.restore(cx));
    cx.run_until_parked();
    let shared = ws.clone();
    let (win, view) = cx
        .update(|cx| {
            gpui_kit::open_window(gpui_kit::WindowOptions::default(), cx, |w, cx| {
                cx.new(|cx| TabArea::new(shared, w, cx))
            })
        })
        .unwrap();
    let a = Area {
        col,
        state,
        ws,
        root,
        win,
        view,
    };
    wait_real(cx, |cx| body_shown(cx, &a).len() == 2);
    let id = a.ws.read_with(cx, |w, _| w.tabs().active().map(|e| e.id));
    rewrite(cx, &a, "f/a.yml", &request("a", "http://x/1"));
    let id = id.expect("a restored runner tab");
    assert_eq!(replans(&seen.borrow(), id), 0, "{:?}", seen.borrow());
}

// Sends an unsaved editor edit of `item`.
fn edit_unsaved(cx: &mut TestAppContext, a: &Area, item: &str, edit: Edit) {
    let command = Command::Edit {
        target: Target::Request(ip(item)),
        edit,
    };
    a.ws.update(cx, |w, cx| w.send(&a.root, command, cx));
    cx.run_until_parked();
}

// The saved plan reads the file, so a rewrite under an unsaved edit still reaches the body.
#[gpui_kit::test]
fn a_rewrite_on_disk_under_an_unsaved_edit_shows_in_a_folder_runner_body(cx: &mut TestAppContext) {
    http_ready(cx);
    let a = folder_area(cx);
    open_folder_runner(cx, &a);
    edit_unsaved(cx, &a, "f/a.yml", Edit::Method("POST".into()));
    rewrite(cx, &a, "f/a.yml", &request("renamed", "http://x/0"));
    wait_real(cx, |cx| body_shown(cx, &a)[0].1 == "renamed");
}

#[gpui_kit::test]
fn a_removal_on_disk_under_an_unsaved_edit_leaves_a_folder_runner_body(cx: &mut TestAppContext) {
    http_ready(cx);
    let a = folder_area(cx);
    open_folder_runner(cx, &a);
    edit_unsaved(cx, &a, "f/a.yml", Edit::Method("POST".into()));
    remove_on_disk(cx, &a, "f/a.yml");
    wait_real(cx, |cx| order(cx, &a) == ["f/b.yml"]);
}

#[gpui_kit::test]
fn a_draft_saved_into_the_folder_enters_its_runner_body(cx: &mut TestAppContext) {
    http_ready(cx);
    let a = folder_area(cx);
    open_folder_runner(cx, &a);
    let create = TreeOp::CreateDraft {
        parent: ip("f"),
        kind: apinata_core::RequestKind::Http,
    };
    a.ws.update(cx, |w, cx| w.send(&a.root, Command::Tree(create), cx));
    cx.run_until_parked();
    let draft = a.ws.read_with(cx, |w, cx| {
        let s = w.snapshot(&a.root, cx).expect("a snapshot");
        (s.content.requests.iter())
            .find(|(_, v)| v.draft)
            .map(|(p, _)| p.clone())
    });
    assert_eq!(order(cx, &a).len(), 2, "a draft is not planned");
    let save = TreeOp::SaveDraft {
        draft: draft.expect("a draft"),
        parent: ip("f"),
        name: "c".into(),
    };
    a.ws.update(cx, |w, cx| w.send(&a.root, Command::Tree(save), cx));
    wait_real(cx, |cx| order(cx, &a).contains(&"f/c.yml".to_owned()));
}

#[gpui_kit::test]
fn a_move_under_an_unsaved_edit_reorders_a_folder_runner_body(cx: &mut TestAppContext) {
    http_ready(cx);
    let a = folder_area(cx);
    open_folder_runner(cx, &a);
    edit_unsaved(cx, &a, "f/a.yml", Edit::Method("POST".into()));
    let mv = TreeOp::Move {
        item: ip("f/a.yml"),
        to: apinata_core::DropTarget::After(ip("f/b.yml")),
    };
    a.ws.update(cx, |w, cx| w.send(&a.root, Command::Tree(mv), cx));
    wait_real(cx, |cx| order(cx, &a) == ["f/b.yml", "f/a.yml"]);
}

fn tagged_folder(name: &str) -> String {
    format!("{}  tags:\n    - smoke\n", folder(name))
}

// Core filters on effective tags, so a folder's tags reach the requests under it.
#[gpui_kit::test]
fn a_folder_tag_added_on_disk_enters_a_filtered_folder_runner_body(cx: &mut TestAppContext) {
    http_ready(cx);
    let a = folder_area(cx);
    open_folder_runner(cx, &a);
    filter_tags(cx, &a, "smoke");
    wait_real(cx, |cx| body_shown(cx, &a).is_empty());
    rewrite(cx, &a, "f/folder.yml", &tagged_folder("f"));
    wait_real(cx, |cx| order(cx, &a) == ["f/a.yml", "f/b.yml"]);
}

#[gpui_kit::test]
fn a_folder_tag_added_on_disk_enters_a_filtered_selection_body(cx: &mut TestAppContext) {
    http_ready(cx);
    let a = folder_area(cx);
    open_folder_selection(cx, &a);
    filter_tags(cx, &a, "smoke");
    wait_real(cx, |cx| body_shown(cx, &a).is_empty());
    rewrite(cx, &a, "f/folder.yml", &tagged_folder("f"));
    wait_real(cx, |cx| order(cx, &a) == ["f/a.yml", "f/b.yml"]);
}

#[gpui_kit::test]
fn a_folder_tag_edited_and_saved_enters_a_filtered_folder_runner_body(cx: &mut TestAppContext) {
    http_ready(cx);
    let a = folder_area(cx);
    let id = open_folder_runner(cx, &a);
    filter_tags(cx, &a, "smoke");
    wait_real(cx, |cx| body_shown(cx, &a).is_empty());
    let seen = events(cx, &a.ws);
    let target = Target::Folder(ip("f"));
    let edit = Command::Edit {
        target: target.clone(),
        edit: Edit::Tags(vec!["smoke".into()]),
    };
    a.ws.update(cx, |w, cx| w.send(&a.root, edit, cx));
    cx.run_until_parked();
    assert_eq!(replans(&seen.borrow(), id), 0, "{:?}", seen.borrow());
    a.ws.update(cx, |w, cx| w.send(&a.root, Command::Save(target), cx));
    cx.run_until_parked();
    wait_real(cx, |cx| order(cx, &a) == ["f/a.yml", "f/b.yml"]);
    assert_eq!(replans(&seen.borrow(), id), 1, "{:?}", seen.borrow());
}

// Neither a folder change other than its tags nor a folder outside the runner's scope
// changes what the runner plans.
#[gpui_kit::test]
fn a_folder_change_besides_tags_does_not_replan(cx: &mut TestAppContext) {
    http_ready(cx);
    let a = folder_area(cx);
    let id = open_folder_runner(cx, &a);
    let seen = events(cx, &a.ws);
    rewrite(
        cx,
        &a,
        "f/folder.yml",
        &format!("{}docs: notes\n", folder("f")),
    );
    std::fs::create_dir_all(a.root.join("g")).unwrap();
    rewrite(cx, &a, "g/folder.yml", &tagged_folder("g"));
    assert_eq!(replans(&seen.borrow(), id), 0, "{:?}", seen.borrow());
}

// A selection tab the session brings back shows its rows and, like a folder runner, is not
// asked for another plan by the restoring delivery or the next unrelated one.
#[gpui_kit::test]
fn a_restored_selection_shows_its_rows_and_does_not_replan(cx: &mut TestAppContext) {
    use apinata_app::session::{CollectionSession, Session};
    http_ready(cx);
    support::init(cx);
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("f/a.yml", &request("a", "http://x/0")),
        ("f/b.yml", &request("b", "http://x/b")),
        ("c.yml", &request("c", "http://x/c")),
    ]);
    let root = col.path().to_path_buf();
    let selection = r#"{"selection": ["c.yml", "f"]}"#;
    let saved: CollectionSession = serde_json::from_str(&format!(
        r#"{{"panels": [{{"kind": {selection}, "position": 0}}], "active": {{"panel": {selection}}}}}"#
    ))
    .unwrap();
    let mut start = Session {
        open: vec![root.clone()],
        ..Session::default()
    };
    start.collections.insert(root.clone(), saved);
    let state = tempfile::tempdir().unwrap();
    let session = session_at(cx, &state, start);
    let ws = workspace(cx, &session);
    let seen = events(cx, &ws);
    ws.update(cx, |w, cx| w.restore(cx));
    cx.run_until_parked();
    let shared = ws.clone();
    let (win, view) = cx
        .update(|cx| {
            gpui_kit::open_window(gpui_kit::WindowOptions::default(), cx, |w, cx| {
                cx.new(|cx| TabArea::new(shared, w, cx))
            })
        })
        .unwrap();
    let a = Area {
        col,
        state,
        ws,
        root,
        win,
        view,
    };
    let id = a.ws.read_with(cx, |w, _| w.tabs().active().map(|e| e.id));
    let id = id.expect("a restored selection tab");
    wait_real(cx, |cx| body_shown(cx, &a).len() == 3);
    rewrite(cx, &a, "f/a.yml", &request("a", "http://x/1"));
    assert_eq!(replans(&seen.borrow(), id), 0, "{:?}", seen.borrow());
}

// Effective tags come from every ancestor folder, not only the nearest one.
#[gpui_kit::test]
fn a_grandparent_folder_tag_added_on_disk_enters_a_filtered_folder_runner_body(
    cx: &mut TestAppContext,
) {
    http_ready(cx);
    let a = area_of(
        cx,
        &[
            ("opencollection.yml", ROOT),
            ("f/folder.yml", &folder("f")),
            ("f/g/folder.yml", &folder("g")),
            ("f/g/c.yml", &request("c", "http://x/c")),
        ],
    );
    a.ws.update(cx, |w, cx| w.open_runner(&a.root, ip("f"), cx))
        .expect("opens");
    wait_real(cx, |cx| body_shown(cx, &a).len() == 1);
    filter_tags(cx, &a, "smoke");
    wait_real(cx, |cx| body_shown(cx, &a).is_empty());
    rewrite(cx, &a, "f/folder.yml", &tagged_folder("f"));
    wait_real(cx, |cx| order(cx, &a) == ["f/g/c.yml"]);
}

// Each outside write to an unsaved request re-plans once, so a second rewrite while the
// conflict stands still reaches the body.
#[gpui_kit::test]
fn a_second_rewrite_under_an_unsaved_edit_shows_in_a_folder_runner_body(cx: &mut TestAppContext) {
    http_ready(cx);
    let a = folder_area(cx);
    let id = open_folder_runner(cx, &a);
    let seen = events(cx, &a.ws);
    edit_unsaved(cx, &a, "f/a.yml", Edit::Method("POST".into()));
    rewrite(cx, &a, "f/a.yml", &request("renamed", "http://x/0"));
    wait_real(cx, |cx| body_shown(cx, &a)[0].1 == "renamed");
    cx.run_until_parked();
    assert_eq!(replans(&seen.borrow(), id), 1, "{:?}", seen.borrow());
    rewrite(cx, &a, "f/a.yml", &request("again", "http://x/0"));
    wait_real(cx, |cx| body_shown(cx, &a)[0].1 == "again");
    cx.run_until_parked();
    assert_eq!(replans(&seen.borrow(), id), 2, "{:?}", seen.borrow());
}

// An unsaved edit while the conflict stands carries no new outside stamp, so the one
// re-plan from the rewrite stays the only one.
#[gpui_kit::test]
fn an_unsaved_edit_after_a_rewrite_in_conflict_does_not_replan_again(cx: &mut TestAppContext) {
    http_ready(cx);
    let a = folder_area(cx);
    let id = open_folder_runner(cx, &a);
    let seen = events(cx, &a.ws);
    edit_unsaved(cx, &a, "f/a.yml", Edit::Method("POST".into()));
    rewrite(cx, &a, "f/a.yml", &request("renamed", "http://x/0"));
    wait_real(cx, |cx| body_shown(cx, &a)[0].1 == "renamed");
    cx.run_until_parked();
    assert_eq!(replans(&seen.borrow(), id), 1, "{:?}", seen.borrow());
    edit_unsaved(cx, &a, "f/a.yml", Edit::Method("PUT".into()));
    cx.run_until_parked();
    assert_eq!(replans(&seen.borrow(), id), 1, "{:?}", seen.borrow());
}

#[gpui_kit::test]
fn a_rewrite_of_a_request_unsaved_before_the_runner_opened_shows_in_its_body(
    cx: &mut TestAppContext,
) {
    http_ready(cx);
    let a = folder_area(cx);
    edit_unsaved(cx, &a, "f/a.yml", Edit::Method("POST".into()));
    open_folder_runner(cx, &a);
    rewrite(cx, &a, "f/a.yml", &request("renamed", "http://x/0"));
    wait_real(cx, |cx| body_shown(cx, &a)[0].1 == "renamed");
}
