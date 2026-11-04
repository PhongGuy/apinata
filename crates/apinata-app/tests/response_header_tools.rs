//! The Headers and Cookies tabs: a filter field and a Copy button above the table, and a
//! row click that copies the value.

use crate::support;

use std::sync::Arc;

use apinata_app::response_pane::{
    CookieCells, ResponsePane, ResponseTab, cookie_line, cookie_lines, cookie_shown, header_lines,
    header_shown,
};
use apinata_core::{Outcome, RequestResult, Response};
use gpui_kit::component::WindowExt as _;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AnyWindowHandle, AppContext as _, Entity, TestAppContext, WindowOptions, px, size};
use support::*;
use wiremock::ResponseTemplate;

fn pairs() -> Vec<(String, String)> {
    [
        ("Cache-Control", "max-age=60"),
        ("Content-Type", "application/json"),
        ("X-Trace", "CACHE-hit"),
    ]
    .map(|(n, v)| (n.to_owned(), v.to_owned()))
    .to_vec()
}

fn cookie(name: &str, value: &str, domain: &str) -> CookieCells {
    CookieCells {
        name: name.into(),
        value: value.into(),
        domain: domain.into(),
        path: "/".into(),
        expires: "Session".into(),
        http_only: true,
        secure: false,
    }
}

#[test]
fn the_filter_matches_names_and_values_ignoring_case() {
    let all = pairs();
    assert_eq!(header_shown(&all, "").len(), 3);
    let names: Vec<_> = header_shown(&all, "cache")
        .into_iter()
        .map(|(_, p)| p.0.as_str())
        .collect();
    assert_eq!(names, ["Cache-Control", "X-Trace"]);
    assert_eq!(
        header_shown(&all, "JSON")[0].0,
        1,
        "the index is the unfiltered one"
    );
    assert!(header_shown(&all, "nothing").is_empty());
}

#[test]
fn copy_lines_are_the_shown_rows_as_name_colon_value() {
    let all = pairs();
    assert_eq!(
        header_lines(&all, "cache"),
        "Cache-Control: max-age=60\nX-Trace: CACHE-hit"
    );
    assert_eq!(header_lines(&all, "nothing"), "");
}

#[test]
fn a_cookie_line_is_name_value_then_its_scope_and_flags() {
    let c = cookie("sid", "abc", "example.com");
    assert_eq!(
        cookie_line(&c),
        "sid=abc; Domain=example.com; Path=/; HttpOnly"
    );
    let mut secure = cookie("t", "1", "a.test");
    secure.http_only = false;
    secure.secure = true;
    assert_eq!(cookie_line(&secure), "t=1; Domain=a.test; Path=/; Secure");
}

#[test]
fn the_cookie_filter_matches_name_value_and_domain() {
    let all = vec![
        cookie("sid", "abc", "example.com"),
        cookie("t", "XYZ", "a.test"),
    ];
    assert_eq!(cookie_shown(&all, "SID")[0].0, 0);
    assert_eq!(cookie_shown(&all, "xyz")[0].0, 1);
    assert_eq!(cookie_shown(&all, "example")[0].0, 0);
    assert_eq!(cookie_shown(&all, "").len(), 2);
    assert_eq!(
        cookie_lines(&all, "a.test"),
        "t=XYZ; Domain=a.test; Path=/; HttpOnly"
    );
}

fn headers_result() -> Arc<RequestResult> {
    let mut response = Response::new(200);
    for (n, v) in pairs() {
        response = response.with_header(&n, &v);
    }
    Arc::new(RequestResult::from_parts(
        ip("a.yml"),
        "a",
        Outcome::Passed,
        Some(response),
        Vec::new(),
    ))
}

fn pane(
    cx: &mut TestAppContext,
    tab: ResponseTab,
    width: Option<f32>,
) -> (Sent, AnyWindowHandle, Entity<ResponsePane>) {
    let s = send_once(
        cx,
        ResponseTemplate::new(200)
            .append_header("set-cookie", "a=1; Path=/")
            .append_header("set-cookie", "b=2; Path=/"),
        "",
    );
    support::init(cx);
    if tab == ResponseTab::Headers {
        s.model.update(cx, |m, cx| m.show(headers_result(), cx));
    }
    let model = s.model.clone();
    let (win, view) = cx
        .update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |w, cx| {
                cx.new(|cx| ResponsePane::new(model, w, cx))
            })
        })
        .unwrap();
    if let Some(w) = width {
        cx.simulate_window_resize(win, size(px(w), px(700.)));
    }
    view.update(cx, |p, cx| jar_tab(p, tab, cx));
    cx.run_until_parked();
    (s, win, view)
}

