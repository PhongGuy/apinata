//! Subscriptions over WebSocket: detection, the HTTP/1.1 handshake and the transcript.

use crate::common::ws_server::{
    QUERY, Step, WsScript, chunks, graphql_request, record, recording, sent, sub, tick, ws_exec,
    ws_server, wss_server,
};
use crate::common::{ROOT, at, dead_port, resolved_one, setup};
use apinata_core::HttpVersionPolicy::{Auto, Http2PriorKnowledge};
use apinata_core::{CancelHandle, ExecError, HttpVersion, StreamEnd, StreamEvent, StreamLimits};
use serde_json::json;
use wiremock::matchers::any;
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn a_subscription_streams_records_until_the_server_completes() {
    let s = ws_server(WsScript::new(vec![tick(1), tick(2), Step::Complete])).await;
    let (opts, seen) = recording(StreamLimits::new());
    let res = ws_exec(Auto)
        .send_streaming(&sub(&s.http("/graphql")), &CancelHandle::new(), opts)
        .await
        .unwrap();
    assert_eq!(res.status(), 101);
    assert_eq!(res.stream_end(), Some(&StreamEnd::Closed));
    assert_eq!(
        res.events(),
        vec![
            record("next", r#"{"data":{"tick":1}}"#),
            record("next", r#"{"data":{"tick":2}}"#),
            record("complete", ""),
        ]
    );
    assert!(matches!(
        seen.lock().unwrap()[0],
        StreamEvent::Started {
            status: 101,
            event_stream: true,
            ..
        }
    ));
    assert_eq!(chunks(&seen), res.body());
    let lines = s.seen.until(|l| l.starts_with("close: ")).await;
    assert!(lines.contains(&"path: /graphql".to_owned()), "{lines:?}");
    assert!(
        lines.contains(&"sec-websocket-protocol: graphql-transport-ws, graphql-ws".to_owned()),
        "{lines:?}"
    );
    assert_eq!(
        sent(&lines),
        vec![
            json!({"type": "connection_init"}),
            json!({"id": "1", "type": "subscribe",
                   "payload": {"query": QUERY, "variables": {"n": 2}}}),
        ]
    );
    assert!(lines.contains(&"close: 1000".to_owned()), "{lines:?}");
}

#[tokio::test]
async fn the_handshake_is_a_bodiless_get_with_upgrade_headers() {
    let s = ws_server(WsScript::new(vec![Step::Complete])).await;
    let res = ws_exec(Auto).send(&sub(&s.http("/graphql"))).await.unwrap();
    assert_eq!(res.timeline().len(), 1);
    let hop = &res.timeline()[0];
    assert_eq!(
        (hop.method.as_str(), hop.status, hop.request_body.len()),
        ("GET", 101, 0)
    );
    let header = |name: &str| {
        hop.request_headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    };
    assert_eq!(header("upgrade"), Some("websocket"));
    assert_eq!(header("connection"), Some("Upgrade"));
    assert_eq!(header("sec-websocket-version"), Some("13"));
    assert_eq!(header("sec-websocket-key").map(str::len), Some(24));
    assert_eq!(header("content-type"), None);
}

#[tokio::test]
async fn wss_offers_only_http1_even_to_a_server_preferring_h2() {
    let s = wss_server(WsScript::new(vec![tick(1), Step::Complete])).await;
    let res = ws_exec(Auto)
        .send(&sub(&s.https("/graphql")))
        .await
        .unwrap();
    assert_eq!(res.stream_end(), Some(&StreamEnd::Closed));
    assert_eq!(res.events().len(), 2);
    let lines = s.seen.lines();
    assert!(lines.contains(&"alpn: http/1.1".to_owned()), "{lines:?}");
}

#[tokio::test]
async fn the_handshake_client_is_http1_whatever_the_policy() {
    let s = ws_server(WsScript::new(vec![tick(1), Step::Complete])).await;
    let exec = ws_exec(Http2PriorKnowledge);
    let res = exec.send(&sub(&s.http("/graphql"))).await.unwrap();
    assert_eq!(res.stream_end(), Some(&StreamEnd::Closed));
    assert_eq!(res.timeline()[0].version(), Some(HttpVersion::Http11));
    // The policy's own client is untouched: a plain request on this executor still speaks h2c
    // (wiremock's auto server speaks h1 and h2c, as tests/execute_http2.rs relies on).
    let plain = MockServer::start().await;
    Mock::given(any())
        .respond_with(ResponseTemplate::new(200))
        .mount(&plain)
        .await;
    let res = exec
        .send(&resolved_one("GET", &plain.uri(), "", ""))
        .await
        .unwrap();
    assert_eq!(res.timeline()[0].version(), Some(HttpVersion::Http2));
}

#[tokio::test]
async fn a_query_document_keeps_the_http_post() {
    let server = MockServer::start().await;
    Mock::given(any())
        .respond_with(ResponseTemplate::new(200).set_body_string(r#"{"data":{}}"#))
        .mount(&server)
        .await;
    let url = format!("{}/graphql", server.uri());
    let req = graphql_request(ROOT, &url, "query { a }", "{}", "");
    let res = ws_exec(Auto).send(&req).await.unwrap();
    assert_eq!(
        (res.status(), res.timeline()[0].method.as_str()),
        (200, "POST")
    );
    assert_eq!(res.stream_end(), None);
}

#[tokio::test]
async fn a_subscription_beside_another_operation_fails_before_connecting() {
    let url = format!("http://127.0.0.1:{}/graphql", dead_port());
    let req = graphql_request(ROOT, &url, "subscription A { a } query B { b }", "{}", "");
    let err = ws_exec(Auto).send(&req).await.unwrap_err();
    assert!(
        matches!(&err, ExecError::InvalidRequest { message, .. }
            if message == "a subscription request must hold one operation"),
        "{err}"
    );
}

#[tokio::test]
async fn the_handshake_replaces_user_set_upgrade_headers() {
    let s = ws_server(WsScript::new(vec![tick(1), Step::Complete])).await;
    let item = format!(
        "info:\n  name: s\n  type: graphql\ngraphql:\n  method: POST\n  url: \"{}\"\n  headers:\n    - name: Connection\n      value: keep-alive\n    - name: Sec-WebSocket-Protocol\n      value: x\n  body:\n    query: {}\n",
        s.http("/"),
        serde_json::to_string(QUERY).expect("query"),
    );
    let (_dir, c, ctx) = setup(&[("opencollection.yml", ROOT), ("s.yml", &item)]);
    let req = at(&c, &ctx, "s.yml").expect("resolves");
    let res = ws_exec(Auto).send(&req).await.unwrap();
    assert_eq!(res.stream_end(), Some(&StreamEnd::Closed));
    let lines = s.seen.lines();
    let named =
        |prefix: &str| -> Vec<&String> { lines.iter().filter(|l| l.starts_with(prefix)).collect() };
    assert_eq!(named("connection: "), ["connection: Upgrade"], "{lines:?}");
    assert_eq!(
        named("sec-websocket-protocol: "),
        ["sec-websocket-protocol: x"],
        "{lines:?}"
    );
}
