//! The item Copy, Cut and Paste chords act on the sidebar selection only while the sidebar
//! has focus; the palette commands still reach it from anywhere.

use crate::support;

use apinata_app::sidebar::paste::ClipMode;
use apinata_app::sidebar::{CopySelected, CutSelected};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AppContext as _, ClipboardItem, FocusHandle, Focusable as _, TestAppContext, VisualTestContext,
};
use support::*;

/// A shell with `a.yml` selected in the sidebar and a sentinel on the system clipboard.
fn selected(cx: &mut TestAppContext) -> Started {
    let col = two_requests();
    let s = started(cx, &col);
    cx.update_window(s.win, |_, w, cx| w.click(row("a.yml"), cx))
        .unwrap();
    frame(cx, s.win);
    cx.write_to_clipboard(ClipboardItem::new_string("sentinel".into()));
    s
}

fn item_clip(cx: &mut TestAppContext, s: &Started) -> (usize, Option<ClipMode>) {
    let sb = s.shell.read_with(cx, |sh, _| sh.sidebar().clone());
    sb.read_with(cx, |sb, _| (sb.clipboard_len(), sb.clipboard_mode()))
}

fn system(cx: &mut TestAppContext) -> Option<String> {
    cx.read_from_clipboard().and_then(|c| c.text())
}

/// Focuses `handle`, then presses the chord for `key` with the secondary modifier.
fn press_with(
    cx: &mut TestAppContext,
    s: &Started,
    handle: impl FnOnce(&Started, &mut TestAppContext) -> FocusHandle,
    key: &str,
) {
    let h = handle(s, cx);
    let mut v = VisualTestContext::from_window(s.win, cx);
    v.update(|w, cx| h.focus(w, cx));
    v.simulate_keystrokes(&secondary(key));
    v.run_until_parked();
}

fn dock_handle(s: &Started, cx: &mut TestAppContext) -> FocusHandle {
    press_in_shell(cx, s, "ctrl-`");
    frame(cx, s.win);
    let dock = s.shell.read_with(cx, |sh, _| sh.dock().clone());
    cx.update(|cx| dock.read(cx).focus_handle(cx))
}

fn tab_area_handle(s: &Started, cx: &mut TestAppContext) -> FocusHandle {
    let ta = s.shell.read_with(cx, |sh, _| sh.tab_area().clone());
    cx.update(|cx| ta.read(cx).focus_handle(cx))
}

fn shell_handle(s: &Started, cx: &mut TestAppContext) -> FocusHandle {
    s.shell.read_with(cx, |sh, cx| sh.focus_handle(cx))
}

fn open_tab_content(s: &Started, cx: &mut TestAppContext) -> FocusHandle {
    open_tab(cx, &s.ws, &s.root, "a.yml");
    frame(cx, s.win);
    let ta = s.shell.read_with(cx, |sh, _| sh.tab_area().clone());
    cx.update(|cx| ta.read(cx).active_focus(cx))
        .expect("the open tab has a focus handle")
}

fn assert_untouched(cx: &mut TestAppContext, s: &Started, key: &str) {
    assert_eq!(item_clip(cx, s), (0, None), "{key}: item clipboard");
    assert_eq!(system(cx).as_deref(), Some("sentinel"), "{key}: system");
}

#[gpui_kit::test]
fn chords_in_the_console_dock_leave_the_clipboards_alone(cx: &mut TestAppContext) {
    let s = selected(cx);
    for key in ["c", "x"] {
        press_with(cx, &s, dock_handle, key);
        assert_untouched(cx, &s, key);
    }
}

#[gpui_kit::test]
fn chords_in_the_open_tab_leave_the_clipboards_alone(cx: &mut TestAppContext) {
    let s = selected(cx);
    for key in ["c", "x"] {
        press_with(cx, &s, open_tab_content, key);
        assert_untouched(cx, &s, key);
    }
}

#[gpui_kit::test]
fn chords_in_the_tab_strip_leave_the_clipboards_alone(cx: &mut TestAppContext) {
    let s = selected(cx);
    for key in ["c", "x"] {
        press_with(cx, &s, tab_area_handle, key);
        assert_untouched(cx, &s, key);
    }
}

#[gpui_kit::test]
fn chords_in_an_empty_editor_area_leave_the_clipboards_alone(cx: &mut TestAppContext) {
    let s = selected(cx);
    for key in ["c", "x"] {
        press_with(cx, &s, shell_handle, key);
        assert_untouched(cx, &s, key);
    }
}

#[gpui_kit::test]
fn the_palette_actions_still_reach_the_sidebar_from_elsewhere(cx: &mut TestAppContext) {
    let s = selected(cx);
    let h = shell_handle(&s, cx);
    let mut v = VisualTestContext::from_window(s.win, cx);
    v.update(|w, cx| h.focus(w, cx));
    v.dispatch_action(CopySelected);
    v.run_until_parked();
    assert_eq!(item_clip(cx, &s), (1, Some(ClipMode::Copy)));
    let mut v = VisualTestContext::from_window(s.win, cx);
    v.dispatch_action(CutSelected);
    v.run_until_parked();
    assert_eq!(item_clip(cx, &s), (1, Some(ClipMode::Cut)));
}

#[gpui_kit::test]
fn chords_with_the_tree_focused_still_copy_in_the_shell(cx: &mut TestAppContext) {
    let s = selected(cx);
    let sb = s.shell.read_with(cx, |sh, _| sh.sidebar().clone());
    let tree = sb.read_with(cx, |sb, _| sb.tree().clone());
    cx.update_window(s.win, |_, w, cx| tree.update(cx, |t, cx| t.focus(w, cx)))
        .unwrap();
    let mut v = VisualTestContext::from_window(s.win, cx);
    v.simulate_keystrokes(&secondary("c"));
    v.run_until_parked();
    assert_eq!(item_clip(cx, &s), (1, Some(ClipMode::Copy)));
}
