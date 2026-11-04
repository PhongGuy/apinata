//! The URL input keeps 16 rem: a side-by-side request pane too narrow for it stacks the split
//! until the room returns, without touching the saved orientation. An empty response stays a
//! strip below the request.

use crate::support;

use apinata_app::app::AppState;
use apinata_app::settings::Orientation;
use apinata_app::style::{
    CUSTOM_METHOD, METHOD_SELECT, REQUEST_BUTTONS_WIDTH, SPACE_2, URL_FIELD_MIN, URL_ROW_SLACK,
    at_zoom,
};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, Bounds, Pixels, TestAppContext, VisualTestContext, px, size};
use support::*;

struct Case {
    s: Started,
    ed: gpui_kit::Entity<apinata_app::request_editor::RequestEditor>,
}

fn open_case(cx: &mut TestAppContext, col: &tempfile::TempDir) -> Case {
    let s = started(cx, col);
    let a = open_tab(cx, &s.ws, &s.root, "a.yml");
    frame(cx, s.win);
    let ed = editor_of(cx, &s, a);
    Case { s, ed }
}

// Sets the window width, the zoom and the saved orientation, then lays out twice.
fn room(cx: &mut TestAppContext, c: &Case, w: f32, zoom: f32, o: Orientation) {
    room_wh(cx, c, (w, 800.), zoom, o);
}

fn room_wh(cx: &mut TestAppContext, c: &Case, wh: (f32, f32), zoom: f32, o: Orientation) {
    VisualTestContext::from_window(c.s.win, cx).simulate_resize(size(px(wh.0), px(wh.1)));
    cx.update(|cx| {
        let p = cx.global::<AppState>().settings.clone();
        p.update(cx, |p, cx| {
            p.update(cx, |st| {
                st.zoom = zoom;
                st.response_orientation = o;
            })
        });
    });
    cx.run_until_parked();
    for _ in 0..3 {
        frame(cx, c.s.win);
    }
}

struct Seen {
    url: Bounds<Pixels>,
    column: Bounds<Pixels>,
    response: Bounds<Pixels>,
}

fn seen(cx: &mut TestAppContext, c: &Case) -> Seen {
    cx.update_window(c.s.win, |_, w, _| Seen {
        url: w.find("url").bounds(),
        column: w.find("request-column").bounds(),
        response: w.find("response").bounds(),
    })
    .unwrap()
}

fn stacked(s: &Seen) -> bool {
    s.response.origin.y >= s.column.bottom() - px(1.)
}

fn saved(cx: &mut TestAppContext) -> Orientation {
    cx.update(|cx| {
        cx.global::<AppState>()
            .settings
            .read(cx)
            .get()
            .response_orientation
    })
}

fn floor(zoom: f32) -> Pixels {
    at_zoom(URL_FIELD_MIN, zoom)
}

#[gpui_kit::test]
fn a_pane_too_narrow_for_the_url_stacks_at_150_percent(cx: &mut TestAppContext) {
    let col = two_requests();
    let c = open_case(cx, &col);
    room(cx, &c, 1280., 1.5, Orientation::Horizontal);
    let v = seen(cx, &c);
    assert!(stacked(&v), "{:?} beside {:?}", v.column, v.response);
    assert!(v.url.size.width >= floor(1.5) - px(0.5), "{:?}", v.url);
}

#[gpui_kit::test]
fn a_pane_too_narrow_for_the_url_stacks_at_200_percent(cx: &mut TestAppContext) {
    let col = two_requests();
    let c = open_case(cx, &col);
    room(cx, &c, 1280., 2.0, Orientation::Horizontal);
    let v = seen(cx, &c);
    assert!(stacked(&v), "{:?} beside {:?}", v.column, v.response);
    assert!(v.url.size.width >= floor(2.0) - px(0.5), "{:?}", v.url);
}

#[gpui_kit::test]
fn a_640_px_window_at_200_percent_keeps_the_url_inside_the_pane(cx: &mut TestAppContext) {
    let col = two_requests();
    let c = open_case(cx, &col);
    room(cx, &c, 640., 2.0, Orientation::Horizontal);
    let v = seen(cx, &c);
    assert!(stacked(&v), "{:?} beside {:?}", v.column, v.response);
    assert!(v.url.left() >= v.column.left() - px(0.5), "{:?}", v.url);
    assert!(
        v.url.right() <= v.column.right() + px(0.5),
        "{:?} in {:?}",
        v.url,
        v.column
    );
    assert!(v.url.size.width > px(0.), "{:?}", v.url);
}