fn clipboard(cx: &mut TestAppContext) -> Option<String> {
    cx.read_from_clipboard().and_then(|c| c.text())
}

fn header_rows_shown(cx: &mut TestAppContext, win: AnyWindowHandle) -> usize {
    (0..5usize)
        .filter(|ix| present(cx, win, ("header", *ix)))
        .count()
}

#[gpui_kit::test]
fn typing_filters_the_headers_and_clearing_restores_them(cx: &mut TestAppContext) {
    let (_s, win, _view) = pane(cx, ResponseTab::Headers, None);
    assert!(present(cx, win, "headers-filter") && present(cx, win, "headers-copy"));
    assert_eq!(header_rows_shown(cx, win), 3);
    replace_input(cx, win, None, "headers-filter", "cache");
    frame(cx, win);
    assert_eq!(header_rows_shown(cx, win), 2);
    replace_input(cx, win, None, "headers-filter", "");
    frame(cx, win);
    assert_eq!(header_rows_shown(cx, win), 3);
}

#[gpui_kit::test]
fn copy_puts_only_the_shown_headers_on_the_clipboard(cx: &mut TestAppContext) {
    let (_s, win, _view) = pane(cx, ResponseTab::Headers, None);
    replace_input(cx, win, None, "headers-filter", "cache");
    frame(cx, win);
    cx.update_window(win, |_, w, cx| w.click("headers-copy", cx))
        .unwrap();
    // The response stores names lowercased, and Copy writes them as stored.
    assert_eq!(
        clipboard(cx).as_deref(),
        Some("cache-control: max-age=60\nx-trace: CACHE-hit")
    );
}

#[gpui_kit::test]
fn the_filter_survives_a_tab_change(cx: &mut TestAppContext) {
    let (_s, win, view) = pane(cx, ResponseTab::Headers, None);
    replace_input(cx, win, None, "headers-filter", "cache");
    frame(cx, win);
    view.update(cx, |p, cx| p.set_tab(ResponseTab::Body, cx));
    view.update(cx, |p, cx| p.set_tab(ResponseTab::Headers, cx));
    frame(cx, win);
    assert_eq!(header_rows_shown(cx, win), 2);
}

#[gpui_kit::test]
fn the_controls_are_labelled_and_tab_reachable(cx: &mut TestAppContext) {
    let (_s, win, _view) = pane(cx, ResponseTab::Headers, None);
    for id in ["headers-filter", "headers-copy"] {
        let label = cx
            .update_window(win, |_, w, cx| {
                w.render_frame(cx);
                w.find(id).label().map(str::to_owned)
            })
            .unwrap();
        assert!(label.is_some_and(|l| !l.is_empty()), "{id} has no label");
    }
    // Tab from the filter lands on Copy.
    cx.update_window(win, |_, w, cx| {
        w.click("headers-filter", cx);
        w.press("tab", cx);
    })
    .unwrap();
    let copy_focused = cx
        .update_window(win, |_, w, cx| {
            w.render_frame(cx);
            w.find("headers-copy").focused()
        })
        .unwrap();
    assert_eq!(copy_focused, Some(true));
}

#[gpui_kit::test]
fn the_headers_controls_fit_a_narrow_pane(cx: &mut TestAppContext) {
    let (_s, win, _view) = pane(cx, ResponseTab::Headers, Some(360.));
    assert!(present(cx, win, "headers-filter") && present(cx, win, "headers-copy"));
    replace_input(cx, win, None, "headers-filter", "cache");
    frame(cx, win);
    assert_eq!(header_rows_shown(cx, win), 2);
    let right = cx
        .update_window(win, |_, w, cx| {
            w.render_frame(cx);
            w.find("headers-copy").bounds().right()
        })
        .unwrap();
    assert!(right <= px(360.), "Copy is cut off at {right:?}");
}

fn cookie_names_shown(cx: &mut TestAppContext, win: AnyWindowHandle) -> usize {
    (0..5usize)
        .filter(|ix| present(cx, win, ("cookie", *ix)))
        .count()
}

