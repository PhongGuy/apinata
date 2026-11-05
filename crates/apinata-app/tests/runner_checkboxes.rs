//! The runner's request checkboxes: tri-state folders, select all, the Run count, a run that
//! sends only the checked requests, and a choice kept for the session.

use crate::support;

use apinata_app::command::{Command, TreeOp};
use apinata_app::runner_model::{RunState, RunnerModel};
use apinata_app::tabs::TabId;
use apinata_core::{DropTarget, ItemPath};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, ElementId, Entity, TestAppContext};
use std::collections::HashSet;
use support::*;
use wiremock::matchers::path;
use wiremock::{Mock, MockServer, ResponseTemplate};

const FOLDER: (&str, usize) = ("run-plan-folder-check", 0);

fn tree(uri: &str) -> tempfile::TempDir {
    write_tree(&[
        ("opencollection.yml", ROOT),
        ("api/folder.yml", &folder("api")),
        ("api/one.yml", &request("one", &format!("{uri}/one"))),
        ("api/two.yml", &request("two", &format!("{uri}/two"))),
        ("other.yml", &request("other", &format!("{uri}/other"))),
    ])
}

// Opens the collection's runner and waits for its three planned rows.
fn open(cx: &mut TestAppContext, s: &Started) -> (TabId, Entity<RunnerModel>) {
    let tab =
        s.ws.update(cx, |w, cx| w.open_runner(&s.root, ItemPath::root(), cx))
            .expect("a runner tab");
    wait_real(cx, |cx| present(cx, s.win, ("run-plan-row", 2usize)));
    let model = s
        .shell
        .read_with(cx, |v, cx| v.tab_area().read(cx).runner(tab).cloned())
        .expect("a runner view")
        .read_with(cx, |v, _| v.model().clone());
    (tab, model)
}

fn click(cx: &mut TestAppContext, s: &Started, id: impl Into<ElementId>) {
    let id = id.into();
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| w.click(id, cx)).unwrap();
    cx.run_until_parked();
}

// The checked state and accessibility label of element `id`.
fn state(
    cx: &mut TestAppContext,
    s: &Started,
    id: impl Into<ElementId>,
) -> (Option<bool>, Option<String>) {
    let id = id.into();
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, _| {
        let e = w.find(id);
        (e.checked(), e.label().map(str::to_owned))
    })
    .unwrap()
}

fn run_label(cx: &mut TestAppContext, s: &Started) -> Option<String> {
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, _| {
        w.try_find("run-start")
            .and_then(|e| e.label().map(str::to_owned))
    })
    .unwrap()
}

#[gpui_kit::test]
fn a_folder_box_checks_everything_below_it_and_shows_a_partial_choice(cx: &mut TestAppContext) {
    http_ready(cx);
    let col = tree("http://127.0.0.1:9");
    let s = started(cx, &col);
    let (_, model) = open(cx, &s);
    assert_eq!(state(cx, &s, FOLDER), (Some(true), Some("api".into())));
    assert_eq!(run_label(cx, &s).as_deref(), Some("Run 3 requests"));

    click(cx, &s, ("run-plan-check", 0usize));
    assert_eq!(
        state(cx, &s, FOLDER),
        (Some(false), Some("api, partly checked".into()))
    );
    assert_eq!(
        state(cx, &s, ("run-plan-check", 0usize)).0,
        Some(false),
        "the request box itself"
    );
    assert_eq!(run_label(cx, &s).as_deref(), Some("Run 2 requests"));

    // A partial folder asks to check all; checked, it asks to clear all.
    click(cx, &s, FOLDER);
    assert_eq!(state(cx, &s, FOLDER), (Some(true), Some("api".into())));
    assert_eq!(run_label(cx, &s).as_deref(), Some("Run 3 requests"));
    click(cx, &s, FOLDER);
    assert_eq!(state(cx, &s, FOLDER).0, Some(false));
    assert_eq!(run_label(cx, &s).as_deref(), Some("Run 1 request"));
    model.read_with(cx, |m, cx| {
        assert!(!m.is_checked(&ip("api/one.yml"), cx));
        assert!(!m.is_checked(&ip("api/two.yml"), cx));
        assert!(m.is_checked(&ip("other.yml"), cx));
    });
}

#[gpui_kit::test]
fn select_all_clears_and_restores_and_nothing_checked_locks_run(cx: &mut TestAppContext) {
    http_ready(cx);
    let col = tree("http://127.0.0.1:9");
    let s = started(cx, &col);
    let (_, model) = open(cx, &s);
    assert_eq!(state(cx, &s, "run-select-all").0, Some(true));

    click(cx, &s, "run-select-all");
    assert_eq!(state(cx, &s, "run-select-all").0, Some(false));
    assert_eq!(run_label(cx, &s).as_deref(), Some("Run 0 requests"));
    assert!(present(cx, s.win, "run-start"));
    click(cx, &s, "run-start");
    assert!(
        model.read_with(cx, |m, _| matches!(m.state(), RunState::Idle)),
        "a locked Run starts nothing"
    );
    assert!(!model.update(cx, |m, cx| m.start(cx)), "nor does start()");

    click(cx, &s, "run-select-all");
    assert_eq!(run_label(cx, &s).as_deref(), Some("Run 3 requests"));
}

