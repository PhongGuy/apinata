//! Close to the right and Reveal in sidebar: the tab menu items and the palette commands.

use crate::support;

use apinata_app::commands::registry;
use apinata_app::palette::{Availability, palette_items, palette_items_for};
use apinata_app::tabs::{TabId, TabKind};
use gpui_kit::AppContext as _;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AnyWindowHandle, TestAppContext};
use support::*;

fn five() -> tempfile::TempDir {
    write_tree(&[
        ("opencollection.yml", ROOT),
        ("a.yml", &request("a", "http://x/a")),
        ("b.yml", &request("b", "http://x/b")),
        ("c.yml", &request("c", "http://x/c")),
        ("d.yml", &request("d", "http://x/d")),
        ("e.yml", &request("e", "http://x/e")),
        ("users/folder.yml", &folder("Users")),
        ("users/admin/folder.yml", &folder("admin")),
        ("users/admin/list.yml", &request("List", "http://x/")),
    ])
}

fn opened(cx: &mut TestAppContext, col: &tempfile::TempDir) -> (Started, [TabId; 5]) {
    let s = started(cx, col);
    let ids =
        ["a.yml", "b.yml", "c.yml", "d.yml", "e.yml"].map(|n| open_tab(cx, &s.ws, &s.root, n));
    cx.run_until_parked();
    (s, ids)
}

fn run(cx: &mut TestAppContext, win: AnyWindowHandle, id: &str) {
    let cmd = registry().into_iter().find(|c| c.id == id).expect(id);
    let entry = palette_items()
        .into_iter()
        .find(|e| e.label == cmd.label)
        .unwrap_or_else(|| panic!("{id} is in the palette"));
    cx.update_window(win, |_, w, cx| {
        w.dispatch_action(entry.action.boxed_clone(), cx)
    })
    .unwrap();
    frame(cx, win);
}

fn activate(cx: &mut TestAppContext, s: &Started, id: TabId) {
    s.ws.update(cx, |w, cx| w.update_tabs(cx, |t| t.activate(id)));
}

fn pin(cx: &mut TestAppContext, s: &Started, id: TabId) {
    s.ws.update(cx, |w, cx| w.update_tabs(cx, |t| t.set_pinned(id, true)));
    cx.run_until_parked();
}

// Right-clicks tab `tab` and clicks the menu item called `label`, found by scanning the menu.
fn menu_click(cx: &mut TestAppContext, s: &Started, tab: usize, label: &str) {
    cx.update_window(s.win, |_, w, cx| w.right_click(("tab", tab), cx))
        .unwrap();
    frame(cx, s.win);
    let ix = (0..16usize)
        .find(|ix| {
            cx.update_window(s.win, |_, w, _| {
                w.within("popup-menu")
                    .try_find(*ix)
                    .and_then(|n| n.label().map(str::to_owned))
            })
            .unwrap()
            .as_deref()
                == Some(label)
        })
        .unwrap_or_else(|| panic!("{label} is in the tab menu"));
    cx.update_window(s.win, |_, w, cx| w.within("popup-menu").click(ix, cx))
        .unwrap();
    cx.run_until_parked();
    frame(cx, s.win);
}

fn selected(cx: &mut TestAppContext, s: &Started) -> Option<apinata_core::ItemPath> {
    s.shell
        .read_with(cx, |sh, cx| sh.sidebar().read(cx).selected(cx))
        .map(|(_, p)| p)
}

#[gpui_kit::test]
fn the_menu_closes_the_tabs_right_of_the_clicked_one(cx: &mut TestAppContext) {
    let col = five();
    let (s, _) = opened(cx, &col);
    menu_click(cx, &s, 2, "Close to the right");
    assert_eq!(tab_names(cx, &s.ws), ["a.yml", "b.yml", "c.yml"]);
}

#[gpui_kit::test]
fn the_palette_closes_the_tabs_right_of_the_active_one(cx: &mut TestAppContext) {
    let col = five();
    let (s, ids) = opened(cx, &col);
    activate(cx, &s, ids[2]);
    run(cx, s.win, "tabs.close_to_right");
    assert_eq!(tab_names(cx, &s.ws), ["a.yml", "b.yml", "c.yml"]);
}

#[gpui_kit::test]
fn close_to_the_right_skips_pinned_tabs(cx: &mut TestAppContext) {
    let col = five();
    let (s, ids) = opened(cx, &col);
    pin(cx, &s, ids[3]);
    // The pinned d.yml moved to the front: d a b c e.
    activate(cx, &s, ids[0]);
    run(cx, s.win, "tabs.close_to_right");
    assert_eq!(tab_names(cx, &s.ws), ["d.yml", "a.yml"]);
}

