//! Undoing a delete into the trash reopens the tabs it closed, and the delete stays undoable
//! from the keyboard and the palette with the sidebar hidden.

use crate::sidebar_trash::{
    assert_restored, click_undo, ctrl_z, focus_url, menu_delete, palette_delete, press,
    select_and_delete, shell_with_trash,
};
use crate::support::*;

use apinata_app::sidebar::model::NodeKind;
use apinata_app::tabs::TabKind;
use gpui_kit::component::WindowExt as _;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, TestAppContext};

// The open tabs' request paths in strip order, and the active one's.
fn strip(cx: &mut TestAppContext, h: &SidebarHarness) -> (Vec<String>, Option<String>) {
    h.ws.read_with(cx, |w, _| {
        let name = |k: &TabKind| match k {
            TabKind::Request(p) => p.as_str().to_string(),
            other => format!("{other:?}"),
        };
        let tabs = w.tabs();
        let order = tabs.entries().iter().map(|e| name(&e.kind)).collect();
        (order, tabs.active().map(|e| name(&e.kind)))
    })
}

// Opens `rels` as permanent tabs in order, then makes `active` the active one.
fn open_tabs(cx: &mut TestAppContext, h: &SidebarHarness, rels: &[&str], active: &str) {
    for rel in rels {
        open_tab(cx, &h.ws, &h.root, rel);
    }
    open_tab(cx, &h.ws, &h.root, active);
    frame(cx, h.win);
}

fn hide_sidebar(cx: &mut TestAppContext, h: &SidebarHarness) {
    press(cx, h, &secondary("b"));
    frame(cx, h.win);
}

// Runs the palette command `label` from wherever focus is now.
fn palette_run(cx: &mut TestAppContext, h: &SidebarHarness, label: &str) {
    cx.simulate_keystrokes(h.win, &secondary("shift-p"));
    frame(cx, h.win);
    cx.update_window(h.win, |_, w, cx| w.input(label, cx))
        .unwrap();
    frame(cx, h.win);
    cx.simulate_keystrokes(h.win, "enter");
    cx.run_until_parked();
    frame(cx, h.win);
}

// Counts the palette's rows left after filtering to `label`, then closes it.
fn palette_rows(cx: &mut TestAppContext, h: &SidebarHarness, label: &str) -> usize {
    cx.simulate_keystrokes(h.win, &secondary("shift-p"));
    frame(cx, h.win);
    cx.update_window(h.win, |_, w, cx| w.input(label, cx))
        .unwrap();
    frame(cx, h.win);
    let found = cx
        .update_window(h.win, |_, w, _| {
            gpui_kit::base::test_support::snapshots(w)
                .iter()
                .filter(|n| n.role() == Some(gpui_kit::Role::ListBoxOption))
                .count()
        })
        .unwrap();
    cx.simulate_keystrokes(h.win, "escape");
    frame(cx, h.win);
    found
}

// Lets every toast time out with the sidebar hidden; the pointer was last on the URL input,
// off the toasts, whose timers it would otherwise pause.
fn expire_toasts_hidden(cx: &mut TestAppContext, h: &SidebarHarness) {
    for _ in 0..20 {
        cx.executor()
            .advance_clock(std::time::Duration::from_secs(10));
        frame(cx, h.win);
    }
    let left = cx
        .update_window(h.win, |_, w, cx| w.notifications(cx).len())
        .unwrap();
    assert_eq!(left, 0, "every toast timed out");
}

fn tabs(names: &[&str]) -> Vec<String> {
    names.iter().map(|n| n.to_string()).collect()
}

#[gpui_kit::test]
fn undo_reopens_the_deleted_requests_tab_where_it_was_and_active(cx: &mut TestAppContext) {
    let (h, bin) = shell_with_trash(cx);
    open_tabs(cx, &h, &["c.yml", "a.yml", "b.yml"], "a.yml");
    select_and_delete(cx, &h, "a.yml");
    assert_eq!(strip(cx, &h).0, tabs(&["c.yml", "b.yml"]));
    click_undo(cx, &h);
    frame(cx, h.win);
    assert_restored(cx, &h, "a.yml", &bin);
    let (order, active) = strip(cx, &h);
    assert_eq!(order, tabs(&["c.yml", "a.yml", "b.yml"]));
    assert_eq!(active.as_deref(), Some("a.yml"));
}

#[gpui_kit::test]
fn ctrl_z_reopens_every_tab_a_folder_delete_closed(cx: &mut TestAppContext) {
    let (h, bin) = shell_with_trash(cx);
    open_tabs(cx, &h, &["f/x.yml", "a.yml"], "f/x.yml");
    select_and_delete(cx, &h, "f");
    assert_eq!(strip(cx, &h).0, tabs(&["a.yml"]));
    ctrl_z(cx, &h);
    assert_restored(cx, &h, "f/x.yml", &bin);
    let (order, active) = strip(cx, &h);
    assert_eq!(order, tabs(&["f/x.yml", "a.yml"]));
    assert_eq!(active.as_deref(), Some("f/x.yml"));
}

