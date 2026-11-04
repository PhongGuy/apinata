//! The sidebar steps aside in a narrow window and comes back when the window widens, unless
//! the user has taken charge of it; ToggleSidebar is on secondary-b.

use crate::support;

use apinata_app::app::AppState;
use gpui_kit::{TestAppContext, VisualTestContext, px, size};
use support::*;

fn width(cx: &mut TestAppContext, s: &Started, w: f32) {
    VisualTestContext::from_window(s.win, cx).simulate_resize(size(px(w), px(800.)));
    cx.run_until_parked();
    frame(cx, s.win);
}

fn zoom(cx: &mut TestAppContext, s: &Started, z: f32) {
    cx.update(|cx| {
        let p = cx.global::<AppState>().settings.clone();
        p.update(cx, |p, cx| p.update(cx, |st| st.zoom = z));
    });
    cx.run_until_parked();
    frame(cx, s.win);
}

fn visible(cx: &mut TestAppContext, s: &Started) -> bool {
    s.shell.read_with(cx, |sh, _| sh.sidebar_visible())
}

#[gpui_kit::test]
fn a_narrow_window_hides_the_sidebar_and_widening_brings_it_back(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    width(cx, &s, 1000.);
    assert!(visible(cx, &s));
    width(cx, &s, 700.);
    assert!(!visible(cx, &s), "700 px is below 720 ui px at 100%");
    assert!(!present(cx, s.win, "sidebar"));
    width(cx, &s, 1000.);
    assert!(visible(cx, &s), "an auto-hidden sidebar comes back");
}

// At 200% a 1280 px window is 640 ui px wide, so the sidebar steps aside.
#[gpui_kit::test]
fn zooming_in_counts_toward_narrow(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    width(cx, &s, 1280.);
    assert!(visible(cx, &s));
    zoom(cx, &s, 2.0);
    assert!(!visible(cx, &s));
    zoom(cx, &s, 1.0);
    assert!(visible(cx, &s));
}

// A zoom crossing with the window size unchanged follows the resize rules: a manual show
// while narrow sticks, and a manual hide while wide survives a zoom round trip.
#[gpui_kit::test]
fn a_zoom_crossing_behaves_like_a_resize(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    width(cx, &s, 1280.);
    zoom(cx, &s, 2.0);
    assert!(!visible(cx, &s));
    assert!(!present(cx, s.win, "sidebar"));
    press_in_shell(cx, &s, &key("b"));
    frame(cx, s.win);
    assert!(visible(cx, &s));
    zoom(cx, &s, 1.9);
    assert!(visible(cx, &s), "a zoom that stays narrow leaves it shown");
    zoom(cx, &s, 1.0);
    press_in_shell(cx, &s, &key("b"));
    assert!(!visible(cx, &s));
    zoom(cx, &s, 2.0);
    zoom(cx, &s, 1.0);
    assert!(
        !visible(cx, &s),
        "a manual hide is not undone by zooming back out"
    );
}

// Showing it by hand while narrow keeps it; it hides again only on the next crossing.
#[gpui_kit::test]
fn a_manual_show_while_narrow_sticks(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    width(cx, &s, 700.);
    assert!(!visible(cx, &s));
    press_in_shell(cx, &s, &key("b"));
    frame(cx, s.win);
    assert!(visible(cx, &s));
    width(cx, &s, 680.);
    assert!(
        visible(cx, &s),
        "a resize that stays narrow leaves it shown"
    );
    width(cx, &s, 1000.);
    assert!(visible(cx, &s));
    width(cx, &s, 700.);
    assert!(!visible(cx, &s), "crossing below again hides it");
}

// A sidebar the user hid while wide stays hidden through a narrow and back.
#[gpui_kit::test]
fn a_manual_hide_is_not_undone_by_widening(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    width(cx, &s, 1000.);
    press_in_shell(cx, &s, &key("b"));
    assert!(!visible(cx, &s));
    width(cx, &s, 700.);
    width(cx, &s, 1000.);
    assert!(!visible(cx, &s));
}

#[gpui_kit::test]
fn secondary_b_toggles_the_sidebar(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    width(cx, &s, 1000.);
    press_in_shell(cx, &s, &key("b"));
    assert!(!visible(cx, &s));
    press_in_shell(cx, &s, &key("b"));
    assert!(visible(cx, &s));
}

// A sidebar dragged wider comes back from an auto-hide at the width it was dragged to.
#[gpui_kit::test]
fn an_auto_hide_keeps_the_dragged_width(cx: &mut TestAppContext) {
    use gpui_kit::AppContext as _;
    use gpui_kit::test::TestWindowExt as _;
    let col = two_requests();
    let s = started(cx, &col);
    width(cx, &s, 1000.);
    let split = s.shell.read_with(cx, |sh, _| sh.split().clone());
    cx.update_window(s.win, |_, w, cx| {
        split.update(cx, |sp, cx| sp.resize_panel(0, px(320.), w, cx))
    })
    .unwrap();
    frame(cx, s.win);
    width(cx, &s, 700.);
    assert!(!visible(cx, &s));
    width(cx, &s, 1000.);
    assert!(visible(cx, &s));
    let sidebar = cx
        .update_window(s.win, |_, w, _| w.find("sidebar").bounds())
        .unwrap();
    assert!(
        (sidebar.size.width - px(320.)).abs() < px(1.),
        "{sidebar:?} lost the dragged width"
    );
}

