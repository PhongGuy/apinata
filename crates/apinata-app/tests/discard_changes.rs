//! Discard changes: the palette's revert of the active tab, asked first.

use crate::support;

use apinata_app::keymap::DiscardChanges;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, Focusable as _, TestAppContext};
use support::*;

fn discard(cx: &mut TestAppContext, s: &Started) {
    let shell = s.shell.clone();
    cx.update_window(s.win, |_, w, cx| {
        shell.read(cx).focus_handle(cx).focus(w, cx);
        w.dispatch_action(Box::new(DiscardChanges), cx);
    })
    .unwrap();
    cx.run_until_parked();
}

fn dirty(cx: &mut TestAppContext, s: &Started) -> bool {
    ws_snap(cx, &s.ws, &s.root).content.requests[&ip("a.yml")].dirty
}

#[gpui_kit::test]
fn discard_asks_then_reverts_the_active_tab(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    open_tab(cx, &s.ws, &s.root, "a.yml");
    frame(cx, s.win);
    s.ws.update(cx, |w, cx| {
        w.send(&s.root, edit_url("a.yml", "http://changed"), cx)
    });
    cx.run_until_parked();
    assert!(dirty(cx, &s));
    discard(cx, &s);
    assert!(confirm_open(cx));
    answer_confirm(cx, "confirm-cancel");
    assert!(dirty(cx, &s), "cancel keeps the edit");
    discard(cx, &s);
    answer_confirm(cx, "confirm-discard");
    cx.run_until_parked();
    assert!(!dirty(cx, &s));
    assert_eq!(url(&ws_snap(cx, &s.ws, &s.root), "a.yml"), "http://x/0");
}

#[gpui_kit::test]
fn a_clean_tab_discards_without_asking(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    open_tab(cx, &s.ws, &s.root, "a.yml");
    frame(cx, s.win);
    discard(cx, &s);
    assert!(!confirm_open(cx));
}

// Opens the first tab's menu and clicks Discard changes.
fn discard_from_menu(cx: &mut TestAppContext, s: &Started) {
    cx.update_window(s.win, |_, w, cx| w.right_click(("tab", 0usize), cx))
        .unwrap();
    frame(cx, s.win);
    click_menu(cx, s.win, "Discard changes");
}

#[gpui_kit::test]
fn the_tab_menu_discards_a_changed_tab_after_asking(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    open_tab(cx, &s.ws, &s.root, "a.yml");
    frame(cx, s.win);
    s.ws.update(cx, |w, cx| {
        w.send(&s.root, edit_url("a.yml", "http://changed"), cx)
    });
    cx.run_until_parked();
    discard_from_menu(cx, &s);
    answer_confirm(cx, "confirm-discard");
    assert!(!dirty(cx, &s));
}

#[gpui_kit::test]
fn the_tab_menu_discard_does_nothing_for_a_clean_tab(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    open_tab(cx, &s.ws, &s.root, "a.yml");
    frame(cx, s.win);
    discard_from_menu(cx, &s);
    assert!(!confirm_open(cx));
}

// A draft has no saved file to revert to, so its tab menu greys Discard changes out even
// when it holds an edit; the kit's menu does not expose the flag, so the click is the check.
#[gpui_kit::test]
fn the_tab_menu_discard_is_disabled_for_a_draft(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    s.ws.update(cx, |w, cx| {
        w.new_draft(&s.root, apinata_core::ItemPath::root(), cx)
    })
    .expect("a draft");
    cx.run_until_parked();
    let draft = s.ws.read_with(cx, |w, _| {
        let item = w.tabs().active().and_then(|e| e.item()).expect("a draft");
        item.as_str().to_owned()
    });
    s.ws.update(cx, |w, cx| {
        w.send(&s.root, edit_url(&draft, "http://changed"), cx)
    });
    cx.run_until_parked();
    frame(cx, s.win);
    discard_from_menu(cx, &s);
    assert!(!confirm_open(cx));
    assert_eq!(url(&ws_snap(cx, &s.ws, &s.root), &draft), "http://changed");
}
