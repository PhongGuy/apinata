use crate::support;

use apinata_app::app::AppState;
use apinata_app::settings::Orientation;
use apinata_app::tabs::TabKind;
use gpui_kit::component::WindowExt;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, Focusable, TestAppContext, VisualTestContext};
use support::*;

/// Presses secondary-`k` with the shell root focused, outside the sidebar and the tab area.
fn keys(cx: &mut TestAppContext, s: &Started, k: &str) {
    let shell = s.shell.clone();
    let mut v = VisualTestContext::from_window(s.win, cx);
    v.update(|w, cx| shell.read(cx).focus_handle(cx).focus(w, cx));
    v.simulate_keystrokes(&key(k));
    v.run_until_parked();
}

/// Presses `ks` verbatim with the shell root focused.
fn raw(cx: &mut TestAppContext, s: &Started, ks: &str) {
    let shell = s.shell.clone();
    let mut v = VisualTestContext::from_window(s.win, cx);
    v.update(|w, cx| shell.read(cx).focus_handle(cx).focus(w, cx));
    v.simulate_keystrokes(ks);
    v.run_until_parked();
}

fn sidebar(
    cx: &mut TestAppContext,
    s: &Started,
) -> gpui_kit::Entity<apinata_app::sidebar::Sidebar> {
    s.shell.read_with(cx, |v, _| v.sidebar().clone())
}

fn select_a(cx: &mut TestAppContext, s: &Started) {
    cx.update_window(s.win, |_, w, cx| w.click(row("a.yml"), cx))
        .unwrap();
    cx.run_until_parked();
}

fn active(cx: &mut TestAppContext, s: &Started) -> Option<TabKind> {
    s.ws.read_with(cx, |w, _| w.tabs().active().map(|e| e.kind.clone()))
}

#[gpui_kit::test]
fn tab_keys_work_from_the_window(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    open_tab(cx, &s.ws, &s.root, "a.yml");
    open_tab(cx, &s.ws, &s.root, "b.yml");
    keys(cx, &s, "1");
    assert_eq!(active(cx, &s), Some(TabKind::Request(ip("a.yml"))));
    keys(cx, &s, "9");
    assert_eq!(active(cx, &s), Some(TabKind::Request(ip("b.yml"))));
    keys(cx, &s, "{");
    assert_eq!(active(cx, &s), Some(TabKind::Request(ip("a.yml"))));
    keys(cx, &s, "}");
    assert_eq!(active(cx, &s), Some(TabKind::Request(ip("b.yml"))));
    raw(cx, &s, "ctrl-pageup");
    assert_eq!(active(cx, &s), Some(TabKind::Request(ip("a.yml"))));
    raw(cx, &s, "ctrl-pagedown");
    keys(cx, &s, "w");
    assert_eq!(tab_names(cx, &s.ws), ["a.yml"]);
    keys(cx, &s, "shift-t");
    assert_eq!(tab_names(cx, &s.ws), ["a.yml", "b.yml"]);
    keys(cx, &s, "shift-w");
    assert!(tab_names(cx, &s.ws).is_empty());
}

// Ctrl+Tab and Ctrl+Shift+Tab step through the tabs as in a browser, wrapping at the ends,
// from the window and from inside the URL input.
#[gpui_kit::test]
fn ctrl_tab_cycles_tabs(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    open_tab(cx, &s.ws, &s.root, "a.yml");
    open_tab(cx, &s.ws, &s.root, "b.yml");
    raw(cx, &s, "ctrl-tab");
    assert_eq!(active(cx, &s), Some(TabKind::Request(ip("a.yml"))));
    raw(cx, &s, "ctrl-shift-tab");
    assert_eq!(active(cx, &s), Some(TabKind::Request(ip("b.yml"))));
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| w.click("url", cx))
        .unwrap();
    cx.simulate_keystrokes(s.win, "ctrl-tab");
    cx.run_until_parked();
    assert_eq!(active(cx, &s), Some(TabKind::Request(ip("a.yml"))));
}

#[gpui_kit::test]
fn tab_keys_work_from_inside_the_url_input(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    open_tab(cx, &s.ws, &s.root, "a.yml");
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| w.click("url", cx))
        .unwrap();
    cx.simulate_keystrokes(s.win, &key("w"));
    cx.run_until_parked();
    assert!(tab_names(cx, &s.ws).is_empty());
}

