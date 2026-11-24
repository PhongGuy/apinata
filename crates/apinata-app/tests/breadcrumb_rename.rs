//! Double-clicking the request name in the breadcrumb renames the request in place, through
//! the sidebar's one rename path.

use crate::git_support::Sandbox;
use crate::support;
use crate::support::team::*;

use apinata_core::team::Role;
use gpui_kit::component::WindowExt as _;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, TestAppContext};
use std::time::Duration;
use support::*;

const FIELD: &str = "breadcrumb-rename-field";
const CRUMB: &str = "breadcrumb-rename";
const ERROR: &str = "breadcrumb-rename-error";

fn nested() -> tempfile::TempDir {
    write_tree(&[
        ("opencollection.yml", ROOT),
        ("users/folder.yml", &folder("Users")),
        ("users/list.yml", &request("List", "http://x/list")),
        ("users/other.yml", &request("Other", "http://x/other")),
    ])
}

fn start(cx: &mut TestAppContext) -> (tempfile::TempDir, Started) {
    let col = nested();
    let s = started(cx, &col);
    // Focus events carry empty paths while the window is inactive, so no blur would fire.
    cx.update_window(s.win, |_, w, _| w.activate_window())
        .unwrap();
    open_tab(cx, &s.ws, &s.root, "users/list.yml");
    frame(cx, s.win);
    (col, s)
}

fn double_click_name(cx: &mut TestAppContext, s: &Started) {
    cx.update_window(s.win, |_, w, cx| {
        w.render_frame(cx);
        w.double_click("breadcrumb-file", cx)
    })
    .unwrap();
    frame(cx, s.win);
}

fn type_and(cx: &mut TestAppContext, s: &Started, text: &str, key: &str) {
    cx.update_window(s.win, |_, w, cx| {
        w.input(text, cx);
        w.press(key, cx);
    })
    .unwrap();
    settle(cx);
    frame(cx, s.win);
}

fn editing(cx: &mut TestAppContext, s: &Started) -> bool {
    shown(cx, s.win, FIELD)
}

fn acked(cx: &mut TestAppContext, s: &Started) -> u64 {
    ws_snap(cx, &s.ws, &s.root).acked
}

fn toasts(cx: &mut TestAppContext, s: &Started) -> usize {
    cx.update_window(s.win, |_, w, cx| w.notifications(cx).len())
        .unwrap()
}

fn dirty(cx: &mut TestAppContext, s: &Started, rel: &str) -> bool {
    let snap = ws_snap(cx, &s.ws, &s.root);
    let row = snap.content.rows.iter().find(|r| r.path == ip(rel));
    row.expect("a row").dirty
}

fn tip_after_hover(cx: &mut TestAppContext, s: &Started, target: impl Into<gpui_kit::ElementId>) {
    let target = target.into();
    cx.update_window(s.win, |_, w, cx| {
        w.render_frame(cx);
        w.hover(target, cx)
    })
    .unwrap();
    cx.executor().advance_clock(Duration::from_millis(800));
    frame(cx, s.win);
}

fn label_of(cx: &mut TestAppContext, s: &Started, id: &'static str) -> Option<String> {
    cx.update_window(s.win, |_, w, _| {
        w.try_find(id).and_then(|e| e.label().map(str::to_owned))
    })
    .unwrap()
}

#[gpui_kit::test]
fn double_click_then_enter_renames_the_file_and_the_tab(cx: &mut TestAppContext) {
    let (_col, s) = start(cx);
    cx.update_window(s.win, |_, w, cx| w.click("breadcrumb-file", cx))
        .unwrap();
    frame(cx, s.win);
    assert!(!editing(cx, &s), "a single click stays inert");
    double_click_name(cx, &s);
    assert!(editing(cx, &s), "a double-click opens the field");
    // The whole name is selected, so typing replaces it.
    type_and(cx, &s, "Everyone", "enter");
    assert!(!editing(cx, &s));
    assert!(s.root.join("users/Everyone.yml").exists());
    assert!(!s.root.join("users/list.yml").exists());
    assert_eq!(tab_names(cx, &s.ws), vec!["users/Everyone.yml".to_string()]);
    assert_eq!(
        view_of(cx, &s.ws, &s.root, "users/Everyone.yml").name,
        "Everyone"
    );
    assert_eq!(
        label_of(cx, &s, CRUMB).as_deref(),
        Some("Rename request Everyone")
    );
}