#[gpui_kit::test]
fn a_run_sends_only_the_checked_requests_in_plan_order(cx: &mut TestAppContext) {
    http_ready(cx);
    let handle = cx.update(|cx| apinata_app::tokio_bridge::handle(cx));
    let server = handle.block_on(MockServer::builder().start());
    handle.block_on(async {
        for p in ["/one", "/two", "/other"] {
            Mock::given(path(p))
                .respond_with(ResponseTemplate::new(200))
                .mount(&server)
                .await;
        }
    });
    let col = tree(&server.uri());
    let s = started(cx, &col);
    let (_, model) = open(cx, &s);
    click(cx, &s, ("run-plan-check", 1usize));
    click(cx, &s, "run-start");
    wait_real(cx, |cx| {
        model.read_with(cx, |m, _| matches!(m.state(), RunState::Finished(_)))
    });
    let rows: Vec<String> =
        model.read_with(cx, |m, _| m.rows().iter().map(|r| r.name.clone()).collect());
    assert_eq!(rows, ["one", "other"]);
    let seen: Vec<String> = handle
        .block_on(server.received_requests())
        .unwrap_or_default()
        .iter()
        .map(|r| r.url.path().to_owned())
        .collect();
    assert_eq!(seen, ["/one", "/other"]);
}

#[gpui_kit::test]
fn the_choice_survives_reopening_and_a_new_request_starts_checked(cx: &mut TestAppContext) {
    http_ready(cx);
    let col = tree("http://127.0.0.1:9");
    let s = started(cx, &col);
    let (tab, _) = open(cx, &s);
    click(cx, &s, ("run-plan-check", 2usize));
    assert_eq!(run_label(cx, &s).as_deref(), Some("Run 2 requests"));

    let area = s.shell.read_with(cx, |sh, _| sh.tab_area().clone());
    cx.update_window(s.win, |_, w, cx| {
        area.update(cx, |a, cx| a.close(vec![tab], w, cx))
    })
    .unwrap();
    cx.run_until_parked();
    std::fs::write(
        col.path().join("api/three.yml"),
        request("three", "http://127.0.0.1:9/three"),
    )
    .unwrap();

    let tab =
        s.ws.update(cx, |w, cx| w.open_runner(&s.root, ItemPath::root(), cx))
            .expect("a runner tab");
    wait_real(cx, |cx| present(cx, s.win, ("run-plan-row", 3usize)));
    let model = s
        .shell
        .read_with(cx, |v, cx| v.tab_area().read(cx).runner(tab).cloned())
        .expect("a runner view")
        .read_with(cx, |v, _| v.model().clone());
    model.read_with(cx, |m, cx| {
        assert!(!m.is_checked(&ip("other.yml"), cx), "kept unchecked");
        assert!(m.is_checked(&ip("api/three.yml"), cx), "new, so checked");
    });
    assert_eq!(run_label(cx, &s).as_deref(), Some("Run 3 requests"));
}

#[gpui_kit::test]
fn the_boxes_are_reached_by_tab_and_toggled_by_space(cx: &mut TestAppContext) {
    http_ready(cx);
    let col = tree("http://127.0.0.1:9");
    let s = started(cx, &col);
    let (_, model) = open(cx, &s);
    let target = ("run-plan-check", 0usize);
    for _ in 0..60 {
        if holds_focus(cx, s.win, target) {
            break;
        }
        press_key(cx, s.win, "tab");
    }
    assert!(
        holds_focus(cx, s.win, target),
        "Tab reaches the request box"
    );
    press_key(cx, s.win, "space");
    cx.run_until_parked();
    model.read_with(cx, |m, cx| assert!(!m.is_checked(&ip("api/one.yml"), cx)));
}

#[gpui_kit::test]
fn a_folder_heading_shows_the_folder_name_not_its_directory(cx: &mut TestAppContext) {
    http_ready(cx);
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("api/folder.yml", &folder("Service")),
        ("api/one.yml", &request("one", "http://127.0.0.1:9/one")),
        ("api/two.yml", &request("two", "http://127.0.0.1:9/two")),
        ("other.yml", &request("other", "http://127.0.0.1:9/other")),
    ]);
    let s = started(cx, &col);
    open(cx, &s);
    assert_eq!(state(cx, &s, FOLDER), (Some(true), Some("Service".into())));
    click(cx, &s, ("run-plan-check", 0usize));
    assert_eq!(
        state(cx, &s, FOLDER).1.as_deref(),
        Some("Service, partly checked")
    );
}

