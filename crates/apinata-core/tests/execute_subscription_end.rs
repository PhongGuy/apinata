//! How a subscription ends: Stop, server error, close codes, a dropped socket, a missing ack,
//! keep-alive and unreadable messages.

use crate::common::ROOT;
use crate::common::ws_server::{
    QUERY, Step, VARS, WsScript, graphql_request, record, sent, sub, tick, ws_exec, ws_server,
};
use apinata_core::HttpVersionPolicy::Auto;
use apinata_core::{CancelHandle, StreamEnd, StreamEvent, StreamOptions};
use serde_json::json;
use std::time::{Duration, Instant};

#[tokio::test]
async fn stop_completes_the_operation_and_closes_normally() {
    let s = ws_server(WsScript::new(vec![tick(1)])).await;
    let cancel = CancelHandle::new();
    let stopper = cancel.clone();
    let opts = StreamOptions::new().sink(move |e| {
        if matches!(e, StreamEvent::Chunk(_)) {
            stopper.stop();
        }
    });
    let started = Instant::now();
    let res = ws_exec(Auto)
        .send_streaming(&sub(&s.http("/")), &cancel, opts)
        .await
        .unwrap();
    assert_eq!(res.stream_end(), Some(&StreamEnd::Stopped));
    assert_eq!(res.events(), vec![record("next", r#"{"data":{"tick":1}}"#)]);
    assert!(started.elapsed() < Duration::from_secs(5));
    let lines = s.seen.until(|l| l.starts_with("close: ")).await;
    assert_eq!(
        sent(&lines).last(),
        Some(&json!({"id": "1", "type": "complete"}))
    );
    assert!(lines.contains(&"close: 1000".to_owned()), "{lines:?}");
}

#[tokio::test]
async fn an_error_message_is_recorded_and_ends_closed() {
    let s = ws_server(WsScript::new(vec![Step::Error(
        json!([{"message": "boom"}]),
    )]))
    .await;
    let res = ws_exec(Auto).send(&sub(&s.http("/"))).await.unwrap();
    assert_eq!(res.stream_end(), Some(&StreamEnd::Closed));
    assert_eq!(
        res.events(),
        vec![record("error", r#"[{"message":"boom"}]"#)]
    );
    let lines = s.seen.until(|l| l.starts_with("close: ")).await;
    assert!(lines.contains(&"close: 1000".to_owned()), "{lines:?}");
}

#[tokio::test]
async fn close_codes_map_onto_stream_ends() {
    for (code, reason, end) in [
        (1000, "", StreamEnd::Closed),
        (
            4403,
            "Forbidden",
            StreamEnd::Error("closed by server: 4403 Forbidden".into()),
        ),
        (4500, "", StreamEnd::Error("closed by server: 4500".into())),
    ] {
        let s = ws_server(WsScript::new(vec![tick(1), Step::Close(code, reason)])).await;
        let res = ws_exec(Auto).send(&sub(&s.http("/"))).await.unwrap();
        assert_eq!(res.stream_end(), Some(&end), "{code}");
        assert_eq!(res.events().len(), 1, "{code}");
    }
}

#[tokio::test]
async fn a_close_without_a_status_code_is_an_error() {
    let s = ws_server(WsScript::new(vec![tick(1), Step::CloseBare])).await;
    let res = ws_exec(Auto).send(&sub(&s.http("/"))).await.unwrap();
    assert_eq!(
        res.stream_end(),
        Some(&StreamEnd::Error("closed by server: no status".into()))
    );
    assert_eq!(res.events().len(), 1);
}

#[tokio::test]
async fn a_dropped_socket_is_an_error() {
    let s = ws_server(WsScript::new(vec![tick(1), Step::Drop])).await;
    let res = ws_exec(Auto).send(&sub(&s.http("/"))).await.unwrap();
    assert_eq!(
        res.stream_end(),
        Some(&StreamEnd::Error(
            "closed by server: connection dropped".into()
        ))
    );
    assert_eq!(res.events().len(), 1);
}

#[tokio::test]
async fn no_ack_within_the_request_timeout_closes_4408() {
    let s = ws_server(WsScript {
        ack: false,
        ..WsScript::new(Vec::new())
    })
    .await;
    let req = graphql_request(
        ROOT,
        &s.http("/"),
        QUERY,
        VARS,
        "settings:\n  timeout: 300\n",
    );
    let started = Instant::now();
    // Streaming with no limits, so only the ack wait can end it.
    let res = ws_exec(Auto)
        .send_streaming(&req, &CancelHandle::new(), StreamOptions::new())
        .await
        .unwrap();
    assert_eq!(res.status(), 101);
    assert_eq!(
        res.stream_end(),
        Some(&StreamEnd::Error("no connection_ack within 300 ms".into()))
    );
    assert!(started.elapsed() < Duration::from_secs(5));
    let lines = s.seen.until(|l| l.starts_with("close: ")).await;
    assert!(lines.contains(&"close: 4408".to_owned()), "{lines:?}");
    assert_eq!(sent(&lines), vec![json!({"type": "connection_init"})]);
}

#[tokio::test]
async fn ping_is_answered_and_other_ids_are_ignored() {
    let foreign = r#"{"id":"2","type":"next","payload":{"data":null}}"#.to_owned();
    let s = ws_server(WsScript::new(vec![
        Step::Ping,
        Step::Text(foreign),
        tick(1),
        Step::Complete,
    ]))
    .await;
    let res = ws_exec(Auto).send(&sub(&s.http("/"))).await.unwrap();
    assert_eq!(res.stream_end(), Some(&StreamEnd::Closed));
    assert_eq!(
        res.events(),
        vec![
            record("next", r#"{"data":{"tick":1}}"#),
            record("complete", ""),
        ]
    );
    let lines = s.seen.until(|l| l.starts_with("close: ")).await;
    assert!(sent(&lines).contains(&json!({"type": "pong"})), "{lines:?}");
}

#[tokio::test]
async fn an_unreadable_message_closes_4400() {
    let s = ws_server(WsScript::new(vec![Step::Text("not json".into())])).await;
    let res = ws_exec(Auto).send(&sub(&s.http("/"))).await.unwrap();
    assert_eq!(
        res.stream_end(),
        Some(&StreamEnd::Error(
            "unexpected message from server: not json".into()
        ))
    );
    let lines = s.seen.until(|l| l.starts_with("close: ")).await;
    assert!(lines.contains(&"close: 4400".to_owned()), "{lines:?}");
}
