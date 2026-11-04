//! The sidebar shows at most two fifths of the window; only a drag changes the width kept.

use crate::support;

use apinata_app::app::AppState;
use gpui_kit::component::ResizablePanelEvent;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, Pixels, TestAppContext, VisualTestContext, px, size};
use support::*;

fn room(cx: &mut TestAppContext, s: &Started, w: f32, zoom: f32) {
    VisualTestContext::from_window(s.win, cx).simulate_resize(size(px(w), px(800.)));
    cx.update(|cx| {
        let p = cx.global::<AppState>().settings.clone();
        p.update(cx, |p, cx| p.update(cx, |st| st.zoom = zoom));
    });
    cx.run_until_parked();
    frame(cx, s.win);
    frame(cx, s.win);
}

fn sidebar_width(cx: &mut TestAppContext, s: &Started) -> Pixels {
    cx.update_window(s.win, |_, w, _| w.find("sidebar").bounds().size.width)
        .unwrap()
}

fn saved(cx: &mut TestAppContext, s: &Started) -> Option<f32> {
    s.ws.read_with(cx, |w, cx| w.session().read(cx).get().sidebar_width)
}

fn drag_to(cx: &mut TestAppContext, s: &Started, width: Pixels) {
    let split = s.shell.read_with(cx, |sh, _| sh.split().clone());
    cx.update_window(s.win, |_, w, cx| {
        split.update(cx, |sp, cx| sp.resize_panel(0, width, w, cx))
    })
    .unwrap();
    frame(cx, s.win);
    frame(cx, s.win);
}

#[gpui_kit::test]
fn at_200_percent_in_a_640_px_window_the_sidebar_shows_at_most_256_px(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    room(cx, &s, 640., 2.0);
    // Auto-hidden at this width; shown by hand, as the m10l report did.
    press_in_shell(cx, &s, &key("b"));
    frame(cx, s.win);
    frame(cx, s.win);
    assert!(s.shell.read_with(cx, |sh, _| sh.sidebar_visible()));
    let width = sidebar_width(cx, &s);
    assert!(width <= px(256.5), "{width:?}");
}

#[gpui_kit::test]
fn a_drag_clamps_to_the_cap_and_keeps_the_clamped_width(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    room(cx, &s, 1000., 1.0);
    drag_to(cx, &s, px(700.));
    let width = sidebar_width(cx, &s);
    assert!((width - px(400.)).abs() < px(1.), "{width:?}");
    let kept = saved(cx, &s).expect("a saved width");
    assert!((kept - 400.).abs() < 1., "{kept}");
}

// Hiding the sidebar re-lays the split out; the width a drag chose is neither lost nor
// overwritten by the hidden panel's zero width, and it is the width the reveal shows.
#[gpui_kit::test]
fn a_dragged_width_survives_hiding_and_showing_the_sidebar(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    room(cx, &s, 1000., 1.0);
    drag_to(cx, &s, px(350.));
    assert!((sidebar_width(cx, &s) - px(350.)).abs() < px(1.));
    for step in ["hide", "show"] {
        press_in_shell(cx, &s, &key("b"));
        frame(cx, s.win);
        frame(cx, s.win);
        let visible = s.shell.read_with(cx, |sh, _| sh.sidebar_visible());
        assert_eq!(visible, step == "show", "{step}");
        // A drag handle can end while the panel is hidden; the hidden panel's size is no width.
        let split = s.shell.read_with(cx, |sh, _| sh.split().clone());
        split.update(cx, |_, cx| cx.emit(ResizablePanelEvent::Resized));
        cx.run_until_parked();
        let kept = saved(cx, &s).expect("a saved width");
        assert!((kept - 350.).abs() < 1., "{step}: saved {kept}");
    }
    let width = sidebar_width(cx, &s);
    assert!(
        (width - px(350.)).abs() < px(1.),
        "shown again at {width:?}"
    );
}

#[gpui_kit::test]
fn a_narrower_window_shrinks_only_the_shown_width(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    room(cx, &s, 1000., 1.0);
    drag_to(cx, &s, px(380.));
    let kept = saved(cx, &s).expect("a saved width");
    room(cx, &s, 760., 1.0);
    let narrow = sidebar_width(cx, &s);
    assert!((narrow - px(304.)).abs() < px(1.), "{narrow:?}");
    assert_eq!(saved(cx, &s), Some(kept), "a shrink is not a drag");
    room(cx, &s, 1000., 1.0);
    let wide = sidebar_width(cx, &s);
    assert!(
        (wide - px(kept)).abs() < px(1.),
        "{wide:?} lost the preferred {kept}"
    );
}

// A width saved under an older, smaller minimum shows at today's minimum, the tab area's
// room counts that shown width, and the saved width is kept.
#[gpui_kit::test]
fn a_saved_width_below_the_minimum_shows_at_the_minimum(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    s.ws.update(cx, |w, cx| {
        w.session()
            .update(cx, |p, cx| p.update(cx, |st| st.sidebar_width = Some(100.)))
    });
    room(cx, &s, 1280., 1.0);
    let min = apinata_app::style::sidebar_min_at(1.0);
    let width = sidebar_width(cx, &s);
    assert!((width - min).abs() < px(0.5), "{width:?} vs {min:?}");
    let room = cx.update(|cx| cx.global::<apinata_app::request_editor::TabAreaRoom>().0);
    assert_eq!(room, px(1280.) - min);
    assert_eq!(saved(cx, &s), Some(100.));
}
