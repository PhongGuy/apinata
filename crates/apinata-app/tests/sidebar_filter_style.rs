//! The filter shows its key hint and still filters.

use crate::support;

use gpui_kit::AppContext as _;
use gpui_kit::TestAppContext;
use gpui_kit::test::TestWindowExt;
use support::*;

#[gpui_kit::test]
fn the_filter_shows_its_key_hint(cx: &mut TestAppContext) {
    let h = open_sidebar(cx);
    assert!(shown(cx, h.win, "sidebar-filter-kbd"));
    let (filter, hint) = cx
        .update_window(h.win, |_, w, _| {
            (
                w.find("sidebar-filter").bounds(),
                w.find("sidebar-filter-kbd").bounds(),
            )
        })
        .unwrap();
    assert!(
        filter.contains(&hint.center()),
        "the hint sits inside the filter"
    );
}

#[gpui_kit::test]
fn typing_after_the_restyle_still_filters(cx: &mut TestAppContext) {
    let h = open_sidebar(cx);
    let filter = h.view.read_with(cx, |s, _| s.filter().clone());
    cx.update_window(h.win, |_, w, cx| filter.update(cx, |i, cx| i.focus(w, cx)))
        .unwrap();
    cx.update_window(h.win, |_, w, cx| w.input("health", cx))
        .unwrap();
    assert!(shown(cx, h.win, row("health.yml")));
    assert!(!shown(cx, h.win, row("status.yml")));
}
