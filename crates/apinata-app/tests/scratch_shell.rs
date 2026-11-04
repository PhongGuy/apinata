//! The window with only the scratch: when the welcome state shows.

use crate::support;

use gpui_kit::TestAppContext;
use support::*;

#[gpui_kit::test]
fn a_fresh_launch_shows_the_welcome_state(cx: &mut TestAppContext) {
    let s = started_blank(cx, blank_data_dir());
    assert!(present(cx, s.win, "no-collection"));
    assert!(s.ws.read_with(cx, |w, cx| w.shows_empty_state(cx)));
}

#[gpui_kit::test]
fn an_open_scratch_without_work_still_shows_the_welcome_state(cx: &mut TestAppContext) {
    let s = started_blank(cx, blank_data_dir());
    s.ws.update(cx, |w, cx| w.open_scratch(cx));
    frame(cx, s.win);
    assert!(present(cx, s.win, "no-collection"));
}

#[gpui_kit::test]
fn a_scratch_draft_replaces_the_welcome_state(cx: &mut TestAppContext) {
    let s = started_blank(cx, blank_data_dir());
    let root = s.ws.update(cx, |w, cx| w.open_scratch(cx)).unwrap();
    cx.run_until_parked();
    s.ws.update(cx, |w, cx| {
        w.new_draft(&root, apinata_core::ItemPath::root(), cx)
    });
    cx.run_until_parked();
    frame(cx, s.win);
    assert!(!present(cx, s.win, "no-collection"));
    assert!(s.ws.read_with(cx, |w, cx| !w.shows_empty_state(cx)));
}

#[gpui_kit::test]
fn the_sidebar_lists_the_scratch_only_while_it_holds_work(cx: &mut TestAppContext) {
    use apinata_app::sidebar::model::node_id;
    let col = collection();
    let s = started(cx, &col);
    let root = s.ws.update(cx, |w, cx| w.open_scratch(cx)).unwrap();
    frame(cx, s.win);
    let (all, listed) = s.ws.read_with(cx, |w, cx| {
        (w.collections().len(), w.listed_collections(cx).len())
    });
    assert_eq!((all, listed), (2, 1));
    let top = apinata_core::ItemPath::root();
    assert!(shown(cx, s.win, node_id(0, &top)));
    assert!(!shown(cx, s.win, node_id(1, &top)));
    s.ws.update(cx, |w, cx| w.new_draft(&root, top.clone(), cx));
    frame(cx, s.win);
    let listed = s.ws.read_with(cx, |w, cx| w.listed_collections(cx).len());
    assert_eq!(listed, 2);
    assert!(shown(cx, s.win, node_id(1, &top)));
}