#[gpui_kit::test]
fn close_to_the_right_does_nothing_on_the_last_tab(cx: &mut TestAppContext) {
    let col = five();
    let (s, ids) = opened(cx, &col);
    activate(cx, &s, ids[4]);
    run(cx, s.win, "tabs.close_to_right");
    assert_eq!(tab_names(cx, &s.ws).len(), 5);
    menu_click(cx, &s, 4, "Close to the right");
    assert_eq!(
        tab_names(cx, &s.ws).len(),
        5,
        "the greyed item does nothing"
    );
}

#[gpui_kit::test]
fn close_to_the_right_with_no_active_tab_does_nothing(cx: &mut TestAppContext) {
    let col = five();
    let s = started(cx, &col);
    run(cx, s.win, "tabs.close_to_right");
    assert!(tab_names(cx, &s.ws).is_empty());
}

#[gpui_kit::test]
fn a_dirty_tab_to_the_right_prompts_once(cx: &mut TestAppContext) {
    let col = five();
    let (s, _) = opened(cx, &col);
    s.ws.update(cx, |w, cx| {
        w.send(&s.root, edit_url("d.yml", "http://x/2"), cx)
    });
    cx.run_until_parked();
    menu_click(cx, &s, 2, "Close to the right");
    assert!(confirm_open(cx), "the dirty d.yml asks first");
    assert_eq!(tab_names(cx, &s.ws).len(), 5);
}

#[gpui_kit::test]
fn the_menu_reveals_a_request_in_the_sidebar(cx: &mut TestAppContext) {
    let col = five();
    let s = started(cx, &col);
    open_tab(cx, &s.ws, &s.root, "users/admin/list.yml");
    frame(cx, s.win);
    // Opening a tab already follows it in the sidebar, so a hidden sidebar is the sign.
    press_in_shell(cx, &s, &key("b"));
    frame(cx, s.win);
    assert!(!s.shell.read_with(cx, |sh, _| sh.sidebar_visible()));
    menu_click(cx, &s, 0, "Reveal in sidebar");
    assert!(s.shell.read_with(cx, |sh, _| sh.sidebar_visible()));
    assert_eq!(selected(cx, &s), Some(ip("users/admin/list.yml")));
}

#[gpui_kit::test]
fn the_palette_reveals_the_active_tab(cx: &mut TestAppContext) {
    let col = five();
    let s = started(cx, &col);
    open_tab(cx, &s.ws, &s.root, "users/admin/list.yml");
    frame(cx, s.win);
    press_in_shell(cx, &s, &key("b"));
    frame(cx, s.win);
    run(cx, s.win, "tabs.reveal_in_sidebar");
    assert!(s.shell.read_with(cx, |sh, _| sh.sidebar_visible()));
    assert_eq!(selected(cx, &s), Some(ip("users/admin/list.yml")));
}

#[gpui_kit::test]
fn reveal_selects_a_folder_for_its_settings_and_runner_tabs(cx: &mut TestAppContext) {
    let col = five();
    let s = started(cx, &col);
    s.ws.update(cx, |w, cx| {
        w.open_settings(&s.root, TabKind::Folder(ip("users/admin")), cx)
    });
    frame(cx, s.win);
    run(cx, s.win, "tabs.reveal_in_sidebar");
    assert_eq!(selected(cx, &s), Some(ip("users/admin")));
    s.ws.update(cx, |w, cx| {
        w.open_settings(&s.root, TabKind::Runner(ip("users")), cx)
    });
    frame(cx, s.win);
    run(cx, s.win, "tabs.reveal_in_sidebar");
    assert_eq!(selected(cx, &s), Some(ip("users")));
}

#[gpui_kit::test]
fn reveal_is_disabled_for_the_environments_tab(cx: &mut TestAppContext) {
    let col = five();
    let s = started(cx, &col);
    open_tab(cx, &s.ws, &s.root, "a.yml");
    s.ws.update(cx, |w, cx| {
        w.open_settings(&s.root, TabKind::Environments, cx)
    });
    frame(cx, s.win);
    press_in_shell(cx, &s, &key("b"));
    frame(cx, s.win);
    run(cx, s.win, "tabs.reveal_in_sidebar");
    assert!(!s.shell.read_with(cx, |sh, _| sh.sidebar_visible()));
    menu_click(cx, &s, 1, "Reveal in sidebar");
    assert!(
        !s.shell.read_with(cx, |sh, _| sh.sidebar_visible()),
        "the greyed item does nothing"
    );
}