#[gpui_kit::test]
fn save_and_save_all_work_from_the_window(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    open_tab(cx, &s.ws, &s.root, "a.yml");
    s.ws.update(cx, |w, cx| {
        w.send(&s.root, edit_url("a.yml", "http://x/7"), cx);
        w.send(&s.root, edit_url("b.yml", "http://x/8"), cx);
    });
    cx.run_until_parked();
    keys(cx, &s, "s");
    assert!(!view_of(cx, &s.ws, &s.root, "a.yml").dirty);
    assert!(
        view_of(cx, &s.ws, &s.root, "b.yml").dirty,
        "Save is the active tab only"
    );
    open_tab(cx, &s.ws, &s.root, "b.yml");
    keys(cx, &s, "shift-s");
    assert!(!view_of(cx, &s.ws, &s.root, "b.yml").dirty);
}

#[gpui_kit::test]
fn rename_works_from_the_window(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    select_a(cx, &s);
    keys(cx, &s, "r");
    assert_eq!(
        sidebar(cx, &s).read_with(cx, |v, _| v.renaming()),
        Some((s.root.clone(), ip("a.yml")))
    );
}

#[gpui_kit::test]
fn clone_works_from_the_window(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    select_a(cx, &s);
    keys(cx, &s, "d");
    assert!(s.root.join("a copy.yml").exists());
}

#[gpui_kit::test]
fn filter_and_new_request_work_from_the_window(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    keys(cx, &s, "f");
    let filter = sidebar(cx, &s).read_with(cx, |v, _| v.filter().clone());
    assert!(
        cx.update_window(s.win, |_, w, cx| filter.focus_handle(cx).is_focused(w))
            .unwrap()
    );
    keys(cx, &s, "n");
    assert!(
        !s.root.join("Untitled.yml").exists(),
        "ctrl+n writes no file"
    );
    let tabs = tab_names(cx, &s.ws);
    let [draft] = tabs.as_slice() else {
        panic!("{tabs:?}")
    };
    assert!(ip(draft).is_draft(), "{draft}");
}

#[gpui_kit::test]
fn hiding_the_sidebar_gives_the_tab_area_the_width(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    let saved = |cx: &mut TestAppContext| {
        s.ws.read_with(cx, |w, cx| w.session().read(cx).get().sidebar_width)
    };
    let before = saved(cx);
    let tab_width = |cx: &mut TestAppContext| {
        frame(cx, s.win);
        cx.update_window(s.win, |_, w, _| {
            (
                w.find("tab-area").bounds().size.width,
                w.viewport_size().width,
            )
        })
        .unwrap()
    };
    keys(cx, &s, "b");
    let (hidden, viewport) = tab_width(cx);
    assert!(
        hidden >= viewport - gpui_kit::px(4.),
        "{hidden:?} of {viewport:?}"
    );
    keys(cx, &s, "b");
    let (shown_again, _) = tab_width(cx);
    assert!(shown_again < hidden);
    assert_eq!(
        saved(cx),
        before,
        "toggling never writes a zero or changed width"
    );
}

#[gpui_kit::test]
fn window_keys_reach_their_targets(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    let tab = open_tab(cx, &s.ws, &s.root, "a.yml");
    keys(cx, &s, "b");
    assert!(!s.shell.read_with(cx, |v, _| v.sidebar_visible()));
    keys(cx, &s, "b");
    assert!(s.shell.read_with(cx, |v, _| v.sidebar_visible()));
    keys(cx, &s, "j");
    let orientation = cx.update(|cx| {
        cx.global::<AppState>()
            .settings
            .read(cx)
            .get()
            .response_orientation
    });
    assert_eq!(orientation, Orientation::Horizontal);
    let editor = s
        .shell
        .read_with(cx, |v, cx| v.tab_area().read(cx).editor(tab).cloned())
        .unwrap();
    assert_eq!(
        editor.read_with(cx, |e, cx| e.orientation(cx)),
        Orientation::Horizontal
    );
    keys(cx, &s, "e");
    assert_eq!(active(cx, &s), Some(TabKind::Environments));
    keys(cx, &s, ",");
    assert!(
        cx.update_window(s.win, |_, w, cx| w.has_active_dialog(cx))
            .unwrap()
    );
}