#[gpui_kit::test]
fn select_all_names_a_partial_choice(cx: &mut TestAppContext) {
    http_ready(cx);
    let col = tree("http://127.0.0.1:9");
    let s = started(cx, &col);
    open(cx, &s);
    assert_eq!(
        state(cx, &s, "run-select-all").1.as_deref(),
        Some("Select all")
    );
    click(cx, &s, ("run-plan-check", 2usize));
    assert_eq!(
        state(cx, &s, "run-select-all").1.as_deref(),
        Some("Select all, partly checked")
    );
}

fn unchecked(cx: &mut TestAppContext, s: &Started) -> HashSet<String> {
    s.ws.read_with(cx, |w, _| {
        (w.run_unchecked(&s.root).iter())
            .map(|i| i.as_str().to_owned())
            .collect()
    })
}

fn uncheck(cx: &mut TestAppContext, s: &Started, items: &[&str]) {
    let items: Vec<ItemPath> = items.iter().map(|i| ip(i)).collect();
    s.ws.update(cx, |w, _| w.set_run_checked(&s.root, &items, false));
}

// Sends a tree command and waits until the snapshot holds request `then`.
fn tree_op(cx: &mut TestAppContext, s: &Started, op: TreeOp, then: &str) {
    s.ws.update(cx, |w, cx| w.send(&s.root, Command::Tree(op), cx))
        .expect("sent");
    let want = ip(then);
    wait_real(cx, |cx| {
        s.ws.read_with(cx, |w, cx| {
            w.snapshot(&s.root, cx)
                .is_some_and(|snap| snap.content.requests.contains_key(&want))
        })
    });
}

fn set(items: &[&str]) -> HashSet<String> {
    items.iter().map(|i| (*i).to_owned()).collect()
}

#[gpui_kit::test]
fn an_unchecked_request_stays_unchecked_when_moved(cx: &mut TestAppContext) {
    let col = tree("http://127.0.0.1:9");
    let s = started(cx, &col);
    uncheck(cx, &s, &["other.yml"]);
    let op = TreeOp::Move {
        item: ip("other.yml"),
        to: DropTarget::Inside(ip("api")),
    };
    tree_op(cx, &s, op, "api/other.yml");
    assert_eq!(unchecked(cx, &s), set(&["api/other.yml"]));
}

#[gpui_kit::test]
fn a_moved_folder_carries_the_choice_of_the_requests_below(cx: &mut TestAppContext) {
    let col = tree("http://127.0.0.1:9");
    std::fs::create_dir(col.path().join("dest")).unwrap();
    std::fs::write(col.path().join("dest/folder.yml"), folder("dest")).unwrap();
    let s = started(cx, &col);
    uncheck(cx, &s, &["api/one.yml"]);
    let op = TreeOp::Move {
        item: ip("api"),
        to: DropTarget::Inside(ip("dest")),
    };
    tree_op(cx, &s, op, "dest/api/one.yml");
    assert_eq!(unchecked(cx, &s), set(&["dest/api/one.yml"]));
}

#[gpui_kit::test]
fn a_request_deleted_and_made_again_starts_checked(cx: &mut TestAppContext) {
    let col = tree("http://127.0.0.1:9");
    let s = started(cx, &col);
    uncheck(cx, &s, &["other.yml"]);
    s.ws.update(cx, |w, cx| {
        w.send(
            &s.root,
            Command::Tree(TreeOp::Delete {
                item: ip("other.yml"),
            }),
            cx,
        )
    })
    .expect("sent");
    wait_real(cx, |cx| {
        s.ws.read_with(cx, |w, cx| {
            w.snapshot(&s.root, cx)
                .is_some_and(|snap| !snap.content.requests.contains_key(&ip("other.yml")))
        })
    });
    assert!(
        unchecked(cx, &s).is_empty(),
        "the deleted request is forgotten"
    );
    let create = TreeOp::CreateRequest {
        parent: ItemPath::root(),
        name: "other".into(),
    };
    tree_op(cx, &s, create, "other.yml");
    assert!(s.ws.read_with(cx, |w, _| w.run_checked(&s.root, &ip("other.yml"))));
}

#[gpui_kit::test]
fn closing_the_collection_forgets_its_choice(cx: &mut TestAppContext) {
    let col = tree("http://127.0.0.1:9");
    let s = started(cx, &col);
    uncheck(cx, &s, &["other.yml"]);
    s.ws.update(cx, |w, cx| w.close(&s.root, cx));
    cx.run_until_parked();
    assert!(unchecked(cx, &s).is_empty());
}