#[gpui_kit::test]
fn escape_a_blank_or_an_unchanged_name_send_nothing(cx: &mut TestAppContext) {
    let (_col, s) = start(cx);
    let before = acked(cx, &s);
    double_click_name(cx, &s);
    assert!(editing(cx, &s));
    type_and(cx, &s, "Another", "escape");
    assert!(!editing(cx, &s), "Escape closes the field");
    double_click_name(cx, &s);
    assert!(editing(cx, &s));
    type_and(cx, &s, "List", "enter");
    assert!(!editing(cx, &s), "an unchanged name closes the field");
    double_click_name(cx, &s);
    assert!(editing(cx, &s));
    type_and(cx, &s, "   ", "enter");
    assert!(!editing(cx, &s), "a blank name closes the field");
    assert!(s.root.join("users/list.yml").exists());
    assert!(!s.root.join("users/Another.yml").exists());
    assert_eq!(acked(cx, &s), before, "no command reached the actor");
}

#[gpui_kit::test]
fn a_siblings_name_keeps_the_field_open_with_the_error(cx: &mut TestAppContext) {
    let (_col, s) = start(cx);
    let before = acked(cx, &s);
    double_click_name(cx, &s);
    type_and(cx, &s, "Other", "enter");
    assert!(editing(cx, &s), "the field stays open");
    assert_eq!(
        label_of(cx, &s, ERROR).as_deref(),
        Some("A request named \"Other\" is already in \"Users\"")
    );
    assert_eq!(acked(cx, &s), before, "nothing was sent");
    // One more keystroke answers the refusal, and Enter then renames.
    type_and(cx, &s, "s", "enter");
    assert!(!editing(cx, &s));
    assert!(s.root.join("users/Others.yml").exists());
}

#[gpui_kit::test]
fn clicking_away_commits_and_a_taken_name_is_dropped_with_a_toast(cx: &mut TestAppContext) {
    let (_col, s) = start(cx);
    double_click_name(cx, &s);
    cx.update_window(s.win, |_, w, cx| w.input("Everyone", cx))
        .unwrap();
    cx.update_window(s.win, |_, w, cx| w.click("url", cx))
        .unwrap();
    settle(cx);
    assert!(!editing(cx, &s));
    assert!(s.root.join("users/Everyone.yml").exists(), "blur commits");

    let before = toasts(cx, &s);
    double_click_name(cx, &s);
    cx.update_window(s.win, |_, w, cx| w.input("Other", cx))
        .unwrap();
    cx.update_window(s.win, |_, w, cx| w.click("url", cx))
        .unwrap();
    settle(cx);
    assert!(!editing(cx, &s), "a field left behind closes");
    assert_eq!(toasts(cx, &s), before + 1, "the refusal shows as a toast");
    assert!(s.root.join("users/Everyone.yml").exists());
}

#[gpui_kit::test]
fn a_draft_does_not_start_a_rename(cx: &mut TestAppContext) {
    let col = nested();
    let s = started(cx, &col);
    s.ws.update(cx, |w, cx| w.new_draft(&s.root, ip("users"), cx));
    settle(cx);
    frame(cx, s.win);
    active_draft(cx, &s.ws);
    double_click_name(cx, &s);
    assert!(!editing(cx, &s));
    assert!(!shown(cx, s.win, CRUMB), "the name is not a rename target");
}

#[gpui_kit::test]
fn a_scratch_request_does_not_start_a_rename(cx: &mut TestAppContext) {
    let s = started_blank(cx, blank_data_dir());
    s.ws.update(cx, |w, cx| w.new_scratch_request(cx));
    settle(cx);
    frame(cx, s.win);
    double_click_name(cx, &s);
    assert!(!editing(cx, &s));
    assert!(!shown(cx, s.win, CRUMB));
}

#[gpui_kit::test]
fn a_read_only_collection_does_not_start_a_rename(cx: &mut TestAppContext) {
    let (sb, col) = (Sandbox::new(), collection());
    shared_origin(&sb, col.path());
    http_ready(cx);
    let s = team_started(cx, &sb, &col, |_, _| {}).0;
    open_tab(cx, &s.ws, &s.root, "a.yml");
    assert!(shown(cx, s.win, CRUMB), "a writable request can rename");
    start_team_as(cx, &sb, &s, Role::Read, Duration::from_millis(50));
    assert!(shown(cx, s.win, "breadcrumb-read-only"));
    double_click_name(cx, &s);
    assert!(!editing(cx, &s));
    assert!(!shown(cx, s.win, CRUMB));
}

