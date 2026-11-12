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

// A command with keys offers Unbind, which leaves it with none; its Reset brings the defaults
// back. A command already without keys offers no Unbind.
#[gpui_kit::test]
fn unbind_removes_a_default_shortcut(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    open_prefs(cx, &s);
    assert!(!shown(cx, &s, "key-view.toggle_theme-unbind"));
    let name = cx
        .update_window(s.win, |_, w, cx| {
            w.render_frame(cx);
            w.find("key-view.toggle_sidebar-unbind-button")
                .label()
                .map(str::to_owned)
        })
        .unwrap();
    assert_eq!(name.as_deref(), Some("Unbind"));
    click(cx, &s, "key-view.toggle_sidebar-unbind");
    assert_eq!(
        keymap_overrides(cx),
        overrides(&[("view.toggle_sidebar", &[])])
    );
    assert_eq!(
        key_label(cx, &s, "view.toggle_sidebar"),
        Some("Unbound".into())
    );
    assert!(!shown(cx, &s, "key-view.toggle_sidebar-unbind"));
    click(cx, &s, "key-view.toggle_sidebar-reset");
    assert_eq!(
        key_label(cx, &s, "view.toggle_sidebar"),
        Some(secondary("b"))
    );
}

const LIST: &str = "prefs-keyboard-list";

// Whether row `id` shows the focused-row highlight the arrow keys move.
fn on_row(cx: &mut TestAppContext, s: &Started, id: &str) -> bool {
    shown(cx, s, &format!("key-{id}-focus-edge"))
}

fn option(cx: &mut TestAppContext, s: &Started, id: &str) -> (Option<String>, Option<bool>) {
    let id = format!("key-{id}-option");
    cx.update_window(s.win, |_, w, cx| {
        w.render_frame(cx);
        let e = w.find(id);
        (e.label().map(str::to_owned), e.selected())
    })
    .unwrap()
}

// The whole list is one stop: the next Tab leaves the rows for Reset all, and Up, Down, Home
// and End move the highlighted row.
#[gpui_kit::test]
fn the_shortcut_list_is_one_tab_stop_with_arrow_keys(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    open_prefs(cx, &s);
    let ids: Vec<_> = registry().iter().map(|c| c.id).collect();
    let (first, second, last) = (ids[0], ids[1], ids[ids.len() - 1]);
    tab_to(cx, s.win, LIST);
    assert!(on_row(cx, &s, first), "focus lands on the first row");
    press_key(cx, s.win, "down");
    assert!(on_row(cx, &s, second));
    assert!(!on_row(cx, &s, first));
    assert_eq!(option(cx, &s, second).1, Some(true));
    press_key(cx, s.win, "up");
    assert!(on_row(cx, &s, first));
    press_key(cx, s.win, "end");
    assert!(on_row(cx, &s, last));
    press_key(cx, s.win, "home");
    assert!(on_row(cx, &s, first));
    press_key(cx, s.win, "tab");
    assert!(holds_focus(cx, s.win, "keys-reset-all"));
    assert!(
        !on_row(cx, &s, first),
        "the highlight shows only while the list has focus"
    );
}

#[gpui_kit::test]
fn enter_on_the_focused_row_records_a_shortcut(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    open_prefs(cx, &s);
    search(cx, &s, "toggle sidebar");
    tab_to(cx, s.win, LIST);
    assert!(on_row(cx, &s, "view.toggle_sidebar"));
    press_key(cx, s.win, "enter");
    assert_eq!(
        key_label(cx, &s, "view.toggle_sidebar").as_deref(),
        Some("Press a shortcut, or Escape to cancel")
    );
    cx.simulate_keystrokes(s.win, &key("\\"));
    frame(cx, s.win);
    assert_eq!(
        keymap_overrides(cx),
        overrides(&[("view.toggle_sidebar", &["secondary-\\"])])
    );
    assert!(holds_focus(cx, s.win, LIST), "the list keeps focus");
}

#[gpui_kit::test]
fn escape_cancels_a_recording_started_from_the_list(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    open_prefs(cx, &s);
    search(cx, &s, "toggle sidebar");
    tab_to(cx, s.win, LIST);
    press_key(cx, s.win, "enter");
    press_key(cx, s.win, "escape");
    assert_eq!(
        key_label(cx, &s, "view.toggle_sidebar"),
        Some(secondary("b"))
    );
    assert!(shown(cx, &s, "preferences"), "Preferences stays open");
    assert!(holds_focus(cx, s.win, LIST));
    assert!(keymap_overrides(cx).is_empty());
}

