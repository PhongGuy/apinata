use crate::support;

use gpui_kit::component::WindowExt as _;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, Focusable, TestAppContext, VisualTestContext};
use support::*;

fn three_requests() -> tempfile::TempDir {
    write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", "http://x/a")),
        ("b.yml", &request("b", "http://x/b")),
        ("c.yml", &request("c", "http://x/c")),
    ])
}

// Opens the three requests as tabs and puts focus in the active one, as a click into the
// page would.
fn three_tabs(cx: &mut TestAppContext, col: &tempfile::TempDir) -> Started {
    let s = started(cx, col);
    for rel in ["a.yml", "b.yml", "c.yml"] {
        open_tab(cx, &s.ws, &s.root, rel);
    }
    frame(cx, s.win);
    let focus = active_focus(cx, &s).expect("an active tab");
    let mut v = VisualTestContext::from_window(s.win, cx);
    v.update(|w, cx| focus.focus(w, cx));
    frame(cx, s.win);
    s
}

fn active_focus(cx: &mut TestAppContext, s: &Started) -> Option<gpui_kit::FocusHandle> {
    let area = s.shell.read_with(cx, |v, _| v.tab_area().clone());
    area.read_with(cx, |a, cx| a.active_focus(cx))
}

fn focused(cx: &mut TestAppContext, s: &Started, handle: &gpui_kit::FocusHandle) -> bool {
    cx.update_window(s.win, |_, w, _| handle.is_focused(w))
        .unwrap()
}

// After a close, the new active tab holds focus, or with no tab left the shell does, so
// the window's keys keep working without a click.
fn assert_focus_follows(cx: &mut TestAppContext, s: &Started) {
    match active_focus(cx, s) {
        Some(handle) => assert!(focused(cx, s, &handle), "the new active tab is focused"),
        None => {
            let shell = s.shell.clone();
            let inside = cx
                .update_window(s.win, |_, w, cx| {
                    shell.read(cx).focus_handle(cx).contains_focused(w, cx)
                })
                .unwrap();
            assert!(inside, "focus stays inside the shell");
        }
    }
}

#[gpui_kit::test]
fn repeated_close_keys_close_every_tab(cx: &mut TestAppContext) {
    let col = three_requests();
    let s = three_tabs(cx, &col);
    for left in [2, 1, 0] {
        cx.simulate_keystrokes(s.win, &key("w"));
        frame(cx, s.win);
        assert_eq!(tab_names(cx, &s.ws).len(), left);
        assert_focus_follows(cx, &s);
    }
}

#[gpui_kit::test]
fn the_close_button_and_a_save_prompt_answer_focus_the_next_tab(cx: &mut TestAppContext) {
    let col = three_requests();
    let s = three_tabs(cx, &col);
    cx.update_window(s.win, |_, w, cx| w.click(("tab-close", 2usize), cx))
        .unwrap();
    frame(cx, s.win);
    assert_eq!(tab_names(cx, &s.ws), ["a.yml", "b.yml"]);
    assert_focus_follows(cx, &s);
    s.ws.update(cx, |w, cx| {
        w.send(&s.root, edit_url("b.yml", "http://x/2"), cx)
    });
    cx.run_until_parked();
    cx.simulate_keystrokes(s.win, &key("w"));
    cx.run_until_parked();
    answer_confirm(cx, "confirm-discard");
    frame(cx, s.win);
    assert_eq!(tab_names(cx, &s.ws), ["a.yml"]);
    assert_focus_follows(cx, &s);
    cx.simulate_keystrokes(s.win, &key("w"));
    frame(cx, s.win);
    assert!(tab_names(cx, &s.ws).is_empty());
    assert_focus_follows(cx, &s);
}