#[gpui_kit::test]
fn send_works_from_the_url_input_but_not_the_filter(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = mock_server(cx, wiremock::ResponseTemplate::new(200));
    let col = collection_for(&server.uri(), "");
    let s = started(cx, &col);
    open_tab(cx, &s.ws, &s.root, "a.yml");
    keys(cx, &s, "f");
    let typed = sidebar(cx, &s).read_with(cx, |v, cx| v.filter().read(cx).value().to_string());
    cx.simulate_keystrokes(s.win, &key("enter"));
    cx.run_until_parked();
    std::thread::sleep(std::time::Duration::from_millis(300));
    cx.run_until_parked();
    assert!(
        served(cx, &server).is_empty(),
        "Send does nothing in the sidebar filter"
    );
    assert_eq!(
        sidebar(cx, &s).read_with(cx, |v, cx| v.filter().read(cx).value().to_string()),
        typed
    );
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| w.click("url", cx))
        .unwrap();
    cx.simulate_keystrokes(s.win, &key("enter"));
    wait_real(cx, |cx| !served(cx, &server).is_empty());
    assert_eq!(served(cx, &server), ["/ok"]);
}

// Focuses a select's trigger, then presses Send with nothing else in between, as after picking
// a value from its list.
fn send_from_select(
    cx: &mut TestAppContext,
    s: &Started,
    focus: impl FnOnce(&mut gpui_kit::Window, &mut gpui_kit::App),
) {
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| focus(w, cx)).unwrap();
    frame(cx, s.win);
    cx.simulate_keystrokes(s.win, &key("enter"));
}

fn expanded(cx: &mut TestAppContext, s: &Started, id: &'static str) -> Option<bool> {
    cx.update_window(s.win, |_, w, cx| {
        w.render_frame(cx);
        w.find(id).expanded()
    })
    .unwrap()
}

#[gpui_kit::test]
fn send_works_from_a_closed_select_in_the_editor(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = mock_server(cx, wiremock::ResponseTemplate::new(200));
    let col = collection_for(&server.uri(), "");
    let s = started(cx, &col);
    let tab = open_tab(cx, &s.ws, &s.root, "a.yml");
    let method = editor_of(cx, &s, tab).read_with(cx, |e, _| e.method_select().clone());
    send_from_select(cx, &s, |w, cx| method.update(cx, |m, cx| m.focus(w, cx)));
    wait_real(cx, |cx| !served(cx, &server).is_empty());
    assert_eq!(served(cx, &server), ["/ok"]);
    assert_eq!(
        expanded(cx, &s, "method"),
        Some(false),
        "the list stays shut"
    );
}

#[gpui_kit::test]
fn send_works_from_the_title_bar_environment_picker(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = mock_server(cx, wiremock::ResponseTemplate::new(200));
    let col = collection_for(&server.uri(), "");
    let s = started(cx, &col);
    open_tab(cx, &s.ws, &s.root, "a.yml");
    let env = s
        .shell
        .read_with(cx, |v, cx| v.env_selector().read(cx).select().clone());
    send_from_select(cx, &s, |w, cx| env.update(cx, |m, cx| m.focus(w, cx)));
    wait_real(cx, |cx| !served(cx, &server).is_empty());
    assert_eq!(served(cx, &server), ["/ok"]);
    assert_eq!(expanded(cx, &s, "env-select"), Some(false));
}

// An open list keeps the kit's own Ctrl+Enter, which picks the highlighted option.
#[gpui_kit::test]
fn an_open_select_list_keeps_its_own_ctrl_enter(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = mock_server(cx, wiremock::ResponseTemplate::new(200));
    let col = collection_for(&server.uri(), "");
    let s = started(cx, &col);
    let tab = open_tab(cx, &s.ws, &s.root, "a.yml");
    let method = editor_of(cx, &s, tab).read_with(cx, |e, _| e.method_select().clone());
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| method.update(cx, |m, cx| m.focus(w, cx)))
        .unwrap();
    frame(cx, s.win);
    cx.simulate_keystrokes(s.win, "down");
    frame(cx, s.win);
    assert_eq!(
        expanded(cx, &s, "method"),
        Some(true),
        "down opens the list"
    );
    cx.simulate_keystrokes(s.win, &key("enter"));
    cx.run_until_parked();
    std::thread::sleep(std::time::Duration::from_millis(300));
    cx.run_until_parked();
    assert!(served(cx, &server).is_empty(), "an open list does not send");
    assert_eq!(
        expanded(cx, &s, "method"),
        Some(false),
        "the pick closes it"
    );
}