// Delete and Backspace unbind the focused row as its Unbind button does; Reset stays a stop.
#[gpui_kit::test]
fn delete_or_backspace_on_the_focused_row_unbinds_it(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    open_prefs(cx, &s);
    search(cx, &s, "toggle sidebar");
    tab_to(cx, s.win, LIST);
    press_key(cx, s.win, "delete");
    assert_eq!(
        keymap_overrides(cx),
        overrides(&[("view.toggle_sidebar", &[])])
    );
    enter_on(cx, s.win, "key-view.toggle_sidebar-reset-button");
    frame(cx, s.win);
    assert!(keymap_overrides(cx).is_empty(), "Reset is reachable");
    tab_to(cx, s.win, LIST);
    press_key(cx, s.win, "backspace");
    assert_eq!(
        keymap_overrides(cx),
        overrides(&[("view.toggle_sidebar", &[])])
    );
    press_key(cx, s.win, "delete");
    assert_eq!(
        keymap_overrides(cx),
        overrides(&[("view.toggle_sidebar", &[])]),
        "an unbound row stays as it is"
    );
}

#[gpui_kit::test]
fn the_focused_row_names_its_command_keys_and_actions(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    open_prefs(cx, &s);
    let list = cx
        .update_window(s.win, |_, w, cx| {
            w.render_frame(cx);
            w.find(LIST).label().map(str::to_owned)
        })
        .unwrap();
    assert_eq!(list.as_deref(), Some("Shortcuts"));
    let keys = gpui_kit::component::kbd::Kbd::format(
        &gpui_kit::Keystroke::parse_with_os(&key("b"), apinata_app::shortcuts::HOST_OS).unwrap(),
    );
    assert_eq!(
        option(cx, &s, "view.toggle_sidebar").0,
        Some(format!(
            "Toggle sidebar, {keys}. Press Enter to record a new shortcut or Delete to remove it."
        ))
    );
    assert_eq!(
        option(cx, &s, "view.toggle_theme").0,
        Some("Toggle theme, unbound. Press Enter to record a shortcut.".into())
    );
}

#[gpui_kit::test]
fn the_focused_list_draws_a_ring(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    open_prefs(cx, &s);
    search(cx, &s, "toggle sidebar");
    tab_to(cx, s.win, LIST);
    cx.update(|cx| apinata_app::focus_mode::set(apinata_app::focus_mode::Modality::Keyboard, cx));
    frame(cx, s.win);
    assert!(ring_drawn(cx, s.win, LIST));
}

// Moving the highlight past the visible rows scrolls the dialog so the row stays in sight,
// with the row and the ring under it clear of the bottom fade.
#[gpui_kit::test]
fn end_scrolls_the_last_row_into_view(cx: &mut TestAppContext) {
    use apinata_app::style::FOCUS_RING_ROOM;
    let col = two_requests();
    let s = started(cx, &col);
    open_prefs(cx, &s);
    let last = registry().last().unwrap().id;
    tab_to(cx, s.win, LIST);
    press_key(cx, s.win, "end");
    frame(cx, s.win);
    let (row, view, fade) = cx
        .update_window(s.win, |_, w, cx| {
            w.render_frame(cx);
            let row = w.find(format!("key-{last}-option")).bounds();
            let fade = w
                .try_find("prefs-scroll-fade")
                .map_or(gpui_kit::px(0.), |f| f.bounds().size.height);
            (row, w.find("preferences-rows").bounds(), fade)
        })
        .unwrap();
    assert!(
        row.top() >= view.top() && row.bottom() + FOCUS_RING_ROOM <= view.bottom() - fade,
        "{row:?} inside {view:?} above a {fade:?} fade"
    );
}

// Home after End brings back the top of the section: its heading and the ring's top edge.
#[gpui_kit::test]
fn home_shows_the_keyboard_heading_and_the_ring_top(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    open_prefs(cx, &s);
    tab_to(cx, s.win, LIST);
    press_key(cx, s.win, "end");
    frame(cx, s.win);
    press_key(cx, s.win, "home");
    frame(cx, s.win);
    let (heading, list, view) = cx
        .update_window(s.win, |_, w, cx| {
            w.render_frame(cx);
            (
                w.find("prefs-keyboard-heading").bounds(),
                w.find(LIST).bounds(),
                w.find("preferences-rows").bounds(),
            )
        })
        .unwrap();
    assert!(
        heading.top() >= view.top() && list.top() >= view.top(),
        "heading {heading:?} and list {list:?} inside {view:?}"
    );
}

