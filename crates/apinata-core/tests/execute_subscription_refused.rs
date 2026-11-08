//! Handshakes that do not open a subscription: other statuses, a wrong accept and the
//! server's choice of subprotocol.

use crate::common::ROOT;
use crate::common::raw_server::raw_server;
use crate::common::ws_server::{
    QUERY, Step, VARS, WsScript, graphql_request, sub, ws_exec, ws_server,
};
use apinata_core::HttpVersionPolicy::Auto;
use apinata_core::{ExecError, StreamEnd};

fn failed(port: u16, why: &str) -> Option<StreamEnd> {
    Some(StreamEnd::Error(format!(
        "WebSocket handshake to ws://127.0.0.1:{port}/graphql failed: {why}"
    )))
}

#[tokio::test]
async fn a_refused_handshake_keeps_the_reply_and_names_the_ws_url() {
    let port = raw_server(
        "HTTP/1.1 404 Not Found\r\ncontent-type: text/html\r\ncontent-length: 13\r\n\r\n<h1>nope</h1>",
    )
    .await;
    let url = format!("http://127.0.0.1:{port}/graphql");
    let res = ws_exec(Auto).send(&sub(&url)).await.unwrap();
    assert_eq!((res.status(), res.text()), (404, "<h1>nope</h1>"));
    assert_eq!(res.stream_end().cloned(), failed(port, "404 Not Found"));
}

#[tokio::test]
async fn a_redirect_is_not_followed_and_names_its_location() {
    let port =
        raw_server("HTTP/1.1 302 Found\r\nlocation: /elsewhere\r\ncontent-length: 0\r\n\r\n").await;
    let url = format!("http://127.0.0.1:{port}/graphql");
    let res = ws_exec(Auto).send(&sub(&url)).await.unwrap();
    assert_eq!(res.status(), 302);
    assert_eq!(res.timeline().len(), 1);
    assert_eq!(
        res.stream_end().cloned(),
        failed(port, "302 Found (Location: /elsewhere)")
    );
}

#[tokio::test]
async fn a_redirect_location_is_redacted_like_any_url() {
    let port = raw_server(
        "HTTP/1.1 302 Found\r\nlocation: https://user:secret@host/x?token=t0k\r\ncontent-length: 0\r\n\r\n",
    )
    .await;
    let url = format!("http://127.0.0.1:{port}/graphql");
    let res = ws_exec(Auto).send(&sub(&url)).await.unwrap();
    assert_eq!(
        res.stream_end().cloned(),
        failed(
            port,
            "302 Found (Location: https://<redacted>@host/x?token=<redacted>)"
        )
    );
}

#[tokio::test]
async fn a_wrong_accept_is_refused() {
    let port = raw_server(
        "HTTP/1.1 101 Switching Protocols\r\nupgrade: websocket\r\nconnection: Upgrade\r\nsec-websocket-accept: bm90LXRoZS1yaWdodC1rZXk=\r\nsec-websocket-protocol: graphql-transport-ws\r\n\r\n",
    )
    .await;
    let url = format!("http://127.0.0.1:{port}/graphql");
    let res = ws_exec(Auto).send(&sub(&url)).await.unwrap();
    assert_eq!(res.status(), 101);
    assert!(res.body().is_empty());
    assert_eq!(
        res.stream_end().cloned(),
        failed(port, "Sec-WebSocket-Accept does not match")
    );
}

#[tokio::test]
async fn the_reply_must_choose_a_known_subprotocol() {
    for (chosen, why) in [
        (None, "the server chose no subprotocol".to_owned()),
        (
            Some("weird"),
            "the server chose subprotocol \"weird\"".to_owned(),
        ),
    ] {
        let s = ws_server(WsScript {
            protocol: chosen,
            ..WsScript::new(vec![Step::Complete])
        })
        .await;
        let res = ws_exec(Auto).send(&sub(&s.http("/graphql"))).await.unwrap();
        assert_eq!(res.status(), 101, "{chosen:?}");
        assert!(res.body().is_empty(), "{chosen:?}");
        assert_eq!(
            res.stream_end().cloned(),
            failed(s.port, &why),
            "{chosen:?}"
        );
        assert!(
            !s.seen.lines().iter().any(|l| l.starts_with("recv: ")),
            "{chosen:?}"
        );
    }
}

#[tokio::test]
async fn ntlm_auth_is_refused_before_connecting() {
    let s = ws_server(WsScript::new(vec![Step::Complete])).await;
    let ntlm = "  auth:\n    type: ntlm\n    username: u\n    password: p\n    domain: D\n";
    let req = graphql_request(ROOT, &s.http("/graphql"), QUERY, VARS, ntlm);
    let err = ws_exec(Auto).send(&req).await.unwrap_err();
    assert!(
        matches!(&err, ExecError::InvalidRequest { message, .. }
            if message == "NTLM is not supported for subscriptions"),
        "{err}"
    );
    assert!(s.seen.lines().is_empty());
}