#[test]
fn the_registry_names_both_commands() {
    let labels: Vec<_> = registry()
        .into_iter()
        .filter(|c| c.id.starts_with("tabs."))
        .map(|c| (c.id, c.label))
        .collect();
    assert!(labels.contains(&("tabs.close_to_right", "Close to the right")));
    assert!(labels.contains(&("tabs.reveal_in_sidebar", "Reveal in sidebar")));
}

// Opens the palette, filters it to the exact `label` and reports whether any row is left.
fn palette_lists(cx: &mut TestAppContext, s: &Started, label: &str) -> bool {
    cx.simulate_keystrokes(s.win, &key("shift-p"));
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| w.input(label, cx))
        .unwrap();
    frame(cx, s.win);
    // The query is exact, so a listed command leaves one option row and a hidden one none.
    let found = cx
        .update_window(s.win, |_, w, _| {
            gpui_kit::base::test_support::snapshots(w)
                .iter()
                .any(|n| n.role() == Some(gpui_kit::Role::ListBoxOption))
        })
        .unwrap();
    cx.simulate_keystrokes(s.win, "escape");
    frame(cx, s.win);
    found
}

// Chooses the row called `label` in the real palette by filtering to it and pressing enter.
fn palette_choose(cx: &mut TestAppContext, s: &Started, label: &str) {
    cx.simulate_keystrokes(s.win, &key("shift-p"));
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| w.input(label, cx))
        .unwrap();
    frame(cx, s.win);
    cx.simulate_keystrokes(s.win, "enter");
    cx.run_until_parked();
    frame(cx, s.win);
}

#[test]
fn availability_filters_the_two_tab_commands_by_name() {
    let has = |a: Availability, label: &str| palette_items_for(a).iter().any(|e| e.label == label);
    let none = Availability {
        close_to_right: false,
        reveal: false,
        ..Availability::all()
    };
    assert!(!has(none, "Close to the right") && !has(none, "Reveal in sidebar"));
    assert!(has(Availability::all(), "Close to the right"));
    assert!(has(Availability::default(), "Reveal in sidebar"));
    assert!(has(none, "Run selected requests"));
}

#[gpui_kit::test]
fn the_palette_hides_both_commands_on_the_last_tab(cx: &mut TestAppContext) {
    let col = five();
    let (s, ids) = opened(cx, &col);
    activate(cx, &s, ids[4]);
    frame(cx, s.win);
    assert!(palette_lists(cx, &s, "Reveal in sidebar"));
    assert!(!palette_lists(cx, &s, "Close to the right"));
}

#[gpui_kit::test]
fn the_palette_hides_close_to_the_right_on_the_last_unpinned_tab(cx: &mut TestAppContext) {
    let col = five();
    let (s, ids) = opened(cx, &col);
    // Pinned tabs move to the front, so a pinned tab never sits right of an unpinned one; the
    // last unpinned tab then has nothing to close even with pinned tabs in the strip.
    pin(cx, &s, ids[0]);
    pin(cx, &s, ids[1]);
    activate(cx, &s, ids[4]);
    assert!(!palette_lists(cx, &s, "Close to the right"));
}

#[gpui_kit::test]
fn the_palette_hides_reveal_on_the_environments_tab(cx: &mut TestAppContext) {
    let col = five();
    let s = started(cx, &col);
    open_tab(cx, &s.ws, &s.root, "a.yml");
    s.ws.update(cx, |w, cx| {
        w.open_settings(&s.root, TabKind::Environments, cx)
    });
    frame(cx, s.win);
    assert!(!palette_lists(cx, &s, "Reveal in sidebar"));
}

#[gpui_kit::test]
fn the_palette_hides_both_with_no_tab_open(cx: &mut TestAppContext) {
    let col = five();
    let s = started(cx, &col);
    assert!(!palette_lists(cx, &s, "Close to the right"));
    assert!(!palette_lists(cx, &s, "Reveal in sidebar"));
}

#[gpui_kit::test]
fn the_palette_lists_and_runs_both_on_a_middle_tab(cx: &mut TestAppContext) {
    let col = five();
    let (s, ids) = opened(cx, &col);
    activate(cx, &s, ids[2]);
    assert!(palette_lists(cx, &s, "Close to the right"));
    assert!(palette_lists(cx, &s, "Reveal in sidebar"));
    press_in_shell(cx, &s, &key("b"));
    frame(cx, s.win);
    palette_choose(cx, &s, "Reveal in sidebar");
    assert!(s.shell.read_with(cx, |sh, _| sh.sidebar_visible()));
    assert_eq!(selected(cx, &s), Some(ip("c.yml")));
    palette_choose(cx, &s, "Close to the right");
    assert_eq!(tab_names(cx, &s.ws), ["a.yml", "b.yml", "c.yml"]);
}