// A middle-button press and release over tab `ix`, as the platform reports them.
fn middle_click(cx: &mut TestAppContext, s: &Started, ix: usize) {
    use gpui_kit::{InputEvent as _, MouseButton, MouseDownEvent, MouseUpEvent};
    frame(cx, s.win);
    let at = cx
        .update_window(s.win, |_, w, _| w.find(("tab", ix)).bounds().center())
        .unwrap();
    let down = MouseDownEvent {
        button: MouseButton::Middle,
        position: at,
        modifiers: Default::default(),
        click_count: 1,
        first_mouse: false,
    };
    let up = MouseUpEvent {
        button: MouseButton::Middle,
        position: at,
        modifiers: Default::default(),
        click_count: 1,
    };
    for event in [down.to_platform_input(), up.to_platform_input()] {
        cx.update_window(s.win, |_, w, cx| {
            w.dispatch_event(event, cx);
        })
        .unwrap();
        frame(cx, s.win);
    }
}

#[gpui_kit::test]
fn a_middle_click_closes_a_tab_and_asks_first_when_dirty(cx: &mut TestAppContext) {
    let col = three_requests();
    let s = three_tabs(cx, &col);
    middle_click(cx, &s, 0);
    assert_eq!(tab_names(cx, &s.ws), ["b.yml", "c.yml"]);
    assert_focus_follows(cx, &s);
    s.ws.update(cx, |w, cx| {
        w.send(&s.root, edit_url("c.yml", "http://x/2"), cx)
    });
    cx.run_until_parked();
    middle_click(cx, &s, 1);
    assert!(confirm_open(cx));
    answer_confirm(cx, "confirm-discard");
    frame(cx, s.win);
    assert_eq!(tab_names(cx, &s.ws), ["b.yml"]);
    assert_focus_follows(cx, &s);
    middle_click(cx, &s, 0);
    assert!(tab_names(cx, &s.ws).is_empty());
    assert_focus_follows(cx, &s);
}

fn tree_handle(cx: &mut TestAppContext, s: &Started) -> gpui_kit::FocusHandle {
    let sidebar = s.shell.read_with(cx, |v, _| v.sidebar().clone());
    let tree = sidebar.read_with(cx, |v, _| v.tree().clone());
    cx.update_window(s.win, |_, w, cx| {
        tree.update(cx, |t, cx| t.focus(w, cx));
        w.focused(cx)
    })
    .unwrap()
    .expect("the tree takes focus")
}

