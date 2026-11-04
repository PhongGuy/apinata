//! The Changes dialog's file list by keyboard, frame by frame: Shift+Tab reaches it with a
//! visible ring, Down selects, and Space or Enter stage the selected row without closing.

use crate::git_support;
use crate::h;
use crate::support;

use apinata_app::git_status::GitStage::{Staged, Unstaged};
use git_support::Sandbox;
use gpui_kit::component::WindowExt as _;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, TestAppContext};
use h::*;
use support::*;

// Whether the last drawn frame paints the list's focus ring marker.
fn list_ring_drawn(cx: &mut TestAppContext, s: &Started) -> bool {
    cx.update_window(s.win, |_, w, _| w.try_find("changes-files-ring").is_some())
        .unwrap()
}

fn dialog_open(cx: &mut TestAppContext, s: &Started) -> bool {
    cx.update_window(s.win, |_, w, cx| w.has_active_dialog(cx))
        .unwrap()
}

fn selected(cx: &mut TestAppContext) -> Option<String> {
    view(cx).read_with(cx, |v, _| {
        v.selected().map(|p| p.to_string_lossy().into_owned())
    })
}

// The dialog open with focus in the message field, its frame drawn.
fn from_message(cx: &mut TestAppContext, s: &Started, col: &tempfile::TempDir) {
    open_on(cx, s, col.path());
    click(cx, s, "changes-message");
    frame(cx, s.win);
}

// Two Shift+Tabs from the message field, through Stage all, onto the list.
fn back_to_list(cx: &mut TestAppContext, s: &Started) {
    for _ in 0..2 {
        platform_press(cx, s.win, "shift-tab");
        next_frame(cx, s.win);
    }
}

#[gpui_kit::test]
fn shift_tab_from_the_message_reaches_the_list_with_its_ring(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let col = changed(&sb);
    let s = git_started(cx, &sb, &col);
    from_message(cx, &s, &col);
    assert!(
        !list_ring_drawn(cx, &s),
        "no ring while the message has focus"
    );
    back_to_list(cx, &s);
    assert!(ring_on(cx, s.win, "changes-files"), "the list holds focus");
    assert!(list_ring_drawn(cx, &s), "the drawn frame shows its ring");
}

#[gpui_kit::test]
fn down_in_the_list_selects_the_next_row_and_loads_its_diff(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let col = changed(&sb);
    let s = git_started(cx, &sb, &col);
    from_message(cx, &s, &col);
    back_to_list(cx, &s);
    assert_eq!(selected(cx).as_deref(), Some("a.yml"));
    platform_press(cx, s.win, "down");
    next_frame(cx, s.win);
    assert_eq!(selected(cx).as_deref(), Some("new.yml"));
    wait_for(cx, "the diff loads", |cx| {
        view(cx).read_with(cx, |v, _| v.doc().is_some())
    });
}

#[gpui_kit::test]
fn space_in_the_list_toggles_the_selected_rows_stage(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let col = changed(&sb);
    let s = git_started(cx, &sb, &col);
    from_message(cx, &s, &col);
    back_to_list(cx, &s);
    platform_press(cx, s.win, "space");
    idle(cx, &s);
    wait_for(cx, "staged", |cx| stages(cx) == [Staged, Unstaged]);
    platform_press(cx, s.win, "space");
    idle(cx, &s);
    wait_for(cx, "unstaged", |cx| stages(cx) == [Unstaged, Unstaged]);
}

#[gpui_kit::test]
fn enter_in_the_list_toggles_the_selected_rows_stage_and_keeps_the_dialog(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    let col = changed(&sb);
    let s = git_started(cx, &sb, &col);
    from_message(cx, &s, &col);
    back_to_list(cx, &s);
    platform_press(cx, s.win, "down");
    next_frame(cx, s.win);
    platform_press(cx, s.win, "enter");
    next_frame(cx, s.win);
    assert!(dialog_open(cx, &s), "Enter leaves the dialog open");
    idle(cx, &s);
    wait_for(cx, "the second file staged", |cx| {
        stages(cx) == [Unstaged, Staged]
    });
    platform_press(cx, s.win, "enter");
    idle(cx, &s);
    wait_for(cx, "unstaged again", |cx| {
        stages(cx) == [Unstaged, Unstaged]
    });
    assert!(dialog_open(cx, &s));
}
