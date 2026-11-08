//! A subscription's handshake routed like any request: CONNECT for wss, SOCKS for ws, the
//! refusal of ws through an HTTP proxy, and the cookie jar.

use crate::common::ws_server::{
    QUERY, Step, VARS, WsScript, graphql_request, sub, tick, ws_exec, ws_server, wss_server,
};
use crate::network_support::{custom_proxy, http_proxy, root_with, socks5_proxy};
use apinata_core::HttpVersionPolicy::Auto;
use apinata_core::{CookieInfo, CookieJar, ExecError, ExecOptions, Executor, StreamEnd};
use std::sync::Arc;

fn via(protocol: &str, port: u16) -> String {
    root_with(&custom_proxy(
        protocol,
        "127.0.0.1",
        &port.to_string(),
        None,
        "",
    ))
}

#[tokio::test]
async fn wss_tunnels_through_an_http_proxy_with_connect() {
    let s = wss_server(WsScript::new(vec![tick(1), Step::Complete])).await;
    let proxy = http_proxy(None).await;
    let req = graphql_request(
        &via("http", proxy.port),
        &s.https("/graphql"),
        QUERY,
        VARS,
        "",
    );
    let res = ws_exec(Auto).send(&req).await.unwrap();
    assert_eq!(res.stream_end(), Some(&StreamEnd::Closed));
    assert_eq!(res.events().len(), 2);
    assert_eq!(
        proxy.seen.lines(),
        vec![format!("CONNECT 127.0.0.1:{} HTTP/1.1", s.port)]
    );
}

#[tokio::test]
async fn ws_passes_through_socks5() {
    let s = ws_server(WsScript::new(vec![tick(1), Step::Complete])).await;
    let proxy = socks5_proxy(None).await;
    let req = graphql_request(
        &via("socks5", proxy.port),
        &s.http("/graphql"),
        QUERY,
        VARS,
        "",
    );
    let res = ws_exec(Auto).send(&req).await.unwrap();
    assert_eq!(res.stream_end(), Some(&StreamEnd::Closed));
    assert_eq!(res.events().len(), 2);
    assert_eq!(
        proxy.seen.lines(),
        vec![format!("connect 127.0.0.1:{}", s.port)]
    );
}

#[tokio::test]
async fn ws_through_an_http_proxy_is_refused_before_connecting() {
    let s = ws_server(WsScript::new(vec![Step::Complete])).await;
    let proxy = http_proxy(None).await;
    let req = graphql_request(
        &via("http", proxy.port),
        &s.http("/graphql"),
        QUERY,
        VARS,
        "",
    );
    let err = ws_exec(Auto).send(&req).await.unwrap_err();
    assert!(
        matches!(&err, ExecError::InvalidRequest { message, .. }
            if message == "a ws:// subscription cannot pass through an HTTP proxy; \
                           use wss:// or bypass the proxy for this host"),
        "{err}"
    );
    assert!(proxy.seen.lines().is_empty());
    assert!(s.seen.lines().is_empty());
}

#[tokio::test]
async fn the_jar_cookie_reaches_the_handshake() {
    let s = ws_server(WsScript::new(vec![Step::Complete])).await;
    let jar = Arc::new(CookieJar::default());
    jar.insert(CookieInfo::new("127.0.0.1", "sid", "s1"))
        .unwrap();
    let exec = Executor::new(ExecOptions::new().cookie_jar(jar)).unwrap();
    let res = exec.send(&sub(&s.http("/graphql"))).await.unwrap();
    assert_eq!(res.stream_end(), Some(&StreamEnd::Closed));
    let lines = s.seen.lines();
    assert!(lines.contains(&"cookie: sid=s1".to_owned()), "{lines:?}");
}