// A background tab comes back in place and the tab in front stays in front.
#[gpui_kit::test]
fn undo_of_a_background_tabs_delete_keeps_the_active_tab(cx: &mut TestAppContext) {
    let (h, bin) = shell_with_trash(cx);
    open_tabs(cx, &h, &["a.yml", "c.yml", "b.yml"], "b.yml");
    menu_delete(cx, &h, "c.yml", NodeKind::Request);
    assert_eq!(
        strip(cx, &h),
        (tabs(&["a.yml", "b.yml"]), Some("b.yml".into()))
    );
    click_undo(cx, &h);
    frame(cx, h.win);
    assert_restored(cx, &h, "c.yml", &bin);
    let (order, active) = strip(cx, &h);
    assert_eq!(order, tabs(&["a.yml", "c.yml", "b.yml"]));
    assert_eq!(active.as_deref(), Some("b.yml"));
}

#[gpui_kit::test]
fn with_the_sidebar_hidden_the_toasts_undo_reopens_the_tab(cx: &mut TestAppContext) {
    let (h, bin) = shell_with_trash(cx);
    open_tabs(cx, &h, &["c.yml", "a.yml"], "a.yml");
    focus_url(cx, &h);
    hide_sidebar(cx, &h);
    palette_delete(cx, &h);
    assert_eq!(strip(cx, &h).0, tabs(&["c.yml"]));
    click_undo(cx, &h);
    frame(cx, h.win);
    assert_restored(cx, &h, "a.yml", &bin);
    let (order, active) = strip(cx, &h);
    assert_eq!(order, tabs(&["c.yml", "a.yml"]));
    assert_eq!(active.as_deref(), Some("a.yml"));
}

#[gpui_kit::test]
fn with_the_sidebar_hidden_the_palettes_undo_delete_restores_after_the_toast(
    cx: &mut TestAppContext,
) {
    let (h, bin) = shell_with_trash(cx);
    open_tabs(cx, &h, &["c.yml", "a.yml"], "a.yml");
    focus_url(cx, &h);
    hide_sidebar(cx, &h);
    palette_delete(cx, &h);
    expire_toasts_hidden(cx, &h);
    palette_run(cx, &h, "Undo delete");
    frame(cx, h.win);
    assert_restored(cx, &h, "a.yml", &bin);
    let (order, active) = strip(cx, &h);
    assert_eq!(order, tabs(&["c.yml", "a.yml"]));
    assert_eq!(active.as_deref(), Some("a.yml"));
}

#[gpui_kit::test]
fn the_palette_lists_undo_delete_only_with_a_delete_to_undo(cx: &mut TestAppContext) {
    let (h, _bin) = shell_with_trash(cx);
    open_tabs(cx, &h, &["c.yml"], "c.yml");
    assert_eq!(palette_rows(cx, &h, "Undo delete"), 0, "nothing to undo");
    select_and_delete(cx, &h, "a.yml");
    assert_eq!(palette_rows(cx, &h, "Undo delete"), 1);
    click_undo(cx, &h);
    frame(cx, h.win);
    assert_eq!(
        palette_rows(cx, &h, "Undo delete"),
        0,
        "the delete was undone"
    );
}

// With the sidebar hidden and no editor or input holding focus, Ctrl+Z undoes the delete.
#[gpui_kit::test]
fn with_the_sidebar_hidden_ctrl_z_outside_an_editor_undoes_the_delete(cx: &mut TestAppContext) {
    let (h, bin) = shell_with_trash(cx);
    open_tabs(cx, &h, &["a.yml"], "a.yml");
    focus_url(cx, &h);
    hide_sidebar(cx, &h);
    palette_delete(cx, &h);
    assert!(strip(cx, &h).0.is_empty(), "the only tab closed");
    expire_toasts_hidden(cx, &h);
    ctrl_z(cx, &h);
    assert_restored(cx, &h, "a.yml", &bin);
    let (order, active) = strip(cx, &h);
    assert_eq!(order, tabs(&["a.yml"]));
    assert_eq!(active.as_deref(), Some("a.yml"));
}

#[gpui_kit::test]
fn with_the_sidebar_hidden_ctrl_z_in_a_text_input_edits_the_text(cx: &mut TestAppContext) {
    let (h, bin) = shell_with_trash(cx);
    open_tabs(cx, &h, &["c.yml", "a.yml"], "a.yml");
    focus_url(cx, &h);
    hide_sidebar(cx, &h);
    palette_delete(cx, &h);
    expire_toasts_hidden(cx, &h);
    type_at_end(cx, h.win, None, "url", "/mine");
    cx.run_until_parked();
    let url = |cx: &mut TestAppContext| view_of(cx, &h.ws, &h.root, "c.yml").url.clone();
    assert!(url(cx).ends_with("/mine"));
    ctrl_z(cx, &h);
    cx.run_until_parked();
    assert!(!url(cx).ends_with("/mine"), "the typing was undone");
    assert!(!h.root.join("a.yml").exists(), "the delete stays");
    assert_eq!(bin.len(), 1);
}
