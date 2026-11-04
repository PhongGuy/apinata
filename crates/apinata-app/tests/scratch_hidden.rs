//! The scratch stays out of every place that lists or acts on the user's collections.

use crate::import_update::rig;
use crate::support;

use apinata_app::persist::Paths;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, TestAppContext};
use support::*;

#[gpui_kit::test]
fn the_switcher_does_not_list_the_scratch(cx: &mut TestAppContext) {
    let col = collection();
    let s = started(cx, &col);
    s.ws.update(cx, |w, cx| w.new_scratch_request(cx));
    cx.run_until_parked();
    frame(cx, s.win);
    cx.update_window(s.win, |_, w, cx| {
        w.within("title-bar").click("collection-switcher", cx)
    })
    .unwrap();
    frame(cx, s.win);
    let rows = |cx: &mut TestAppContext, ix: usize| {
        cx.update_window(s.win, |_, w, _| w.try_find(("switch-open", ix)).is_some())
            .unwrap()
    };
    assert!(rows(cx, 0), "the user's collection is listed");
    assert!(!rows(cx, 1), "the scratch is not");
}

#[gpui_kit::test]
fn collection_actions_skip_an_active_scratch_tab(cx: &mut TestAppContext) {
    let col = collection();
    let s = started(cx, &col);
    s.ws.update(cx, |w, cx| w.new_scratch_request(cx));
    cx.run_until_parked();
    let (root, scratch) = s.ws.read_with(cx, |w, _| {
        (w.current_root().unwrap(), w.current_user_root())
    });
    assert!(s.ws.read_with(cx, |w, _| w.is_scratch(&root)));
    assert_eq!(scratch, None);
}

#[gpui_kit::test]
fn the_update_dialog_does_not_offer_the_scratch(cx: &mut TestAppContext) {
    let r = rig(cx);
    let ws = r.shell.read_with(cx, |s, _| s.workspace().clone());
    let dir = Paths::in_dir(r._files.path()).scratch;
    ws.update(cx, |w, cx| w.set_scratch_dir(dir, cx));
    r.open_v1(cx);
    ws.update(cx, |w, cx| w.new_scratch_request(cx));
    cx.run_until_parked();
    let server = crate::import_update::serve(cx, crate::import_update::V2);
    let d = r.dialog(cx, &format!("{}/spec.json", server.uri()));
    assert_eq!(d.read_with(cx, |d, _| d.targets().len()), 1);
}

#[gpui_kit::test]
fn the_status_bar_ignores_a_watch_failure_of_an_unlisted_scratch(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let col = collection();
    let s = started(cx, &col);
    let scratch = Paths::in_dir(s.files.path()).scratch;
    s.ws.update(cx, |w, cx| {
        w.set_watch_factory(Some(failing_on(scratch.clone())), cx)
    });
    s.ws.update(cx, |w, cx| w.open_scratch(cx));
    wait_for(cx, "the failure", |cx| {
        s.ws.read_with(cx, |w, _| w.watch_failed(&scratch).is_some())
    });
    frame(cx, s.win);
    let shown = cx
        .update_window(s.win, |_, w, _| w.try_find(("unwatched", 1usize)).is_some())
        .unwrap();
    assert!(!shown, "an unlisted scratch has no marker");
}
