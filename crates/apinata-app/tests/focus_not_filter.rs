//! Focus goes somewhere useful, never onto the shell's bare handle or the sidebar Filter, when
//! a collection closes, Preferences closes or Ctrl+E opens Environments.

use crate::support;

use apinata_app::sidebar::menu::{MenuEntry, menu_entries};
use apinata_app::sidebar::model::{NodeKind, node_id};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, Focusable as _, TestAppContext, VisualTestContext};
use support::*;

fn press(cx: &mut TestAppContext, s: &Started, ks: &str) {
    let mut v = VisualTestContext::from_window(s.win, cx);
    v.simulate_keystrokes(ks);
    v.run_until_parked();
    settle(cx, s);
}

// Draws, then lets every dialog exit and deferred focus move finish.
fn settle(cx: &mut TestAppContext, s: &Started) {
    frame(cx, s.win);
    cx.executor()
        .advance_clock(std::time::Duration::from_secs(2));
    frame(cx, s.win);
}

fn filter_focused(cx: &mut TestAppContext, s: &Started) -> bool {
    let filter = s
        .shell
        .read_with(cx, |sh, cx| sh.sidebar().read(cx).filter().focus_handle(cx));
    cx.update_window(s.win, |_, w, _| filter.is_focused(w))
        .unwrap()
}

fn shell_focused(cx: &mut TestAppContext, s: &Started) -> bool {
    let shell = s.shell.read_with(cx, |sh, cx| sh.focus_handle(cx));
    cx.update_window(s.win, |_, w, _| shell.is_focused(w))
        .unwrap()
}

fn tree_focused(cx: &mut TestAppContext, s: &Started) -> bool {
    cx.update_window(s.win, |_, w, _| w.find("sidebar-tree").focused())
        .unwrap()
        == Some(true)
}

fn focus_tree(cx: &mut TestAppContext, s: &Started) {
    let tree = s
        .shell
        .read_with(cx, |sh, cx| sh.sidebar().read(cx).tree().clone());
    cx.update_window(s.win, |_, w, cx| tree.update(cx, |t, cx| t.focus(w, cx)))
        .unwrap();
    settle(cx, s);
}

#[gpui_kit::test]
fn ctrl_e_moves_focus_into_the_environments_tab(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    focus_tree(cx, &s);
    press(cx, &s, &key("e"));
    let area = s.shell.read_with(cx, |sh, _| sh.tab_area().clone());
    let (stop, own) = area.read_with(cx, |a, cx| (a.active_focus(cx), a.focus_handle(cx)));
    let stop = stop.expect("the Environments tab is open");
    assert_ne!(stop, own, "the tab's view is built");
    let held = cx
        .update_window(s.win, |_, w, cx| stop.contains_focused(w, cx))
        .unwrap();
    assert!(held, "focus is in the new tab");
    assert!(!filter_focused(cx, &s));
}

// Closing one of two collections from its menu leaves focus in the tree of the one still open.
#[gpui_kit::test]
fn closing_a_collection_leaves_focus_in_the_tree(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    let other = two_requests();
    s.ws.update(cx, |w, cx| w.open(other.path().to_path_buf(), cx));
    settle(cx, &s);
    let tab = open_tab(cx, &s.ws, other.path(), "a.yml");
    settle(cx, &s);
    let url = editor_of(cx, &s, tab).read_with(cx, |e, cx| e.url_focus(cx));
    cx.update_window(s.win, |_, w, cx| url.focus(w, cx))
        .unwrap();
    settle(cx, &s);
    let ix = menu_entries(NodeKind::Collection)
        .iter()
        .position(|e| *e == MenuEntry::Close)
        .expect("Close is offered");
    cx.update_window(s.win, |_, w, cx| {
        w.render_frame(cx);
        w.right_click(node_id(1, &ip("")), cx)
    })
    .unwrap();
    settle(cx, &s);
    cx.update_window(s.win, |_, w, cx| w.within("popup-menu").click(ix, cx))
        .unwrap();
    settle(cx, &s);
    assert_eq!(s.ws.read_with(cx, |w, _| w.collections().len()), 1);
    assert!(!filter_focused(cx, &s), "not the Filter");
    assert!(!shell_focused(cx, &s), "not the bare shell");
    assert!(tree_focused(cx, &s), "the tree holds focus");
}

// Preferences hands focus back to the control that had it.
#[gpui_kit::test]
fn closing_preferences_returns_focus_to_where_it_was(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    let tab = open_tab(cx, &s.ws, &s.root, "a.yml");
    settle(cx, &s);
    let url = editor_of(cx, &s, tab).read_with(cx, |e, cx| e.url_focus(cx));
    cx.update_window(s.win, |_, w, cx| url.focus(w, cx))
        .unwrap();
    settle(cx, &s);
    press(cx, &s, &key(","));
    press(cx, &s, "escape");
    let back = cx
        .update_window(s.win, |_, w, _| url.is_focused(w))
        .unwrap();
    assert!(back, "the URL has focus again");
    assert!(!filter_focused(cx, &s));
}
