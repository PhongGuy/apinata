//! A collection opened while others are open becomes current, selected and scrolled to the
//! sidebar's top; the collections opened at startup leave the sidebar where it starts.

use crate::support;

use apinata_app::sidebar::model::node_id;
use apinata_core::ItemPath;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AnyWindowHandle, AppContext as _, ExternalPaths, FileDropEvent, InputEvent as _,
    TestAppContext, point, px,
};
use std::path::{Path, PathBuf};
use support::*;

// The sidebar row id of the root of the open collection at `root`.
fn root_row(cx: &mut TestAppContext, s: &Started, root: &Path) -> gpui_kit::SharedString {
    let ix =
        s.ws.read_with(cx, |w, _| {
            (w.collections().iter()).position(|c| c.root() == root)
        })
        .expect("open");
    node_id(ix, &ItemPath::root())
}

fn at_list_top(cx: &mut TestAppContext, win: AnyWindowHandle, row: gpui_kit::SharedString) -> bool {
    frame(cx, win);
    cx.update_window(win, |_, w, _| {
        let list = w.find("sidebar-tree").bounds().top();
        w.try_find(row)
            .is_some_and(|r| (r.bounds().top() - list).abs() <= px(1.))
    })
    .unwrap()
}

fn selected(cx: &mut TestAppContext, s: &Started) -> Option<(PathBuf, ItemPath)> {
    s.shell
        .read_with(cx, |v, cx| v.sidebar().read(cx).selected(cx))
}

fn drop_folder(cx: &mut TestAppContext, win: AnyWindowHandle, dir: &Path) {
    let position = point(px(300.), px(300.));
    let paths = ExternalPaths(vec![dir.to_path_buf()].into());
    cx.update_window(win, |_, w, cx| {
        w.dispatch_event(
            FileDropEvent::Entered { position, paths }.to_platform_input(),
            cx,
        );
        w.dispatch_event(FileDropEvent::Submit { position }.to_platform_input(), cx);
        w.dispatch_event(FileDropEvent::Ended.to_platform_input(), cx);
    })
    .unwrap();
    frame(cx, win);
}

// Two long collections open, so a third lands below the fold unless it is revealed.
fn two_long(cx: &mut TestAppContext) -> (tempfile::TempDir, tempfile::TempDir, Started) {
    let a = long_collection(80);
    let s = started(cx, &a);
    let b = long_collection(40);
    s.ws.update(cx, |w, cx| w.open(b.path().to_path_buf(), cx));
    frame(cx, s.win);
    (a, b, s)
}

#[gpui_kit::test]
fn a_dropped_collection_becomes_current_and_scrolls_to_the_top(cx: &mut TestAppContext) {
    let (_a, _b, s) = two_long(cx);
    let c = long_collection(40);
    drop_folder(cx, s.win, c.path());
    frame(cx, s.win);
    let root = c.path().to_path_buf();
    let current = s.ws.read_with(cx, |w, _| w.current_root());
    assert_eq!(
        current.as_ref(),
        Some(&root),
        "the new collection is current"
    );
    assert_eq!(selected(cx, &s), Some((root.clone(), ItemPath::root())));
    let row = root_row(cx, &s, &root);
    assert!(
        at_list_top(cx, s.win, row),
        "its root row is at the list top"
    );
}

#[gpui_kit::test]
fn collections_opened_at_startup_do_not_scroll_the_sidebar(cx: &mut TestAppContext) {
    let (a, b) = (long_collection(80), long_collection(40));
    let launch = long_collection(40);
    let s = started_with_session(cx, &[a.path(), b.path()]);
    // The launch arguments open after the restored session, the way the binary does it.
    s.ws.update(cx, |w, cx| {
        w.open_roots(vec![launch.path().to_path_buf()], cx)
    });
    frame(cx, s.win);
    assert_eq!(s.ws.read_with(cx, |w, _| w.collections().len()), 3);
    let first = root_row(cx, &s, a.path());
    assert!(
        at_list_top(cx, s.win, first),
        "the first collection stays at the top"
    );
    assert_eq!(selected(cx, &s), None, "nothing is selected");
}
