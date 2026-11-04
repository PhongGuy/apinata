//! The Keyboard section of Preferences: every command with its keys, resets and the filter.

use crate::support;

use apinata_app::commands::registry;
use gpui_kit::AppContext as _;
use gpui_kit::TestAppContext;
use gpui_kit::test::TestWindowExt as _;
use support::*;

fn search(cx: &mut TestAppContext, s: &Started, text: &str) {
    replace_input(cx, s.win, None, "prefs-search", text);
    frame(cx, s.win);
}

fn click(cx: &mut TestAppContext, s: &Started, id: &str) {
    prefs_click(cx, s, id);
}

fn shown(cx: &mut TestAppContext, s: &Started, id: &str) -> bool {
    present(cx, s.win, id.to_string())
}

#[gpui_kit::test]
fn every_command_has_a_row_with_its_keys(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    open_prefs(cx, &s);
    assert!(shown(cx, &s, "prefs-keyboard"));
    assert!(shown(cx, &s, "prefs-keyboard-note"));
    for c in registry() {
        assert!(shown(cx, &s, &format!("key-{}", c.id)), "{}", c.id);
    }
    assert_eq!(
        key_label(cx, &s, "view.toggle_sidebar"),
        Some(secondary("b"))
    );
    assert_eq!(
        key_label(cx, &s, "tabs.next"),
        Some(format!("{}, ctrl-pagedown", secondary("}")))
    );
    assert_eq!(
        key_label(cx, &s, "view.toggle_theme"),
        Some("Unbound".into())
    );
    assert!(
        !shown(cx, &s, "key-view.toggle_sidebar-reset"),
        "a default row has no reset"
    );
}

#[gpui_kit::test]
fn the_keyboard_section_lists_undo_delete(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    open_prefs(cx, &s);
    search(cx, &s, "undo delete");
    assert!(shown(cx, &s, "key-request.undo_delete"));
    assert!(!shown(cx, &s, "key-request.undo"));
    assert_eq!(
        key_label(cx, &s, "request.undo_delete"),
        Some(secondary("z"))
    );
    // It shares the Undo edit chord, but only inside the sidebar, so it is no conflict.
    assert!(!shown(cx, &s, "key-request.undo_delete-conflict"));
}

// A pane command names its area beside its keys; a global one names none.
#[gpui_kit::test]
fn a_pane_shortcut_names_its_area(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    open_prefs(cx, &s);
    search(cx, &s, "undo");
    assert!(shown(cx, &s, "key-request.undo"));
    assert!(!shown(cx, &s, "key-request.undo-scope"));
    let scope = cx
        .update_window(s.win, |_, w, cx| {
            w.render_frame(cx);
            w.find("key-request.undo_delete-scope")
                .label()
                .map(str::to_owned)
        })
        .unwrap();
    assert_eq!(scope.as_deref(), Some("Sidebar"));
}

#[gpui_kit::test]
fn the_search_matches_label_group_and_key_text(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    open_prefs(cx, &s);
    search(cx, &s, "toggle sidebar");
    assert!(shown(cx, &s, "key-view.toggle_sidebar"));
    assert!(!shown(cx, &s, "key-tabs.close"));
    search(cx, &s, "environment");
    assert!(shown(cx, &s, "key-environment.import"));
    assert!(!shown(cx, &s, "key-tabs.close"));
    search(cx, &s, &secondary("shift-p"));
    assert!(shown(cx, &s, "key-view.command_palette"));
    assert!(!shown(cx, &s, "key-tabs.close"));
    search(cx, &s, "no command has this");
    assert!(!shown(cx, &s, "prefs-keyboard"));
}

#[gpui_kit::test]
fn a_reset_button_restores_one_command(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    edit_keys(cx, |o| {
        o.insert("view.toggle_sidebar".into(), vec!["secondary-\\".into()]);
        o.insert("tabs.close".into(), vec![]);
    });
    open_prefs(cx, &s);
    assert_eq!(
        key_label(cx, &s, "view.toggle_sidebar"),
        Some(secondary("\\"))
    );
    assert_eq!(key_label(cx, &s, "tabs.close"), Some("Unbound".into()));
    click(cx, &s, "key-view.toggle_sidebar-reset");
    assert_eq!(keymap_overrides(cx), overrides(&[("tabs.close", &[])]));
    assert_eq!(
        key_label(cx, &s, "view.toggle_sidebar"),
        Some(secondary("b"))
    );
    assert!(!shown(cx, &s, "key-view.toggle_sidebar-reset"));
}

#[gpui_kit::test]
fn reset_all_asks_and_keeps_unknown_entries(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    edit_keys(cx, |o| {
        o.insert("view.toggle_sidebar".into(), vec!["secondary-\\".into()]);
        o.insert("old.command".into(), vec!["secondary-q".into()]);
    });
    open_prefs(cx, &s);
    click(cx, &s, "keys-reset-all");
    assert!(shown(cx, &s, "confirm-dialog"));
    click(cx, &s, "keys-reset-all-confirm");
    assert_eq!(
        keymap_overrides(cx),
        overrides(&[("old.command", &["secondary-q"])])
    );
    assert!(shown(cx, &s, "preferences"), "Preferences stays open");
    assert_eq!(
        key_label(cx, &s, "view.toggle_sidebar"),
        Some(secondary("b"))
    );
}

#[gpui_kit::test]
fn hand_edited_conflicts_and_invalid_entries_are_marked(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    edit_keys(cx, |o| {
        o.insert("view.toggle_sidebar".into(), vec!["secondary-w".into()]);
        o.insert("tabs.reopen".into(), vec!["secondary-shift-[".into()]);
    });
    open_prefs(cx, &s);
    assert!(shown(cx, &s, "key-view.toggle_sidebar-conflict"));
    assert!(shown(cx, &s, "key-tabs.close-conflict"));
    assert!(!shown(cx, &s, "key-tabs.reopen-conflict"));
    let invalid = cx
        .update_window(s.win, |_, w, cx| {
            w.render_frame(cx);
            w.find("key-tabs.reopen-invalid".to_string())
                .label()
                .map(str::to_owned)
        })
        .unwrap();
    assert_eq!(
        invalid.as_deref(),
        Some("Invalid entry in keymap.json; using the default")
    );
    assert_eq!(key_label(cx, &s, "tabs.reopen"), Some(secondary("shift-t")));
}

#[gpui_kit::test]
fn enter_on_a_focused_reset_button_restores_the_command(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    edit_keys(cx, |o| {
        o.insert("view.toggle_sidebar".into(), vec!["secondary-\\".into()]);
    });
    open_prefs(cx, &s);
    search(cx, &s, "toggle sidebar");
    enter_on(cx, s.win, "key-view.toggle_sidebar-reset-button");
    frame(cx, s.win);
    assert_eq!(
        key_label(cx, &s, "view.toggle_sidebar"),
        Some(secondary("b")),
        "Enter pressed Reset"
    );
}
