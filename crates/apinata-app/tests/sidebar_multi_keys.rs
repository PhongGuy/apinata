use crate::support;

use apinata_app::sidebar::{CloneSelected, RenameSelected};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, TestAppContext};
use support::*;

fn keys(cx: &mut TestAppContext, h: &SidebarHarness) -> Vec<String> {
    h.view.read_with(cx, |s, _| {
        s.multi_keys()
            .into_iter()
            .map(|(_, p)| p.as_str().to_owned())
            .collect()
    })
}
fn focus(cx: &mut TestAppContext, h: &SidebarHarness) -> Option<String> {
    h.view.read_with(cx, |s, cx| {
        s.selected(cx).map(|(_, p)| p.as_str().to_owned())
    })
}
fn press(cx: &mut TestAppContext, h: &SidebarHarness, key: &str) {
    cx.update_window(h.win, |_, w, cx| w.press(key, cx))
        .unwrap();
    frame(cx, h.win);
}
fn start(cx: &mut TestAppContext) -> SidebarHarness {
    let h = open_sidebar_with(cx, multi_files());
    cx.update_window(h.win, |_, w, cx| w.click(row("a.yml"), cx))
        .unwrap();
    focus_tree(cx, &h);
    frame(cx, h.win);
    h
}

#[gpui_kit::test]
fn shift_down_extends_and_shift_up_shrinks_from_the_anchor(cx: &mut TestAppContext) {
    let h = start(cx);
    press(cx, &h, "shift-down");
    press(cx, &h, "shift-down");
    assert_eq!(keys(cx, &h), ["a.yml", "b.yml", "c.yml"]);
    assert_eq!(focus(cx, &h).as_deref(), Some("c.yml"));
    press(cx, &h, "shift-up");
    assert_eq!(keys(cx, &h), ["a.yml", "b.yml"]);
    assert_eq!(focus(cx, &h).as_deref(), Some("b.yml"));
}

#[gpui_kit::test]
fn escape_clears_the_set_and_keeps_the_focus_row(cx: &mut TestAppContext) {
    let h = start(cx);
    secondary_click(cx, h.win, row("c.yml"));
    press(cx, &h, "escape");
    assert!(keys(cx, &h).is_empty());
    assert_eq!(focus(cx, &h).as_deref(), Some("c.yml"));
}

#[gpui_kit::test]
fn plain_arrows_clear_the_set(cx: &mut TestAppContext) {
    let h = start(cx);
    secondary_click(cx, h.win, row("c.yml"));
    press(cx, &h, "down");
    assert!(keys(cx, &h).is_empty());
}

#[gpui_kit::test]
fn escape_with_no_set_still_ends_a_rename(cx: &mut TestAppContext) {
    let h = start(cx);
    press(cx, &h, "f2");
    assert!(h.view.read_with(cx, |s, _| s.renaming().is_some()));
    press(cx, &h, "escape");
    assert!(h.view.read_with(cx, |s, _| s.renaming().is_none()));
}

#[gpui_kit::test]
fn delete_and_backspace_inside_the_rename_input_edit_text_and_never_delete(
    cx: &mut TestAppContext,
) {
    let h = start(cx);
    press(cx, &h, "f2");
    assert!(h.view.read_with(cx, |s, _| s.renaming().is_some()));
    press(cx, &h, "backspace");
    press(cx, &h, "delete");
    assert!(
        !confirm_open(cx),
        "DeleteSelected must not fire from the rename input"
    );
    assert!(
        h.view.read_with(cx, |s, _| s.renaming().is_some()),
        "the rename is still open"
    );
    assert!(h.root.join("a.yml").exists());
}

#[gpui_kit::test]
fn rename_duplicate_and_open_are_ignored_while_the_set_is_active(cx: &mut TestAppContext) {
    let h = start(cx);
    let seen = sidebar_events(cx, &h.view);
    secondary_click(cx, h.win, row("c.yml"));
    let before = seen.borrow().len();
    cx.update_window(h.win, |_, w, cx| {
        h.view.update(cx, |s, cx| {
            s.rename_selected(&RenameSelected, w, cx);
            s.clone_selected(&CloneSelected, w, cx);
        })
    })
    .unwrap();
    press(cx, &h, "enter");
    press(cx, &h, "space");
    assert!(h.view.read_with(cx, |s, _| s.renaming().is_none()));
    assert!(!h.root.join("C copy.yml").exists() && !h.root.join("A copy.yml").exists());
    assert_eq!(
        seen.borrow().len(),
        before,
        "no OpenItem: {:?}",
        &seen.borrow()[before..]
    );
}

#[gpui_kit::test]
fn backspace_in_the_filter_edits_text_and_leaves_the_set(cx: &mut TestAppContext) {
    let h = start(cx);
    secondary_click(cx, h.win, row("c.yml"));
    let filter = h.view.read_with(cx, |s, _| s.filter().clone());
    cx.update_window(h.win, |_, w, cx| filter.update(cx, |i, cx| i.focus(w, cx)))
        .unwrap();
    cx.update_window(h.win, |_, w, cx| w.input("zz", cx))
        .unwrap();
    press(cx, &h, "backspace");
    assert_eq!(filter.read_with(cx, |i, _| i.value().to_string()), "z");
    assert!(
        h.root.join("a.yml").exists() && h.root.join("c.yml").exists(),
        "nothing was deleted"
    );
}