// A left press and release over the element `id`, as the platform reports them: unlike the
// test click, the press gives the control the focus a real click does.
fn real_click(cx: &mut TestAppContext, s: &Started, id: (&'static str, usize)) {
    use gpui_kit::{InputEvent as _, MouseButton, MouseDownEvent, MouseUpEvent};
    frame(cx, s.win);
    let at = cx
        .update_window(s.win, |_, w, _| w.find(id).bounds().center())
        .unwrap();
    let down = MouseDownEvent {
        button: MouseButton::Left,
        position: at,
        modifiers: Default::default(),
        click_count: 1,
        first_mouse: false,
    };
    let up = MouseUpEvent {
        button: MouseButton::Left,
        position: at,
        modifiers: Default::default(),
        click_count: 1,
    };
    for event in [down.to_platform_input(), up.to_platform_input()] {
        cx.update_window(s.win, |_, w, cx| {
            w.dispatch_event(event, cx);
        })
        .unwrap();
        frame(cx, s.win);
    }
}

// Tabs through the window until the close button of tab `ix` holds focus, as a keyboard user
// or a click on the button leaves it.
fn focus_close_button(cx: &mut TestAppContext, s: &Started, ix: usize) {
    let reached = (0..80).any(|_| {
        cx.simulate_keystrokes(s.win, "tab");
        frame(cx, s.win);
        cx.update_window(s.win, |_, w, _| {
            w.find(("tab-close", ix)).focused() == Some(true)
        })
        .unwrap()
    });
    assert!(reached, "tab reaches the close button of tab {ix}");
}

fn opened_with_two_tabs(cx: &mut TestAppContext, col: &tempfile::TempDir) -> Started {
    let s = started(cx, col);
    cx.update_window(s.win, |_, w, _| w.activate_window())
        .unwrap();
    for rel in ["a.yml", "b.yml"] {
        open_tab(cx, &s.ws, &s.root, rel);
    }
    frame(cx, s.win);
    s
}

#[gpui_kit::test]
fn closing_from_the_close_button_focuses_the_new_active_tab(cx: &mut TestAppContext) {
    let col = three_requests();
    let s = opened_with_two_tabs(cx, &col);
    focus_close_button(cx, &s, 1);
    real_click(cx, &s, ("tab-close", 1));
    assert_eq!(tab_names(cx, &s.ws), ["a.yml"]);
    let handle = active_focus(cx, &s).expect("an active tab");
    assert!(focused(cx, &s, &handle), "focus sits in the new active tab");
}

#[gpui_kit::test]
fn closing_the_last_tab_from_the_close_button_focuses_the_tree(cx: &mut TestAppContext) {
    let col = three_requests();
    let s = started(cx, &col);
    cx.update_window(s.win, |_, w, _| w.activate_window())
        .unwrap();
    open_tab(cx, &s.ws, &s.root, "a.yml");
    frame(cx, s.win);
    let tree = tree_handle(cx, &s);
    focus_close_button(cx, &s, 0);
    real_click(cx, &s, ("tab-close", 0));
    assert!(tab_names(cx, &s.ws).is_empty());
    assert!(focused(cx, &s, &tree), "focus moves to the tree");
}

#[gpui_kit::test]
fn closing_the_last_tab_with_the_key_focuses_the_tree(cx: &mut TestAppContext) {
    let col = three_requests();
    let s = started(cx, &col);
    cx.update_window(s.win, |_, w, _| w.activate_window())
        .unwrap();
    open_tab(cx, &s.ws, &s.root, "a.yml");
    frame(cx, s.win);
    let tree = tree_handle(cx, &s);
    let focus = active_focus(cx, &s).expect("an active tab");
    cx.update_window(s.win, |_, w, cx| focus.focus(w, cx))
        .unwrap();
    cx.simulate_keystrokes(s.win, &key("w"));
    frame(cx, s.win);
    assert!(tab_names(cx, &s.ws).is_empty());
    assert!(focused(cx, &s, &tree), "focus moves to the tree");
}

fn open_runner_tab(cx: &mut TestAppContext, s: &Started) {
    use apinata_core::ItemPath;
    s.ws.update(cx, |w, cx| w.open_runner(&s.root, ItemPath::root(), cx))
        .expect("a runner tab");
    frame(cx, s.win);
}

fn tab_count(cx: &mut TestAppContext, s: &Started) -> usize {
    s.ws.read_with(cx, |w, _| w.tabs().entries().len())
}

#[gpui_kit::test]
fn closing_a_lone_runner_tab_from_its_close_button_focuses_the_tree(cx: &mut TestAppContext) {
    let col = three_requests();
    let s = started(cx, &col);
    cx.update_window(s.win, |_, w, _| w.activate_window())
        .unwrap();
    open_runner_tab(cx, &s);
    let tree = tree_handle(cx, &s);
    focus_close_button(cx, &s, 0);
    real_click(cx, &s, ("tab-close", 0));
    assert_eq!(tab_count(cx, &s), 0);
    assert!(focused(cx, &s, &tree), "focus moves to the tree");
}

// A runner tab takes focus on its own Tab stop, not on the tab area's handle.
#[gpui_kit::test]
fn closing_a_tab_in_front_of_a_runner_tab_focuses_the_runner(cx: &mut TestAppContext) {
    let col = three_requests();
    let s = started(cx, &col);
    cx.update_window(s.win, |_, w, _| w.activate_window())
        .unwrap();
    open_runner_tab(cx, &s);
    open_tab(cx, &s.ws, &s.root, "a.yml");
    frame(cx, s.win);
    focus_close_button(cx, &s, 1);
    real_click(cx, &s, ("tab-close", 1));
    assert_eq!(tab_count(cx, &s), 1);
    let runner = active_focus(cx, &s).expect("the runner tab");
    assert!(focused(cx, &s, &runner), "focus moves to the runner");
}

// Drops focus from every element, as a click on a spot that takes none does.
fn blur_all(cx: &mut TestAppContext, s: &Started) {
    cx.update_window(s.win, |_, w, cx| w.blur(cx)).unwrap();
}

#[gpui_kit::test]
fn closing_a_lone_runner_tab_with_nothing_focused_focuses_the_tree(cx: &mut TestAppContext) {
    let col = three_requests();
    let s = started(cx, &col);
    cx.update_window(s.win, |_, w, _| w.activate_window())
        .unwrap();
    open_runner_tab(cx, &s);
    let tree = tree_handle(cx, &s);
    blur_all(cx, &s);
    let none = cx.update_window(s.win, |_, w, cx| w.focused(cx).is_none());
    assert!(none.unwrap(), "nothing holds focus");
    cx.update_window(s.win, |_, w, cx| w.click(("tab-close", 0usize), cx))
        .unwrap();
    frame(cx, s.win);
    assert_eq!(tab_count(cx, &s), 0);
    assert!(focused(cx, &s, &tree), "focus moves to the tree");
}

// With a request tab left behind the runner, the close hands focus to its editor.
#[gpui_kit::test]
fn closing_a_tab_with_nothing_focused_focuses_the_remaining_editor(cx: &mut TestAppContext) {
    let col = three_requests();
    let s = started(cx, &col);
    cx.update_window(s.win, |_, w, _| w.activate_window())
        .unwrap();
    open_tab(cx, &s.ws, &s.root, "a.yml");
    open_runner_tab(cx, &s);
    let tree = tree_handle(cx, &s);
    cx.update_window(s.win, |_, w, cx| tree.focus(w, cx))
        .unwrap();
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| w.blur(cx)).unwrap();
    cx.update_window(s.win, |_, w, cx| w.click(("tab-close", 1usize), cx))
        .unwrap();
    frame(cx, s.win);
    assert_eq!(tab_names(cx, &s.ws), ["a.yml"]);
    let handle = active_focus(cx, &s).expect("an active tab");
    assert!(focused(cx, &s, &handle), "the request editor holds focus");
}

