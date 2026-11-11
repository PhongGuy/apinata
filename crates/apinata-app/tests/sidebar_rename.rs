use crate::support;

use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, TestAppContext, VisualTestContext};
use support::*;

fn rename(cx: &mut TestAppContext, h: &SidebarHarness, path: &str) {
    let (view, root) = (h.view.clone(), h.root.clone());
    cx.update_window(h.win, |_, w, cx| {
        view.update(cx, |s, cx| s.start_rename(&root, &ip(path), w, cx))
    })
    .unwrap();
    frame(cx, h.win);
}

fn type_and(cx: &mut TestAppContext, h: &SidebarHarness, text: &str, key: &str) {
    cx.update_window(h.win, |_, w, cx| {
        w.input(text, cx);
        w.press(key, cx);
    })
    .unwrap();
    cx.run_until_parked();
}

#[gpui_kit::test]
fn enter_renames_the_item_and_its_file(cx: &mut TestAppContext) {
    let h = open_sidebar(cx);
    rename(cx, &h, "health.yml");
    assert!(shown(
        cx,
        h.win,
        apinata_app::sidebar::marker_id("rename", &row("health.yml"))
    ));
    type_and(cx, &h, "Liveness", "enter");
    assert!(h.root.join("Liveness.yml").exists());
    assert!(!h.root.join("health.yml").exists());
    assert_eq!(h.view.read_with(cx, |s, _| s.renaming()), None);
    assert!(shown(cx, h.win, row("Liveness.yml")));
}

#[gpui_kit::test]
fn escape_an_unchanged_or_a_blank_name_send_nothing(cx: &mut TestAppContext) {
    let h = open_sidebar(cx);
    rename(cx, &h, "health.yml");
    type_and(cx, &h, "Other", "escape");
    assert_eq!(h.view.read_with(cx, |s, _| s.renaming()), None);
    rename(cx, &h, "health.yml");
    type_and(cx, &h, "Health", "enter");
    rename(cx, &h, "health.yml");
    type_and(cx, &h, "   ", "enter");
    assert!(h.root.join("health.yml").exists());
    let s = h.ws.read_with(cx, |w, cx| w.snapshot(&h.root, cx)).unwrap();
    assert_eq!(s.acked, 0, "no command reached the actor");
}

#[gpui_kit::test]
fn a_folder_rename_moves_its_directory_and_keeps_it_selected(cx: &mut TestAppContext) {
    let h = open_sidebar(cx);
    cx.update_window(h.win, |_, w, cx| w.click(row("users"), cx))
        .unwrap();
    rename(cx, &h, "users");
    type_and(cx, &h, "people", "enter");
    assert!(h.root.join("people/list.yml").exists());
    assert!(shown(cx, h.win, row("people")));
    assert_eq!(
        h.view.read_with(cx, |s, cx| s.selected(cx)),
        Some((h.root.clone(), ip("people")))
    );
}

#[gpui_kit::test]
fn secondary_r_renames_the_selected_row(cx: &mut TestAppContext) {
    let h = open_sidebar(cx);
    cx.update_window(h.win, |_, w, cx| w.click(row("status.yml"), cx))
        .unwrap();
    // A snapshot between the click and the key rebuilds the tree; the selection must survive it.
    h.ws.update(cx, |w, cx| {
        w.send(&h.root, edit_url("health.yml", "http://x/9"), cx)
    });
    frame(cx, h.win);
    focus_tree(cx, &h);
    cx.simulate_keystrokes(h.win, &secondary("r"));
    assert_eq!(
        h.view.read_with(cx, |s, _| s.renaming()),
        Some((h.root.clone(), ip("status.yml")))
    );
}

