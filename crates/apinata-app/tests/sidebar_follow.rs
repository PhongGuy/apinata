//! The sidebar selects the active tab's request, opening its folders and scrolling to it,
//! without taking focus.

use crate::support;

use apinata_app::sidebar::marker_id;
use apinata_app::tabs::TabKind;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, Focusable as _, TestAppContext};
use support::*;

fn selected(cx: &mut TestAppContext, h: &SidebarHarness) -> Option<String> {
    h.view
        .read_with(cx, |s, cx| s.selected(cx))
        .map(|(_, p)| p.as_str().to_owned())
}

/// Edits `status.yml`, then notifies the workspace so the sidebar rebuilds, and proves the
/// rebuild ran by the dirty dot it draws. A test that expects a selection to stay put calls
/// this, so "unchanged" means "unchanged through a rebuild".
fn rebuild(cx: &mut TestAppContext, h: &SidebarHarness) {
    h.ws.update(cx, |w, cx| {
        w.send(&h.root, edit_url("status.yml", "http://x/9"), cx)
    });
    cx.run_until_parked();
    h.ws.update(cx, |_, cx| cx.notify());
    frame(cx, h.win);
    assert!(
        shown(cx, h.win, marker_id("dirty", &row("status.yml"))),
        "the sidebar rebuilt"
    );
}

#[gpui_kit::test]
fn activating_a_tab_selects_its_row_and_opens_its_folder(cx: &mut TestAppContext) {
    let h = open_sidebar(cx);
    assert!(
        !shown(cx, h.win, row("users/list.yml")),
        "users starts collapsed"
    );
    open_tab(cx, &h.ws, &h.root, "users/list.yml");
    frame(cx, h.win);
    assert_eq!(selected(cx, &h).as_deref(), Some("users/list.yml"));
    assert!(shown(cx, h.win, row("users/list.yml")), "the folder opened");
}

#[gpui_kit::test]
fn switching_tabs_moves_the_selection(cx: &mut TestAppContext) {
    let h = open_sidebar(cx);
    open_tab(cx, &h.ws, &h.root, "health.yml");
    open_tab(cx, &h.ws, &h.root, "status.yml");
    frame(cx, h.win);
    assert_eq!(selected(cx, &h).as_deref(), Some("status.yml"));
    h.ws.update(cx, |w, cx| w.update_tabs(cx, |t| t.activate_index(0)));
    frame(cx, h.win);
    assert_eq!(selected(cx, &h).as_deref(), Some("health.yml"));
}

#[gpui_kit::test]
fn a_click_elsewhere_is_kept_until_the_active_tab_changes(cx: &mut TestAppContext) {
    let h = open_sidebar(cx);
    open_tab(cx, &h.ws, &h.root, "health.yml");
    frame(cx, h.win);
    cx.update_window(h.win, |_, w, cx| w.click(row("users"), cx))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(selected(cx, &h).as_deref(), Some("users"));
    rebuild(cx, &h);
    assert_eq!(
        selected(cx, &h).as_deref(),
        Some("users"),
        "a rebuild keeps the click"
    );
}

#[gpui_kit::test]
fn a_settings_tab_leaves_the_selection_alone(cx: &mut TestAppContext) {
    let h = open_sidebar(cx);
    open_tab(cx, &h.ws, &h.root, "health.yml");
    frame(cx, h.win);
    h.ws.update(cx, |w, cx| {
        w.open_settings(&h.root, TabKind::Collection, cx)
    })
    .expect("a settings tab");
    rebuild(cx, &h);
    assert_eq!(selected(cx, &h).as_deref(), Some("health.yml"));
}

#[gpui_kit::test]
fn following_does_not_take_focus(cx: &mut TestAppContext) {
    let h = open_sidebar(cx);
    let filter = h.view.read_with(cx, |s, _| s.filter().clone());
    cx.update_window(h.win, |_, w, cx| filter.update(cx, |i, cx| i.focus(w, cx)))
        .unwrap();
    open_tab(cx, &h.ws, &h.root, "users/list.yml");
    frame(cx, h.win);
    assert_eq!(selected(cx, &h).as_deref(), Some("users/list.yml"));
    let focused = cx
        .update_window(h.win, |_, w, cx| filter.focus_handle(cx).is_focused(w))
        .unwrap();
    assert!(focused, "the filter keeps focus");
}

#[gpui_kit::test]
fn a_row_below_the_fold_scrolls_into_view(cx: &mut TestAppContext) {
    // 80 rows of 28 px overflow the 1095 px default test window.
    let mut files: Vec<(&'static str, String)> = vec![("opencollection.yml", ROOT.to_string())];
    for i in 0..80 {
        let path: &'static str = Box::leak(format!("r{i:02}.yml").into_boxed_str());
        files.push((path, request(&format!("r{i:02}"), "http://x/0")));
    }
    files.push(("zz/folder.yml", folder("zz")));
    files.push(("zz/last.yml", request("last", "http://x/last")));
    let h = open_sidebar_with(cx, files);
    assert!(!shown(cx, h.win, row("zz/last.yml")));
    open_tab(cx, &h.ws, &h.root, "zz/last.yml");
    frame(cx, h.win);
    frame(cx, h.win);
    cx.update_window(h.win, |_, w, _| {
        let b = w.find(row("zz/last.yml")).bounds();
        assert!(b.bottom() <= w.viewport_size().height, "{b:?} is on screen");
    })
    .unwrap();
}

#[gpui_kit::test]
fn renaming_the_followed_folder_keeps_the_click(cx: &mut TestAppContext) {
    let h = open_sidebar(cx);
    open_tab(cx, &h.ws, &h.root, "users/list.yml");
    frame(cx, h.win);
    cx.update_window(h.win, |_, w, cx| w.click(row("users"), cx))
        .unwrap();
    cx.run_until_parked();
    let (view, root) = (h.view.clone(), h.root.clone());
    cx.update_window(h.win, |_, w, cx| {
        view.update(cx, |s, cx| s.start_rename(&root, &ip("users"), w, cx))
    })
    .unwrap();
    frame(cx, h.win);
    cx.update_window(h.win, |_, w, cx| {
        w.input("people", cx);
        w.press("enter", cx);
    })
    .unwrap();
    frame(cx, h.win);
    assert!(h.root.join("people/list.yml").exists());
    assert_eq!(
        selected(cx, &h).as_deref(),
        Some("people"),
        "the click stays"
    );
}

#[gpui_kit::test]
fn revealing_a_root_clears_a_filter_that_hides_it(cx: &mut TestAppContext) {
    let h = open_sidebar(cx);
    let filter = h.view.read_with(cx, |s, _| s.filter().clone());
    cx.update_window(h.win, |_, w, cx| filter.update(cx, |i, cx| i.focus(w, cx)))
        .unwrap();
    cx.update_window(h.win, |_, w, cx| w.input("no-such-row", cx))
        .unwrap();
    frame(cx, h.win);
    assert!(!shown(cx, h.win, row("")), "the filter hides the root");
    cx.update_window(h.win, |_, w, cx| {
        h.view.update(cx, |s, cx| s.reveal_root(&h.root, w, cx))
    })
    .unwrap();
    frame(cx, h.win);
    assert_eq!(filter.read_with(cx, |i, _| i.value().to_string()), "");
    assert_eq!(selected(cx, &h).as_deref(), Some(""));
    assert!(shown(cx, h.win, row("")));
}