const WITH_EXAMPLE: &str = "info:\n  name: Users\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: http://x/users\nexamples:\n  - name: ok\n    request:\n      url: http://x/users\n      method: GET\n    response:\n      status: 200\n      statusText: OK\n";

// Steps through a request, a runner and an example tab with the next-tab key. Each switch
// leaves the new tab's focus target focused, and one Tab press stays inside the tab.
#[gpui_kit::test]
fn keyboard_activation_of_each_tab_kind_focuses_a_tab_stop(cx: &mut TestAppContext) {
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("users.yml", WITH_EXAMPLE),
        ("b.yml", &request("b", "http://x/b")),
    ]);
    let s = started(cx, &col);
    cx.update_window(s.win, |_, w, _| w.activate_window())
        .unwrap();
    open_tab(cx, &s.ws, &s.root, "b.yml");
    open_runner_tab(cx, &s);
    s.ws.update(cx, |w, cx| {
        w.open_example(&s.root, ip("users.yml"), 0, false, cx)
    })
    .expect("an example tab");
    frame(cx, s.win);
    let start = active_focus(cx, &s).expect("an active tab");
    cx.update_window(s.win, |_, w, cx| start.focus(w, cx))
        .unwrap();
    frame(cx, s.win);
    for _ in 0..3 {
        cx.simulate_keystrokes(s.win, "ctrl-pagedown");
        frame(cx, s.win);
        let handle = active_focus(cx, &s).expect("an active tab");
        assert!(
            focused(cx, &s, &handle),
            "the new tab's focus target is focused"
        );
        // A request tab's target is its URL input, so Tab is checked against the whole editor.
        let area = s.shell.read_with(cx, |v, _| v.tab_area().clone());
        let id = s.ws.read_with(cx, |w, _| w.tabs().active().unwrap().id);
        let tab = area
            .read_with(cx, |a, cx| a.editor(id).map(|e| e.focus_handle(cx)))
            .unwrap_or_else(|| handle.clone());
        cx.simulate_keystrokes(s.win, "tab");
        frame(cx, s.win);
        let inside = cx
            .update_window(s.win, |_, w, cx| tab.contains_focused(w, cx))
            .unwrap();
        assert!(inside, "Tab moves within the tab");
    }
}

