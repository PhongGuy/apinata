//! Recording a key in the Keyboard section: capture, refusals, conflicts and cancelling.

use crate::support;

use gpui_kit::AppContext as _;
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::kbd::Kbd;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{Keystroke, Modifiers, TestAppContext};
use support::*;

fn click(cx: &mut TestAppContext, s: &Started, id: &str) {
    prefs_click(cx, s, id);
}

fn record(cx: &mut TestAppContext, s: &Started, id: &str) {
    click(cx, s, &format!("key-{id}-keys"));
    assert_eq!(
        key_label(cx, s, id).as_deref(),
        Some("Press a shortcut, or Escape to cancel")
    );
}

fn press(cx: &mut TestAppContext, s: &Started, ks: &str) {
    cx.simulate_keystrokes(s.win, ks);
    frame(cx, s.win);
}

fn message(cx: &mut TestAppContext, s: &Started, id: &str) -> Option<String> {
    let id = format!("key-{id}-message");
    cx.update_window(s.win, |_, w, cx| {
        w.render_frame(cx);
        w.try_find(id).and_then(|e| e.label().map(str::to_owned))
    })
    .unwrap()
}

// The chord as the prompt names it: Ctrl+W on Linux and Windows.
fn shown_chord(k: &str) -> String {
    Kbd::format(&Keystroke::parse_with_os(&key(k), apinata_app::shortcuts::HOST_OS).unwrap())
}

fn shown(cx: &mut TestAppContext, s: &Started, id: &str) -> bool {
    present(cx, s.win, id.to_string())
}

#[gpui_kit::test]
fn a_pressed_key_is_recorded_and_works_at_once(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    open_prefs(cx, &s);
    record(cx, &s, "view.toggle_sidebar");
    press(cx, &s, &key("\\"));
    assert_eq!(
        keymap_overrides(cx),
        overrides(&[("view.toggle_sidebar", &["secondary-\\"])])
    );
    assert_eq!(
        key_label(cx, &s, "view.toggle_sidebar"),
        Some(secondary("\\"))
    );
    assert!(shown(cx, &s, "preferences"), "the dialog stays open");
    cx.update_window(s.win, |_, w, cx| w.close_dialog(cx))
        .unwrap();
    press_in_shell(cx, &s, &key("\\"));
    assert!(!s.shell.read_with(cx, |v, _| v.sidebar_visible()));
}

#[gpui_kit::test]
fn escape_cancels_and_modifier_presses_are_ignored(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    open_prefs(cx, &s);
    record(cx, &s, "view.toggle_sidebar");
    let control = Keystroke {
        modifiers: Modifiers::none(),
        key: "control".into(),
        key_char: None,
    };
    cx.update_window(s.win, |_, w, cx| w.dispatch_keystroke(control, cx))
        .unwrap();
    frame(cx, s.win);
    assert_eq!(
        key_label(cx, &s, "view.toggle_sidebar").as_deref(),
        Some("Press a shortcut, or Escape to cancel")
    );
    press(cx, &s, "escape");
    assert_eq!(
        key_label(cx, &s, "view.toggle_sidebar"),
        Some(secondary("b"))
    );
    assert!(
        shown(cx, &s, "preferences"),
        "Escape while recording does not close Preferences"
    );
    assert!(keymap_overrides(cx).is_empty());
}

#[gpui_kit::test]
fn a_bare_key_is_refused_but_a_function_key_is_taken(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    let prefs = open_prefs(cx, &s);
    record(cx, &s, "view.toggle_theme");
    press(cx, &s, "b");
    assert_eq!(
        message(cx, &s, "view.toggle_theme").as_deref(),
        Some("Hold Ctrl, Alt or another modifier with this key")
    );
    assert_eq!(
        key_label(cx, &s, "view.toggle_theme").as_deref(),
        Some("Press a shortcut, or Escape to cancel")
    );
    let typed = prefs.read_with(cx, |p, cx| p.search().read(cx).value().to_string());
    assert_eq!(typed, "", "the refused key types nothing");
    press(cx, &s, "f5");
    assert_eq!(
        keymap_overrides(cx),
        overrides(&[("view.toggle_theme", &["f5"])])
    );
}

