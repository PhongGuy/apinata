//! History is one step away: a Shell shortcut, a sidebar button, the app menu.

use crate::support;

use apinata_app::app_menu::app_menu_groups;
use apinata_app::commands::registry;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, TestAppContext, px};
use support::*;

const CHORD: &str = if cfg!(target_os = "macos") {
    "cmd-y"
} else {
    "ctrl-h"
};

fn history_open(cx: &mut TestAppContext, s: &Started) -> bool {
    frame(cx, s.win);
    present(cx, s.win, "history-palette")
}

#[test]
fn history_has_a_default_chord_and_ends_the_data_group() {
    let cmds = registry();
    let history = cmds
        .iter()
        .find(|c| c.id == "view.history")
        .expect("listed");
    assert_eq!(history.default_chords, [CHORD]);
    let groups = app_menu_groups();
    let (label, action) = groups[3].last().expect("an item");
    assert_eq!(*label, "History");
    assert_eq!(action.name(), "apinata::OpenHistory");
}

#[gpui_kit::test]
fn the_shortcut_opens_history_from_the_shell(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    press_in_shell(cx, &s, CHORD);
    wait_real(cx, |cx| history_open(cx, &s));
}

#[cfg(not(target_os = "macos"))]
#[gpui_kit::test]
fn ctrl_h_in_a_text_field_leaves_history_closed(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    let tab = open_tab(cx, &s.ws, &s.root, "a.yml");
    frame(cx, s.win);
    let editor = s
        .shell
        .read_with(cx, |sh, cx| sh.tab_area().read(cx).editor(tab).cloned())
        .expect("the editor");
    let url = editor.read_with(cx, |e, cx| e.url_focus(cx));
    let mut v = gpui_kit::VisualTestContext::from_window(s.win, cx);
    v.update(|w, cx| url.focus(w, cx));
    v.simulate_keystrokes("ctrl-h");
    v.run_until_parked();
    assert!(!history_open(cx, &s));
}

#[gpui_kit::test]
fn the_sidebar_button_sits_right_of_the_filter_and_opens_history(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    cx.update_window(s.win, |_, w, cx| {
        w.render_frame(cx);
        let hint = w.find("sidebar-filter-kbd").bounds();
        let button = w.find("sidebar-history").bounds();
        assert!(button.left() >= hint.right(), "{button:?} vs {hint:?}");
        assert!(
            (button.center().y - hint.center().y).abs() < px(4.),
            "{button:?} vs {hint:?}"
        );
        assert_eq!(w.find("sidebar-history").label(), Some("History"));
    })
    .unwrap();
    cx.update_window(s.win, |_, w, cx| w.click("sidebar-history", cx))
        .unwrap();
    wait_real(cx, |cx| history_open(cx, &s));
}

// The request body is a searchable editor, so ctrl-h there stays the kit's replace panel.
#[cfg(not(target_os = "macos"))]
#[gpui_kit::test]
fn ctrl_h_in_the_request_body_opens_replace_not_history(cx: &mut TestAppContext) {
    use apinata_app::request_editor::Section;
    use gpui_kit::Focusable as _;
    let a = "info:\n  name: a\n  type: http\n  seq: 1\nhttp:\n  method: POST\n  url: \"http://x/0\"\n  body:\n    type: json\n    data: \"{}\"\n";
    let col = write_tree(&[("opencollection.yml", ROOT), ("a.yml", a)]);
    let s = started(cx, &col);
    let tab = open_tab(cx, &s.ws, &s.root, "a.yml");
    frame(cx, s.win);
    let editor = s
        .shell
        .read_with(cx, |sh, cx| sh.tab_area().read(cx).editor(tab).cloned())
        .expect("the editor");
    cx.update_window(s.win, |_, w, cx| {
        w.click(("section", Section::Body as usize), cx);
        w.render_frame(cx)
    })
    .unwrap();
    let code = editor.read_with(cx, |e, cx| e.body().read(cx).code().clone());
    let mut v = gpui_kit::VisualTestContext::from_window(s.win, cx);
    v.update(|w, cx| code.read(cx).focus_handle(cx).focus(w, cx));
    v.run_until_parked();
    v.simulate_keystrokes("ctrl-h");
    v.run_until_parked();
    assert!(
        code.read_with(cx, |c, _| c.search_session().open),
        "replace opens"
    );
    assert!(!history_open(cx, &s));
}
