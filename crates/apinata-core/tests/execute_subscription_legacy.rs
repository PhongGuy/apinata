//! The legacy graphql-ws protocol (subscriptions-transport-ws), chosen by the server's reply.

use crate::common::ROOT;
use crate::common::ws_server::{
    QUERY, Step, VARS, WsScript, graphql_request, record, sent, sub, tick, ws_exec, ws_server,
};
use apinata_core::HttpVersionPolicy::Auto;
use apinata_core::{
    CancelHandle, StreamBound, StreamEnd, StreamEvent, StreamLimits, StreamOptions,
};
use serde_json::json;

fn legacy(steps: Vec<Step>) -> WsScript {
    WsScript {
        protocol: Some("graphql-ws"),
        ..WsScript::new(steps)
    }
}

const TICK1: &str = r#"{"data":{"tick":1}}"#;

#[tokio::test]
async fn a_legacy_server_gets_start_and_its_data_becomes_next_records() {
    let s = ws_server(legacy(vec![tick(1), tick(2), Step::Complete])).await;
    let res = ws_exec(Auto).send(&sub(&s.http("/graphql"))).await.unwrap();
    assert_eq!(res.status(), 101);
    assert_eq!(res.stream_end(), Some(&StreamEnd::Closed));
    assert_eq!(
        res.events(),
        vec![
            record("next", TICK1),
            record("next", r#"{"data":{"tick":2}}"#),
            record("complete", ""),
        ]
    );
    let lines = s.seen.until(|l| l.starts_with("close: ")).await;
    assert!(
        lines.contains(&"sec-websocket-protocol: graphql-transport-ws, graphql-ws".to_owned()),
        "{lines:?}"
    );
    assert_eq!(
        sent(&lines),
        vec![
            json!({"type": "connection_init"}),
            json!({"id": "1", "type": "start",
                   "payload": {"query": QUERY, "variables": {"n": 2}}}),
        ]
    );
    assert!(lines.contains(&"close: 1000".to_owned()), "{lines:?}");
}

#[tokio::test]
async fn keep_alives_are_ignored_before_and_after_start() {
    let s = ws_server(WsScript {
        greeting: vec![Step::Ka],
        ..legacy(vec![Step::Ka, tick(1), Step::Ka, Step::Complete])
    })
    .await;
    let res = ws_exec(Auto).send(&sub(&s.http("/"))).await.unwrap();
    assert_eq!(res.stream_end(), Some(&StreamEnd::Closed));
    assert_eq!(
        res.events(),
        vec![record("next", TICK1), record("complete", "")]
    );
}

#[tokio::test]
async fn stop_sends_stop_then_connection_terminate_and_closes_normally() {
    let s = ws_server(legacy(vec![tick(1)])).await;
    let cancel = CancelHandle::new();
    let stopper = cancel.clone();
    let opts = StreamOptions::new().sink(move |e| {
        if matches!(e, StreamEvent::Chunk(_)) {
            stopper.stop();
        }
    });
    let res = ws_exec(Auto)
        .send_streaming(&sub(&s.http("/")), &cancel, opts)
        .await
        .unwrap();
    assert_eq!(res.stream_end(), Some(&StreamEnd::Stopped));
    let lines = s.seen.until(|l| l.starts_with("close: ")).await;
    let sent = sent(&lines);
    assert_eq!(
        sent[sent.len() - 2..].to_vec(),
        vec![
            json!({"id": "1", "type": "stop"}),
            json!({"type": "connection_terminate"}),
        ],
        "{lines:?}"
    );
    assert!(lines.contains(&"close: 1000".to_owned()), "{lines:?}");
}

#[tokio::test]
async fn the_event_limit_ends_a_legacy_run_politely() {
    let s = ws_server(legacy(vec![tick(1), tick(2), tick(3)])).await;
    let opts = StreamOptions::new().limits(StreamLimits::new().max_events(Some(2)));
    let res = ws_exec(Auto)
        .send_streaming(&sub(&s.http("/")), &CancelHandle::new(), opts)
        .await
        .unwrap();
    assert_eq!(
        res.stream_end(),
        Some(&StreamEnd::Bound(StreamBound::Events))
    );
    assert_eq!(res.events().len(), 2);
    let lines = s.seen.until(|l| l.starts_with("close: ")).await;
    assert_eq!(
        sent(&lines).last(),
        Some(&json!({"type": "connection_terminate"})),
        "{lines:?}"
    );
}

#[tokio::test]
async fn a_legacy_error_is_recorded_and_ends_closed() {
    let s = ws_server(legacy(vec![Step::Error(json!([{"message": "boom"}]))])).await;
    let res = ws_exec(Auto).send(&sub(&s.http("/"))).await.unwrap();
    assert_eq!(res.stream_end(), Some(&StreamEnd::Closed));
    assert_eq!(
        res.events(),
        vec![record("error", r#"[{"message":"boom"}]"#)]
    );
}

#[tokio::test]
async fn connection_error_ends_the_run_with_its_payload() {
    let refusal = r#"{"type":"connection_error","payload":{"message":"denied"}}"#;
    let s = ws_server(WsScript {
        ack: false,
        greeting: vec![Step::Text(refusal.to_owned())],
        ..legacy(Vec::new())
    })
    .await;
    let res = ws_exec(Auto).send(&sub(&s.http("/"))).await.unwrap();
    assert_eq!(res.status(), 101);
    assert!(res.body().is_empty());
    assert_eq!(
        res.stream_end(),
        Some(&StreamEnd::Error(
            r#"connection_error from server: {"message":"denied"}"#.into()
        ))
    );
    let lines = s.seen.until(|l| l.starts_with("close: ")).await;
    assert!(lines.contains(&"close: 1000".to_owned()), "{lines:?}");
    assert_eq!(sent(&lines), vec![json!({"type": "connection_init"})]);
}

#[tokio::test]
async fn a_graphql_transport_ws_message_is_unexpected_on_a_legacy_socket() {
    let next = r#"{"id":"1","type":"next","payload":{}}"#;
    let s = ws_server(legacy(vec![Step::Text(next.to_owned())])).await;
    let res = ws_exec(Auto).send(&sub(&s.http("/"))).await.unwrap();
    assert_eq!(
        res.stream_end(),
        Some(&StreamEnd::Error(format!(
            "unexpected message from server: {next}"
        )))
    );
    let lines = s.seen.until(|l| l.starts_with("close: ")).await;
    assert!(lines.contains(&"close: 4400".to_owned()), "{lines:?}");
}

#[tokio::test]
async fn data_for_another_id_is_ignored() {
    let other = r#"{"id":"2","type":"data","payload":{"data":{"tick":9}}}"#;
    let s = ws_server(legacy(vec![Step::Text(other.to_owned()), Step::Complete])).await;
    let res = ws_exec(Auto).send(&sub(&s.http("/"))).await.unwrap();
    assert_eq!(res.stream_end(), Some(&StreamEnd::Closed));
    assert_eq!(res.events(), vec![record("complete", "")]);
    assert!(!res.text().contains("tick"), "{}", res.text());
}

#[tokio::test]
async fn a_pinned_subprotocol_is_sent_alone_and_selects_the_codec() {
    let s = ws_server(legacy(vec![tick(1), Step::Complete])).await;
    let pin = "  headers:\n    - name: Sec-WebSocket-Protocol\n      value: graphql-ws\n";
    let req = graphql_request(ROOT, &s.http("/"), QUERY, VARS, pin);
    let res = ws_exec(Auto).send(&req).await.unwrap();
    assert_eq!(res.stream_end(), Some(&StreamEnd::Closed));
    assert_eq!(
        res.events(),
        vec![record("next", TICK1), record("complete", "")]
    );
    let lines = s.seen.until(|l| l.starts_with("close: ")).await;
    let offered: Vec<&str> = lines
        .iter()
        .filter(|l| l.starts_with("sec-websocket-protocol: "))
        .map(String::as_str)
        .collect();
    assert_eq!(
        offered,
        vec!["sec-websocket-protocol: graphql-ws"],
        "{lines:?}"
    );
    assert_eq!(sent(&lines)[1]["type"], json!("start"), "{lines:?}");
}
