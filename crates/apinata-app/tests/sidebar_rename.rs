use crate::support;

use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, TestAppContext};
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