// A New Folder made from a row near the bottom of a tree taller than the sidebar starts its
// rename on a row the user can see, wherever the new row lands.
#[gpui_kit::test]
fn a_new_folders_rename_row_is_scrolled_into_view(cx: &mut TestAppContext) {
    // 80 rows of 28 px overflow the 1095 px default test window.
    let mut files: Vec<(&'static str, String)> = vec![("opencollection.yml", ROOT.to_string())];
    for i in 0..80 {
        let path: &'static str = Box::leak(format!("r{i:02}.yml").into_boxed_str());
        files.push((path, request(&format!("r{i:02}"), "http://x/0")));
    }
    let h = open_sidebar_with(cx, files);
    open_tab(cx, &h.ws, &h.root, "r79.yml");
    frame(cx, h.win);
    frame(cx, h.win);
    cx.update_window(h.win, |_, w, cx| {
        h.view.update(cx, |s, cx| {
            s.new_folder(&apinata_app::sidebar::NewFolder, w, cx)
        })
    })
    .unwrap();
    cx.run_until_parked();
    frame(cx, h.win);
    frame(cx, h.win);
    let name = apinata_app::sidebar::menu::NEW_FOLDER_NAME;
    assert_eq!(
        h.view.read_with(cx, |s, _| s.renaming()),
        Some((h.root.clone(), ip(name)))
    );
    let marker = apinata_app::sidebar::marker_id("rename", &row(name));
    assert!(shown(cx, h.win, marker.clone()), "the rename row is drawn");
    cx.update_window(h.win, |_, w, _| {
        let tree = w.find("sidebar-tree").bounds();
        for b in [w.find(marker).bounds(), w.find(row(name)).bounds()] {
            assert!(
                b.top() >= tree.top() && b.bottom() <= tree.bottom(),
                "{b:?} lies inside the tree's visible {tree:?}"
            );
        }
    })
    .unwrap();
}

const FRESH: &str = apinata_app::sidebar::menu::NEW_FOLDER_NAME;

fn new_folder(cx: &mut TestAppContext, h: &SidebarHarness) {
    cx.update_window(h.win, |_, w, cx| {
        h.view.update(cx, |s, cx| {
            s.new_folder(&apinata_app::sidebar::NewFolder, w, cx)
        })
    })
    .unwrap();
    cx.run_until_parked();
    frame(cx, h.win);
    assert_eq!(
        h.view.read_with(cx, |s, _| s.renaming()),
        Some((h.root.clone(), ip(FRESH)))
    );
}

fn gone(cx: &mut TestAppContext, h: &SidebarHarness, name: &str) -> bool {
    frame(cx, h.win);
    !h.root.join(name).exists() && !shown(cx, h.win, row(name))
}

#[gpui_kit::test]
fn the_default_folder_name_is_in_sentence_case(cx: &mut TestAppContext) {
    let h = open_sidebar(cx);
    new_folder(cx, &h);
    assert!(h.root.join("New folder").is_dir());
}

#[gpui_kit::test]
fn escape_on_a_new_folder_removes_it_even_after_typing(cx: &mut TestAppContext) {
    let h = open_sidebar(cx);
    new_folder(cx, &h);
    type_and(cx, &h, "", "escape");
    assert!(
        gone(cx, &h, FRESH),
        "an untouched Escape removes the folder"
    );
    new_folder(cx, &h);
    type_and(cx, &h, "Accounts", "escape");
    assert!(
        gone(cx, &h, FRESH),
        "a typed name then Escape still cancels"
    );
    assert!(!h.root.join("Accounts").exists());
    assert_eq!(h.view.read_with(cx, |s, _| s.renaming()), None);
}

#[gpui_kit::test]
fn clicking_away_from_an_unchanged_new_folder_removes_it(cx: &mut TestAppContext) {
    let h = open_sidebar(cx);
    // Focus events carry empty paths while the window is inactive, so no blur would fire.
    cx.update_window(h.win, |_, w, _| w.activate_window())
        .unwrap();
    new_folder(cx, &h);
    cx.update_window(h.win, |_, w, cx| w.click(row("health.yml"), cx))
        .unwrap();
    frame(cx, h.win);
    cx.run_until_parked();
    assert_eq!(h.view.read_with(cx, |s, _| s.renaming()), None);
    assert!(gone(cx, &h, FRESH));
    assert!(h.root.join("health.yml").exists());
}

#[gpui_kit::test]
fn enter_keeps_a_new_folder_under_its_default_name(cx: &mut TestAppContext) {
    let h = open_sidebar(cx);
    new_folder(cx, &h);
    type_and(cx, &h, "", "enter");
    assert!(h.root.join(FRESH).is_dir());
    assert!(shown(cx, h.win, row(FRESH)));
}

#[gpui_kit::test]
fn escape_never_removes_a_folder_that_existed_before(cx: &mut TestAppContext) {
    let h = open_sidebar(cx);
    cx.update_window(h.win, |_, w, _| w.activate_window())
        .unwrap();
    rename(cx, &h, "users");
    type_and(cx, &h, "", "escape");
    rename(cx, &h, "users");
    cx.update_window(h.win, |_, w, cx| w.click(row("health.yml"), cx))
        .unwrap();
    frame(cx, h.win);
    cx.run_until_parked();
    assert_eq!(h.view.read_with(cx, |s, _| s.renaming()), None);
    assert!(h.root.join("users/list.yml").exists());
    assert!(shown(cx, h.win, row("users")));
}

// Something written into the new folder before Escape keeps it, with every file in it.
#[gpui_kit::test]
fn a_new_folder_that_gained_contents_is_kept(cx: &mut TestAppContext) {
    let h = open_sidebar(cx);
    new_folder(cx, &h);
    std::fs::write(h.root.join(FRESH).join("notes.txt"), "keep me").unwrap();
    type_and(cx, &h, "", "escape");
    assert_eq!(
        std::fs::read_to_string(h.root.join(FRESH).join("notes.txt")).unwrap(),
        "keep me"
    );
    assert!(h.root.join(FRESH).join("folder.yml").exists());
}

// Typing then clicking away ends the rename without its name, as any rename's blur does.
#[gpui_kit::test]
fn typing_a_name_then_clicking_away_keeps_the_new_folder(cx: &mut TestAppContext) {
    let h = open_sidebar(cx);
    cx.update_window(h.win, |_, w, _| w.activate_window())
        .unwrap();
    new_folder(cx, &h);
    cx.update_window(h.win, |_, w, cx| w.input("Accounts", cx))
        .unwrap();
    cx.update_window(h.win, |_, w, cx| w.click(row("health.yml"), cx))
        .unwrap();
    frame(cx, h.win);
    cx.run_until_parked();
    assert_eq!(h.view.read_with(cx, |s, _| s.renaming()), None);
    assert!(h.root.join(FRESH).is_dir());
    assert!(!h.root.join("Accounts").exists());
}

// Switching to another window is not leaving the name field: the rename stays open.
#[gpui_kit::test]
fn switching_windows_keeps_a_new_folder_and_its_rename(cx: &mut TestAppContext) {
    let h = open_sidebar(cx);
    cx.update_window(h.win, |_, w, _| w.activate_window())
        .unwrap();
    new_folder(cx, &h);
    VisualTestContext::from_window(h.win, cx).deactivate_window();
    frame(cx, h.win);
    cx.run_until_parked();
    assert!(h.root.join(FRESH).is_dir());
    assert_eq!(
        h.view.read_with(cx, |s, _| s.renaming()),
        Some((h.root.clone(), ip(FRESH)))
    );
}
