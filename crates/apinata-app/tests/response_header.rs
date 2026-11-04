//! The response header row: tabs with counts, then the status pill, time and size.

use crate::support;

use apinata_app::response_pane::{ResponsePane, ResponseTab};
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AnyWindowHandle, AppContext, Entity, TestAppContext, WindowOptions, px};
use support::*;
use wiremock::ResponseTemplate;

fn pane(cx: &mut TestAppContext, s: &Sent) -> (AnyWindowHandle, Entity<ResponsePane>) {
    support::init(cx);
    let model = s.model.clone();
    let (win, view) = cx
        .update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |w, cx| {
                cx.new(|cx| ResponsePane::new(model, w, cx))
            })
        })
        .unwrap();
    frame(cx, win);
    (win, view)
}

#[gpui_kit::test]
fn the_header_row_holds_tabs_counts_and_the_pill(cx: &mut TestAppContext) {
    let s = send_once(
        cx,
        ResponseTemplate::new(201)
            .insert_header("x-a", "1")
            .set_body_raw("{\"a\":1}", "application/json"),
        "",
    );
    let (win, view) = pane(cx, &s);
    let headers = s.result.response.as_ref().unwrap().headers().len();
    let count = |cx: &mut TestAppContext, t| view.read_with(cx, |p, cx| p.tab_count(t, cx));
    assert_eq!(count(cx, ResponseTab::Headers), Some(headers));
    assert_eq!(count(cx, ResponseTab::Body), None);
    assert_eq!(count(cx, ResponseTab::Timeline), None);
    assert!(present(
        cx,
        win,
        ("response-count", ResponseTab::Headers as usize)
    ));
    assert!(!present(
        cx,
        win,
        ("response-count", ResponseTab::Body as usize)
    ));
    for id in ["response-status", "response-time", "response-size"] {
        assert!(present(cx, win, id), "{id}");
    }
    assert!(!present(cx, win, "response-truncated"));
    cx.update_window(win, |_, w, _| {
        let tab = w.find(("response-tab", 0usize)).bounds();
        let last = w
            .find(("response-tab", ResponseTab::Console as usize))
            .bounds();
        let pill = w.find("response-status").bounds();
        assert!(
            (tab.center().y - pill.center().y).abs() < px(4.),
            "one row: {tab:?} {pill:?}"
        );
        assert!(pill.left() > last.right(), "the pill is right of the tabs");
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_failed_assertion_marks_its_tab(cx: &mut TestAppContext) {
    let fails = "runtime:\n  assertions:\n    - expression: res.status\n      operator: eq\n      value: \"404\"\n";
    let s = send_once(cx, ResponseTemplate::new(200), fails);
    let (win, view) = pane(cx, &s);
    assert_eq!(
        view.read_with(cx, |p, cx| p.tab_count(ResponseTab::Tests, cx)),
        Some(1)
    );
    assert!(view.read_with(cx, |p, cx| p.assertions_failed(cx)));
    // Only the Tests count turns red; the others stay neutral.
    use ResponseTab::*;
    for tab in [Body, Headers, Cookies, Timeline, Tests, Console] {
        assert_eq!(
            view.read_with(cx, |p, cx| p.count_is_danger(tab, cx)),
            tab == ResponseTab::Tests,
            "{tab:?}"
        );
    }
    assert!(present(
        cx,
        win,
        ("response-count", ResponseTab::Tests as usize)
    ));
}

#[gpui_kit::test]
fn passing_assertions_are_not_failed(cx: &mut TestAppContext) {
    let passes = "runtime:\n  assertions:\n    - expression: res.status\n      operator: eq\n      value: \"200\"\n";
    let s = send_once(cx, ResponseTemplate::new(200), passes);
    let (_win, view) = pane(cx, &s);
    assert!(!view.read_with(cx, |p, cx| p.assertions_failed(cx)));
    assert!(!view.read_with(cx, |p, cx| p.count_is_danger(ResponseTab::Tests, cx)));
}
