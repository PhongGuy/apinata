//! NTLM under the transport policies: HTTP version, host policy, timeout, cancel.

use crate::common;
use crate::ntlm_support;

use apinata_core::{
    CancelHandle, ExecError, ExecOptions, Executor, HttpVersion, HttpVersionPolicy,
};
use common::resolved_one;
use ntlm_support::{Options, PASSWORD, USER, ntlm_server};
use std::sync::Arc;
use std::time::Duration;

fn auth() -> String {
    format!(
        "  auth:\n    type: ntlm\n    username: {USER}\n    password: {PASSWORD}\n    domain: CORP\n"
    )
}

#[tokio::test]
async fn prior_knowledge_still_negotiates_over_http_1_1() {
    let s = ntlm_server(Options::default()).await;
    let exec =
        Executor::new(ExecOptions::new().http_version(HttpVersionPolicy::Http2PriorKnowledge))
            .unwrap();
    let res = exec
        .send(&resolved_one(
            "GET",
            &format!("{}/ok", s.origin()),
            &auth(),
            "",
        ))
        .await
        .expect("sent");
    assert_eq!(res.status(), 200);
    assert_eq!(res.timeline()[0].version(), Some(HttpVersion::Http11));
    assert_eq!(s.connections(), 1);
}

#[tokio::test]
async fn a_host_policy_still_allows_the_pinned_handshake() {
    let s = ntlm_server(Options::default()).await;
    let exec = Executor::new(ExecOptions::new().host_policy(Arc::new(|_: &str| false))).unwrap();
    let res = exec
        .send(&resolved_one(
            "GET",
            &format!("{}/ok", s.origin()),
            &auth(),
            "",
        ))
        .await
        .expect("sent");
    assert_eq!(res.status(), 200);
    assert_eq!(s.connections(), 1);
}

#[tokio::test]
async fn tls_origins_handshake_too() {
    let s = ntlm_server(Options {
        tls: true,
        ..Options::default()
    })
    .await;
    let exec = Executor::new(ExecOptions::new().insecure(true)).unwrap();
    let res = exec
        .send(&resolved_one(
            "GET",
            &format!("{}/ok", s.origin()),
            &auth(),
            "",
        ))
        .await
        .expect("sent");
    assert_eq!((res.status(), res.text()), (200, "GET 0"));
    assert_eq!(s.connections(), 1);
}

#[tokio::test]
async fn a_cancel_during_the_handshake_ends_the_send() {
    // A server that accepts and never answers holds the probe open.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        let _held = listener.accept().await;
        std::future::pending::<()>().await;
    });
    let cancel = CancelHandle::new();
    let trigger = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(200)).await;
        trigger.cancel();
    });
    let exec = Executor::new(ExecOptions::new()).unwrap();
    let req = resolved_one("GET", &format!("http://127.0.0.1:{port}/ok"), &auth(), "");
    let err = exec.send_with(&req, &cancel).await.expect_err("cancelled");
    assert!(matches!(err, ExecError::Cancelled), "{err:?}");
}

#[tokio::test]
async fn the_timeout_applies_to_each_leg() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        let _held = listener.accept().await;
        std::future::pending::<()>().await;
    });
    let exec = Executor::new(ExecOptions::new()).unwrap();
    let req = resolved_one(
        "GET",
        &format!("http://127.0.0.1:{port}/ok"),
        &auth(),
        "settings:\n  timeout: 300\n",
    );
    let err = exec.send(&req).await.expect_err("timed out");
    assert!(matches!(err, ExecError::Timeout { .. }), "{err:?}");
}
