//! The tab menu's Duplicate and Close to the left, and the close prompts' shared wording.

use crate::support;

use apinata_app::commands::registry;
use apinata_app::palette::palette_items;
use apinata_app::tabs::{TabId, TabKind};
use gpui_kit::AppContext as _;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AnyWindowHandle, TestAppContext, VisualTestContext};
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
        ("users/list.yml", &request("List", "http://x/list")),
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
    cx.run_until_parked();
    frame(cx, win);
}

// Right-clicks tab `tab` and clicks the menu item called `label`, found by scanning the menu.
fn menu_click(cx: &mut TestAppContext, s: &Started, tab: usize, label: &str) {
    cx.update_window(s.win, |_, w, cx| w.right_click(("tab", tab), cx))
        .unwrap();
    frame(cx, s.win);
    let ix = (0..20usize)
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

fn files(s: &Started) -> usize {
    fn walk(dir: &std::path::Path) -> usize {
        (std::fs::read_dir(dir).unwrap())
            .map(|e| {
                let p = e.unwrap().path();
                if p.is_dir() { walk(&p) } else { 1 }
            })
            .sum()
    }
    walk(&s.root)
}

fn confirm_text(cx: &mut TestAppContext, id: &'static str) -> Option<String> {
    let win = confirm_window(cx).expect("a prompt");
    cx.update_window(win, |_, w, _| w.find(id).label().map(str::to_owned))
        .unwrap()
}

#[gpui_kit::test]
fn the_menu_closes_the_tabs_left_of_the_clicked_one(cx: &mut TestAppContext) {
    let col = five();
    let (s, _) = opened(cx, &col);
    menu_click(cx, &s, 2, "Close to the left");
    assert_eq!(tab_names(cx, &s.ws), ["c.yml", "d.yml", "e.yml"]);
}

#[gpui_kit::test]
fn the_palette_closes_the_tabs_left_of_the_active_one(cx: &mut TestAppContext) {
    let col = five();
    let (s, ids) = opened(cx, &col);
    s.ws.update(cx, |w, cx| w.update_tabs(cx, |t| t.activate(ids[3])));
    run(cx, s.win, "tabs.close_to_left");
    assert_eq!(tab_names(cx, &s.ws), ["d.yml", "e.yml"]);
}

#[gpui_kit::test]
fn close_to_the_left_keeps_pinned_tabs_and_is_greyed_on_the_first(cx: &mut TestAppContext) {
    let col = five();
    let (s, ids) = opened(cx, &col);
    s.ws.update(cx, |w, cx| {
        w.update_tabs(cx, |t| t.set_pinned(ids[4], true))
    });
    cx.run_until_parked();
    // The pinned e.yml moved to the front: e a b c d.
    menu_click(cx, &s, 3, "Close to the left");
    assert_eq!(tab_names(cx, &s.ws), ["e.yml", "c.yml", "d.yml"]);
    menu_click(cx, &s, 1, "Close to the left");
    assert_eq!(
        tab_names(cx, &s.ws),
        ["e.yml", "c.yml", "d.yml"],
        "only a pinned tab is left of it, so the greyed item does nothing"
    );
}

#[gpui_kit::test]
fn a_dirty_tab_to_the_left_prompts_once(cx: &mut TestAppContext) {
    let col = five();
    let (s, _) = opened(cx, &col);
    s.ws.update(cx, |w, cx| {
        w.send(&s.root, edit_url("a.yml", "http://x/2"), cx)
    });
    cx.run_until_parked();
    menu_click(cx, &s, 2, "Close to the left");
    assert!(confirm_open(cx), "the dirty a.yml asks first");
    assert_eq!(tab_names(cx, &s.ws).len(), 5);
    answer_confirm(cx, "confirm-cancel");
}

#[gpui_kit::test]
fn duplicate_opens_an_unsaved_copy_in_the_same_folder(cx: &mut TestAppContext) {
    let col = five();
    let s = started(cx, &col);
    open_tab(cx, &s.ws, &s.root, "users/list.yml");
    cx.run_until_parked();
    s.ws.update(cx, |w, cx| {
        w.send(&s.root, edit_url("users/list.yml", "http://x/edited"), cx)
    });
    cx.run_until_parked();
    let before = files(&s);
    menu_click(cx, &s, 0, "Duplicate");
    let names = tab_names(cx, &s.ws);
    assert_eq!(names.len(), 2, "{names:?}");
    let copy = names[1].clone();
    assert!(
        copy.starts_with("users/") && copy.ends_with(".draft"),
        "{copy}"
    );
    let view = view_of(cx, &s.ws, &s.root, &copy);
    assert!(view.draft);
    assert_eq!(view.name, "List (copy)");
    assert_eq!(
        view.url, "http://x/edited",
        "the copy holds the unsaved edits"
    );
    let active = s.ws.read_with(cx, |w, _| {
        w.tabs()
            .active()
            .and_then(|e| e.item())
            .map(|p| p.as_str().to_owned())
    });
    assert_eq!(active.as_deref(), Some(copy.as_str()));
    assert_eq!(files(&s), before, "nothing is written");
}

#[gpui_kit::test]
fn the_palette_duplicates_the_active_tab(cx: &mut TestAppContext) {
    let col = five();
    let s = started(cx, &col);
    open_tab(cx, &s.ws, &s.root, "a.yml");
    cx.run_until_parked();
    run(cx, s.win, "tabs.duplicate");
    let names = tab_names(cx, &s.ws);
    assert_eq!(names.len(), 2, "{names:?}");
    assert_eq!(view_of(cx, &s.ws, &s.root, &names[1]).name, "a (copy)");
}

#[gpui_kit::test]
fn duplicate_is_greyed_for_a_tab_without_a_request(cx: &mut TestAppContext) {
    let col = five();
    let s = started(cx, &col);
    s.ws.update(cx, |w, cx| {
        w.open_settings(&s.root, TabKind::Environments, cx)
    });
    frame(cx, s.win);
    menu_click(cx, &s, 0, "Duplicate");
    let count = s.ws.read_with(cx, |w, _| w.tabs().entries().len());
    assert_eq!(count, 1, "the greyed item does nothing");
}

#[gpui_kit::test]
fn the_tab_prompt_says_dont_save_and_names_what_changed(cx: &mut TestAppContext) {
    let col = five();
    let (s, ids) = opened(cx, &col);
    for (rel, url) in [("a.yml", "http://x/1"), ("b.yml", "http://x/2")] {
        s.ws.update(cx, |w, cx| w.send(&s.root, edit_url(rel, url), cx));
    }
    cx.run_until_parked();
    let area = s.shell.read_with(cx, |sh, _| sh.tab_area().clone());
    cx.update_window(s.win, |_, w, cx| {
        area.update(cx, |a, cx| a.close(vec![ids[0]], w, cx))
    })
    .unwrap();
    assert_eq!(
        confirm_text(cx, "confirm-discard").as_deref(),
        Some("Don't save")
    );
    assert_eq!(
        confirm_text(cx, "confirm-detail").as_deref(),
        Some("\"a\" has unsaved changes.")
    );
    answer_confirm(cx, "confirm-cancel");
    cx.update_window(s.win, |_, w, cx| {
        area.update(cx, |a, cx| a.close(vec![ids[0], ids[1]], w, cx))
    })
    .unwrap();
    assert_eq!(
        confirm_text(cx, "confirm-detail").as_deref(),
        Some("\"a\" and \"b\" have unsaved changes.")
    );
    answer_confirm(cx, "confirm-cancel");
}

#[gpui_kit::test]
fn the_window_prompt_says_dont_save_like_the_tab_prompt(cx: &mut TestAppContext) {
    let col = five();
    let (s, _) = opened(cx, &col);
    s.ws.update(cx, |w, cx| {
        w.send(&s.root, edit_url("a.yml", "http://x/1"), cx)
    });
    cx.run_until_parked();
    assert!(!VisualTestContext::from_window(s.win, cx).simulate_close());
    assert_eq!(
        confirm_text(cx, "confirm-discard").as_deref(),
        Some("Don't save")
    );
    answer_confirm(cx, "confirm-cancel");
}

#[test]
fn the_registry_names_both_new_commands() {
    let labels: Vec<_> = registry().into_iter().map(|c| (c.id, c.label)).collect();
    assert!(labels.contains(&("tabs.close_to_left", "Close to the left")));
    assert!(labels.contains(&("tabs.duplicate", "Duplicate tab")));
}
