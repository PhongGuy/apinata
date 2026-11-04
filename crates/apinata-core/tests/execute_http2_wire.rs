//! An h2 hop records the headers that went over the wire, keeps secrets redacted in the
//! session log, and works under a host policy.

use crate::common;
use crate::session_log_support;

use apinata_core::{
    Collection, ContextOptions, ExecOptions, Executor, HttpVersion, HttpVersionPolicy,
    ResolvedRequest, VarContext,
};
use common::{at, resolved_one, write_tree};
use session_log_support::{assert_absent, header, logged, only};
use std::sync::Arc;
use wiremock::matchers::any;
use wiremock::{Mock, MockServer, ResponseTemplate};

const ROOT: &str = "opencollection: \"1.0.0\"\ninfo:\n  name: t\n";
const SECRET: &str = "H2s3cr/et-55";
/// Connection-specific headers, plus `x-drop` named by `Connection`, plus one to keep.
const WIRE: &str = "  headers:\n    - name: Connection\n      value: \"X-Drop, Keep-Alive\"\n    - name: X-Drop\n      value: \"1\"\n    - name: Keep-Alive\n      value: \"timeout=5\"\n    - name: Proxy-Connection\n      value: keep-alive\n    - name: Upgrade\n      value: websocket\n    - name: TE\n      value: gzip\n    - name: X-Keep\n      value: \"1\"\n";
const STRIPPED: [&str; 6] = [
    "connection",
    "x-drop",
    "keep-alive",
    "proxy-connection",
    "upgrade",
    "te",
];

async fn ok_server() -> MockServer {
    let s = MockServer::start().await;
    Mock::given(any())
        .respond_with(ResponseTemplate::new(200))
        .mount(&s)
        .await;
    s
}

fn prior_knowledge() -> ExecOptions {
    ExecOptions::new().http_version(HttpVersionPolicy::Http2PriorKnowledge)
}

fn names(headers: &[(String, String)]) -> Vec<&str> {
    headers.iter().map(|(n, _)| n.as_str()).collect()
}

/// `r.yml` with `under_http` under `http:`, `tok` = `secret` given as a sensitive override.
fn resolved_secret(url: &str, under_http: &str, secret: &str) -> ResolvedRequest {
    let text = format!(
        "info:\n  name: r\n  type: http\nhttp:\n  method: GET\n  url: \"{url}\"\n{under_http}"
    );
    let dir = write_tree(&[("opencollection.yml", ROOT), ("r.yml", &text)]);
    let c = Collection::open(dir.path()).expect("opens");
    let ctx = VarContext::new(&c, ContextOptions::new().sensitive_env_var("tok", secret))
        .expect("context");
    at(&c, &ctx, "r.yml").expect("resolves")
}

#[tokio::test]
async fn an_h2_hop_records_only_the_headers_that_went_over_the_wire() {
    let s = ok_server().await;
    let exec = Executor::new(prior_knowledge()).expect("executor");
    let res = exec
        .send(&resolved_one("GET", &format!("{}/", s.uri()), WIRE, ""))
        .await
        .expect("sent");
    let hop = &res.timeline()[0];
    assert_eq!(hop.version(), Some(HttpVersion::Http2));
    let recorded = names(&hop.request_headers);
    for name in STRIPPED {
        assert!(!recorded.contains(&name), "{name} recorded: {recorded:?}");
    }
    assert!(recorded.contains(&"x-keep") && recorded.contains(&"user-agent"));
    let received = &s.received_requests().await.expect("recorded")[0];
    for name in STRIPPED {
        assert!(
            !received.headers.contains_key(name),
            "{name} reached the server"
        );
    }
    assert!(received.headers.contains_key("x-keep"));
}

#[tokio::test]
async fn te_trailers_survives_an_h2_hop() {
    let s = ok_server().await;
    let exec = Executor::new(prior_knowledge()).expect("executor");
    let under = "  headers:\n    - name: TE\n      value: trailers\n";
    let res = exec
        .send(&resolved_one("GET", &format!("{}/", s.uri()), under, ""))
        .await
        .expect("sent");
    assert_eq!(
        header(&res.timeline()[0].request_headers, "te"),
        Some("trailers")
    );
    let received = &s.received_requests().await.expect("recorded")[0];
    assert_eq!(
        received.headers.get("te").and_then(|v| v.to_str().ok()),
        Some("trailers")
    );
}

#[tokio::test]
async fn an_http1_hop_records_its_headers_as_before() {
    let s = ok_server().await;
    let under = "  headers:\n    - name: Connection\n      value: X-Drop\n    - name: X-Drop\n      value: \"1\"\n    - name: Keep-Alive\n      value: \"timeout=5\"\n    - name: TE\n      value: gzip\n";
    let res = Executor::new(ExecOptions::new())
        .expect("executor")
        .send(&resolved_one("GET", &format!("{}/", s.uri()), under, ""))
        .await
        .expect("sent");
    let hop = &res.timeline()[0];
    assert_eq!(hop.version(), Some(HttpVersion::Http11));
    let recorded = names(&hop.request_headers);
    for name in ["connection", "x-drop", "keep-alive", "te"] {
        assert!(recorded.contains(&name), "{name} missing: {recorded:?}");
    }
}

#[tokio::test]
async fn a_secret_header_on_an_h2_hop_is_redacted_in_the_session_log() {
    let s = MockServer::start().await;
    Mock::given(any())
        .respond_with(ResponseTemplate::new(200).insert_header("x-echo", SECRET))
        .mount(&s)
        .await;
    let (exec, log) = logged(prior_knowledge());
    let under = "  headers:\n    - name: X-Key\n      value: \"{{tok}}\"\n";
    exec.send(&resolved_secret(&format!("{}/", s.uri()), under, SECRET))
        .await
        .expect("sent");
    assert_absent(&log, SECRET);
    let e = only(&log);
    assert_eq!(e.hops[0].version(), Some(HttpVersion::Http2));
    assert_eq!(
        header(&e.hops[0].request_headers, "x-key"),
        Some("[redacted]")
    );
    assert_eq!(
        header(&e.hops[0].response_headers, "x-echo"),
        Some("[redacted]")
    );
}

#[tokio::test]
async fn h2c_works_under_a_host_policy() {
    let s = ok_server().await;
    let exec =
        Executor::new(prior_knowledge().host_policy(Arc::new(|_: &str| false))).expect("executor");
    let url = format!("{}/", s.uri());
    for _ in 0..2 {
        let res = exec
            .send(&resolved_one("GET", &url, "", ""))
            .await
            .expect("sent");
        assert_eq!(res.status(), 200);
        assert_eq!(res.timeline()[0].version(), Some(HttpVersion::Http2));
    }
    assert_eq!(s.received_requests().await.expect("recorded").len(), 2);
}
