//! The collection header: create buttons that act on their own collection, and a busy
//! spinner that stays still under reduced motion.

use crate::support;

use apinata_app::shell::collection_row;
use apinata_core::ItemPath;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    AnyWindowHandle, AppContext as _, Context, IntoElement, ParentElement, Render, Styled,
    TestAppContext, Window, WindowOptions, div,
};
use support::*;

#[gpui_kit::test]
fn the_header_buttons_create_in_their_collection(cx: &mut TestAppContext) {
    let h = open_sidebar(cx);
    cx.update_window(h.win, |_, w, cx| w.click(("new-request", 0usize), cx))
        .unwrap();
    cx.run_until_parked();
    let tabs = tab_names(cx, &h.ws);
    let [draft] = tabs.as_slice() else {
        panic!("{tabs:?}")
    };
    assert!(ip(draft).is_draft(), "{draft}");
    assert_eq!(ip(draft).parent(), Some(ItemPath::root()));
    assert!(!h.root.join("Untitled.yml").exists());
    assert_eq!(h.view.read_with(cx, |s, _| s.renaming()), None);
    assert!(
        shown(cx, h.win, row("health.yml")),
        "the click did not collapse the collection"
    );
    cx.update_window(h.win, |_, w, cx| w.press("escape", cx))
        .unwrap();
    cx.run_until_parked();
    cx.update_window(h.win, |_, w, cx| w.click(("new-folder", 0usize), cx))
        .unwrap();
    cx.run_until_parked();
    assert!(h.root.join("New folder").is_dir());
    assert!(shown(cx, h.win, row("health.yml")));
}

struct Busy;

impl Render for Busy {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full().child(collection_row(0, "a".into(), true))
    }
}

fn frames_requested(cx: &mut TestAppContext, win: AnyWindowHandle) -> usize {
    frame(cx, win);
    cx.update_window(win, |_, w, cx| w.simulate_next_frame(cx))
        .unwrap()
}

#[gpui_kit::test]
fn the_busy_spinner_is_static_under_reduced_motion(cx: &mut TestAppContext) {
    support::init(cx);
    let (win, _view) = cx
        .update(|cx| gpui_kit::open_window(WindowOptions::default(), cx, |_, cx| cx.new(|_| Busy)))
        .unwrap();
    assert!(present(cx, win, ("busy", 0usize)));
    assert_eq!(frames_requested(cx, win), 0);
    cx.update(|cx| cx.set_reduce_motion(false));
    assert!(
        frames_requested(cx, win) > 0,
        "the spinner animates when motion is allowed"
    );
}
