//! A kept response's age grows while it is shown, but the status row keeps its height, so
//! the tabs' content below it never moves as time passes.

use crate::support;

use std::sync::Arc;
use std::time::Duration;

use apinata_app::app::AppState;
use apinata_app::last_response::wall_now;
use apinata_app::request_editor::RequestEditor;
use apinata_app::send_model::Restored;
use apinata_app::settings::Orientation;
use apinata_app::style::{RESPONSE_COMPACT_BELOW, at_zoom};
use apinata_core::{Outcome, RequestResult, Response};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, Bounds, Entity, Pixels, TestAppContext, px, size};
use support::*;

// A 1280x800 window showing a response kept five seconds ago, at `zoom`, laid out `layout`.
fn showing_kept(
    cx: &mut TestAppContext,
    zoom: f32,
    layout: Orientation,
) -> (tempfile::TempDir, Started, Entity<RequestEditor>) {
    let col = two_requests();
    let s = started(cx, &col);
    let tab = open_tab(cx, &s.ws, &s.root, "a.yml");
    let editor = editor_of(cx, &s, tab);
    cx.simulate_window_resize(s.win, size(px(1280.), px(800.)));
    cx.update(|cx| {
        let p = cx.global::<AppState>().settings.clone();
        p.update(cx, |p, cx| {
            p.update(cx, |st| {
                st.response_orientation = layout;
                st.zoom = zoom;
            })
        });
    });
    let result = RequestResult::from_parts(
        ip("a.yml"),
        "a",
        Outcome::Passed,
        Some(
            Response::new(200)
                .with_header("content-type", "application/json")
                .with_text_body("{\"a\":1}"),
        ),
        Vec::new(),
    );
    let restored = Restored {
        saved_at: cx.update(|cx| wall_now(cx)) - Duration::from_secs(5),
        body_dropped: false,
        body_masked: false,
        masked_headers: Vec::new(),
        header_digests: None,
    };
    let model = editor.read_with(cx, |e, _| e.send_model().clone());
    model.update(cx, |m, cx| m.restore(Arc::new(result), restored, cx));
    cx.run_until_parked();
    frame(cx, s.win);
    (col, s, editor)
}

// Drags the side-by-side response pane to its narrowest.
fn narrowest(cx: &mut TestAppContext, s: &Started, editor: &Entity<RequestEditor>) {
    let split = editor.read_with(cx, |e, _| e.split().clone());
    cx.update_window(s.win, |_, w, cx| {
        split.update(cx, |sp, cx| sp.resize_panel(0, px(5000.), w, cx))
    })
    .unwrap();
    for _ in 0..3 {
        frame(cx, s.win);
    }
}

fn bounds(cx: &mut TestAppContext, s: &Started, id: &'static str) -> Bounds<Pixels> {
    cx.update_window(s.win, |_, w, cx| {
        w.render_frame(cx);
        w.find(id).bounds()
    })
    .unwrap()
}

// Whether the pane is under the width where the status row drops the saved date.
fn compact(cx: &mut TestAppContext, s: &Started, zoom: f32) -> bool {
    bounds(cx, s, "response").size.width < at_zoom(RESPONSE_COMPACT_BELOW, zoom)
}

// Whether the status facts sit on the tab strip's row rather than a line of their own.
fn facts_beside_tabs(cx: &mut TestAppContext, s: &Started) -> bool {
    let (saved, strip) = (
        bounds(cx, s, "response-saved"),
        bounds(cx, s, "response-tab-strip"),
    );
    saved.top() >= strip.top() && saved.bottom() <= strip.bottom()
}

// The response header's height (tab strip top to the lowest status fact) and the top of the
// body toolbar under it.
fn header_and_content(cx: &mut TestAppContext, s: &Started) -> (Pixels, Pixels) {
    let top = bounds(cx, s, "response-tab-strip").top();
    let bottom = ["response-tab-strip", "response-status", "response-saved"]
        .into_iter()
        .map(|id| bounds(cx, s, id).bottom())
        .fold(top, Pixels::max);
    (bottom - top, bounds(cx, s, "body-filter").top())
}

// Runs the clock past a minute, ten minutes and hours; the header never moves.
fn assert_steady(cx: &mut TestAppContext, s: &Started, leg: &str) {
    let start = header_and_content(cx, s);
    for (step, label) in [
        (60, "1 min"),
        (540, "10 min"),
        (3_000, "1 h"),
        (43_200, "13 h"),
    ] {
        cx.executor().advance_clock(Duration::from_secs(step));
        cx.run_until_parked();
        frame(cx, s.win);
        let now = header_and_content(cx, s);
        assert_eq!(now, start, "{leg} after {label}: (header, content y)");
    }
}

#[gpui_kit::test]
fn the_narrowest_pane_keeps_its_status_row_as_the_age_grows(cx: &mut TestAppContext) {
    let (_col, s, editor) = showing_kept(cx, 1.0, Orientation::Horizontal);
    narrowest(cx, &s, &editor);
    assert!(compact(cx, &s, 1.0), "the narrowest pane is compact");
    assert!(
        !present(cx, s.win, "response-saved-date"),
        "no date when compact"
    );
    assert_steady(cx, &s, "narrowest");
}

// At 2x the window is too narrow for side by side, so the pane stacks at full width. The
// header never wraps, and its facts do not fit beside the tabs at 2x, so the pane folds to
// its compact form: the facts on a row of their own under the tabs.
#[gpui_kit::test]
fn a_zoomed_pane_keeps_its_status_row_as_the_age_grows(cx: &mut TestAppContext) {
    let (_col, s, editor) = showing_kept(cx, 2.0, Orientation::Horizontal);
    narrowest(cx, &s, &editor);
    assert!(!compact(cx, &s, 2.0), "at 2x the pane stacks at full width");
    assert!(
        present(cx, s.win, "response-more"),
        "a header too long for one row folds rather than wraps"
    );
    assert!(
        !facts_beside_tabs(cx, &s),
        "at 2x the facts take a row of their own"
    );
    assert_steady(cx, &s, "2x");
}

// A stacked pane at the narrowest window that still fits "From <date> <age>" beside the
// tabs, so any growth of the age would push the facts onto a line of their own.
#[gpui_kit::test]
fn a_wide_pane_near_the_wrap_keeps_its_status_row_as_the_age_grows(cx: &mut TestAppContext) {
    let (_col, s, _editor) = showing_kept(cx, 1.0, Orientation::Vertical);
    let fits = |cx: &mut TestAppContext, width: f32| {
        cx.simulate_window_resize(s.win, size(px(width), px(800.)));
        frame(cx, s.win);
        !compact(cx, &s, 1.0) && facts_beside_tabs(cx, &s)
    };
    let (mut lo, mut hi) = (600., 2400.);
    assert!(fits(cx, hi), "the facts fit beside the tabs at {hi}px");
    assert!(!fits(cx, lo), "the facts leave the tab row at {lo}px");
    while hi - lo > 1. {
        let mid = ((lo + hi) / 2.0_f32).round();
        if fits(cx, mid) { hi = mid } else { lo = mid }
    }
    assert!(fits(cx, hi), "the facts fit beside the tabs at {hi}px");
    assert_steady(cx, &s, &format!("wide at {hi}px"));
}