// The History button, then the tree itself, are the Tab stops after the filter, ahead of
// the rows' buttons, so the arrows and Enter reach its rows.
#[gpui_kit::test]
fn tab_from_the_sidebar_filter_passes_history_to_the_tree(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    let tree = sidebar(cx, &s).read_with(cx, |v, _| v.tree().clone());
    let tree_handle = cx
        .update_window(s.win, |_, w, cx| {
            tree.update(cx, |t, cx| t.focus(w, cx));
            w.focused(cx)
        })
        .unwrap();
    assert!(tree_handle.is_some());
    // Focus listeners run only in an active window, as a window taking keys is.
    cx.update_window(s.win, |_, w, _| w.activate_window())
        .unwrap();
    keys(cx, &s, "f");
    frame(cx, s.win);
    cx.simulate_keystrokes(s.win, "tab");
    frame(cx, s.win);
    let on_history = cx
        .update_window(s.win, |_, w, _| w.find("sidebar-history").focused())
        .unwrap();
    assert_eq!(on_history, Some(true), "tab reaches History first");
    cx.simulate_keystrokes(s.win, "tab");
    frame(cx, s.win);
    let now = cx.update_window(s.win, |_, w, cx| w.focused(cx)).unwrap();
    assert_eq!(now, tree_handle);
}

// Shift+tab leaves the tree for History, then the filter, instead of landing back on the
// tree's stand-in.
#[gpui_kit::test]
fn shift_tab_from_the_tree_passes_history_to_the_sidebar_filter(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    cx.update_window(s.win, |_, w, _| w.activate_window())
        .unwrap();
    let view = sidebar(cx, &s);
    let (tree, filter) = view.read_with(cx, |v, _| (v.tree().clone(), v.filter().clone()));
    cx.update_window(s.win, |_, w, cx| tree.update(cx, |t, cx| t.focus(w, cx)))
        .unwrap();
    frame(cx, s.win);
    cx.simulate_keystrokes(s.win, "shift-tab");
    frame(cx, s.win);
    let on_history = cx
        .update_window(s.win, |_, w, _| w.find("sidebar-history").focused())
        .unwrap();
    assert_eq!(on_history, Some(true), "shift+tab reaches History first");
    cx.simulate_keystrokes(s.win, "shift-tab");
    frame(cx, s.win);
    let (on_filter, in_tree) = cx
        .update_window(s.win, |_, w, cx| {
            (
                filter.focus_handle(cx).is_focused(w),
                w.find("sidebar-tree").focused(),
            )
        })
        .unwrap();
    assert!(on_filter, "shift+tab reaches the filter");
    assert_eq!(in_tree, Some(false));
}

// Shift+tab from the stop after the tree returns to the tree, not past it to History. The
// tree is one stop, so Tab leaves the sidebar at once.
#[gpui_kit::test]
fn shift_tab_from_the_stop_after_the_tree_lands_on_the_tree(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    cx.update_window(s.win, |_, w, _| w.activate_window())
        .unwrap();
    let tree = sidebar(cx, &s).read_with(cx, |v, _| v.tree().clone());
    let tree_handle = cx
        .update_window(s.win, |_, w, cx| {
            tree.update(cx, |t, cx| t.focus(w, cx));
            w.focused(cx)
        })
        .unwrap();
    frame(cx, s.win);
    cx.simulate_keystrokes(s.win, "tab");
    frame(cx, s.win);
    let after = cx.update_window(s.win, |_, w, cx| w.focused(cx)).unwrap();
    assert_ne!(after, tree_handle, "Tab moves on from the tree");
    let on_button = cx
        .update_window(s.win, |_, w, _| w.find(("new-request", 0usize)).focused())
        .unwrap();
    assert_ne!(on_button, Some(true), "not onto New request");
    cx.simulate_keystrokes(s.win, "shift-tab");
    frame(cx, s.win);
    let now = cx.update_window(s.win, |_, w, cx| w.focused(cx)).unwrap();
    assert_eq!(now, tree_handle, "shift+tab returns to the tree");
}

// The tree's shift+tab must not become the hint the empty state shows for FocusFilter.
#[gpui_kit::test]
fn the_filter_hint_from_the_tree_stays_the_filter_key(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    let tree = sidebar(cx, &s).read_with(cx, |v, _| v.tree().clone());
    let keys = cx
        .update_window(s.win, |_, w, cx| {
            tree.update(cx, |t, cx| t.focus(w, cx));
            let focus = w.focused(cx).unwrap();
            let action = apinata_app::sidebar::FocusFilter;
            w.highest_precedence_binding_for_action_in(&action, &focus)
                .map(|b| format!("{:?}", b.keystrokes()))
        })
        .unwrap()
        .unwrap();
    assert!(!keys.contains("tab"), "{keys}");
    assert!(keys.contains("\"f\""), "{keys}");
}

