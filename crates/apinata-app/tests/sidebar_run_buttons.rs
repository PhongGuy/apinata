//! Run buttons on collection and folder rows.

use crate::support;

use apinata_app::sidebar::{SidebarEvent, marker_id};
use apinata_app::tabs::TabKind;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AnyWindowHandle, AppContext as _, ElementId, TestAppContext};
use support::*;

fn click(cx: &mut TestAppContext, win: AnyWindowHandle, id: impl Into<ElementId>) {
    let id = id.into();
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        w.click(id, cx)
    })
    .unwrap();
    cx.run_until_parked();
}

#[gpui_kit::test]
fn the_collection_row_runs_the_collection(cx: &mut TestAppContext) {
    let h = open_sidebar(cx);
    let seen = sidebar_events(cx, &h.view);
    click(cx, h.win, ("run-collection", 0usize));
    let expected = SidebarEvent::Run {
        root: h.root.clone(),
        folder: None,
    };
    assert_eq!(seen.borrow().last(), Some(&expected));
}

#[gpui_kit::test]
fn a_folder_row_runs_its_folder_without_toggling(cx: &mut TestAppContext) {
    let h = open_sidebar(cx);
    let seen = sidebar_events(cx, &h.view);
    let open_before = shown(cx, h.win, row("users/list.yml"));
    click(cx, h.win, marker_id("run-folder", &row("users")));
    let expected = SidebarEvent::Run {
        root: h.root.clone(),
        folder: Some(ip("users")),
    };
    assert_eq!(seen.borrow().last(), Some(&expected));
    assert_eq!(
        shown(cx, h.win, row("users/list.yml")),
        open_before,
        "the click leaves the folder as it was"
    );
}

#[gpui_kit::test]
fn only_folders_and_collections_have_a_run_button(cx: &mut TestAppContext) {
    let h = open_sidebar(cx);
    assert!(present(cx, h.win, marker_id("run-folder", &row("users"))));
    assert!(!present(
        cx,
        h.win,
        marker_id("run-folder", &row("health.yml"))
    ));
    assert!(!present(cx, h.win, ("run-collection", 1usize)));
}

#[gpui_kit::test]
fn the_folder_button_opens_the_runner_tab(cx: &mut TestAppContext) {
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("users/folder.yml", &folder("Users")),
        (
            "users/list.yml",
            &request("List users", "http://127.0.0.1:9/users"),
        ),
    ]);
    let s = started(cx, &col);
    click(cx, s.win, marker_id("run-folder", &row("users")));
    let kind =
        s.ws.read_with(cx, |w, _| w.tabs().active().map(|t| t.kind.clone()));
    assert_eq!(kind, Some(TabKind::Runner(ip("users"))));
}

// At rest the run button is transparent, so a long folder name may run under it; the button
// keeps its place at the row's right edge and still takes the click.
#[gpui_kit::test]
fn a_folder_name_runs_under_the_run_slot_at_rest(cx: &mut TestAppContext) {
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        (
            "users/folder.yml",
            &folder("A folder name far too long to fit the sidebar row"),
        ),
        (
            "users/list.yml",
            &request("List users", "http://127.0.0.1:9/users"),
        ),
    ]);
    let s = started(cx, &col);
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, _| {
        let name = w.find(marker_id("name", &row("users"))).bounds();
        let run = w.find(marker_id("run-folder", &row("users"))).bounds();
        let item = w.find(row("users")).bounds();
        assert!(
            name.right() > run.left(),
            "name {name:?} stops short of the run button {run:?}"
        );
        assert!(
            run.right() <= item.right(),
            "button {run:?} outside {item:?}"
        );
    })
    .unwrap();
    click(cx, s.win, marker_id("run-folder", &row("users")));
    let kind =
        s.ws.read_with(cx, |w, _| w.tabs().active().map(|t| t.kind.clone()));
    assert_eq!(kind, Some(TabKind::Runner(ip("users"))));
}

fn long_name_folder(cx: &mut TestAppContext) -> Started {
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        (
            "users/folder.yml",
            &folder("A folder name far too long to fit the sidebar row"),
        ),
        (
            "users/list.yml",
            &request("List users", "http://127.0.0.1:9/users"),
        ),
    ]);
    let s = started(cx, &col);
    frame(cx, s.win);
    s
}