#[gpui_kit::test]
fn a_taken_key_asks_and_never_runs_its_command(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    open_tab(cx, &s.ws, &s.root, "a.yml");
    open_prefs(cx, &s);
    record(cx, &s, "view.toggle_sidebar");
    press(cx, &s, &key("w"));
    assert_eq!(
        tab_names(cx, &s.ws),
        ["a.yml"],
        "ctrl+w was recorded, not run"
    );
    assert_eq!(
        message(cx, &s, "view.toggle_sidebar").as_deref(),
        Some(format!("{} is already used by Close tab", shown_chord("w")).as_str())
    );
    assert!(shown(cx, &s, "key-view.toggle_sidebar-replace"));
    assert!(shown(cx, &s, "key-view.toggle_sidebar-cancel"));
    assert!(keymap_overrides(cx).is_empty());
    click(cx, &s, "key-view.toggle_sidebar-swap");
    assert_eq!(
        keymap_overrides(cx),
        overrides(&[
            ("tabs.close", &["secondary-b"]),
            ("view.toggle_sidebar", &["secondary-w"])
        ])
    );
    assert!(!shown(cx, &s, "key-view.toggle_sidebar-swap"));
}

#[gpui_kit::test]
fn cancel_changes_nothing_and_replace_unbinds_the_other(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    open_prefs(cx, &s);
    record(cx, &s, "view.toggle_sidebar");
    press(cx, &s, &key("w"));
    click(cx, &s, "key-view.toggle_sidebar-cancel");
    assert!(keymap_overrides(cx).is_empty());
    assert!(!shown(cx, &s, "key-view.toggle_sidebar-replace"));
    record(cx, &s, "view.toggle_sidebar");
    press(cx, &s, &key("w"));
    click(cx, &s, "key-view.toggle_sidebar-replace");
    assert_eq!(
        keymap_overrides(cx),
        overrides(&[
            ("tabs.close", &[]),
            ("view.toggle_sidebar", &["secondary-w"])
        ])
    );
}

#[gpui_kit::test]
fn swap_is_offered_only_for_one_conflict(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    edit_keys(cx, |o| {
        o.insert("view.toggle_theme".into(), vec!["secondary-k".into()]);
        o.insert("environment.cookies".into(), vec!["secondary-k".into()]);
    });
    open_prefs(cx, &s);
    record(cx, &s, "view.toggle_sidebar");
    press(cx, &s, &key("k"));
    assert_eq!(
        message(cx, &s, "view.toggle_sidebar").as_deref(),
        Some(
            format!(
                "{} is already used by Toggle theme, Cookies",
                shown_chord("k")
            )
            .as_str()
        )
    );
    assert!(shown(cx, &s, "key-view.toggle_sidebar-replace"));
    assert!(!shown(cx, &s, "key-view.toggle_sidebar-swap"));
}

#[gpui_kit::test]
fn clicking_elsewhere_or_closing_stops_recording(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    open_prefs(cx, &s);
    record(cx, &s, "view.toggle_sidebar");
    click(cx, &s, "prefs-search");
    assert_eq!(
        key_label(cx, &s, "view.toggle_sidebar"),
        Some(secondary("b"))
    );
    record(cx, &s, "view.toggle_sidebar");
    cx.update_window(s.win, |_, w, cx| w.close_dialog(cx))
        .unwrap();
    frame(cx, s.win);
    press_in_shell(cx, &s, &key("\\"));
    assert!(
        keymap_overrides(cx).is_empty(),
        "nothing records after the dialog closes"
    );
}

#[gpui_kit::test]
fn a_key_text_fields_use_gets_a_note(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    open_prefs(cx, &s);
    record(cx, &s, "view.toggle_theme");
    press(cx, &s, &key("a"));
    assert_eq!(
        keymap_overrides(cx),
        overrides(&[("view.toggle_theme", &["secondary-a"])])
    );
    assert_eq!(
        message(cx, &s, "view.toggle_theme").as_deref(),
        Some("Text fields use this key; it will not work while one has focus")
    );
}

#[gpui_kit::test]
fn reset_after_swap_asks(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    open_prefs(cx, &s);
    record(cx, &s, "request.send");
    press(cx, &s, &key("shift-p"));
    click(cx, &s, "key-request.send-swap");
    let swapped = overrides(&[
        ("request.send", &["secondary-shift-p"]),
        ("view.command_palette", &["secondary-enter"]),
    ]);
    assert_eq!(keymap_overrides(cx), swapped);
    click(cx, &s, "key-request.send-reset");
    let enter = if cfg!(target_os = "macos") {
        shown_chord("enter")
    } else {
        "Ctrl+Enter".to_string()
    };
    assert_eq!(
        message(cx, &s, "request.send"),
        Some(format!("{enter} is already used by Command palette"))
    );
    assert!(shown(cx, &s, "key-request.send-replace"));
    assert!(shown(cx, &s, "key-request.send-swap"));
    click(cx, &s, "key-request.send-cancel");
    assert_eq!(keymap_overrides(cx), swapped, "Cancel keeps both bindings");
    assert!(!shown(cx, &s, "key-request.send-conflict"));
    assert!(!shown(cx, &s, "key-view.command_palette-conflict"));
    click(cx, &s, "key-request.send-reset");
    click(cx, &s, "key-request.send-replace");
    assert_eq!(
        keymap_overrides(cx),
        overrides(&[("view.command_palette", &[])])
    );
    assert_eq!(key_label(cx, &s, "request.send"), Some(secondary("enter")));
    assert!(!shown(cx, &s, "key-request.send-conflict"));
    assert!(!shown(cx, &s, "key-view.command_palette-conflict"));
}

