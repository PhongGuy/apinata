//! Rows, the method column and the status bar follow the app zoom.

use crate::support;

use apinata_app::app::AppState;
use apinata_app::settings::Settings;
use apinata_app::sidebar::marker_id;
use apinata_app::style::{
    METHOD_COLUMN, ROW_HEIGHT, SPACE_3, STATUS_BAR_HEIGHT, at_zoom, kit_size, ui,
};
use gpui_kit::component::Size;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, Bounds, ElementId, Pixels, TestAppContext, px, rems};
use support::*;

const ZOOMS: [f32; 4] = [0.5, 1.0, 1.5, 2.0];

fn set(cx: &mut TestAppContext, f: impl FnOnce(&mut Settings)) {
    cx.update(|cx| {
        let s = cx.global::<AppState>().settings.clone();
        s.update(cx, |p, cx| p.update(cx, f));
    });
    cx.run_until_parked();
}

fn bounds(cx: &mut TestAppContext, s: &Started, id: impl Into<ElementId>) -> Bounds<Pixels> {
    frame(cx, s.win);
    let id = id.into();
    cx.update_window(s.win, |_, w, _| w.find(id).bounds())
        .unwrap()
}

fn near(a: Pixels, b: Pixels) -> bool {
    (f32::from(a) - f32::from(b)).abs() < 0.5
}

#[test]
fn sizes_are_their_px_value_at_100_percent_and_scale_with_zoom() {
    assert_eq!(ui(13.), rems(1.));
    assert_eq!(at_zoom(ROW_HEIGHT, 1.0), px(28.));
    assert_eq!(at_zoom(ROW_HEIGHT, 2.0), px(56.));
    assert_eq!(at_zoom(METHOD_COLUMN, 0.5), px(16.));
    assert_eq!(at_zoom(ROW_HEIGHT, 9.0), px(56.), "zoom is clamped to 200%");
}

#[gpui_kit::test]
fn sidebar_rows_and_the_method_column_scale_with_zoom(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    for z in ZOOMS {
        set(cx, |p| p.zoom = z);
        let row_b = bounds(cx, &s, row("a.yml"));
        let method = bounds(cx, &s, marker_id("method", &row("a.yml")));
        assert!(
            near(row_b.size.height, at_zoom(ROW_HEIGHT, z)),
            "{z}: {row_b:?}"
        );
        assert!(
            near(method.size.width, at_zoom(METHOD_COLUMN, z)),
            "{z}: {method:?}"
        );
        // One line of text: the tag never wraps below the row or spills above it.
        assert!(
            method.top() >= row_b.top() && method.bottom() <= row_b.bottom(),
            "{z}: {method:?} in {row_b:?}"
        );
    }
}

#[gpui_kit::test]
fn kv_rows_and_the_status_bar_scale_with_zoom(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    open_tab(cx, &s.ws, col.path(), "a.yml");
    for z in ZOOMS {
        set(cx, |p| p.zoom = z);
        let kv = bounds(cx, &s, ("kv-row", 0usize));
        let status = bounds(cx, &s, "status-bar");
        assert!(near(kv.size.height, at_zoom(ROW_HEIGHT, z)), "{z}: {kv:?}");
        assert!(
            near(status.size.height, at_zoom(STATUS_BAR_HEIGHT, z)),
            "{z}: {status:?}"
        );
    }
}

#[gpui_kit::test]
fn the_palette_list_grows_with_zoom(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    let height = |cx: &mut TestAppContext, z: f32| {
        set(cx, |p| p.zoom = z);
        cx.update_window(s.win, |_, w, cx| {
            let fallback = w.focused(cx).unwrap_or_else(|| cx.focus_handle());
            apinata_app::palette::open(fallback, w, cx);
        })
        .unwrap();
        let h = bounds(cx, &s, "command-palette").size.height;
        cx.update_window(s.win, |_, w, cx| {
            use gpui_kit::component::WindowExt as _;
            w.close_all_dialogs(cx);
        })
        .unwrap();
        h
    };
    let one = height(cx, 1.0);
    let two = height(cx, 2.0);
    let ratio = f32::from(two) / f32::from(one);
    assert!((1.8..=2.2).contains(&ratio), "{one:?} -> {two:?}");
}