#[gpui_kit::test]
fn the_cookie_tab_filters_copies_and_copies_a_value(cx: &mut TestAppContext) {
    let (_s, win, _view) = pane(cx, ResponseTab::Cookies, None);
    assert!(present(cx, win, "cookies-filter") && present(cx, win, "cookies-copy"));
    assert_eq!(cookie_names_shown(cx, win), 2);
    replace_input(cx, win, None, "cookies-filter", "b");
    frame(cx, win);
    assert_eq!(cookie_names_shown(cx, win), 1);
    cx.update_window(win, |_, w, cx| w.click("cookies-copy", cx))
        .unwrap();
    assert_eq!(
        clipboard(cx).as_deref(),
        Some("b=[redacted]; Domain=127.0.0.1; Path=/")
    );
    cx.update_window(win, |_, w, cx| w.click(("cookie-copy", 0usize), cx))
        .unwrap();
    assert_eq!(clipboard(cx).as_deref(), Some("[redacted]"));
    // The eye reveals the value, and the copies follow it.
    cx.update_window(win, |_, w, cx| w.click(("cookie-reveal", 0usize), cx))
        .unwrap();
    frame(cx, win);
    cx.update_window(win, |_, w, cx| w.click(("cookie-copy", 0usize), cx))
        .unwrap();
    assert_eq!(clipboard(cx).as_deref(), Some("2"));
}

#[gpui_kit::test]
fn the_cookie_controls_fit_a_narrow_pane(cx: &mut TestAppContext) {
    let (_s, win, _view) = pane(cx, ResponseTab::Cookies, Some(360.));
    assert!(present(cx, win, "cookies-filter") && present(cx, win, "cookies-copy"));
    let right = cx
        .update_window(win, |_, w, cx| {
            w.render_frame(cx);
            w.find("cookies-copy").bounds().right()
        })
        .unwrap();
    assert!(right <= px(360.), "Copy is cut off at {right:?}");
}

#[gpui_kit::test]
fn deleting_under_a_filter_removes_the_cookie_shown(cx: &mut TestAppContext) {
    let (_s, win, view) = pane(cx, ResponseTab::Cookies, None);
    replace_input(cx, win, None, "cookies-filter", "b");
    frame(cx, win);
    cx.update_window(win, |_, w, cx| w.click(("cookie-delete", 0usize), cx))
        .unwrap();
    cx.run_until_parked();
    let left: Vec<_> = view
        .read_with(cx, |p, _| p.cookie_rows())
        .into_iter()
        .map(|c| c.0)
        .collect();
    assert_eq!(left, ["a"]);
}

fn toasts(cx: &mut TestAppContext, win: AnyWindowHandle) -> usize {
    cx.update_window(win, |_, w, cx| w.notifications(cx).len())
        .unwrap()
}

#[gpui_kit::test]
fn a_row_copy_button_and_copy_each_confirm_with_a_toast(cx: &mut TestAppContext) {
    let (_s, win, _view) = pane(cx, ResponseTab::Headers, None);
    assert_eq!(toasts(cx, win), 0);
    cx.update_window(win, |_, w, cx| w.click(("header-copy", 0usize), cx))
        .unwrap();
    assert_eq!(toasts(cx, win), 1);
    cx.update_window(win, |_, w, cx| w.click("headers-copy", cx))
        .unwrap();
    assert_eq!(toasts(cx, win), 2);
}

#[gpui_kit::test]
fn a_new_send_keeps_the_filter_text_and_refilters_the_rows(cx: &mut TestAppContext) {
    let (s, win, _view) = pane(cx, ResponseTab::Headers, None);
    replace_input(cx, win, None, "headers-filter", "cache");
    frame(cx, win);
    assert_eq!(header_rows_shown(cx, win), 2);
    let next = Response::new(200)
        .with_header("x-cache", "miss")
        .with_header("server", "a")
        .with_header("etag", "b");
    let result =
        RequestResult::from_parts(ip("a.yml"), "a", Outcome::Passed, Some(next), Vec::new());
    s.model.update(cx, |m, cx| m.show(Arc::new(result), cx));
    frame(cx, win);
    assert_eq!(header_rows_shown(cx, win), 1);
    cx.update_window(win, |_, w, cx| w.click("headers-copy", cx))
        .unwrap();
    assert_eq!(clipboard(cx).as_deref(), Some("x-cache: miss"));
}