#[gpui_kit::test]
fn f2_or_enter_on_the_focused_crumb_starts_the_rename(cx: &mut TestAppContext) {
    let (_col, s) = start(cx);
    key_on(cx, s.win, CRUMB, "f2");
    frame(cx, s.win);
    assert!(editing(cx, &s), "F2 starts it");
    type_and(cx, &s, "", "escape");
    assert!(!editing(cx, &s));
    assert!(holds_focus(cx, s.win, CRUMB), "Escape hands focus back");
    cx.update_window(s.win, |_, w, cx| w.press("enter", cx))
        .unwrap();
    frame(cx, s.win);
    assert!(editing(cx, &s), "Enter starts it");
}

#[gpui_kit::test]
fn a_rebound_rename_key_starts_the_rename_and_f2_no_longer_does(cx: &mut TestAppContext) {
    let (_col, s) = start(cx);
    edit_keys(cx, |o| {
        *o = overrides(&[("request.rename_in_sidebar", &["shift-f2"])])
    });
    key_on(cx, s.win, CRUMB, "f2");
    frame(cx, s.win);
    assert!(!editing(cx, &s), "F2 left with the rebinding");
    key_on(cx, s.win, CRUMB, "shift-f2");
    frame(cx, s.win);
    assert!(editing(cx, &s), "the new key starts it");
}

#[gpui_kit::test]
fn undo_in_sidebar_reverts_a_breadcrumb_rename(cx: &mut TestAppContext) {
    let (_col, s) = start(cx);
    double_click_name(cx, &s);
    type_and(cx, &s, "Everyone", "enter");
    assert!(s.root.join("users/Everyone.yml").exists());
    let sidebar = s.shell.read_with(cx, |sh, _| sh.sidebar().clone());
    let tree = sidebar.read_with(cx, |v, _| v.tree().clone());
    cx.update_window(s.win, |_, w, cx| tree.update(cx, |t, cx| t.focus(w, cx)))
        .unwrap();
    cx.update_window(s.win, |_, w, cx| w.press(&secondary("z"), cx))
        .unwrap();
    settle(cx);
    assert!(
        s.root.join("users/list.yml").exists(),
        "the rename was undone"
    );
    assert!(!s.root.join("users/Everyone.yml").exists());
    let back = std::fs::read_to_string(s.root.join("users/list.yml")).unwrap();
    assert!(back.contains("name: List"), "the old name is back");
}

#[gpui_kit::test]
fn unsaved_edits_survive_the_rename(cx: &mut TestAppContext) {
    let (_col, s) = start(cx);
    type_at_end(cx, s.win, None, "url", "/early");
    settle(cx);
    assert!(dirty(cx, &s, "users/list.yml"));
    double_click_name(cx, &s);
    // An edit typed after Enter, before the Moved notice comes back, still lands.
    cx.update_window(s.win, |_, w, cx| {
        w.input("Everyone", cx);
        w.press("enter", cx);
        w.click("url", cx);
        w.press("end", cx);
        w.input("/late", cx);
    })
    .unwrap();
    settle(cx);
    frame(cx, s.win);
    let rel = "users/Everyone.yml";
    assert!(s.root.join(rel).exists());
    assert_eq!(
        view_of(cx, &s.ws, &s.root, rel).url,
        "http://x/list/early/late"
    );
    assert!(dirty(cx, &s, rel), "the edits stay unsaved");
    let saved = std::fs::read_to_string(s.root.join(rel)).unwrap();
    assert!(!saved.contains("/early"), "{saved}");
}

#[gpui_kit::test]
fn the_crumbs_tooltip_replaces_the_lines_path_tooltip(cx: &mut TestAppContext) {
    let (_col, s) = start(cx);
    tip_after_hover(cx, &s, ("breadcrumb-folder", 0usize));
    assert_eq!(
        label_of(cx, &s, "breadcrumb-tooltip").as_deref(),
        Some("users/list.yml")
    );
    tip_after_hover(cx, &s, "breadcrumb-file");
    assert_eq!(
        label_of(cx, &s, "breadcrumb-file-tooltip").as_deref(),
        Some("Double-click to rename")
    );
    assert_eq!(label_of(cx, &s, "breadcrumb-tooltip"), None);
}