#[gpui_kit::test]
fn the_toast_margin_clears_the_zoomed_status_bar(cx: &mut TestAppContext) {
    let col = two_requests();
    let _s = started(cx, &col);
    set(cx, |p| p.zoom = 2.0);
    let bottom = cx.update(|cx| {
        gpui_kit::component::Theme::global(cx)
            .notification
            .margins
            .bottom
    });
    assert_eq!(
        bottom,
        at_zoom(STATUS_BAR_HEIGHT, 2.0) + at_zoom(SPACE_3, 2.0)
    );
}

// The kit clips a tab's content to an inner box of fixed px per Size; these are the
// underline variant's heights (gpui-component-0.9.0/src/tab/tab.rs, `inner_height`).
fn underline_inner_height(size: Size) -> Pixels {
    match size {
        Size::XSmall => px(20.),
        Size::Small => px(22.),
        Size::Large => px(32.),
        _ => px(26.),
    }
}

#[gpui_kit::test]
fn section_tab_labels_fit_their_strip_at_every_zoom(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    open_tab(cx, &s.ws, col.path(), "a.yml");
    let mut heights = Vec::new();
    for z in ZOOMS {
        set(cx, |p| p.zoom = z);
        let strip = bounds(cx, &s, "section-strip");
        for ix in 0..3usize {
            let label = bounds(cx, &s, ("section", ix));
            assert!(
                label.top() >= strip.top() && label.bottom() <= strip.bottom(),
                "{z}: label {ix} {label:?} outside {strip:?}"
            );
            let inner = underline_inner_height(kit_size(z));
            assert!(
                label.size.height <= inner,
                "{z}: label {ix} {label:?} taller than the clipped inner box {inner:?}"
            );
        }
        heights.push(strip.size.height);
    }
    // Four zooms share three kit sizes, so the strip only steps; 100% keeps the default look.
    assert!(
        heights.windows(2).all(|p| p[0] <= p[1]),
        "strip never shrinks with zoom: {heights:?}"
    );
    assert!(heights[3] > heights[1], "strip grows by 200%: {heights:?}");
    assert_eq!(heights[1], px(36.), "100% keeps the default strip");
}

#[test]
fn every_tab_bar_keeps_the_default_kit_size_at_100_percent() {
    assert_eq!(kit_size(1.0), Size::default());
}

#[gpui_kit::test]
fn the_default_sidebar_width_follows_the_zoom(cx: &mut TestAppContext) {
    let col = two_requests();
    let s = started(cx, &col);
    let one = bounds(cx, &s, "sidebar").size.width;
    set(cx, |p| p.zoom = 2.0);
    let two = bounds(cx, &s, "sidebar").size.width;
    let ratio = f32::from(two) / f32::from(one);
    assert!((1.9..=2.1).contains(&ratio), "{one:?} -> {two:?}");
}

// Whether tab `ix` lies wholly in the tab strip's viewport, after the two frames a reveal takes.
fn tab_inside(cx: &mut TestAppContext, s: &Started, ix: usize) -> bool {
    cx.run_until_parked();
    cx.update_window(s.win, |_, w, cx| {
        w.render_frame(cx);
        w.render_frame(cx);
        let tab = w.find(("tab", ix)).bounds();
        let left = w.find("tab-inset").bounds().right();
        let right = w.find("tab-strip-viewport").bounds().right();
        tab.left() >= left - px(0.5) && tab.right() <= right + px(0.5)
    })
    .unwrap()
}

#[gpui_kit::test]
fn the_active_tab_stays_visible_at_200(cx: &mut TestAppContext) {
    let names: Vec<(String, String)> = (0..10)
        .map(|i| {
            (
                format!("r{i}.yml"),
                request(&format!("Request {i}"), "http://x/"),
            )
        })
        .collect();
    let mut files: Vec<(&str, &str)> = vec![("opencollection.yml", ROOT)];
    files.extend(names.iter().map(|(p, t)| (p.as_str(), t.as_str())));
    let col = write_tree(&files);
    let s = started(cx, &col);
    for i in 0..9 {
        open_tab(cx, &s.ws, col.path(), &format!("r{i}.yml"));
    }
    assert!(tab_inside(cx, &s, 8));
    set(cx, |p| p.zoom = 2.0);
    assert!(
        tab_inside(cx, &s, 8),
        "zooming keeps the active tab in view"
    );
    cx.update_window(s.win, |_, w, cx| w.click(row("r9.yml"), cx))
        .unwrap();
    assert!(tab_inside(cx, &s, 9), "a request opened at 200% is in view");
}
