//! One network entry per send: hops, bodies, failures and cancels.

use crate::common;
use crate::session_log_support;

use crate::common::dead_port;
use apinata_core::{CancelHandle, ExecError, ExecOptions, SendSource};
use common::resolved_one;
use session_log_support::{entries, header, logged, only};
use std::sync::Arc;
use std::time::Duration;
use wiremock::matchers::{path, path_regex};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

#[tokio::test]
async fn a_redirected_send_is_one_entry_with_both_hops() {
    let s = MockServer::start().await;
    Mock::given(path("/a"))
        .respond_with(ResponseTemplate::new(302).insert_header("location", "/b"))
        .mount(&s)
        .await;
    Mock::given(path("/b"))
        .respond_with(ResponseTemplate::new(200).set_body_string("done"))
        .mount(&s)
        .await;
    let (exec, log) = logged(ExecOptions::new());
    let url = format!("{}/a", s.uri());
    let req = resolved_one(
        "GET",
        &url,
        "  headers:\n    - name: X-A\n      value: one\n",
        "",
    );
    exec.send(&req).await.expect("sent");
    let e = only(&log);
    assert_eq!(e.source, SendSource::Request);
    assert_eq!(
        (
            e.method.as_str(),
            e.url.as_str(),
            e.status,
            e.error.as_deref()
        ),
        ("GET", url.as_str(), Some(200), None)
    );
    assert_eq!(
        e.hops.iter().map(|h| h.status).collect::<Vec<_>>(),
        [302, 200]
    );
    assert!(e.hops[1].url.ends_with("/b"), "{}", e.hops[1].url);
    assert_eq!(header(&e.request_headers, "x-a"), Some("one"));
    assert_eq!(
        (
            e.response_body.text.as_str(),
            e.response_body.size,
            e.response_body.truncated
        ),
        ("done", 4, false)
    );
    assert_eq!(e.hops_omitted, 0);
    assert!(e.duration >= e.hops[0].elapsed);
}

#[tokio::test]
async fn the_request_body_is_what_the_first_hop_sent() {
    let s = MockServer::start().await;
    Mock::given(path("/p"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&s)
        .await;
    let (exec, log) = logged(ExecOptions::new());
    let body = "  body:\n    type: json\n    data: '{\"n\":1}'\n";
    exec.send(&resolved_one("POST", &format!("{}/p", s.uri()), body, ""))
        .await
        .expect("sent");
    let e = only(&log);
    assert_eq!(
        (e.request_body.text.as_str(), e.request_body.size),
        ("{\"n\":1}", 7)
    );
}

#[tokio::test]
async fn a_refused_connection_logs_its_error_and_no_status() {
    let port = dead_port();
    let (exec, log) = logged(ExecOptions::new());
    let err = exec
        .send(&resolved_one(
            "GET",
            &format!("http://127.0.0.1:{port}/x"),
            "",
            "",
        ))
        .await
        .expect_err("refused");
    let e = only(&log);
    assert_eq!(e.status, None);
    assert_eq!(e.error.as_deref(), Some(err.to_string().as_str()));
    assert!(e.hops.is_empty());
}

#[tokio::test]
async fn a_refusal_before_any_io_logs_the_error_and_no_hops() {
    // Loopback passes any policy, so a TEST-NET address is refused before any I/O.
    let deny: Arc<dyn Fn(&str) -> bool + Send + Sync> = Arc::new(|_| false);
    let (exec, log) = logged(ExecOptions::new().host_policy(deny));
    let err = exec
        .send(&resolved_one("GET", "http://192.0.2.1/x", "", ""))
        .await
        .expect_err("refused");
    assert!(matches!(err, ExecError::HostNotAllowed { .. }), "{err:?}");
    let e = only(&log);
    assert_eq!(
        (e.status, e.error.as_deref()),
        (None, Some(err.to_string().as_str()))
    );
    assert!(e.hops.is_empty());
}

#[tokio::test]
async fn a_cancel_mid_send_logs_cancelled_with_the_completed_hop() {
    let s = MockServer::start().await;
    Mock::given(path("/slow"))
        .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_secs(5)))
        .mount(&s)
        .await;
    Mock::given(path("/login"))
        .respond_with(ResponseTemplate::new(302).insert_header("location", "/slow"))
        .mount(&s)
        .await;
    let (exec, log) = logged(ExecOptions::new());
    let req = resolved_one("GET", &format!("{}/login", s.uri()), "", "");
    let cancel = CancelHandle::new();
    let cancel_on_second = async {
        while s.received_requests().await.expect("recorded").len() < 2 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        cancel.cancel();
    };
    let (result, ()) = tokio::join!(exec.send_with(&req, &cancel), cancel_on_second);
    assert!(matches!(result, Err(ExecError::Cancelled)), "{result:?}");
    let e = only(&log);
    assert_eq!((e.status, e.error.as_deref()), (None, Some("cancelled")));
    assert_eq!(e.hops.iter().map(|h| h.status).collect::<Vec<_>>(), [302]);
}

/// `/h/{n}` redirects to `/h/{n+1}` below `last`, then answers 200.
struct Chain {
    last: u32,
}

impl Respond for Chain {
    fn respond(&self, req: &Request) -> ResponseTemplate {
        let n: u32 = req
            .url
            .path()
            .trim_start_matches("/h/")
            .parse()
            .unwrap_or(0);
        if n < self.last {
            ResponseTemplate::new(302).insert_header("location", format!("/h/{}", n + 1))
        } else {
            ResponseTemplate::new(200)
        }
    }
}

#[tokio::test]
async fn a_chain_past_thirty_two_hops_keeps_the_first_thirty_one_and_the_last() {
    let s = MockServer::start().await;
    Mock::given(path_regex("^/h/"))
        .respond_with(Chain { last: 40 })
        .mount(&s)
        .await;
    let (exec, log) = logged(ExecOptions::new());
    let req = resolved_one(
        "GET",
        &format!("{}/h/0", s.uri()),
        "",
        "settings:\n  maxRedirects: 40\n",
    );
    exec.send(&req).await.expect("sent");
    let e = only(&log);
    assert_eq!((e.hops.len(), e.hops_omitted, e.status), (32, 9, Some(200)));
    assert!(e.hops[30].url.ends_with("/h/30"), "{}", e.hops[30].url);
    assert!(e.hops[31].url.ends_with("/h/40"), "{}", e.hops[31].url);
}

#[tokio::test]
async fn a_body_that_is_not_utf8_is_logged_lossily() {
    let s = MockServer::start().await;
    Mock::given(path("/bin"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![0xff, 0xfe, b'a']))
        .mount(&s)
        .await;
    let (exec, log) = logged(ExecOptions::new());
    exec.send(&resolved_one("GET", &format!("{}/bin", s.uri()), "", ""))
        .await
        .expect("sent");
    let e = only(&log);
    assert_eq!(
        (e.response_body.text.as_str(), e.response_body.size),
        ("\u{FFFD}\u{FFFD}a", 3)
    );
}

#[tokio::test]
async fn sends_number_entries_in_publication_order() {
    let s = MockServer::start().await;
    Mock::given(path("/ok"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&s)
        .await;
    let (exec, log) = logged(ExecOptions::new());
    let req = resolved_one("GET", &format!("{}/ok", s.uri()), "", "");
    exec.send(&req).await.expect("sent");
    exec.send_with(&req, &CancelHandle::new())
        .await
        .expect("sent");
    assert_eq!(
        entries(&log).iter().map(|e| e.seq).collect::<Vec<_>>(),
        [0, 1]
    );
}