// The frame that first sees a narrow window already leaves the sidebar out, so a window
// that opens narrow never flashes it.
#[gpui_kit::test]
fn the_first_narrow_frame_paints_without_the_sidebar(cx: &mut TestAppContext) {
    use gpui_kit::AppContext as _;
    use gpui_kit::test::TestWindowExt as _;
    let col = two_requests();
    let s = started(cx, &col);
    width(cx, &s, 1280.);
    let mut zoomed = cx.update(|cx| cx.global::<AppState>().settings.read(cx).get().clone());
    zoomed.zoom = 2.0;
    // The zoom is applied and drawn inside one update, so no deferred work can run between
    // the frame that crosses the threshold and the look at what it painted.
    let painted = cx
        .update_window(s.win, |_, w, cx| {
            apinata_app::theme::apply(&zoomed, w, cx);
            w.render_frame(cx);
            w.try_find("sidebar").is_some()
        })
        .unwrap();
    assert!(
        !painted,
        "the sidebar painted for a frame below the threshold"
    );
}

fn filter_focused(cx: &mut TestAppContext, s: &Started) -> bool {
    use gpui_kit::{AppContext as _, Focusable as _};
    let filter = s
        .shell
        .read_with(cx, |sh, cx| sh.sidebar().read(cx).filter().clone());
    cx.update_window(s.win, |_, w, cx| filter.focus_handle(cx).is_focused(w))
        .unwrap()
}

fn click_toggle(cx: &mut TestAppContext, s: &Started) {
    use gpui_kit::AppContext as _;
    use gpui_kit::test::TestWindowExt as _;
    cx.update_window(s.win, |_, w, cx| {
        w.within("status-bar").click("toggle-sidebar", cx)
    })
    .unwrap();
    frame(cx, s.win);
}

// Filter requests aimed at a sidebar the narrow window hid brings it back first, so the
// focus lands on a drawn input.
#[gpui_kit::test]
fn filter_requests_reveals_an_auto_hidden_sidebar(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    width(cx, &s, 700.);
    assert!(!visible(cx, &s));
    press_in_shell(cx, &s, &key("f"));
    frame(cx, s.win);
    assert!(visible(cx, &s), "Filter requests left the sidebar hidden");
    assert!(filter_focused(cx, &s));
}

// One click on the status-bar toggle shows a sidebar the narrow window hid, also after
// Filter requests, and the next click hides it again.
#[gpui_kit::test]
fn one_toggle_click_shows_an_auto_hidden_sidebar(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    width(cx, &s, 700.);
    click_toggle(cx, &s);
    assert!(visible(cx, &s), "the first click was swallowed");
    click_toggle(cx, &s);
    assert!(!visible(cx, &s));
    press_in_shell(cx, &s, &key("f"));
    frame(cx, s.win);
    click_toggle(cx, &s);
    assert!(
        !visible(cx, &s),
        "the click after Filter requests was swallowed"
    );
    click_toggle(cx, &s);
    assert!(visible(cx, &s));
}

fn sidebar_width(cx: &mut TestAppContext, s: &Started) -> gpui_kit::Pixels {
    use gpui_kit::AppContext as _;
    use gpui_kit::test::TestWindowExt as _;
    cx.update_window(s.win, |_, w, cx| {
        w.render_frame(cx);
        w.find("sidebar").bounds().size.width
    })
    .unwrap()
}

// At 700 px the cap is 280 px, so every way back shows the preferred 240 px.
fn reveals_at_240(
    cx: &mut TestAppContext,
    how: &str,
    reveal: impl FnOnce(&mut TestAppContext, &Started),
) {
    let col = two_requests();
    let s = started(cx, &col);
    width(cx, &s, 700.);
    assert!(!visible(cx, &s));
    reveal(cx, &s);
    frame(cx, s.win);
    assert!(visible(cx, &s), "{how}");
    let shown = sidebar_width(cx, &s);
    assert!((shown - px(240.)).abs() < px(1.), "{how}: {shown:?}");
}

#[gpui_kit::test]
fn the_toggle_reveals_at_the_preferred_width(cx: &mut TestAppContext) {
    reveals_at_240(cx, "toggle", click_toggle);
}

#[gpui_kit::test]
fn secondary_f_reveals_at_the_preferred_width(cx: &mut TestAppContext) {
    reveals_at_240(cx, "Ctrl+F", |cx, s| press_in_shell(cx, s, &key("f")));
}

#[gpui_kit::test]
fn the_filter_requests_button_reveals_at_the_preferred_width(cx: &mut TestAppContext) {
    reveals_at_240(cx, "Filter requests", |cx, s| {
        use gpui_kit::AppContext as _;
        use gpui_kit::test::TestWindowExt as _;
        cx.update_window(s.win, |_, w, cx| {
            w.within("tab-empty").click(("empty-action", 1usize), cx)
        })
        .unwrap();
    });
}