// The button hides the end of a name that runs under it at rest, so while it shows the name
// gives up that room and ends with its ellipsis before the button.
fn assert_name_clears_button(cx: &mut TestAppContext, win: AnyWindowHandle) {
    cx.update_window(win, |_, w, _| {
        let name = w.find(marker_id("name", &row("users"))).bounds();
        let run = w.find(marker_id("run-folder", &row("users"))).bounds();
        assert!(
            name.right() <= run.left(),
            "name {name:?} runs under the shown run button {run:?}"
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_selected_folder_name_ends_before_the_run_button(cx: &mut TestAppContext) {
    let s = long_name_folder(cx);
    click(cx, s.win, row("users"));
    frame(cx, s.win);
    assert_name_clears_button(cx, s.win);
}

#[gpui_kit::test]
fn a_hovered_folder_name_ends_before_the_run_button(cx: &mut TestAppContext) {
    let s = long_name_folder(cx);
    cx.update_window(s.win, |_, w, cx| w.hover(row("users"), cx))
        .unwrap();
    frame(cx, s.win);
    assert_name_clears_button(cx, s.win);
}

// Whether Tab, pressed up to `n` times from the focused tree, ever lands on element `id`.
fn tab_reaches(
    cx: &mut TestAppContext,
    h: &SidebarHarness,
    id: impl Into<ElementId>,
    n: usize,
) -> bool {
    let id = id.into();
    focus_tree(cx, h);
    for _ in 0..n {
        frame(cx, h.win);
        let on = cx
            .update_window(h.win, |_, w, _| {
                w.try_find(id.clone()).and_then(|e| e.focused())
            })
            .unwrap();
        if on == Some(true) {
            return true;
        }
        cx.simulate_keystrokes(h.win, "tab");
        cx.run_until_parked();
    }
    false
}

#[gpui_kit::test]
fn tab_skips_the_run_buttons(cx: &mut TestAppContext) {
    let h = open_sidebar(cx);
    assert!(
        !tab_reaches(cx, &h, marker_id("run-folder", &row("users")), 12),
        "a folder's run button is not a Tab stop"
    );
    assert!(
        !tab_reaches(cx, &h, ("run-collection", 0usize), 12),
        "the collection's run button is not a Tab stop"
    );
}

fn select_row(cx: &mut TestAppContext, h: &SidebarHarness, path: &str) {
    let tree = h.view.read_with(cx, |s, _| s.tree().clone());
    tree.update(cx, |t, cx| {
        let ix = t.index_of(&row(path));
        assert!(ix.is_some(), "{path} is shown");
        t.set_selected_index(ix, cx)
    });
    focus_tree(cx, h);
    frame(cx, h.win);
}

#[gpui_kit::test]
fn shift_enter_runs_the_selected_folder(cx: &mut TestAppContext) {
    let h = open_sidebar(cx);
    let seen = sidebar_events(cx, &h.view);
    select_row(cx, &h, "users");
    cx.update_window(h.win, |_, w, cx| w.press("shift-enter", cx))
        .unwrap();
    cx.run_until_parked();
    let expected = SidebarEvent::Run {
        root: h.root.clone(),
        folder: Some(ip("users")),
    };
    assert_eq!(seen.borrow().last(), Some(&expected));
}

#[gpui_kit::test]
fn shift_enter_on_a_request_runs_nothing(cx: &mut TestAppContext) {
    let h = open_sidebar(cx);
    let seen = sidebar_events(cx, &h.view);
    select_row(cx, &h, "health.yml");
    cx.update_window(h.win, |_, w, cx| w.press("shift-enter", cx))
        .unwrap();
    cx.run_until_parked();
    assert!(!seen.borrow().iter().any(|e| matches!(
        e,
        SidebarEvent::Run { .. } | SidebarEvent::RunSelection { .. }
    )));
}

#[gpui_kit::test]
fn shift_enter_runs_the_selected_collection(cx: &mut TestAppContext) {
    let h = open_sidebar(cx);
    let seen = sidebar_events(cx, &h.view);
    let tree = h.view.read_with(cx, |s, _| s.tree().clone());
    tree.update(cx, |t, cx| t.set_selected_index(Some(0), cx));
    focus_tree(cx, &h);
    frame(cx, h.win);
    cx.update_window(h.win, |_, w, cx| w.press("shift-enter", cx))
        .unwrap();
    cx.run_until_parked();
    let expected = SidebarEvent::Run {
        root: h.root.clone(),
        folder: None,
    };
    assert_eq!(seen.borrow().last(), Some(&expected));
}

// The run button's tooltip names the Shift+Enter key bound to running the selected row.
#[gpui_kit::test]
fn the_run_button_tooltip_names_shift_enter(cx: &mut TestAppContext) {
    open_sidebar(cx);
    let tip = cx.update(|cx| apinata_app::sidebar::run_tooltip("collection.run", cx));
    assert!(tip.ends_with("(Shift+Enter)"), "tooltip was {tip:?}");
}