// The tab's right-click menu, opened with focus in the active editor.
fn open_tab_menu(cx: &mut TestAppContext, s: &Started) {
    cx.update_window(s.win, |_, w, cx| w.right_click(("tab", 1usize), cx))
        .unwrap();
    cx.run_until_parked();
    frame(cx, s.win);
}

fn palette_opens(cx: &mut TestAppContext, s: &Started) -> bool {
    cx.simulate_keystrokes(s.win, &key("shift-p"));
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| w.has_active_dialog(cx))
        .unwrap()
}

// Regression guard: the kit already restores a live focus on Escape; the keys must keep working.
#[gpui_kit::test]
fn dismissing_the_tab_menu_by_escape_restores_focus(cx: &mut TestAppContext) {
    for blur in [false, true] {
        let col = three_requests();
        let s = three_tabs(cx, &col);
        cx.update_window(s.win, |_, w, _| w.activate_window())
            .unwrap();
        let before = active_focus(cx, &s).expect("an active tab");
        if blur {
            blur_all(cx, &s);
        }
        open_tab_menu(cx, &s);
        cx.simulate_keystrokes(s.win, "escape");
        cx.run_until_parked();
        frame(cx, s.win);
        assert!(focused(cx, &s, &before), "focus is back (blurred: {blur})");
        assert!(
            palette_opens(cx, &s),
            "the palette key works (blurred: {blur})"
        );
    }
}

// Regression guard: a click outside leaves focus on what was clicked, never on nothing.
#[gpui_kit::test]
fn dismissing_the_tab_menu_by_a_click_outside_keeps_the_keys_working(cx: &mut TestAppContext) {
    let col = three_requests();
    let s = three_tabs(cx, &col);
    open_tab_menu(cx, &s);
    click_at(cx, &s, gpui_kit::point(gpui_kit::px(2.), gpui_kit::px(2.)));
    cx.run_until_parked();
    frame(cx, s.win);
    let some = cx
        .update_window(s.win, |_, w, cx| w.focused(cx).is_some())
        .unwrap();
    assert!(some, "something holds focus");
    assert!(palette_opens(cx, &s), "the palette key works");
}

// The element focused when the menu opened can be gone by the time it closes; focus then falls
// back to the active tab, so the app's keys still work.
#[gpui_kit::test]
fn dismissing_the_tab_menu_after_its_focus_vanished_falls_back(cx: &mut TestAppContext) {
    let col = three_requests();
    let s = three_tabs(cx, &col);
    open_tab_menu(cx, &s);
    // The focused editor belongs to the last tab, which closes while the menu is open.
    let last = s.ws.read_with(cx, |w, _| w.tabs().entries()[2].id);
    s.ws.update(cx, |w, cx| {
        w.update_tabs(cx, |t| {
            t.close(last);
        })
    });
    frame(cx, s.win);
    cx.simulate_keystrokes(s.win, "escape");
    cx.run_until_parked();
    frame(cx, s.win);
    let handle = active_focus(cx, &s).expect("an active tab");
    assert!(focused(cx, &s, &handle), "the active tab holds focus");
    assert!(palette_opens(cx, &s), "the palette key works");
}

// Regression guard: focus on an undrawn handle still reaches the app-wide keys.
#[gpui_kit::test]
fn the_palette_key_works_with_focus_on_nothing_drawn(cx: &mut TestAppContext) {
    let col = three_requests();
    let s = started(cx, &col);
    cx.update_window(s.win, |_, w, _| w.activate_window())
        .unwrap();
    open_tab(cx, &s.ws, &s.root, "a.yml");
    frame(cx, s.win);
    // Focus on a handle no element draws, as a menu leaves it after restoring a vanished one.
    let dead = cx.update(|cx| cx.focus_handle());
    cx.update_window(s.win, |_, w, cx| dead.focus(w, cx))
        .unwrap();
    frame(cx, s.win);
    cx.simulate_keystrokes(s.win, &key("shift-p"));
    frame(cx, s.win);
    let open = cx
        .update_window(s.win, |_, w, cx| w.has_active_dialog(cx))
        .unwrap();
    assert!(open, "the palette opened");
}

