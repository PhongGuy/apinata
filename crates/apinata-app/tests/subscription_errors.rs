//! A failed subscription: refused at the handshake or ended by a bad message, its response and
//! its error shown together.

use crate::support::ws_server::tick;
use crate::support::*;
use apinata_app::response_pane::{ErrorKind, ErrorTone, ResponseTab, describe_error};
use apinata_core::{Outcome, RunError, StreamEnd};
use gpui_kit::TestAppContext;
use std::time::Duration;

const NOT_FOUND: &str = "HTTP/1.1 404 Not Found\r\nContent-Type: text/html\r\nContent-Length: 17\r\n\r\n<h1>Not here</h1>";

#[test]
fn a_subscription_error_reads_as_a_failed_subscription() {
    let e = RunError::Subscription {
        message: "closed by server: 4403 Forbidden".into(),
    };
    let view = describe_error(&e, &e.to_string(), None);
    assert_eq!(
        (view.kind, view.title, view.tone, view.retry),
        (
            ErrorKind::Subscription,
            "Subscription failed",
            ErrorTone::Danger,
            true
        )
    );
    assert_eq!(
        view.sentence.as_deref(),
        Some("closed by server: 4403 Forbidden")
    );
}

#[gpui_kit::test]
fn a_refused_handshake_shows_its_page_and_the_error_together(cx: &mut TestAppContext) {
    http_ready(cx);
    let port = raw_peer(cx, NOT_FOUND);
    let col = subscription_collection(port);
    let (win, pane, model, root) = pane_for(cx, &col);
    model.update(cx, |m, cx| m.send(&root, ip("a.yml"), cx));
    // A refused handshake sends no Started, so the send never goes live.
    let mut went_live = false;
    wait_real(cx, |cx| {
        model.read_with(cx, |m, _| {
            went_live |= m.live().is_some();
            !m.is_sending()
        })
    });
    assert!(!went_live);
    let result = finished(cx, &model);
    assert_eq!(result.outcome, Outcome::Errored);
    let res = result.response.as_ref().expect("the refusal is a response");
    assert_eq!(res.status(), 404);
    let want = format!("WebSocket handshake to ws://127.0.0.1:{port}/graphql failed: 404");
    match &result.error {
        Some(RunError::Subscription { message }) => {
            assert!(message.starts_with(&want), "{message}")
        }
        other => panic!("{other:?}"),
    }
    assert!(
        pane.read_with(cx, |p, cx| p.error_view(cx)).is_none(),
        "the body is not replaced"
    );
    let shown = pane
        .read_with(cx, |p, cx| p.stream_error(cx))
        .expect("the error beside the response");
    assert_eq!(
        (shown.kind, shown.title),
        (ErrorKind::Subscription, "Subscription failed")
    );
    assert!(
        shown
            .sentence
            .as_deref()
            .is_some_and(|s| s.contains("failed: 404")),
        "{shown:?}"
    );
    assert!(pane.read_with(cx, |p, cx| p.body_shown(cx)));
    assert!(present(cx, win, "response-stream-error"));
    assert!(present(cx, win, "response-body"));
    assert!(!present(cx, win, "response-error"));
    assert!(
        !pane
            .read_with(cx, |p, cx| p.shown_tabs(cx))
            .contains(&ResponseTab::Events)
    );
}

#[gpui_kit::test]
fn a_subscription_ended_by_a_bad_message_keeps_its_events_and_says_why(cx: &mut TestAppContext) {
    http_ready(cx);
    let frames = vec![
        tick(false, 1),
        tick(false, 2),
        r#"{"type":"bogus"}"#.to_owned(),
    ];
    let port = ws_peer(
        cx,
        "graphql-transport-ws",
        frames,
        Duration::from_millis(10),
    );
    let col = subscription_collection(port);
    let (win, pane, model, root) = pane_for(cx, &col);
    model.update(cx, |m, cx| m.send(&root, ip("a.yml"), cx));
    let result = finished(cx, &model);
    assert_eq!(result.outcome, Outcome::Errored);
    let res = result.response.as_ref().expect("a response");
    assert_eq!(res.status(), 101);
    assert!(
        matches!(res.stream_end(), Some(StreamEnd::Error(m)) if m.starts_with("unexpected message from server")),
        "{:?}",
        res.stream_end()
    );
    assert!(
        pane.read_with(cx, |p, cx| p.shown_tabs(cx))
            .contains(&ResponseTab::Events)
    );
    wait_real(cx, |cx| {
        pane.read_with(cx, |p, cx| {
            p.event_rows(cx).iter().filter(|r| r.name == "next").count() == 2
        })
    });
    let shown = pane
        .read_with(cx, |p, cx| p.stream_error(cx))
        .expect("the error");
    assert!(
        (shown.sentence.as_deref())
            .is_some_and(|s| s.starts_with("unexpected message from server")),
        "{shown:?}"
    );
    assert_eq!(
        pane.read_with(cx, |p, cx| p.stream_status(cx)).as_deref(),
        Some("error")
    );
    pane.update(cx, |p, cx| p.set_tab(ResponseTab::Events, cx));
    assert!(present(cx, win, "response-stream-error"));
    assert!(present(cx, win, "response-events"));
}
