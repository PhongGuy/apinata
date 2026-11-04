//! The Inherited headers group inside the request editor: no gap when nothing is inherited,
//! and a click on a source name opens that level's settings tab.

use crate::support;

use apinata_app::request_editor::Section;
use apinata_app::tabs::TabKind;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext as _, TestAppContext};
use support::*;

const REQ: &str =
    "info:\n  name: r\n  type: http\n  seq: 1\nhttp:\n  method: GET\n  url: https://x.test/\n";
const COLLECTION: &str = "opencollection: 1.0.0\ninfo:\n  name: c\nrequest:\n  headers:\n    - name: X-C\n      value: c\n";
const FOLDER: &str = "info:\n  name: Folder F\n  type: folder\nrequest:\n  headers:\n    - name: X-F\n      value: f\n";

fn show_headers(cx: &mut TestAppContext, win: gpui_kit::AnyWindowHandle) {
    cx.update_window(win, |_, w, cx| {
        w.click(("section", Section::Headers as usize), cx)
    })
    .unwrap();
    frame(cx, win);
}

#[gpui_kit::test]
fn nothing_inherited_adds_no_gap_above_the_default_headers(cx: &mut TestAppContext) {
    let col = write_tree(&[
        ("opencollection.yml", ROOT),
        ("f/folder.yml", FOLDER),
        ("f/r.yml", REQ),
        ("top.yml", REQ),
    ]);
    let (_state, ws, root) = open_dir(cx, &col);
    let (with_win, _e1) = editor_for(cx, &ws, &root, "f/r.yml", false);
    show_headers(cx, with_win);
    let (none_win, _e2) = editor_for(cx, &ws, &root, "top.yml", false);
    show_headers(cx, none_win);
    let height = |cx: &mut TestAppContext, win, id: &'static str| {
        cx.update_window(win, |_, w, _| w.find(id).bounds().size.height)
            .unwrap()
    };
    let group = height(cx, with_win, "inherited-headers");
    let (a, b) = (
        height(cx, with_win, "headers"),
        height(cx, none_win, "headers"),
    );
    let gap = apinata_app::style::SPACE_3;
    let rem = cx
        .update_window(with_win, |_, w, _| gap.to_pixels(w.rem_size()))
        .unwrap();
    // The group costs exactly its own height plus one gap; an empty group costs nothing.
    let diff = (a - b - group - rem).abs();
    assert!(diff < gpui_kit::px(1.), "{a:?} {b:?} {group:?} {rem:?}");
}

#[gpui_kit::test]
fn a_source_name_opens_the_settings_of_its_level(cx: &mut TestAppContext) {
    let col = write_tree(&[
        ("opencollection.yml", COLLECTION),
        ("f/folder.yml", FOLDER),
        ("f/r.yml", REQ),
    ]);
    let (_state, ws, root) = open_dir(cx, &col);
    let (win, _ed) = editor_for(cx, &ws, &root, "f/r.yml", false);
    show_headers(cx, win);
    let active = |cx: &mut TestAppContext| {
        ws.read_with(cx, |w, _| w.tabs().active().map(|e| e.kind.clone()))
    };
    cx.update_window(win, |_, w, cx| w.click("inherited-header-1-source", cx))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(active(cx), Some(TabKind::Folder(ip("f"))));
    cx.update_window(win, |_, w, cx| w.click("inherited-header-0-source", cx))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(active(cx), Some(TabKind::Collection));
}
