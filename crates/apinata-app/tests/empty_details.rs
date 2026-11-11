//! The details of empty places: the no-collection screen, the sidebar with nothing open and
//! empty folders.

use crate::support;

use apinata_app::sidebar::marker_id;
use apinata_app::sidebar::model::empty_id;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, TestAppContext, px};
use support::*;

const HEADER: [&str; 3] = [
    "sidebar-collapse-all",
    "sidebar-expand-all",
    "sidebar-history",
];

#[gpui_kit::test]
fn new_request_on_the_empty_screen_shows_its_key(cx: &mut TestAppContext) {
    let s = started_blank(cx, blank_data_dir());
    let chip = cx
        .update_window(s.win, |_, w, _| {
            w.within("no-collection-actions")
                .try_find(("empty-kbd", 0usize))
                .is_some()
        })
        .unwrap();
    assert!(chip, "New request shows its key chip");
}

#[gpui_kit::test]
fn the_internet_note_sits_under_the_demo_action(cx: &mut TestAppContext) {
    let s = started_blank(cx, blank_data_dir());
    cx.update_window(s.win, |_, w, _| {
        let actions = w.within("no-collection-actions");
        let demo = actions.find(("empty-action", 5usize)).bounds();
        let note = actions.find("demo-hint").bounds();
        assert!(note.top() >= demo.bottom(), "{note:?} vs {demo:?}");
        assert!(note.top() - demo.bottom() < px(8.), "{note:?} vs {demo:?}");
    })
    .unwrap();
}

#[gpui_kit::test]
fn with_nothing_open_the_sidebar_header_buttons_are_disabled(cx: &mut TestAppContext) {
    let s = started_blank(cx, blank_data_dir());
    cx.update_window(s.win, |_, w, _| {
        for id in HEADER {
            assert_eq!(w.find(id).disabled(), Some(true), "{id}");
        }
    })
    .unwrap();
}

#[gpui_kit::test]
fn with_a_collection_open_the_sidebar_header_buttons_work(cx: &mut TestAppContext) {
    let col = collection();
    let s = started(cx, &col);
    cx.update_window(s.win, |_, w, _| {
        for id in HEADER {
            assert_ne!(w.find(id).disabled(), Some(true), "{id}");
        }
    })
    .unwrap();
}

#[gpui_kit::test]
fn an_empty_folder_has_no_run_button_and_shows_empty_when_open(cx: &mut TestAppContext) {
    let mut files = sidebar_files();
    files.push(("empty/folder.yml", folder("Nothing here")));
    let h = open_sidebar_with(cx, files);
    assert!(present(cx, h.win, marker_id("run-folder", &row("users"))));
    assert!(!present(cx, h.win, marker_id("run-folder", &row("empty"))));
    assert!(!present(cx, h.win, empty_id(0, &ip("empty"))));
    cx.update_window(h.win, |_, w, cx| w.click("sidebar-expand-all", cx))
        .unwrap();
    frame(cx, h.win);
    assert!(present(cx, h.win, empty_id(0, &ip("empty"))));
}