// A row click starts a text selection, so it must never reach the clipboard: only the
// per-row button and Ctrl+C do.
#[gpui_kit::test]
fn a_row_click_leaves_the_clipboard_and_toasts_alone(cx: &mut TestAppContext) {
    let (_s, win, _view) = pane(cx, ResponseTab::Headers, None);
    cx.update_window(win, |_, w, cx| w.click(("header", 0usize), cx))
        .unwrap();
    assert_eq!(clipboard(cx), None);
    assert_eq!(toasts(cx, win), 0);
    let (_s, win, _view) = pane(cx, ResponseTab::Cookies, None);
    cx.update_window(win, |_, w, cx| w.click(("cookie", 0usize), cx))
        .unwrap();
    assert_eq!(clipboard(cx), None);
    assert_eq!(toasts(cx, win), 0);
}

#[gpui_kit::test]
fn the_row_copy_button_copies_only_that_rows_value(cx: &mut TestAppContext) {
    let (_s, win, _view) = pane(cx, ResponseTab::Headers, None);
    replace_input(cx, win, None, "headers-filter", "json");
    frame(cx, win);
    cx.update_window(win, |_, w, cx| w.click(("header-copy", 0usize), cx))
        .unwrap();
    assert_eq!(clipboard(cx).as_deref(), Some("application/json"));
}

#[gpui_kit::test]
fn the_row_copy_button_shows_on_hover_or_focus_and_is_labelled(cx: &mut TestAppContext) {
    let (_s, win, _view) = pane(cx, ResponseTab::Headers, None);
    cx.update_window(win, |_, w, cx| {
        w.hover("headers-filter", cx);
        w.render_frame(cx);
        assert!(!w.within(("header", 0usize)).find("kv-action").visible());
        assert_eq!(w.find(("header-copy", 0usize)).label(), Some("Copy value"));
        w.hover(("header", 0usize), cx);
        w.render_frame(cx);
        assert!(w.within(("header", 0usize)).find("kv-action").visible());
        w.hover("headers-filter", cx);
    })
    .unwrap();
    let mut presses = 0;
    let focused = |cx: &mut TestAppContext| {
        cx.update_window(win, |_, w, cx| {
            w.render_frame(cx);
            w.find(("header-copy", 1usize)).focused()
        })
        .unwrap()
    };
    while focused(cx) != Some(true) {
        presses += 1;
        assert!(presses < 40, "Tab never reached the row copy button");
        cx.update_window(win, |_, w, cx| w.focus_next(cx)).unwrap();
    }
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        assert!(w.within(("header", 1usize)).find("kv-action").visible());
    })
    .unwrap();
}

#[gpui_kit::test]
fn an_empty_filter_disables_copy_and_says_nothing_matches(cx: &mut TestAppContext) {
    let (_s, win, _view) = pane(cx, ResponseTab::Headers, None);
    assert!(!present(cx, win, "headers-none"));
    replace_input(cx, win, None, "headers-filter", "zzz");
    frame(cx, win);
    assert_eq!(header_rows_shown(cx, win), 0);
    // The kit does not report a button's disabled state, so the click below is the proof.
    let label = cx
        .update_window(win, |_, w, _| {
            w.find("headers-none").label().map(str::to_owned)
        })
        .unwrap();
    assert_eq!(label.as_deref(), Some("No matching headers"));
    cx.update_window(win, |_, w, cx| w.click("headers-copy", cx))
        .unwrap();
    assert_eq!(clipboard(cx), None);
    assert_eq!(toasts(cx, win), 0);
    replace_input(cx, win, None, "headers-filter", "");
    frame(cx, win);
    assert!(!present(cx, win, "headers-none"));
}

#[gpui_kit::test]
fn an_empty_cookie_filter_disables_copy_and_says_nothing_matches(cx: &mut TestAppContext) {
    let (_s, win, _view) = pane(cx, ResponseTab::Cookies, None);
    replace_input(cx, win, None, "cookies-filter", "zzz");
    frame(cx, win);
    // The kit does not report a button's disabled state, so the click below is the proof.
    let label = cx
        .update_window(win, |_, w, _| {
            w.find("cookies-none").label().map(str::to_owned)
        })
        .unwrap();
    assert_eq!(label.as_deref(), Some("No matching cookies"));
    cx.update_window(win, |_, w, cx| w.click("cookies-copy", cx))
        .unwrap();
    assert_eq!(clipboard(cx), None);
    assert_eq!(toasts(cx, win), 0);
}

