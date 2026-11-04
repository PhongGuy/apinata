//! The status bar toggles the sidebar and the response layout, at the designed height.

use crate::support;

use apinata_app::app::AppState;
use apinata_app::settings::Orientation;
use apinata_app::style::{STATUS_BAR_HEIGHT, at_zoom};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, TestAppContext};
use support::*;

fn orientation(cx: &mut TestAppContext) -> Orientation {
    cx.update(|cx| {
        cx.global::<AppState>()
            .settings
            .read(cx)
            .get()
            .response_orientation
    })
}

fn click(cx: &mut TestAppContext, s: &Started, id: &'static str) {
    cx.update_window(s.win, |_, w, cx| w.within("status-bar").click(id, cx))
        .unwrap();
    frame(cx, s.win);
}

#[gpui_kit::test]
fn the_status_bar_toggles_the_sidebar_and_the_layout(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    click(cx, &s, "toggle-sidebar");
    assert!(!s.shell.read_with(cx, |v, _| v.sidebar_visible()));
    click(cx, &s, "toggle-sidebar");
    assert!(s.shell.read_with(cx, |v, _| v.sidebar_visible()));
    assert_eq!(orientation(cx), Orientation::Vertical);
    click(cx, &s, "toggle-layout");
    assert_eq!(orientation(cx), Orientation::Horizontal);
}

#[gpui_kit::test]
fn the_status_bar_has_the_spec_height(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    let height = cx
        .update_window(s.win, |_, w, _| w.find("status-bar").bounds().size.height)
        .unwrap();
    assert_eq!(height, at_zoom(STATUS_BAR_HEIGHT, 1.0));
}

#[gpui_kit::test]
fn the_status_bar_buttons_fit_inside_it(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    cx.update_window(s.win, |_, w, cx| {
        w.render_frame(cx);
        let bar = w.find("status-bar").bounds();
        for id in ["toggle-sidebar", "toggle-layout"] {
            let b = w.within("status-bar").find(id).bounds();
            assert!(
                b.top() >= bar.top() && b.bottom() <= bar.bottom(),
                "{id}: {b:?} in {bar:?}"
            );
        }
    })
    .unwrap();
}