#[gpui_kit::test]
fn reset_when_free_needs_no_prompt(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    open_prefs(cx, &s);
    record(cx, &s, "request.send");
    press(cx, &s, &key("k"));
    assert_eq!(
        keymap_overrides(cx),
        overrides(&[("request.send", &["secondary-k"])])
    );
    click(cx, &s, "key-request.send-reset");
    assert!(!shown(cx, &s, "key-request.send-replace"));
    assert!(keymap_overrides(cx).is_empty());
    assert_eq!(key_label(cx, &s, "request.send"), Some(secondary("enter")));
}

#[gpui_kit::test]
fn a_long_note_wraps_inside_the_dialog(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    open_prefs(cx, &s);
    record(cx, &s, "request.copy_as_curl");
    press(cx, &s, &key("a"));
    assert_eq!(
        message(cx, &s, "request.copy_as_curl").as_deref(),
        Some("Text fields use this key; it will not work while one has focus")
    );
    assert_eq!(
        key_label(cx, &s, "request.copy_as_curl"),
        Some(secondary("a"))
    );
    prefs_reveal(cx, &s, "key-request.copy_as_curl-message");
    let (dialog, note, reset) = cx
        .update_window(s.win, |_, w, cx| {
            w.render_frame(cx);
            (
                w.find("preferences".to_string()).bounds(),
                w.find("key-request.copy_as_curl-message".to_string())
                    .bounds(),
                w.find("key-request.copy_as_curl-reset".to_string())
                    .bounds(),
            )
        })
        .unwrap();
    for (name, b) in [("note", note), ("reset", reset)] {
        assert!(
            b.left() >= dialog.left()
                && b.right() <= dialog.right()
                && b.top() >= dialog.top()
                && b.bottom() <= dialog.bottom(),
            "{name} {b:?} spills out of the dialog {dialog:?}"
        );
    }
}

fn answer_by_key(cx: &mut TestAppContext, s: &Started, answer: &str, key: &str) {
    tab_to(cx, s.win, format!("{answer}-button"));
    cx.update_window(s.win, |_, w, cx| w.press(key, cx))
        .unwrap();
    frame(cx, s.win);
}

#[gpui_kit::test]
fn the_conflict_answers_take_enter_and_space(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    open_prefs(cx, &s);
    record(cx, &s, "view.toggle_sidebar");
    press(cx, &s, &key("w"));
    answer_by_key(cx, &s, "key-view.toggle_sidebar-cancel", "space");
    assert!(
        keymap_overrides(cx).is_empty(),
        "Space on Cancel changes nothing"
    );
    assert!(!shown(cx, &s, "key-view.toggle_sidebar-replace"));
    record(cx, &s, "view.toggle_sidebar");
    press(cx, &s, &key("w"));
    answer_by_key(cx, &s, "key-view.toggle_sidebar-replace", "enter");
    assert_eq!(
        keymap_overrides(cx),
        overrides(&[
            ("tabs.close", &[]),
            ("view.toggle_sidebar", &["secondary-w"])
        ])
    );
}

#[gpui_kit::test]
fn a_key_a_code_editor_takes_names_the_place(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    open_prefs(cx, &s);
    record(cx, &s, "view.toggle_sidebar");
    press(cx, &s, "ctrl-tab");
    let shown = Kbd::format(&Keystroke::parse("ctrl-tab").unwrap());
    assert_eq!(
        message(cx, &s, "view.toggle_sidebar"),
        Some(format!(
            "{shown} is already used by Focus next control in code editors"
        ))
    );
    click(cx, &s, "key-view.toggle_sidebar-cancel");
    // The other way round the window-wide command it hides is named by its label alone.
    record(cx, &s, "view.focus_next_control");
    press(cx, &s, &key("w"));
    assert_eq!(
        message(cx, &s, "view.focus_next_control"),
        Some(format!("{} is already used by Close tab", shown_chord("w")))
    );
}