// Ctrl+enter while renaming belongs to the rename input; from the tree it still sends.
#[gpui_kit::test]
fn send_does_nothing_in_the_rename_input_but_works_from_the_tree(cx: &mut TestAppContext) {
    http_ready(cx);
    let server = mock_server(cx, wiremock::ResponseTemplate::new(200));
    let col = collection_for(&server.uri(), "");
    let s = started(cx, &col);
    open_tab(cx, &s.ws, &s.root, "a.yml");
    let (view, root) = (sidebar(cx, &s), s.root.clone());
    cx.update_window(s.win, |_, w, cx| {
        view.update(cx, |v, cx| v.start_rename(&root, &ip("a.yml"), w, cx))
    })
    .unwrap();
    frame(cx, s.win);
    cx.simulate_keystrokes(s.win, &key("enter"));
    cx.run_until_parked();
    std::thread::sleep(std::time::Duration::from_millis(300));
    cx.run_until_parked();
    assert!(
        served(cx, &server).is_empty(),
        "Send does nothing in the rename input"
    );
    cx.simulate_keystrokes(s.win, "escape");
    cx.run_until_parked();
    assert_eq!(view.read_with(cx, |v, _| v.renaming()), None);
    let tree = view.read_with(cx, |v, _| v.tree().clone());
    cx.update_window(s.win, |_, w, cx| tree.update(cx, |t, cx| t.focus(w, cx)))
        .unwrap();
    cx.simulate_keystrokes(s.win, &key("enter"));
    wait_real(cx, |cx| !served(cx, &server).is_empty());
    assert_eq!(served(cx, &server), ["/ok"]);
}

// Hiding the sidebar while its filter holds focus must leave focus inside the shell, so
// a second press, made without refocusing, shows it again.
#[gpui_kit::test]
fn hiding_a_focused_sidebar_keeps_window_keys_working(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    keys(cx, &s, "f");
    let mut v = VisualTestContext::from_window(s.win, cx);
    v.simulate_keystrokes(&key("b"));
    v.run_until_parked();
    assert!(!s.shell.read_with(&v, |sh, _| sh.sidebar_visible()));
    v.simulate_keystrokes(&key("b"));
    v.run_until_parked();
    assert!(s.shell.read_with(cx, |sh, _| sh.sidebar_visible()));
}

// With a tab open, hiding a focused sidebar hands focus to the active tab's editor.
#[gpui_kit::test]
fn hiding_a_focused_sidebar_focuses_the_active_tab(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    let tab = open_tab(cx, &s.ws, &s.root, "a.yml");
    frame(cx, s.win);
    keys(cx, &s, "f");
    let mut v = VisualTestContext::from_window(s.win, cx);
    v.simulate_keystrokes(&key("b"));
    v.run_until_parked();
    let editor = s
        .shell
        .read_with(cx, |sh, cx| sh.tab_area().read(cx).editor(tab).cloned())
        .unwrap();
    assert!(
        cx.update_window(s.win, |_, w, cx| editor
            .focus_handle(cx)
            .contains_focused(w, cx))
            .unwrap()
    );
}

// F2 renames the selected row from the tree, and does nothing from the shell elsewhere.
#[gpui_kit::test]
fn f2_in_the_tree_starts_a_rename(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    let view = sidebar(cx, &s);
    select_a(cx, &s);
    raw(cx, &s, "f2");
    assert_eq!(
        view.read_with(cx, |v, _| v.renaming()),
        None,
        "not from the shell"
    );
    let tree = view.read_with(cx, |v, _| v.tree().clone());
    cx.update_window(s.win, |_, w, cx| tree.update(cx, |t, cx| t.focus(w, cx)))
        .unwrap();
    cx.simulate_keystrokes(s.win, "f2");
    cx.run_until_parked();
    assert_eq!(
        view.read_with(cx, |v, _| v.renaming()),
        Some((s.root.clone(), ip("a.yml")))
    );
}

// A second F2 inside the rename input keeps the typed name rather than restarting the rename.
#[gpui_kit::test]
fn f2_in_the_rename_input_keeps_the_typed_name(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    let view = sidebar(cx, &s);
    select_a(cx, &s);
    let tree = view.read_with(cx, |v, _| v.tree().clone());
    cx.update_window(s.win, |_, w, cx| tree.update(cx, |t, cx| t.focus(w, cx)))
        .unwrap();
    cx.simulate_keystrokes(s.win, "f2");
    cx.run_until_parked();
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| {
        w.input("Zed", cx);
        w.press("f2", cx);
        w.press("enter", cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert!(s.root.join("Zed.yml").exists());
    assert!(!s.root.join("a.yml").exists());
}