// A left press and release at `at`, as the platform reports them.
fn click_at(cx: &mut TestAppContext, s: &Started, at: gpui_kit::Point<gpui_kit::Pixels>) {
    use gpui_kit::{InputEvent as _, MouseButton, MouseDownEvent, MouseUpEvent};
    let down = MouseDownEvent {
        button: MouseButton::Left,
        position: at,
        modifiers: Default::default(),
        click_count: 1,
        first_mouse: false,
    };
    let up = MouseUpEvent {
        button: MouseButton::Left,
        position: at,
        modifiers: Default::default(),
        click_count: 1,
    };
    for event in [down.to_platform_input(), up.to_platform_input()] {
        cx.update_window(s.win, |_, w, cx| {
            w.dispatch_event(event, cx);
        })
        .unwrap();
        frame(cx, s.win);
    }
}

fn two_folders() -> tempfile::TempDir {
    write_tree(&[
        ("opencollection.yml", ROOT),
        ("alpha/alpha-1.yml", &request("alpha-1", "http://x/a")),
        ("beta/beta-1.yml", &request("beta-1", "http://x/b")),
    ])
}

// The mouse path a user takes: the folder's run button opens a Run tab, whose close button
// then closes it. Any focus left over is whatever those clicks left.
fn run_folder_then_close(cx: &mut TestAppContext, s: &Started, folder: &str) {
    cx.update_window(s.win, |_, w, cx| {
        w.render_frame(cx);
        w.click(
            apinata_app::sidebar::marker_id("run-folder", &row(folder)),
            cx,
        )
    })
    .unwrap();
    cx.run_until_parked();
    frame(cx, s.win);
    let last = tab_count(cx, s) - 1;
    cx.update_window(s.win, |_, w, cx| w.click(("tab-close", last), cx))
        .unwrap();
    cx.run_until_parked();
    frame(cx, s.win);
}

#[gpui_kit::test]
fn closing_a_run_tab_with_the_mouse_focuses_the_tab_behind_it(cx: &mut TestAppContext) {
    let col = two_folders();
    let s = started(cx, &col);
    cx.update_window(s.win, |_, w, _| w.activate_window())
        .unwrap();
    open_tab(cx, &s.ws, &s.root, "alpha/alpha-1.yml");
    frame(cx, s.win);
    run_folder_then_close(cx, &s, "beta");
    assert_eq!(tab_count(cx, &s), 1);
    let handle = active_focus(cx, &s).expect("the request tab");
    assert!(focused(cx, &s, &handle), "the request editor holds focus");
}

// From a fresh launch focus sits on the shell, which is no Tab stop; closing the Run tab
// opened from the sidebar moves it to the tree.
#[gpui_kit::test]
fn a_run_tab_closed_with_the_mouse_after_launch_focuses_the_tree(cx: &mut TestAppContext) {
    for activate in [false, true] {
        let col = two_folders();
        let s = started(cx, &col);
        if activate {
            cx.update_window(s.win, |_, w, _| w.activate_window())
                .unwrap();
        }
        frame(cx, s.win);
        run_folder_then_close(cx, &s, "alpha");
        assert_eq!(tab_count(cx, &s), 0);
        let held = cx
            .update_window(s.win, |_, w, cx| w.focused(cx))
            .unwrap()
            .expect("something holds focus");
        let tree = tree_handle(cx, &s);
        assert_eq!(held, tree, "the tree holds focus (activated: {activate})");
    }
}

// With the only collection closed the window shows "No collection open"; the app's keys
// must still work there.
#[gpui_kit::test]
fn the_palette_key_works_with_no_collection_open(cx: &mut TestAppContext) {
    for in_tree in [false, true] {
        let col = three_requests();
        let s = started(cx, &col);
        cx.update_window(s.win, |_, w, _| w.activate_window())
            .unwrap();
        open_tab(cx, &s.ws, &s.root, "a.yml");
        frame(cx, s.win);
        if in_tree {
            tree_handle(cx, &s);
        } else {
            let f = active_focus(cx, &s).expect("a tab");
            cx.update_window(s.win, |_, w, cx| f.focus(w, cx)).unwrap();
        }
        frame(cx, s.win);
        s.ws.update(cx, |w, cx| w.close(&s.root, cx));
        cx.run_until_parked();
        frame(cx, s.win);
        assert!(present(cx, s.win, "no-collection"), "the empty state shows");
        assert!(
            palette_opens(cx, &s),
            "the palette key works (tree: {in_tree})"
        );
    }
}

