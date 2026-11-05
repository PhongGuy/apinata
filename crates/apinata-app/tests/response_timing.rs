//! The timing bar in the response header row.

use crate::support;

use apinata_app::response_pane::ResponsePane;
use apinata_app::timing_bar::{Phases, tooltip_lines};
use apinata_core::{Outcome, RequestResult, Response};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AnyWindowHandle, AppContext, Entity, TestAppContext, WindowOptions};
use std::sync::Arc;
use std::time::Duration;
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
fn a_measured_send_shows_the_bar_with_its_phases(cx: &mut TestAppContext) {
    let s = send_once(cx, ResponseTemplate::new(200).set_body_string("ok"), "");
    let (win, _view) = pane(cx, &s);
    assert!(present(cx, win, "response-timing"));
    // An IP literal host has no DNS phase, and a fresh connection is not "reused".
    assert!(present(cx, win, "response-timing-connect"));
    assert!(present(cx, win, "response-timing-wait"));
    assert!(!present(cx, win, "response-timing-dns"));
}

/// The timing bar's hover text, read from its tooltip after hovering the bar.
fn tooltip(cx: &mut TestAppContext, win: AnyWindowHandle) -> Option<String> {
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        w.hover("response-timing", cx)
    })
    .unwrap();
    cx.executor().advance_clock(Duration::from_millis(800));
    frame(cx, win);
    cx.update_window(win, |_, w, cx| {
        w.render_frame(cx);
        w.try_find("response-timing-tooltip")
            .and_then(|e| e.label().map(str::to_owned))
    })
    .unwrap()
}

/// The phase labels of tooltip `text`, each line cut at its two-space gap.
fn labels(text: &str) -> Vec<&str> {
    text.lines()
        .map(|l| l.split_once("  ").map_or(l, |(label, _)| label))
        .collect()
}

/// What the bar should say for the final hop of `result`, per the pure layer.
fn expected(result: &RequestResult) -> String {
    let timing = result.response.as_ref().unwrap().timeline().last().unwrap();
    tooltip_lines(&Phases::from(timing.timing.as_ref().unwrap())).join("\n")
}

#[gpui_kit::test]
fn a_fresh_connection_tooltip_lists_connect_and_wait_but_no_dns(cx: &mut TestAppContext) {
    let s = send_once(cx, ResponseTemplate::new(200).set_body_string("ok"), "");
    let (win, _view) = pane(cx, &s);
    let text = tooltip(cx, win).expect("tooltip shown");
    assert_eq!(text, expected(&s.result));
    let shown = labels(&text);
    assert_eq!(shown[..2], ["Connect", "Wait"], "{text}");
    assert!(
        shown
            .iter()
            .all(|l| ["Connect", "Wait", "Download"].contains(l)),
        "{text}"
    );
}

#[gpui_kit::test]
fn a_reused_connection_tooltip_notes_it_and_has_no_connect(cx: &mut TestAppContext) {
    let s = send_once(cx, ResponseTemplate::new(200).set_body_string("ok"), "");
    let r = s.root.clone();
    assert!(s.model.update(cx, |m, cx| m.send(&r, ip("a.yml"), cx)));
    let again = finished(cx, &s.model);
    let (win, _view) = pane(cx, &s);
    assert!(present(cx, win, "response-timing"));
    assert!(!present(cx, win, "response-timing-connect"));
    assert!(!present(cx, win, "response-timing-dns"));
    let text = tooltip(cx, win).expect("tooltip shown");
    assert_eq!(text, expected(&again));
    let shown = labels(&text);
    assert_eq!(shown[..2], ["Connection reused", "Wait"], "{text}");
    assert!(
        shown[1..].iter().all(|l| ["Wait", "Download"].contains(l)),
        "{text}"
    );
}

#[gpui_kit::test]
fn an_untimed_response_hides_the_bar(cx: &mut TestAppContext) {
    let s = send_once(cx, ResponseTemplate::new(200).set_body_string("ok"), "");
    let (win, _view) = pane(cx, &s);
    assert!(present(cx, win, "response-timing"));
    // A response with no timed hop, as one loaded from elsewhere, leaves no bar at all.
    let untimed = Arc::new(RequestResult::from_parts(
        ip("a.yml"),
        "a",
        Outcome::Passed,
        Some(Response::new(200).with_text_body("ok")),
        Vec::new(),
    ));
    s.model.update(cx, |m, cx| m.show(untimed, cx));
    frame(cx, win);
    assert!(
        present(cx, win, "response-status"),
        "the response is still shown"
    );
    assert!(!present(cx, win, "response-timing"));
    assert!(!present(cx, win, "response-timing-wait"));
}