// The focused list's ring stops short of the scrollbar's track at the right edge.
#[gpui_kit::test]
fn the_list_ring_clears_the_scrollbar(cx: &mut TestAppContext) {
    use apinata_app::style::SCROLLBAR_GUTTER;
    let col = two_requests();
    let s = started(cx, &col);
    open_prefs(cx, &s);
    search(cx, &s, "toggle sidebar");
    tab_to(cx, s.win, LIST);
    cx.update(|cx| apinata_app::focus_mode::set(apinata_app::focus_mode::Modality::Keyboard, cx));
    frame(cx, s.win);
    assert!(ring_drawn(cx, s.win, LIST));
    // The right edge of the ring as painted: the ring-coloured quad around the list.
    let (ring, view) = cx
        .update_window(s.win, |_, w, cx| {
            use gpui_kit::component::ActiveTheme as _;
            let colour = cx.theme().ring.to_rgb();
            let scale = w.scale_factor();
            let list = w.find(LIST).bounds();
            let right = w
                .painted_quads()
                .into_iter()
                .filter(|q| {
                    let c = q.border_color.to_rgb();
                    q.border_widths.left.0 > 0.
                        && q.border_color.a > 0.
                        && (c.r - colour.r).abs() < 0.01
                        && (c.g - colour.g).abs() < 0.01
                        && (c.b - colour.b).abs() < 0.01
                })
                .map(|q| (q.bounds.origin.x.0 + q.bounds.size.width.0) / scale)
                .filter(|r| (r - list.right().as_f32()).abs() < 8.)
                .fold(f32::MIN, f32::max);
            (gpui_kit::px(right), w.find("preferences-rows").bounds())
        })
        .unwrap();
    assert!(
        ring <= view.right() - SCROLLBAR_GUTTER,
        "the ring's right edge {ring:?} reaches the scrollbar of {view:?}"
    );
}

// A row whose keys are its defaults again offers no Reset, even when keymap.json still
// names it: here a command unbound by default, given a key and then unbound with Delete.
#[gpui_kit::test]
fn a_row_back_at_its_default_offers_no_reset(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    open_prefs(cx, &s);
    search(cx, &s, "toggle theme");
    tab_to(cx, s.win, LIST);
    press_key(cx, s.win, "enter");
    press_key(cx, s.win, "f5");
    assert!(shown(cx, &s, "key-view.toggle_theme-reset"));
    press_key(cx, s.win, "delete");
    assert_eq!(
        key_label(cx, &s, "view.toggle_theme").as_deref(),
        Some("Unbound")
    );
    assert!(!shown(cx, &s, "key-view.toggle_theme-reset"));
}

// The list's content lines up with every other section: its highlight and ring take the
// gutter outside the rows, not room from them.
#[gpui_kit::test]
fn keyboard_rows_line_up_with_other_sections(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    open_prefs(cx, &s);
    let (key_row, other) = cx
        .update_window(s.win, |_, w, cx| {
            w.render_frame(cx);
            (
                w.find("key-view.toggle_sidebar").bounds(),
                w.find("pref-autosave").bounds(),
            )
        })
        .unwrap();
    assert_eq!(key_row.left(), other.left());
    assert_eq!(key_row.right(), other.right());
}

// Once recording, Delete and Enter are keys offered to the recorder: neither unbinds the
// command nor starts a new recording.
#[gpui_kit::test]
fn delete_and_enter_while_recording_go_to_the_recorder(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    open_prefs(cx, &s);
    search(cx, &s, "toggle sidebar");
    tab_to(cx, s.win, LIST);
    press_key(cx, s.win, "enter");
    let refused = Some("Hold Ctrl, Alt or another modifier with this key");
    let message = |cx: &mut TestAppContext| {
        cx.update_window(s.win, |_, w, cx| {
            w.render_frame(cx);
            w.try_find("key-view.toggle_sidebar-message")
                .and_then(|e| e.label().map(str::to_owned))
        })
        .unwrap()
    };
    for k in ["delete", "backspace", "enter"] {
        press_key(cx, s.win, k);
        assert!(keymap_overrides(cx).is_empty(), "{k} unbinds nothing");
        assert_eq!(
            key_label(cx, &s, "view.toggle_sidebar").as_deref(),
            Some("Press a shortcut, or Escape to cancel"),
            "{k} leaves the recorder waiting"
        );
        assert_eq!(message(cx).as_deref(), refused, "the recorder answers {k}");
    }
    press_key(cx, s.win, "f5");
    assert_eq!(
        keymap_overrides(cx),
        overrides(&[("view.toggle_sidebar", &["f5"])])
    );
}