// A right press and release over the tab `ix`, as the platform reports them. Unlike the test
// helper, the press travels the whole window pipeline, so the tab and the menu see the same
// events and focus changes a user's click causes.
fn real_right_click_tab(cx: &mut TestAppContext, s: &Started, ix: usize) {
    use gpui_kit::{InputEvent as _, MouseButton, MouseDownEvent, MouseUpEvent};
    frame(cx, s.win);
    let at = cx
        .update_window(s.win, |_, w, _| w.find(("tab", ix)).bounds().center())
        .unwrap();
    let down = MouseDownEvent {
        button: MouseButton::Right,
        position: at,
        modifiers: Default::default(),
        click_count: 1,
        first_mouse: false,
    };
    let up = MouseUpEvent {
        button: MouseButton::Right,
        position: at,
        modifiers: Default::default(),
        click_count: 1,
    };
    for event in [down.to_platform_input(), up.to_platform_input()] {
        cx.update_window(s.win, |_, w, cx| {
            w.dispatch_event(event, cx);
        })
        .unwrap();
        cx.run_until_parked();
        frame(cx, s.win);
    }
}

// One keystroke as a driver delivers it: the press and its release reach the window in a single
// update, so nothing settles between them.
fn press_and_release(cx: &mut TestAppContext, s: &Started, keys: &str) {
    use gpui_kit::{KeyUpEvent, Keystroke, PlatformInput};
    let stroke = Keystroke::parse_with_os(keys, apinata_app::shortcuts::HOST_OS).unwrap();
    cx.update_window(s.win, |_, w, cx| {
        w.dispatch_keystroke(stroke.clone(), cx);
        w.dispatch_event(PlatformInput::KeyUp(KeyUpEvent { keystroke: stroke }), cx);
    })
    .unwrap();
}

// The release of Escape arrives before the menu's dismissal has settled. Focus must still come
// back, so the palette key works with no click in between.
#[gpui_kit::test]
fn escape_released_before_the_menu_settles_restores_focus(cx: &mut TestAppContext) {
    let col = three_requests();
    let s = three_tabs(cx, &col);
    let before = active_focus(cx, &s).expect("an active tab");
    real_right_click_tab(cx, &s, 1);
    press_and_release(cx, &s, "escape");
    frame(cx, s.win);
    frame(cx, s.win);
    assert!(focused(cx, &s, &before), "focus is back where it was");
    press_and_release(cx, &s, &key("shift-p"));
    frame(cx, s.win);
    let open = cx
        .update_window(s.win, |_, w, cx| w.has_active_dialog(cx))
        .unwrap();
    assert!(open, "the palette opened");
}

// Same driver timing, but the tab that held focus closes while the menu is open: its handle is
// gone, so focus must fall back to a tab that is drawn and the palette key must still work.
#[gpui_kit::test]
fn escape_released_early_after_the_focused_tab_closed_falls_back(cx: &mut TestAppContext) {
    let col = three_requests();
    let s = three_tabs(cx, &col);
    real_right_click_tab(cx, &s, 0);
    let last = s.ws.read_with(cx, |w, _| w.tabs().entries()[2].id);
    s.ws.update(cx, |w, cx| {
        w.update_tabs(cx, |t| {
            t.close(last);
        })
    });
    frame(cx, s.win);
    press_and_release(cx, &s, "escape");
    frame(cx, s.win);
    frame(cx, s.win);
    let handle = active_focus(cx, &s).expect("an active tab");
    assert!(focused(cx, &s, &handle), "the active tab holds focus");
    press_and_release(cx, &s, &key("shift-p"));
    frame(cx, s.win);
    let open = cx
        .update_window(s.win, |_, w, cx| w.has_active_dialog(cx))
        .unwrap();
    assert!(open, "the palette opened");
}