#[gpui_kit::test]
fn the_copy_buttons_are_named_for_what_they_copy(cx: &mut TestAppContext) {
    for (tab, id, name) in [
        (ResponseTab::Headers, "headers-copy", "Copy shown headers"),
        (ResponseTab::Cookies, "cookies-copy", "Copy shown cookies"),
    ] {
        let (_s, win, _view) = pane(cx, tab, None);
        let label = cx
            .update_window(win, |_, w, cx| {
                w.render_frame(cx);
                w.find(id).label().map(str::to_owned)
            })
            .unwrap();
        assert_eq!(label.as_deref(), Some(name));
    }
}

// The focus ring is drawn outside the button, so the bar keeps a gutter on the right.
#[gpui_kit::test]
fn the_copy_button_leaves_room_for_its_focus_ring(cx: &mut TestAppContext) {
    let (_s, win, _view) = pane(cx, ResponseTab::Headers, Some(480.));
    let right = cx
        .update_window(win, |_, w, cx| {
            w.render_frame(cx);
            w.find("headers-copy").bounds().right()
        })
        .unwrap();
    assert!(right <= px(476.), "no room for the ring at {right:?}");
}

// At 480 px the Value column keeps a readable width and the table scrolls sideways instead.
#[gpui_kit::test]
fn the_cookie_value_column_keeps_a_usable_width_when_narrow(cx: &mut TestAppContext) {
    let (_s, win, _view) = pane(cx, ResponseTab::Cookies, Some(480.));
    let (value, domain) = cx
        .update_window(win, |_, w, cx| {
            w.render_frame(cx);
            (
                w.find(("cookie-value", 0usize)).bounds(),
                w.find(("cookie-domain", 0usize)).bounds(),
            )
        })
        .unwrap();
    assert!(
        value.size.width >= px(120.),
        "Value is {:?} wide",
        value.size.width
    );
    assert!(value.right() <= domain.left(), "Value overlaps Domain");
}

// A 1200 px window with the sidebar leaves a 936 px pane: the table fits without a sideways
// scroll and each row's copy and delete buttons sit inside it.
#[gpui_kit::test]
fn the_cookie_row_actions_stay_in_view_at_a_common_pane_width(cx: &mut TestAppContext) {
    let (_s, win, _view) = pane(cx, ResponseTab::Cookies, Some(936.));
    let (copy, delete) = cx
        .update_window(win, |_, w, cx| {
            w.render_frame(cx);
            (
                w.find(("cookie-copy", 0usize)).bounds(),
                w.find(("cookie-delete", 0usize)).bounds(),
            )
        })
        .unwrap();
    assert!(
        apinata_app::style::at_zoom(apinata_app::style::COOKIE_TABLE_MIN_WIDTH, 1.0) <= px(936.)
    );
    assert!(copy.right() <= px(936.), "copy ends at {:?}", copy.right());
    assert!(
        delete.right() <= px(936.),
        "delete ends at {:?}",
        delete.right()
    );
}

// At 480 px the table still scrolls sideways but Value keeps 160 px.
#[gpui_kit::test]
fn the_cookie_value_keeps_160_px_at_a_narrow_pane(cx: &mut TestAppContext) {
    let (_s, win, _view) = pane(cx, ResponseTab::Cookies, Some(480.));
    let value = cx
        .update_window(win, |_, w, cx| {
            w.render_frame(cx);
            w.find(("cookie-value", 0usize)).bounds()
        })
        .unwrap();
    assert!(
        value.size.width >= px(160.),
        "Value is {:?}",
        value.size.width
    );
}

// The row's copy button is named like the toolbar's, by its tooltip words.
#[gpui_kit::test]
fn the_cookie_row_copy_button_is_named_copy_value(cx: &mut TestAppContext) {
    let (_s, win, _view) = pane(cx, ResponseTab::Cookies, None);
    let label = cx
        .update_window(win, |_, w, cx| {
            w.render_frame(cx);
            w.find(("cookie-copy", 0usize)).label().map(str::to_owned)
        })
        .unwrap();
    assert_eq!(label.as_deref(), Some("Copy value"));
}
