//! Tab content in a window shorter than its minimum stays reachable, with the console dock
//! open: settings scroll in the tab view, the runner keeps Run in view.

use crate::support;

use apinata_app::app::AppState;
use apinata_app::settings::Settings;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, Bounds, Pixels, ScrollDelta, TestAppContext, point, px, size};
use support::*;

fn bounds(cx: &mut TestAppContext, s: &Started, id: &str) -> Bounds<Pixels> {
    let id = id.to_string();
    cx.update_window(s.win, |_, w, cx| {
        w.render_frame(cx);
        w.find(id).bounds()
    })
    .unwrap()
}

fn shrink(cx: &mut TestAppContext, s: &Started, w: f32, h: f32, zoom: f32) {
    gpui_kit::VisualTestContext::from_window(s.win, cx).simulate_resize(size(px(w), px(h)));
    cx.update(|cx| {
        let p = cx.global::<AppState>().settings.clone();
        p.update(cx, |p, cx| p.update(cx, |s: &mut Settings| s.zoom = zoom));
    });
    frame(cx, s.win);
    frame(cx, s.win);
}

// Scrolls the tab view down just far enough to bring the bottom of `id` into its box.
fn reveal(cx: &mut TestAppContext, s: &Started, id: &'static str) {
    let outer = bounds(cx, s, "tab-view-scroll");
    let inner = bounds(cx, s, id);
    let down = inner.bottom() - outer.bottom();
    if down > px(0.) {
        cx.update_window(s.win, |_, w, cx| {
            w.scroll(
                "tab-view-scroll",
                ScrollDelta::Pixels(point(px(0.), -down)),
                cx,
            )
        })
        .unwrap();
        frame(cx, s.win);
    }
}

// In a window below a view's minimum (1000x300 with the dock open leaves the tab view 120 px)
// the tab view scrolls, the view keeps a usable height, and its bottom is reachable.
#[gpui_kit::test]
fn settings_tabs_scroll_in_a_window_below_their_minimum(cx: &mut TestAppContext) {
    use apinata_app::tabs::TabKind;
    let col = two_requests();
    let s = started(cx, &col);
    press_in_shell(cx, &s, "ctrl-`");
    shrink(cx, &s, 1000., 300., 1.0);
    let root = s.root.clone();
    let (kind, bottom) = (TabKind::Collection, "config-body");
    s.ws.update(cx, |w, cx| w.open_settings(&root, kind.clone(), cx));
    frame(cx, s.win);
    frame(cx, s.win);
    let view = bounds(cx, &s, "tab-view-scroll");
    let status = bounds(cx, &s, "status-bar");
    assert!(
        view.bottom() <= status.top(),
        "{kind:?}: {view:?} over {status:?}"
    );
    reveal(cx, &s, bottom);
    let last = bounds(cx, &s, bottom);
    assert!(
        last.bottom() <= view.bottom() + px(0.5),
        "{kind:?}: {bottom} {last:?} below {view:?}"
    );
    assert!(
        last.size.height >= px(60.),
        "{kind:?}: body {last:?} too short to use"
    );
}

// The runner scrolls inside itself, never as a whole, so Run stays in the tab view in a 120 px
// view and, in a view of about 230 px, the plan ends above the status bar too.
#[gpui_kit::test]
fn the_runner_keeps_run_in_a_short_tab_view(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    press_in_shell(cx, &s, "ctrl-`");
    let root = s.root.clone();
    s.ws.update(cx, |w, cx| {
        w.open_runner(&root, apinata_core::ItemPath::root(), cx)
    });
    for height in [300., 410.] {
        shrink(cx, &s, 1000., height, 1.0);
        let strip = bounds(cx, &s, "tab-strip");
        let status = bounds(cx, &s, "status-bar");
        let run = bounds(cx, &s, "run-start");
        assert!(
            run.top() >= strip.bottom() - px(0.5) && run.bottom() <= status.top() + px(0.5),
            "{height}: Run {run:?} outside the tab view between {strip:?} and {status:?}"
        );
        cx.update_window(s.win, |_, w, cx| {
            w.scroll("runner", ScrollDelta::Pixels(point(px(0.), px(-200.))), cx)
        })
        .unwrap();
        frame(cx, s.win);
        assert_eq!(
            bounds(cx, &s, "run-start"),
            run,
            "{height}: Run moved under a wheel over the runner"
        );
        if height > 300. {
            let plan = bounds(cx, &s, "run-plan");
            assert!(
                plan.top() >= run.bottom() && plan.bottom() <= status.top() + px(0.5),
                "{height}: plan {plan:?} outside the tab view above {status:?}"
            );
            let view = status.top() - strip.bottom();
            assert!(view >= px(200.), "{height}: tab view only {view:?}");
        }
    }
}