#[gpui_kit::test]
fn a_wide_window_stays_side_by_side_and_the_setting_is_kept(cx: &mut TestAppContext) {
    let col = two_requests();
    let c = open_case(cx, &col);
    room(cx, &c, 1800., 1.0, Orientation::Horizontal);
    let v = seen(cx, &c);
    assert!(!stacked(&v), "{:?} beside {:?}", v.column, v.response);
    assert!(v.url.size.width >= floor(1.0) - px(0.5), "{:?}", v.url);
    room(cx, &c, 1280., 2.0, Orientation::Horizontal);
    assert!(stacked(&seen(cx, &c)));
    assert_eq!(saved(cx), Orientation::Horizontal);
    room(cx, &c, 1800., 1.0, Orientation::Horizontal);
    assert!(!stacked(&seen(cx, &c)), "widening did not restore it");
}

#[gpui_kit::test]
fn the_same_holds_once_a_response_exists(cx: &mut TestAppContext) {
    let col = two_requests();
    let c = open_case(cx, &col);
    give_result(cx, &c.ed);
    room(cx, &c, 1280., 2.0, Orientation::Horizontal);
    let v = seen(cx, &c);
    assert!(stacked(&v), "{:?} beside {:?}", v.column, v.response);
    assert!(v.url.size.width >= floor(2.0) - px(0.5), "{:?}", v.url);
    room(cx, &c, 1800., 1.0, Orientation::Horizontal);
    assert!(!stacked(&seen(cx, &c)));
}

#[gpui_kit::test]
fn an_empty_stacked_response_is_only_its_strip(cx: &mut TestAppContext) {
    let col = two_requests();
    let c = open_case(cx, &col);
    room(cx, &c, 1280., 1.0, Orientation::Vertical);
    let v = seen(cx, &c);
    assert!(stacked(&v));
    // The strip is a header and one line, far under the dragged minimum of a real response.
    assert!(
        v.response.size.height < apinata_app::style::PANE_MIN,
        "{:?}",
        v.response
    );
    let total = v.column.size.height + v.response.size.height;
    assert!(
        v.column.size.height > total * 0.7,
        "{:?} / {:?}",
        v.column,
        v.response
    );
}

#[gpui_kit::test]
fn the_divider_still_drags_when_stacked_with_a_result(cx: &mut TestAppContext) {
    let col = two_requests();
    let c = open_case(cx, &col);
    give_result(cx, &c.ed);
    room(cx, &c, 1280., 2.0, Orientation::Horizontal);
    assert!(stacked(&seen(cx, &c)));
    let split = c.ed.read_with(cx, |e, _| e.split().clone());
    let total = split.read_with(cx, |sp, _| sp.sizes()[0] + sp.sizes()[1]);
    // The pane starts near two thirds of the room at 200%, and its rows hold it above 40%,
    // so the drag goes well past the start to show the divider moves.
    cx.update_window(c.s.win, |_, w, cx| {
        split.update(cx, |sp, cx| sp.resize_panel(0, total * 0.8, w, cx))
    })
    .unwrap();
    frame(cx, c.s.win);
    let ratio = c.s.ws.read_with(cx, |w, _| w.split_ratio());
    assert!((ratio - 0.8).abs() < 0.03, "{ratio}");
}

fn need(zoom: f32) -> Pixels {
    floor(zoom)
        + at_zoom(METHOD_SELECT, zoom)
        + at_zoom(CUSTOM_METHOD, zoom)
        + at_zoom(SPACE_2, zoom)
        + at_zoom(REQUEST_BUTTONS_WIDTH, zoom)
        + at_zoom(URL_ROW_SLACK, zoom)
}

#[gpui_kit::test]
fn the_default_split_keeps_the_url_at_16_rem_at_200_percent(cx: &mut TestAppContext) {
    let col = two_requests();
    let c = open_case(cx, &col);
    give_result(cx, &c.ed);
    room_wh(cx, &c, (2400., 900.), 2.0, Orientation::Horizontal);
    let v = seen(cx, &c);
    // The column is the panel less its divider-side gutter.
    let gutter = at_zoom(apinata_app::style::SPACE_2, 2.0);
    assert!(
        v.column.size.width + gutter >= need(2.0) - px(1.),
        "{:?}",
        v.column
    );
    assert!(!stacked(&v), "{:?} beside {:?}", v.column, v.response);
    assert!(v.url.size.width >= floor(2.0) - px(0.5), "{:?}", v.url);
    let ratio = c.s.ws.read_with(cx, |w, _| w.split_ratio());
    assert!((ratio - 0.5).abs() < 0.001, "the clamp leaked into {ratio}");
}

#[gpui_kit::test]
fn a_drag_cannot_shrink_the_request_pane_below_the_url_row(cx: &mut TestAppContext) {
    let col = two_requests();
    let c = open_case(cx, &col);
    give_result(cx, &c.ed);
    room_wh(cx, &c, (2400., 900.), 2.0, Orientation::Horizontal);
    let split = c.ed.read_with(cx, |e, _| e.split().clone());
    let total = split.read_with(cx, |sp, _| sp.sizes()[0] + sp.sizes()[1]);
    let drag = |cx: &mut TestAppContext, to: Pixels| {
        cx.update_window(c.s.win, |_, w, cx| {
            split.update(cx, |sp, cx| sp.resize_panel(0, to, w, cx))
        })
        .unwrap();
        frame(cx, c.s.win);
    };
    drag(cx, px(150.));
    let v = seen(cx, &c);
    assert!(!stacked(&v));
    assert!(v.url.size.width >= floor(2.0) - px(0.5), "{:?}", v.url);
    let request = split.read_with(cx, |sp, _| sp.sizes()[0]);
    assert!(
        request >= need(2.0) - px(1.),
        "{request:?} < {:?}",
        need(2.0)
    );
    // Toward the response, above the clamp, the divider still moves.
    drag(cx, total * 0.7);
    let request = split.read_with(cx, |sp, _| sp.sizes()[0]);
    assert!(
        (request / total - 0.7).abs() < 0.03,
        "{request:?} of {total:?}"
    );
}

#[gpui_kit::test]
fn the_button_group_fits_the_width_the_row_reserves(cx: &mut TestAppContext) {
    for zoom in [1.0, 2.0] {
        let col = two_requests();
        let c = open_case(cx, &col);
        room_wh(cx, &c, (2400., 900.), zoom, Orientation::Vertical);
        c.s.ws.update(cx, |w, cx| {
            w.send(
                &c.s.root,
                edit_url("a.yml", "http://127.0.0.1:9/changed"),
                cx,
            )
        });
        frame(cx, c.s.win);
        let width = cx
            .update_window(c.s.win, |_, w, _| {
                let save = w.find("save-request").bounds();
                let code = w.find("generate-code").bounds();
                code.right() - save.left()
            })
            .unwrap();
        eprintln!("buttons at {zoom}: {width:?}");
        assert!(
            width <= at_zoom(REQUEST_BUTTONS_WIDTH, zoom),
            "{width:?} > {:?} at {zoom}",
            at_zoom(REQUEST_BUTTONS_WIDTH, zoom)
        );
    }
}

// Edits the URL so Save shows, then reads where Send and the URL field sit.
fn dirty_row(cx: &mut TestAppContext, c: &Case) -> (Bounds<Pixels>, Bounds<Pixels>) {
    c.s.ws.update(cx, |w, cx| {
        w.send(
            &c.s.root,
            edit_url("a.yml", "http://127.0.0.1:9/changed"),
            cx,
        )
    });
    frame(cx, c.s.win);
    row(cx, c)
}

fn row(cx: &mut TestAppContext, c: &Case) -> (Bounds<Pixels>, Bounds<Pixels>) {
    cx.update_window(c.s.win, |_, w, _| {
        (w.find("url").bounds(), w.find("send").bounds())
    })
    .unwrap()
}

// The pane widens to the row's need or the split stacks; either way Send never wraps, and the
// chosen layout holds from frame to frame.
#[gpui_kit::test]
fn side_by_side_at_1280_with_save_shown_keeps_the_url_row_on_one_line(cx: &mut TestAppContext) {
    let col = two_requests();
    let c = open_case(cx, &col);
    give_result(cx, &c.ed);
    room(cx, &c, 1280., 1.0, Orientation::Horizontal);
    dirty_row(cx, &c);
    let first = stacked(&seen(cx, &c));
    for i in 0..4 {
        frame(cx, c.s.win);
        let (url, send) = row(cx, &c);
        assert_eq!(
            send.top(),
            url.top(),
            "frame {i}: Send wrapped below {url:?}"
        );
        let v = seen(cx, &c);
        assert_eq!(
            stacked(&v),
            first,
            "frame {i}: {:?} beside {:?}",
            v.column,
            v.response
        );
    }
    assert_eq!(saved(cx), Orientation::Horizontal);
}

#[gpui_kit::test]
fn side_by_side_with_room_keeps_a_one_line_url_row(cx: &mut TestAppContext) {
    let col = two_requests();
    let c = open_case(cx, &col);
    give_result(cx, &c.ed);
    room(cx, &c, 1800., 1.0, Orientation::Horizontal);
    dirty_row(cx, &c);
    for i in 0..4 {
        frame(cx, c.s.win);
        let (url, send) = row(cx, &c);
        assert_eq!(
            send.top(),
            url.top(),
            "frame {i}: Send wrapped below {url:?}"
        );
        let v = seen(cx, &c);
        assert!(
            !stacked(&v),
            "frame {i}: {:?} beside {:?}",
            v.column,
            v.response
        );
    }
}
